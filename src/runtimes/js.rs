//! JavaScript via Boa. `icp.call` / `icp.candid` return pending Promises; the
//! host drains the job queue, dispatches every pending request concurrently
//! (so `Promise.all` becomes a true batch), settles the promises and repeats.

use crate::bridge::{HostBridge, MAX_LOG_BYTES, budget_error, check_budget, normalize_numbers, now_ms, truncate};
use boa_engine::builtins::promise::{PromiseState, ResolvingFunctions};
use boa_engine::context::time::FixedClock;
use boa_engine::context::{ContextBuilder, HostHooks};
use boa_engine::job::{GenericJob, Job, JobExecutor, PromiseJob};
use boa_engine::module::IdleModuleLoader;
use boa_engine::object::ObjectInitializer;
use boa_engine::object::builtins::{JsFunction, JsPromise};
use boa_engine::property::Attribute;
use boa_engine::realm::Realm;
use boa_engine::{Context, JsError, JsNativeError, JsResult, JsString, JsValue, NativeFunction, Source, js_string};
use boa_gc::{Finalize, Gc, GcRefCell, Trace};
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
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
pub const MAX_VALUE_NODES: usize = crate::bridge::MAX_VALUE_NODES;
pub const MAX_VALUE_BYTES: usize = crate::bridge::MAX_VALUE_BYTES;
/// Boa VM steps (bytecode instructions) per run, across every function, callback and
/// promise job. The loop and recursion limits count per call frame, so `f` calling `g`
/// in a loop, or a recursion tree, would otherwise run until the IC's 40B-instruction
/// trap. Measured on a local replica at about 330 instructions per step, so 30M steps
/// is about 10B, leaving room under 40B for steps several times heavier.
pub const MAX_VM_STEPS: usize = 30_000_000;
/// Longest string `padStart`/`padEnd`/`repeat` may build (Boa allocates it up front).
pub const MAX_STRING_LEN: usize = 1 << 24;
/// Promise jobs (microtasks) per script. A chain like `f = () => Promise.resolve().then(f)`
/// has no loop and no call depth, so the loop and recursion limits never see it.
pub const MAX_PROMISE_JOBS: u64 = 100_000;

