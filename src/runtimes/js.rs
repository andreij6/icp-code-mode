//! JavaScript via Boa. `icp.call` / `icp.candid` return pending Promises; the
//! host drains the job queue, dispatches every pending request concurrently
//! (so `Promise.all` becomes a true batch), settles the promises and repeats.

use crate::bridge::{HostBridge, MAX_LOG_BYTES, budget_error, check_budget, normalize_numbers, now_ms, truncate};
use boa_engine::builtins::promise::{PromiseState, ResolvingFunctions};
use boa_engine::context::time::FixedClock;
use boa_engine::context::{ContextBuilder, HostHooks};
use boa_engine::job::SimpleJobExecutor;
use boa_engine::module::IdleModuleLoader;
use boa_engine::object::ObjectInitializer;
use boa_engine::object::builtins::{JsFunction, JsPromise};
use boa_engine::property::Attribute;
use boa_engine::realm::Realm;
use boa_engine::{Context, JsError, JsNativeError, JsResult, JsString, JsValue, NativeFunction, Source, js_string};
use boa_gc::{Finalize, Gc, GcRefCell, Trace};
use serde_json::Value;
use std::rc::Rc;

pub const LOOP_ITERATION_LIMIT: u64 = 1_000_000;
pub const RECURSION_LIMIT: usize = 256;
pub const STACK_SIZE_LIMIT: usize = 10 * 1024;
/// Host round-trips (each one a batch of concurrent requests).
pub const MAX_ROUNDS: usize = 64;
/// Concurrent `icp.*` requests per round.
pub const MAX_PENDING: usize = 64;
/// Values crossing into Rust (results, call args, console.log) are snapshotted with
/// these bounds; Boa's own `to_json` / `JSON.stringify` recurse without a limit and
/// would overflow the wasm stack.
pub const MAX_VALUE_DEPTH: usize = crate::candid_bridge::MAX_DEPTH;
pub const MAX_VALUE_NODES: usize = 200_000;

/// Installed before user code, so it only holds pristine builtins. Reads every value
/// exactly once (getters cannot hand `to_json` a different, deeper value later), and
/// returns a plain copy: own data properties only, depth/size bounded, `toJSON`
/// honoured, functions/symbols dropped like `JSON.stringify` does. Also wraps
/// `JSON.stringify` with the same bounds.
const SNAPSHOT_JS: &str = r#"(() => {
  const keys = Object.keys, isArray = Array.isArray, def = Object.defineProperty, str = JSON.stringify;
  const MAXD = __MAXD__, MAXN = __MAXN__;
  const snapshot = (root) => {
    let nodes = 0;
    const put = (o, k, v) => def(o, k, { value: v, writable: true, enumerable: true, configurable: true });
    const walk = (x, d) => {
      const t = typeof x;
      if (t === "bigint") throw new TypeError("BigInt values cannot cross into the host; convert to string");
      if (t === "function" || t === "symbol") return undefined;
      if (x === null || t !== "object") return x;
      if (d >= MAXD) throw new RangeError("value nested deeper than " + MAXD + " levels");
      if (++nodes > MAXN) throw new RangeError("value has more than " + MAXN + " elements");
      if (typeof x.toJSON === "function") { const j = x.toJSON(); if (j !== x) return walk(j, d + 1); }
      if (isArray(x)) {
        const out = [], n = x.length;
        for (let i = 0; i < n; i++) { const v = walk(x[i], d + 1); put(out, i, v === undefined ? null : v); }
        return out;
      }
      const out = {}, ks = keys(x);
      for (let i = 0; i < ks.length; i++) { const v = walk(x[ks[i]], d + 1); if (v !== undefined) put(out, ks[i], v); }
      return out;
    };
    return walk(root, 0);
  };
  def(JSON, "stringify", { value: function stringify(v, r, s) { return str(snapshot(v), r, s); }, writable: true, configurable: true });
  return snapshot;
})()"#;