/// Installed before user code, so it only holds pristine builtins. Reads every value
/// exactly once (getters cannot hand `to_json` a different, deeper value later), and
/// returns a plain copy: own data properties only, bounded in depth, element count
/// (every value counts, primitives included) and total text size, `toJSON` honoured,
/// functions/symbols dropped like `JSON.stringify` does. Also wraps `JSON.stringify`
/// with the same bounds, replacer included.
const SNAPSHOT_JS: &str = r#"(() => {
  const keys = Object.keys, isArray = Array.isArray, def = Object.defineProperty, str = JSON.stringify, apply = Reflect.apply;
  const MAXD = __MAXD__, MAXN = __MAXN__, MAXB = __MAXB__;
  // Pristine valueOf methods: each throws unless `this` really is that kind of box.
  const UNBOX = [Number.prototype.valueOf, String.prototype.valueOf, Boolean.prototype.valueOf, BigInt.prototype.valueOf];
  // `replacer`: optional JSON.stringify replacer function, applied here (inside the bounds)
  // in the order the spec gives: toJSON, then the replacer, then the type rules.
  const snapshot = (root, replacer) => {
    let nodes = 0, bytes = 0;
    const put = (o, k, v) => def(o, k, { value: v, writable: true, enumerable: true, configurable: true });
    const walk = (x, d, holder, key) => {
      let t = typeof x;
      if ((t === "object" && x !== null) || t === "bigint") {
        const tj = x.toJSON;
        if (typeof tj === "function") { x = apply(tj, x, [key]); t = typeof x; }
      }
      if (replacer) { x = apply(replacer, holder, [key, x]); t = typeof x; }
      if (t === "object" && x !== null) {
        // Boxed primitives (new Number(1), Object("s"), ...) serialise as their value.
        for (let j = 0; j < UNBOX.length; j++) {
          try { x = apply(UNBOX[j], x, []); t = typeof x; break; } catch (_) {}
        }
      }
      if (++nodes > MAXN) throw new RangeError("value has more than " + MAXN + " elements");
      if (t === "bigint") throw new TypeError("BigInt values cannot cross into the host; convert to string");
      if (t === "function" || t === "symbol") return undefined;
      if (t === "string") {
        bytes += x.length;
        if (bytes > MAXB) throw new RangeError("value holds more than " + MAXB + " bytes of text");
        return x;
      }
      if (x === null || t !== "object") return x;
      if (d >= MAXD) throw new RangeError("value nested deeper than " + MAXD + " levels");
      if (isArray(x)) {
        const out = [], n = x.length;
        for (let i = 0; i < n; i++) { const v = walk(x[i], d + 1, x, "" + i); put(out, i, v === undefined ? null : v); }
        return out;
      }
      const out = {}, ks = keys(x);
      for (let i = 0; i < ks.length; i++) {
        const k = ks[i];
        bytes += k.length;
        if (bytes > MAXB) throw new RangeError("value holds more than " + MAXB + " bytes of text");
        const v = walk(x[k], d + 1, x, k);
        if (v !== undefined) put(out, k, v);
      }
      return out;
    };
    return walk(root, 0, { "": root }, "");
  };
  // Native JSON.stringify recurses without a limit, so it only ever sees a bounded snapshot.
  // A function replacer runs inside the snapshot; an array replacer (a key filter) is safe
  // to pass through.
  def(JSON, "stringify", { value: function stringify(v, r, s) {
    const fn = typeof r === "function" ? r : undefined;
    return str(snapshot(v, fn), fn ? undefined : r, s);
  }, writable: true, configurable: true });
  // Boa 0.22 panics (a trap on the IC) on `"x".at(-1e19)`, and pads/repeats by allocating
  // the full length up front, up to 8 GB. Replace `at` and bound the length arguments.
  const SP = String.prototype, trunc = Math.trunc, Str = String, MAXLEN = __MAXLEN__;
  def(SP, "at", { value: function at(index) {
    if (this == null) throw new TypeError("String.prototype.at called on null or undefined");
    const s = Str(this), n = s.length;
    let k = trunc(+index) || 0;
    if (k < 0) k += n;
    return k < 0 || k >= n ? undefined : s[k];
  }, writable: true, configurable: true });
  const bounded = (name, length) => {
    const f = SP[name];
    def(SP, name, { value: { [name](a, b) {
      const n = +a;
      if (length(Str(this), n) > MAXLEN) throw new RangeError(name + ": result longer than " + MAXLEN + " characters");
      return apply(f, this, [n, b]);
    } }[name], writable: true, configurable: true });
  };
  bounded("padStart", (s, n) => n);
  bounded("padEnd", (s, n) => n);
  bounded("repeat", (s, n) => s.length * n);
  return snapshot;
})()"#;

/// Job queue with a ceiling. Boa's own executor runs promise jobs until none are left,
/// with no count and no budget check, so a self-rescheduling promise chain would spin
/// until the IC's instruction limit trapped the canister.
#[derive(Default)]
struct BoundedJobs {
    promise: RefCell<VecDeque<PromiseJob>>,
    generic: RefCell<VecDeque<GenericJob>>,
    ran: Cell<u64>,
    deadline: u64,
}

impl BoundedJobs {
    fn tick(&self) -> JsResult<()> {
        let n = self.ran.get() + 1;
        self.ran.set(n);
        if n > MAX_PROMISE_JOBS {
            return Err(JsNativeError::range().with_message(format!("script ran more than {MAX_PROMISE_JOBS} promise jobs")).into());
        }
        if n.is_multiple_of(16) {
            budget_error(self.deadline).map_err(|e| JsNativeError::range().with_message(e))?;
        }
        Ok(())
    }

    fn drain(&self, ctx: &mut Context) -> JsResult<()> {
        loop {
            let job = self.promise.borrow_mut().pop_front(); // borrow released before the job runs
            if let Some(job) = job {
                self.tick()?;
                job.call(ctx)?;
                continue;
            }
            let job = self.generic.borrow_mut().pop_front();
            match job {
                Some(job) => {
                    self.tick()?;
                    job.call(ctx)?;
                }
                None => return Ok(()),
            }
        }
    }
}

impl JobExecutor for BoundedJobs {
    fn enqueue_job(self: Rc<Self>, job: Job, _: &mut Context) {
        match job {
            Job::PromiseJob(p) => self.promise.borrow_mut().push_back(p),
            Job::GenericJob(g) => self.generic.borrow_mut().push_back(g),
            // Async, timer and finalization jobs: nothing in the sandbox can create them (no
            // host async functions, no timers; Atomics and FinalizationRegistry are removed).
            _ => {}
        }
    }

    fn run_jobs(self: Rc<Self>, ctx: &mut Context) -> JsResult<()> {
        let r = self.drain(ctx);
        if r.is_err() {
            self.promise.borrow_mut().clear();
            self.generic.borrow_mut().clear();
        }
        ctx.clear_kept_objects();
        r
    }
}

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
    /// Plain data, no GC references. Kept as a `Value`: a round trip through text would
    /// meet serde_json's 128-level parse limit and turn deep arguments into `null`.
    #[unsafe_ignore_trace]
    args: Value,
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

/// Longest canister id text accepted (a principal's text form is at most 63 characters).
const MAX_ID_CHARS: usize = 64;