/// No local timezone, no `eval` / `new Function`.
struct Hooks;
impl HostHooks for Hooks {
    fn ensure_can_compile_strings(&self, _: Realm, _: &[JsString], _: &JsString, _: bool, _: &mut Context) -> JsResult<()> {
        Err(JsNativeError::eval().with_message("dynamic code evaluation is disabled").into())
    }
    fn local_timezone_offset_seconds(&self, _: i64) -> i32 {
        0
    }
}

#[derive(Trace, Finalize)]
struct Pending {
    /// `None` = icp.candid, `Some(method)` = icp.call
    method: Option<String>,
    canister: String,
    args_json: String,
    resolvers: ResolvingFunctions,
}

#[derive(Trace, Finalize)]
struct State {
    pending: Vec<Pending>,
    logs: Vec<String>,
    log_bytes: usize,
    principal: String,
    /// `instruction_counter()` value at which host functions start refusing work.
    deadline: u64,
    snapshot: Option<JsFunction>,
}

type Shared = Gc<GcRefCell<State>>;

fn arg_string(args: &[JsValue], i: usize, ctx: &mut Context) -> JsResult<String> {
    match args.get(i) {
        Some(v) if v.is_string() => Ok(v.to_string(ctx)?.to_std_string_escaped()),
        _ => Err(JsNativeError::typ().with_message(format!("argument {} must be a string", i + 1)).into()),
    }
}

fn budget(st: &Shared) -> JsResult<()> {
    let deadline = st.borrow().deadline;
    budget_error(deadline).map_err(|e| JsNativeError::range().with_message(e).into())
}

/// Bounded JS -> JSON conversion (see `SNAPSHOT_JS`).
fn to_json_safe(v: &JsValue, st: &Shared, ctx: &mut Context) -> JsResult<Value> {
    let snapshot = st.borrow().snapshot.clone();
    let plain = match snapshot {
        Some(f) => f.call(&JsValue::undefined(), std::slice::from_ref(v), ctx)?,
        None => return Err(JsNativeError::error().with_message("engine not initialised").into()),
    };
    Ok(normalize_numbers(plain.to_json(ctx)?.unwrap_or(Value::Null)))
}

fn display(v: &JsValue, st: &Shared, ctx: &mut Context) -> String {
    if v.is_string() || v.is_undefined() || v.is_symbol() || v.is_bigint() {
        return v.to_string(ctx).map(|s| s.to_std_string_escaped()).unwrap_or_default();
    }
    match to_json_safe(v, st, ctx) {
        Ok(j) => j.to_string(),
        Err(_) => "<unprintable value>".into(),
    }
}

fn console_fn(level: &'static str, state: &Shared) -> NativeFunction {
    NativeFunction::from_copy_closure_with_captures(
        move |_, args, st: &Shared, ctx| {
            budget(st)?;
            if st.borrow().log_bytes >= MAX_LOG_BYTES {
                return Ok(JsValue::undefined()); // cap reached: don't even format
            }
            let line = args.iter().map(|a| display(a, st, ctx)).collect::<Vec<_>>().join(" ");
            let mut line = if level == "log" { line } else { format!("[{level}] {line}") };
            let mut s = st.borrow_mut();
            if s.log_bytes + line.len() > MAX_LOG_BYTES {
                truncate(&mut line, MAX_LOG_BYTES - s.log_bytes);
                s.log_bytes = MAX_LOG_BYTES;
            } else {
                s.log_bytes += line.len();
            }
            s.logs.push(line);
            Ok(JsValue::undefined())
        },
        state.clone(),
    )
}

fn request_fn(is_call: bool, state: &Shared) -> NativeFunction {
    NativeFunction::from_copy_closure_with_captures(
        move |_, args, st: &Shared, ctx| {
            budget(st)?;
            if st.borrow().pending.len() >= MAX_PENDING {
                return Err(JsNativeError::range()
                    .with_message(format!("more than {MAX_PENDING} concurrent icp requests"))
                    .into());
            }
            let canister = arg_string(args, 0, ctx)?;
            let (method, args_json) = if is_call {
                let method = arg_string(args, 1, ctx)?;
                let a = to_json_safe(&args.get(2).cloned().unwrap_or_default(), st, ctx)?;
                (Some(method), a.to_string())
            } else {
                (None, String::new())
            };
            let (promise, resolvers) = JsPromise::new_pending(ctx);
            st.borrow_mut().pending.push(Pending { method, canister, args_json, resolvers });
            Ok(promise.into())
        },
        state.clone(),
    )
}

fn new_context(state: &Shared) -> JsResult<Context> {
    let mut ctx = ContextBuilder::new()
        // Required: the default StdClock calls Instant::now(), which traps on wasm32.
        .clock(Rc::new(FixedClock::from_millis(now_ms())))
        .host_hooks(Rc::new(Hooks))
        .module_loader(Rc::new(IdleModuleLoader))
        .job_executor(Rc::new(SimpleJobExecutor::new()))
        .build()?;
    let limits = ctx.runtime_limits_mut();
    limits.set_loop_iteration_limit(LOOP_ITERATION_LIMIT);
    limits.set_recursion_limit(RECURSION_LIMIT);
    limits.set_stack_size_limit(STACK_SIZE_LIMIT);
    limits.set_backtrace_limit(8);

    let principal = NativeFunction::from_copy_closure_with_captures(
        |_, _, st: &Shared, _| Ok(JsString::from(st.borrow().principal.as_str()).into()),
        state.clone(),
    );
    let icp = ObjectInitializer::new(&mut ctx)
        .function(request_fn(true, state), js_string!("call"), 3)
        .function(request_fn(false, state), js_string!("candid"), 1)
        .function(principal, js_string!("principal"), 0)
        .build();
    ctx.register_global_property(js_string!("icp"), icp, Attribute::READONLY | Attribute::NON_ENUMERABLE)?;

    let mut console = ObjectInitializer::new(&mut ctx);
    for level in ["log", "info", "warn", "error", "debug"] {
        console.function(console_fn(level, state), JsString::from(level), 0);
    }
    let console = console.build();
    ctx.register_global_property(js_string!("console"), console, Attribute::all())?;
    ctx.register_global_callable(js_string!("print"), 0, console_fn("log", state))?;

    // Determinism: no RNG, nothing that can wait on wall-clock timers.
    let math = ctx.global_object().get(js_string!("Math"), &mut ctx)?;
    if let Some(m) = math.as_object() {
        m.delete_property_or_throw(js_string!("random"), &mut ctx)?;
    }
    let global = ctx.global_object();
    for name in ["Atomics", "SharedArrayBuffer", "WeakRef", "FinalizationRegistry"] {
        global.delete_property_or_throw(JsString::from(name), &mut ctx)?;
    }

    let src = SNAPSHOT_JS.replace("__MAXD__", &MAX_VALUE_DEPTH.to_string()).replace("__MAXN__", &MAX_VALUE_NODES.to_string());
    let snapshot = ctx.eval(Source::from_bytes(src.as_bytes()))?;
    state.borrow_mut().snapshot = snapshot.as_object().and_then(JsFunction::from_object);
    Ok(ctx)
}

fn error_string(err: JsError, st: &Shared, ctx: &mut Context) -> String {
    if let Ok(native) = err.try_native(ctx) {
        // Host functions and the snapshot helper report positions that mean nothing to
        // the script author: " (native)" / " (unknown at :L:C)".
        let msg = native.to_string();
        return match msg.rfind(" (unknown at ").or_else(|| msg.rfind(" (native)")) {
            Some(i) if msg.ends_with(')') => msg[..i].to_string(),
            _ => msg,
        };
    }
    match err.as_opaque().map(|v| to_json_safe(v, st, ctx)) {
        Some(Ok(j)) => format!("Uncaught {j}"),
        Some(Err(_)) => "Uncaught <unserializable value>".into(),
        None => err.to_string(),
    }
}