/// String argument `i`, refused before it is copied out of the engine if longer than `max`.
fn arg_string(args: &[JsValue], i: usize, max: usize, _: &mut Context) -> JsResult<String> {
    match args.get(i).and_then(JsValue::as_string) {
        Some(s) if s.len() > max => {
            Err(JsNativeError::range().with_message(format!("argument {} is longer than {max} characters", i + 1)).into())
        }
        Some(s) => Ok(s.to_std_string_escaped()),
        None => Err(JsNativeError::typ().with_message(format!("argument {} must be a string", i + 1)).into()),
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
    let v = normalize_numbers(plain.to_json(ctx)?.unwrap_or(Value::Null));
    // The snapshot counts UTF-16 code units; the limit is in UTF-8 bytes, as elsewhere.
    match crate::bridge::text_bytes(&v) {
        n if n > MAX_VALUE_BYTES => {
            Err(JsNativeError::range().with_message(format!("value holds more than {MAX_VALUE_BYTES} bytes of text ({n})")).into())
        }
        _ => Ok(v),
    }
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
            let room = MAX_LOG_BYTES - st.borrow().log_bytes;
            let mut line = if level == "log" { String::new() } else { format!("[{level}] ") };
            for (i, a) in args.iter().enumerate() {
                if line.len() > room {
                    break; // the rest would be cut anyway: don't format it
                }
                if i > 0 {
                    line.push(' ');
                }
                line.push_str(&display(a, st, ctx));
            }
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
            let canister = arg_string(args, 0, MAX_ID_CHARS, ctx)?;
            let (method, args) = if is_call {
                let method = arg_string(args, 1, crate::bridge::MAX_METHOD_LEN, ctx)?;
                let a = to_json_safe(&args.get(2).cloned().unwrap_or_default(), st, ctx)?;
                (Some(method), a)
            } else {
                (None, Value::Null)
            };
            let (promise, resolvers) = JsPromise::new_pending(ctx);
            st.borrow_mut().pending.push(Pending { method, canister, args, resolvers });
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
        .job_executor(Rc::new(BoundedJobs { deadline: state.borrow().deadline, ..BoundedJobs::default() }))
        .instructions_remaining(MAX_VM_STEPS)
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
    // Boa's `Symbol.for` registry is process-wide and never shrinks: each run could add
    // keys that outlive it.
    let symbol = global.get(js_string!("Symbol"), &mut ctx)?;
    if let Some(sym) = symbol.as_object() {
        sym.delete_property_or_throw(js_string!("for"), &mut ctx)?;
        sym.delete_property_or_throw(js_string!("keyFor"), &mut ctx)?;
    }

    let src = SNAPSHOT_JS.replace("__MAXD__", &MAX_VALUE_DEPTH.to_string()).replace("__MAXN__", &MAX_VALUE_NODES.to_string())
        .replace("__MAXB__", &MAX_VALUE_BYTES.to_string()).replace("__MAXLEN__", &MAX_STRING_LEN.to_string());
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
    if err.to_string().contains("NoInstructionsRemain") {
        return format!("policy: script ran more than {MAX_VM_STEPS} VM steps");
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

    for round in 0..=MAX_ROUNDS {
        ctx.run_jobs().map_err(|e| error_string(e, st, ctx))?;
        check_budget(bridge)?;
        match promise.state() {
            PromiseState::Fulfilled(_) if !st.borrow().pending.is_empty() => {
                // Not sent: the script finished first. Say so instead of reporting success.
                let n = st.borrow().pending.len();
                return Err(format!("{n} icp request(s) were started but never awaited, so they were not sent"));
            }
            PromiseState::Fulfilled(v) => return to_json_safe(&v, st, ctx).map_err(|e| error_string(e, st, ctx)),
            PromiseState::Rejected(e) => return Err(error_string(JsError::from_opaque(e), st, ctx)),
            PromiseState::Pending => {}
        }
        if round == MAX_ROUNDS {
            // Refuse before sending, so this error never follows messages that went out.
            return Err(format!("script exceeded {MAX_ROUNDS} host round-trips; batch calls with Promise.all"));
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
                Some(m) => bridge.call(&p.canister, m, p.args.clone()).await,
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
    unreachable!("the loop returns on its last round")
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
    fn promise_chains_are_bounded() {
        let e = js("const f = () => Promise.resolve().then(f); return f();").unwrap_err();
        assert!(e.contains("promise jobs"), "{e}");
        // Ordinary async code is unaffected.
        assert_eq!(js("let n = 0; for (let i = 0; i < 2000; i++) n += await Promise.resolve(1); return n;").unwrap(), json!(2000));
    }

    #[test]
    fn stringify_replacer_and_size_limits() {
        // A replacer cannot smuggle an unbounded value into the native serializer.
        let deep = "let d = 1; for (let i = 0; i < 100000; i++) d = [d];";
        assert!(js(&format!("{deep} return JSON.stringify({{a: 1}}, (k, v) => k === 'a' ? d : v)")).unwrap_err().contains("nested deeper"));
        // Replacer semantics are preserved: functions, key-filter arrays, `this`, toJSON-then-replacer.
        assert_eq!(js("return JSON.stringify({a: 1, b: 2, c: [1, 2]}, (k, v) => k === 'b' ? undefined : v)").unwrap(), json!("{\"a\":1,\"c\":[1,2]}"));
        assert_eq!(js("return JSON.stringify({a: 1, b: 2}, ['a'])").unwrap(), json!("{\"a\":1}"));
        assert_eq!(js("return JSON.stringify({n: 5n}, (k, v) => typeof v === 'bigint' ? v.toString() : v)").unwrap(), json!("{\"n\":\"5\"}"));
        assert_eq!(js("return JSON.stringify({a: 1}, null, 1)").unwrap(), json!("{\n \"a\": 1\n}"));
        // Every element counts, primitives included.
        assert!(js("return new Array(900000).fill(0)").unwrap_err().contains("elements"));
        assert_eq!(js("return new Array(1000).fill(0).length").unwrap(), json!(1000));
        // One big string referenced many times is small in the engine, huge when copied out.
        assert!(js("const s = 'x'.repeat(1000000); return new Array(300).fill(s)").unwrap_err().contains("bytes of text"));
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

    #[test]
    fn snapshot_counts_object_key_bytes_against_limit() {
        let err = js(
            "const o = {}; const k = 'k'.repeat(60000); \
             for (let i = 0; i < 100; i++) o[i + k] = 1; \
             return o;",
        )
        .unwrap_err();
        assert!(err.contains("bytes of text"), "{err}");
    }

    #[test]
    fn audit_round_two_js_fixes() {
        const L: &str = "ryjl3-tyaaa-aaaaa-aaaba-cai";
        // A request the script never awaited was never sent: say so instead of "ok".
        let e = js(&format!("icp.call('{L}', 'icrc1_decimals', null); return 1")).unwrap_err();
        assert!(e.contains("never awaited"), "{e}");
        // Work spread over many calls is stopped by the VM step cap, uncatchably.
        let e = js("function f() { for (let i = 0; i < 900000; i++) {} } \
                    try { for (let j = 0; j < 1000; j++) f(); } catch (_) {} return 1")
            .unwrap_err();
        assert!(e.contains("VM steps"), "{e}");
        // Boxed primitives serialise as their values.
        assert_eq!(js("return [new Number(1), Object('s'), new Boolean(false)]").unwrap(), json!([1, "s", false]));
        // The process-wide Symbol registry is gone.
        assert_eq!(js("return [typeof Symbol.for, typeof Symbol.keyFor]").unwrap(), json!(["undefined", "undefined"]));
        // Over-long ids and method names are refused before they are copied out.
        let e = js("return await icp.call('x'.repeat(100000), 'm', null)").unwrap_err();
        assert!(e.contains("longer than"), "{e}");
        // Text is measured in UTF-8 bytes, like the other runtimes: 1.5M three-byte chars.
        let e = js("const r = '\\u20ac'.repeat(750000); return r + r").unwrap_err();
        assert!(e.contains("bytes of text"), "{e}");
        // Arguments 127 levels deep are sent as they are, not replaced by null.
        let b = MockBridge::default();
        let deep = "let a = 1; for (let i = 0; i < 126; i++) a = [a]; return await icp.call('ryjl3-tyaaa-aaaaa-aaaba-cai', 'echo', a)";
        assert!(block_on(run(deep, &b)).is_ok());
        assert!(b.calls.borrow().iter().all(|c| !c.2.is_null()), "{:?}", b.calls.borrow().len());
    }

    #[test]
    fn string_at_negative_overflow_returns_undefined_without_panicking() {
        assert_eq!(js("return 'x'.at(-1e19) ?? null").unwrap(), json!(null));
        assert_eq!(js("return ['abc'.at(-1), 'abc'.at(0), 'abc'.at(1.7), 'abc'.at(NaN), 'abc'.at(3) ?? null]").unwrap(), json!(["c", "a", "b", "a", null]));
        assert_eq!(js("return ['ab'.padStart(4, '-'), 'ab'.padEnd(3), 'ab'.repeat(2)]").unwrap(), json!(["--ab", "ab ", "abab"]));
        for code in ["'x'.padStart(2 ** 31)", "'x'.padEnd(1e300)", "'xy'.repeat(2 ** 30)"] {
            let e = js(&format!("return {code}")).unwrap_err();
            assert!(e.contains("RangeError"), "{code}: {e}");
        }
    }

    #[test]
    fn pending_and_round_limits_are_enforced() {
        let e = js(
            "const a = []; for (let i = 0; i < 65; i++) a.push(icp.call('ryjl3-tyaaa-aaaaa-aaaba-cai', 'm', null)); \
             return await Promise.all(a);",
        )
        .unwrap_err();
        assert!(e.contains("concurrent icp requests"), "{e}");

        let rounds = js(
            "let x = 0; for (let i = 0; i < 65; i++) x = (await icp.call('ryjl3-tyaaa-aaaaa-aaaba-cai', 'm', null)).balance; \
             return x;",
        )
        .unwrap_err();
        assert!(rounds.contains("host round-trips"), "{rounds}");

        let err = js("throw new Error('plain');").unwrap_err();
        assert!(err.contains("Error: plain"), "{err}");
    }
}