pub async fn run<B: HostBridge>(code: &str, bridge: &B) -> Result<Value, String> {
    // Declared before the Context so it is dropped after it.
    let state: Shared = Gc::new(GcRefCell::new(State {
        pending: vec![],
        logs: vec![],
        log_bytes: 0,
        principal: bridge.principal(),
        deadline: bridge.instruction_deadline(),
        snapshot: None,
    }));
    let result = {
        let mut ctx = new_context(&state).map_err(|e| format!("engine init: {e}"))?;
        let out = drive(code, bridge, &state, &mut ctx).await;
        for line in std::mem::take(&mut state.borrow_mut().logs) {
            bridge.log(line);
        }
        out
    };
    drop(state);
    boa_gc::force_collect(); // the GC heap is thread-local and outlives the Context
    result
}

async fn drive<B: HostBridge>(code: &str, bridge: &B, st: &Shared, ctx: &mut Context) -> Result<Value, String> {
    // Leading newline keeps `//` comments on the last line from eating the wrapper.
    let wrapped = format!("(async () => {{\n{code}\n}})()");
    let v = ctx.eval(Source::from_bytes(wrapped.as_bytes())).map_err(|e| error_string(e, st, ctx))?;
    let promise = v
        .as_object()
        .and_then(|o| JsPromise::from_object(o).ok())
        .ok_or("script must not escape its async wrapper")?;

    for _ in 0..MAX_ROUNDS {
        ctx.run_jobs().map_err(|e| error_string(e, st, ctx))?;
        check_budget(bridge)?;
        match promise.state() {
            PromiseState::Fulfilled(v) => return to_json_safe(&v, st, ctx).map_err(|e| error_string(e, st, ctx)),
            PromiseState::Rejected(e) => return Err(error_string(JsError::from_opaque(e), st, ctx)),
            PromiseState::Pending => {}
        }
        // Never hold the state borrow while JS runs: resolvers re-enter native fns.
        let batch = std::mem::take(&mut st.borrow_mut().pending);
        if batch.is_empty() {
            // Boa drops uncatchable job errors (loop/recursion limits) after an await,
            // leaving the promise pending, so we cannot tell the two cases apart.
            return Err("script stopped before finishing: a runtime limit (loop iterations, recursion, stack) \
                        was hit after an await, or it awaits a promise that never settles"
                .into());
        }
        let replies = futures::future::join_all(batch.iter().map(|p| async move {
            match &p.method {
                None => bridge.candid(&p.canister).await.map(Value::String),
                Some(m) => {
                    let args: Value = serde_json::from_str(&p.args_json).unwrap_or(Value::Null);
                    bridge.call(&p.canister, m, args).await
                }
            }
        }))
        .await;
        for (p, reply) in batch.iter().zip(replies) {
            let (f, arg) = match reply {
                Ok(j) => (&p.resolvers.resolve, JsValue::from_json(&j, ctx).map_err(|e| error_string(e, st, ctx))?),
                Err(msg) => (&p.resolvers.reject, JsNativeError::error().with_message(msg).into_opaque(ctx).into()),
            };
            f.call(&JsValue::undefined(), &[arg], ctx).map_err(|e| error_string(e, st, ctx))?;
        }
    }
    Err(format!("script exceeded {MAX_ROUNDS} host round-trips; batch calls with Promise.all"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::mock::MockBridge;
    use futures::executor::block_on;
    use serde_json::json;

    fn js(code: &str) -> Result<Value, String> {
        block_on(run(code, &MockBridge::default()))
    }

    #[test]
    fn determinism_hardening() {
        assert_eq!(js("return typeof Math.random").unwrap(), json!("undefined"));
        assert_eq!(js("return Date.now()").unwrap(), json!(0));
        assert_eq!(js("return new Date().getTimezoneOffset()").unwrap(), json!(0));
        assert_eq!(js("return [typeof Atomics, typeof SharedArrayBuffer, typeof WeakRef].join()").unwrap(), json!("undefined,undefined,undefined"));
        assert!(js("return eval('1')").unwrap_err().contains("disabled"));
        assert!(js("return new Function('return 1')()").unwrap_err().contains("disabled"));
    }

    #[test]
    fn promise_all_batches_in_one_round() {
        let b = MockBridge::default();
        let code = "const ids = [1,2,3].map(i => icp.call('ryjl3-tyaaa-aaaaa-aaaba-cai', 'icrc1_balance_of', {owner: 'aaaaa-aa', i}));
                    const rs = await Promise.all(ids); return rs.map(r => r.balance);";
        assert_eq!(block_on(run(code, &b)).unwrap(), json!([100, 200, 300]));
        assert_eq!(b.calls.borrow().len(), 3);
    }

    #[test]
    fn values_and_errors() {
        assert_eq!(js("return {a: 1.5, b: [true, null], c: 'x', d: 2**31}").unwrap(), json!({ "a": 1.5, "b": [true, null], "c": "x", "d": 2147483648u64 }));
        assert_eq!(js("1 + 1").unwrap(), Value::Null); // no `return` => undefined => null
        assert!(js("throw new TypeError('boom')").unwrap_err().contains("TypeError: boom"));
        assert_eq!(js("throw {code: 7}").unwrap_err(), "Uncaught {\"code\":7}");
        assert!(js("return 1n").is_err()); // BigInt is not JSON
        assert!(js("await new Promise(() => {})").unwrap_err().contains("never settle"));
        assert!(js("icp.call(1, 2)").unwrap_err().contains("must be a string"));
        assert!(js("})(); (() => {").is_err());
    }

    #[test]
    fn deep_and_huge_values_are_rejected_not_overflowing() {
        // Each of these used to recurse unboundedly in Boa's to_json / JSON.stringify.
        let deep = "let o = 1; for (let i = 0; i < 100000; i++) o = [o];";
        assert!(js(&format!("{deep} return o")).unwrap_err().contains("nested deeper"));
        assert!(js(&format!("{deep} return JSON.stringify(o)")).unwrap_err().contains("nested deeper"));
        assert!(js(&format!("{deep} console.log(o); return 1")).is_ok());
        assert!(js(&format!("{deep} return await icp.call('ryjl3-tyaaa-aaaaa-aaaba-cai', 'icrc1_balance_of', o)")).unwrap_err().contains("nested deeper"));
        assert!(js(&format!("{deep} throw o")).unwrap_err().contains("unserializable"));
        // A getter cannot return a shallow value to the check and a deep one later.
        let sneaky = "let n = 0; const o = { get x() { n++; let v = 1; if (n > 1) for (let i = 0; i < 100000; i++) v = [v]; return v; } }; return o;";
        assert_eq!(js(sneaky).unwrap(), json!({ "x": 1 }));
        // JSON semantics preserved.
        assert_eq!(js("return JSON.stringify({a: [1, undefined, () => 1], d: new Date(0)})").unwrap(), json!("{\"a\":[1,null,null],\"d\":\"1970-01-01T00:00:00.000Z\"}"));
        assert!(js("return {n: 1n}").unwrap_err().contains("BigInt"));
    }

    #[test]
    fn big_js_integers_pass_as_nat() {
        let b = MockBridge::default();
        block_on(run("await icp.call('ryjl3-tyaaa-aaaaa-aaaba-cai', 'icrc1_balance_of', {owner: 'aaaaa-aa', n: 3000000000})", &b)).unwrap();
        assert_eq!(b.calls.borrow()[0].2["n"], json!(3_000_000_000u64));
    }

    #[test]
    fn console_output_is_capped_while_running() {
        let b = MockBridge::default();
        block_on(run("const s = 'x'.repeat(100000); for (let i = 0; i < 1000; i++) console.log(s); return 1", &b)).unwrap();
        let total: usize = b.logs.take().iter().map(|l| l.len()).sum();
        assert!(total <= MAX_LOG_BYTES + 64, "{total}");
    }
}
