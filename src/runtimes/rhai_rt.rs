//! Rhai: ultra-cheap deterministic rule scripts.
//!
//! Rhai native functions are synchronous, but canister calls are async. We use
//! deterministic replay: when a script reaches an `icp::*` request with no
//! cached answer, it is terminated (uncatchably), the host performs the request,
//! caches the answer by sequence number, and re-runs the script from the top
//! with answers served from the cache. Scripts are deterministic, so every
//! replay takes the same path; a divergence is reported as an error.
//!
//! Note: `call` is a reserved Rhai keyword, so the call function is named
//! `icp::call_canister(...)`.

use crate::bridge::{HostBridge, ValueBudget, budget_error, container_depth, float_json, log_line};
use rhai::packages::{Package, StandardPackage};
use rhai::{Dynamic, Engine, EvalAltResult, FuncRegistration, ImmutableString, Module, NativeCallContext, Position};
use serde_json::Value;
use std::sync::{Arc, Mutex};

/// Operations per run (each replay is a run).
pub const MAX_OPERATIONS: u64 = 1_000_000;
pub const MAX_CALL_LEVELS: usize = 32;
/// Host requests per script, which also bounds replays (each request costs one re-run).
pub const MAX_REQUESTS: usize = 64;
/// Expression nesting at top level and inside functions. A flat `a + b + ...` chain nests
/// one level per operator. Stack use grows with both and with call levels: in a native
/// release build the worst mix (every call level nested to the function limit) runs in
/// 2 MiB, a quarter of the canister's 8 MiB stack.
pub const MAX_EXPR_DEPTH: usize = 128;
pub const MAX_FN_EXPR_DEPTH: usize = 64;
/// Heap a run may grow by. Rhai's size limits bound one value, not how many a script holds,
/// and do not count map keys or values captured by closures. Twice Python's: Rhai copies a
/// whole array on `for` and on assignment, and the largest reply the bridge passes (99,999
/// one-field records) is about 47 MiB per copy natively, 33 MiB on wasm32.
pub const MAX_MEMORY_BYTES: usize = 128 << 20;
const SUSPEND: &str = "__icp_suspend__";

#[derive(Clone, Debug)]
enum Request {
    Candid(String),
    Call(String, String, Value),
}

#[derive(Default)]
struct RunState {
    /// (request, answer) by sequence number, as JSON text (see `key`). Errors are cached
    /// too so replays see them. The cache outlives every replay, so its bytes are charged
    /// to the memory cap; text is a small fraction of the same serde_json value's heap.
    answers: Vec<(String, Result<String, String>)>,
    cached_bytes: usize,
    seq: usize,
    pending: Option<Request>,
    diverged: Option<String>,
    logs: Vec<String>,
    log_bytes: usize,
    /// Live heap when the current replay started.
    mem_base: usize,
}

impl RunState {
    fn log(&mut self, line: String) {
        if let Some((line, used)) = log_line(self.log_bytes, line, false) {
            self.log_bytes = used;
            self.logs.push(line);
        }
    }
}

type Shared = Arc<Mutex<RunState>>;

/// A request as compact, unambiguous text (`{:?}` quotes and escapes the names).
fn key(req: &Request) -> String {
    match req {
        Request::Candid(id) => format!("{id:?}"),
        Request::Call(id, m, a) => format!("{id:?} {m:?} {a}"),
    }
}

/// A cached reply, parsed straight into Rhai values. The bridge held it to 128 levels, one
/// more than serde_json's default limit.
fn reply_to_dynamic(json: &str) -> Res<Dynamic> {
    use serde::Deserialize;
    let mut de = serde_json::Deserializer::from_str(json);
    de.disable_recursion_limit();
    Dynamic::deserialize(&mut de).map_err(|e| format!("reply: {e}").into())
}

fn suspend(pos: Position) -> Box<EvalAltResult> {
    EvalAltResult::ErrorTerminated(SUSPEND.into(), pos).into()
}

/// Serve `req` from the replay cache, or suspend the run so the host can answer it.
fn request(st: &Shared, req: Request, pos: Position) -> Result<Dynamic, Box<EvalAltResult>> {
    let mut s = st.lock().unwrap();
    if s.pending.is_some() || s.diverged.is_some() {
        return Err(suspend(pos));
    }
    let seq = s.seq;
    s.seq += 1;
    match s.answers.get(seq) {
        Some((cached, _)) if *cached != key(&req) => {
            s.diverged = Some(format!("non-deterministic script: request #{seq} changed between replays"));
            Err(suspend(pos))
        }
        Some((_, Ok(v))) => reply_to_dynamic(v),
        Some((_, Err(e))) => Err(EvalAltResult::ErrorRuntime(e.clone().into(), pos).into()),
        None => {
            s.pending = Some(req);
            Err(suspend(pos))
        }
    }
}

/// Blobs are not JSON; hand them over as byte arrays (what candid `blob` accepts).
/// Bounded like every other value leaving a script: Rhai strings are shared, so an array
/// of references to one large string is small in the engine and huge once copied out.
fn to_json_value(d: &Dynamic, depth: usize, budget: &mut ValueBudget) -> Result<Value, Box<EvalAltResult>> {
    budget.node()?;
    if d.is_blob() || d.is_array() || d.is_map() {
        container_depth(depth)?;
    }
    if let Ok(f) = d.as_float() {
        return Ok(float_json(f));
    }
    if d.is_blob() {
        let b = d.clone().into_blob().unwrap_or_default();
        budget.bytes(b.len())?;
        return Ok(Value::Array(b.into_iter().map(Value::from).collect()));
    }
    if d.is_array() {
        let a = d.clone().into_array().unwrap_or_default();
        return a.iter().map(|x| to_json_value(x, depth + 1, budget)).collect::<Result<_, _>>().map(Value::Array);
    }
    if d.is_map() {
        let m = d.clone().cast::<rhai::Map>();
        let mut out = serde_json::Map::new();
        for (k, v) in m.iter() {
            budget.bytes(k.len())?;
            out.insert(k.to_string(), to_json_value(v, depth + 1, budget)?);
        }
        return Ok(Value::Object(out));
    }
    if d.is_string() {
        let s = d.clone().into_immutable_string().unwrap_or_default();
        budget.bytes(s.len())?;
        return Ok(Value::String(s.to_string()));
    }
    // Functions and custom types, refused in the words Python uses.
    rhai::serde::from_dynamic::<Value>(d)
        .map_err(|_| format!("a {} value cannot leave the sandbox; convert it to a map, array or string", d.type_name()).into())
}

type Res<T> = Result<T, Box<EvalAltResult>>;

/// Replacements for Rhai 1.27 builtins that are unsafe in a canister:
/// - the sorts drop comparer errors (`.ok()`), so a failing comparer still "sorts", and
///   rely on `catch_unwind` for comparers that are not a total order (NaN, or a script
///   comparer that lies), which does nothing under `panic = "abort"`: the canister traps.
///   These use a merge sort that cannot panic and pass errors through.
/// - `dedup` with a comparer drops its errors too.
/// - `replace` builds the whole result before the string size limit is checked, so
///   `s.replace("a", big)` could ask for a terabyte. This one checks the size first.
fn register_safe_builtins(engine: &mut Engine) {
    use rhai::{Array, FnPtr};
    use std::cmp::Ordering::{self, Equal, Less};

    fn by_comparer(ctx: &NativeCallContext, f: &FnPtr, a: &Dynamic, b: &Dynamic) -> Res<Ordering> {
        let v = f.call_raw(ctx, None, [a.clone(), b.clone()])?;
        match (v.as_int(), v.as_bool()) {
            (Ok(i), _) => Ok(i.cmp(&0)),
            (_, Ok(b)) => Ok(if b { Less } else { Ordering::Greater }),
            _ => Err(format!("sort comparer returned {}, not a number or bool", v.type_name()).into()),
        }
    }
    fn natural(a: &Dynamic, b: &Dynamic) -> Res<Ordering> {
        if a.type_id() != b.type_id() {
            return Err("elements of different types cannot be sorted".into());
        }
        Ok(if let (Ok(x), Ok(y)) = (a.as_int(), b.as_int()) {
            x.cmp(&y)
        } else if let (Ok(x), Ok(y)) = (a.as_float(), b.as_float()) {
            x.total_cmp(&y)
        } else if let (Ok(x), Ok(y)) = (a.as_char(), b.as_char()) {
            x.cmp(&y)
        } else if let (Ok(x), Ok(y)) = (a.as_bool(), b.as_bool()) {
            x.cmp(&y)
        } else if let (Ok(x), Ok(y)) = (a.clone().into_immutable_string(), b.clone().into_immutable_string()) {
            x.as_str().cmp(y.as_str())
        } else if a.is_unit() {
            Equal
        } else {
            return Err(format!("elements of {} cannot be sorted", a.type_name()).into());
        })
    }
    /// Stable bottom-up merge sort over indices; the array is untouched on error.
    fn sorted(arr: &Array, mut cmp: impl FnMut(&Dynamic, &Dynamic) -> Res<Ordering>) -> Res<Array> {
        let n = arr.len();
        let (mut idx, mut buf): (Vec<usize>, Vec<usize>) = ((0..n).collect(), Vec::with_capacity(n));
        let mut w = 1;
        while w < n {
            buf.clear();
            for lo in (0..n).step_by(2 * w) {
                let (mid, hi) = ((lo + w).min(n), (lo + 2 * w).min(n));
                let (mut i, mut j) = (lo, mid);
                while i < mid && j < hi {
                    if cmp(&arr[idx[j]], &arr[idx[i]])? == Less {
                        buf.push(idx[j]);
                        j += 1;
                    } else {
                        buf.push(idx[i]);
                        i += 1;
                    }
                }
                buf.extend_from_slice(&idx[i..mid]);
                buf.extend_from_slice(&idx[j..hi]);
            }
            std::mem::swap(&mut idx, &mut buf);
            w *= 2;
        }
        Ok(idx.into_iter().map(|i| arr[i].clone()).collect())
    }

    for name in ["sort", "sort_by"] {
        engine.register_fn(name, |ctx: NativeCallContext, a: &mut Array, f: FnPtr| -> Res<()> {
            *a = sorted(a, |x, y| by_comparer(&ctx, &f, x, y))?;
            Ok(())
        });
    }
    for name in ["order", "order_by"] {
        engine.register_fn(name, |ctx: NativeCallContext, a: &mut Array, f: FnPtr| sorted(a, |x, y| by_comparer(&ctx, &f, x, y)));
    }
    engine.register_fn("sort", |a: &mut Array| -> Res<()> {
        *a = sorted(a, natural)?;
        Ok(())
    });
    engine.register_fn("sort_desc", |a: &mut Array| -> Res<()> {
        *a = sorted(a, |x, y| natural(y, x))?;
        Ok(())
    });
    engine.register_fn("order", |a: &mut Array| sorted(a, natural));
    engine.register_fn("order_desc", |a: &mut Array| sorted(a, |x, y| natural(y, x)));
    engine.register_fn("dedup", |ctx: NativeCallContext, a: &mut Array, f: FnPtr| -> Res<()> {
        let mut out = Array::with_capacity(a.len());
        for x in a.iter() {
            if let Some(last) = out.last()
                && f.call_raw(&ctx, None, [last.clone(), x.clone()])?.as_bool().unwrap_or(false) {
                    continue;
                }
            out.push(x.clone());
        }
        *a = out;
        Ok(())
    });

    type Sizes = (usize, usize, usize);
    fn plus(t: Sizes, (a, m, s): Sizes) -> Sizes {
        (t.0 + a, t.1 + m, t.2 + s)
    }
    fn array_sizes(a: &Array) -> Sizes {
        a.iter().fold((a.len(), 0, 0), |t, x| plus(t, sizes(x)))
    }
    /// Deep (array elements, map entries, string bytes) as Rhai counts them, plus map keys,
    /// which Rhai skips.
    fn sizes(d: &Dynamic) -> Sizes {
        if let Ok(a) = d.as_array_ref() {
            array_sizes(&a)
        } else if let Ok(m) = d.as_map_ref() {
            m.iter().fold((0, m.len(), 0), |t, (k, x)| plus(t, plus((0, 0, k.len()), sizes(x))))
        } else if let Ok(b) = d.as_blob_ref() {
            (b.len(), 0, 0)
        } else if let Ok(s) = d.as_immutable_string_ref() {
            (0, 0, s.len())
        } else {
            (0, 0, 0)
        }
    }
    // Rhai's `pad` checks the size before growing, but without map keys: one call could
    // copy a map with a 1 MB key 1,000 times before the memory cap is next checked.
    engine.register_fn("pad", |ctx: NativeCallContext, a: &mut Array, len: rhai::INT, item: Dynamic| -> Res<()> {
        let Some(n) = usize::try_from(len).ok().and_then(|l| l.checked_sub(a.len())).filter(|&n| n > 0) else {
            return Ok(());
        };
        let (ha, hm, hs) = array_sizes(a);
        let (ia, im, is) = sizes(&item);
        let e = ctx.engine();
        let over = |have: usize, each: usize, max: usize| max > 0 && have.saturating_add(each.saturating_mul(n)) > max;
        if over(ha, ia + 1, e.max_array_size()) || over(hm, im, e.max_map_size()) || over(hs, is, e.max_string_size()) {
            return Err(format!("pad to {len} would go over the size limits (map keys count as text)").into());
        }
        a.resize(a.len() + n, item);
        Ok(())
    });

    // Rhai's parse_json is quadratic and runs as one operation; serde_json is linear and
    // limited to 128 levels.
    engine.register_fn("parse_json", |s: &str| -> Res<Dynamic> {
        let v: Value = serde_json::from_str(s).map_err(|e| format!("parse_json: {e}"))?;
        rhai::serde::to_dynamic(v)
    });

    fn replace(ctx: &NativeCallContext, s: &mut ImmutableString, find: &str, sub: &str) -> Res<()> {
        let hits = s.matches(find).count();
        let len = (s.len() - hits * find.len()).saturating_add(hits.saturating_mul(sub.len()));
        let max = ctx.engine().max_string_size();
        if max > 0 && len > max {
            return Err(format!("replace would build a string of {len} bytes, over the {max}-byte limit").into());
        }
        *s = s.replace(find, sub).into();
        Ok(())
    }
    engine.register_fn("replace", |ctx: NativeCallContext, s: &mut ImmutableString, find: &str, sub: &str| {
        replace(&ctx, s, find, sub)
    });
    engine.register_fn("replace", |ctx: NativeCallContext, s: &mut ImmutableString, find: char, sub: &str| {
        replace(&ctx, s, find.encode_utf8(&mut [0; 4]), sub)
    });
}

fn build_engine(state: &Shared, principal: String, deadline: u64) -> Engine {
    let mut engine = Engine::new_raw();
    engine.register_global_module(StandardPackage::new().as_shared_module());
    engine
        .set_max_operations(MAX_OPERATIONS)
        .set_max_call_levels(MAX_CALL_LEVELS)
        .set_max_expr_depths(MAX_EXPR_DEPTH, MAX_FN_EXPR_DEPTH)
        // Sized to the app's value limits, which the bridge also holds replies to, and Rhai
        // never counts more than they do: any reply the bridge passes fits.
        .set_max_string_size(crate::bridge::MAX_VALUE_BYTES)
        .set_max_array_size(crate::bridge::MAX_VALUE_NODES)
        .set_max_map_size(crate::bridge::MAX_VALUE_NODES)
        .set_max_variables(1_000)
        .set_max_functions(256)
        .set_max_modules(0);
    // `eval` would wrap our terminate marker into a catchable error.
    engine.disable_symbol("eval");
    // The optimizer's constant folding is quadratic in script size and runs before any
    // operation is counted. Scripts are short and run once: nothing to gain from it.
    engine.set_optimization_level(rhai::OptimizationLevel::None);
    // Curried function pointers are outside Rhai's size accounting, and deeply nested ones
    // overflow the stack. Closures still capture (Rhai keeps captures in the same list), so
    // what they hold is bounded by the memory cap in `on_progress`, not the size limits.
    engine.disable_symbol("curry");
    // `sleep` calls std::thread::sleep, which traps on wasm32-unknown-unknown.
    engine.register_fn("sleep", |_: rhai::INT| -> Res<()> { Err("sleep is not available in the sandbox".into()) });
    engine.register_fn("sleep", |_: rhai::FLOAT| -> Res<()> { Err("sleep is not available in the sandbox".into()) });
    register_safe_builtins(&mut engine);

    let s = state.clone();
    engine.on_print(move |text| s.lock().unwrap().log(text.to_string()));
    let s = state.clone();
    engine.on_debug(move |text, _, pos| s.lock().unwrap().log(format!("[debug {pos}] {text}")));
    // Closures/FnPtr calls wrap errors into catchable ones; re-terminate on the very next op.
    // Also stop (uncatchably) once the instruction budget is spent: some engine work,
    // like the size checks on deeply nested arrays, costs far more than one "operation".
    let s = state.clone();
    engine.on_progress(move |_| {
        let mut s = s.lock().unwrap();
        if s.pending.is_some() || s.diverged.is_some() {
            return Some(Dynamic::from(SUSPEND));
        }
        if let Err(e) = budget_error(deadline) {
            s.diverged = Some(e);
            return Some(Dynamic::from(SUSPEND));
        }
        if crate::counting_alloc::live().saturating_sub(s.mem_base) + s.cached_bytes > MAX_MEMORY_BYTES {
            s.diverged = Some(format!("script used more than {} MiB of memory", MAX_MEMORY_BYTES >> 20));
            return Some(Dynamic::from(SUSPEND));
        }
        None
    });

    let mut icp = Module::new();
    let s = state.clone();
    FuncRegistration::new("call_canister").with_volatility(true).set_into_module(
        &mut icp,
        move |ctx: NativeCallContext, canister: ImmutableString, method: ImmutableString, args: Dynamic| {
            let args = to_json_value(&args, 0, &mut ValueBudget::default())?;
            request(&s, Request::Call(canister.into(), method.into(), args), ctx.call_position())
        },
    );
    let s = state.clone();
    FuncRegistration::new("call_canister").with_volatility(true).set_into_module(
        &mut icp,
        move |ctx: NativeCallContext, canister: ImmutableString, method: ImmutableString| {
            request(&s, Request::Call(canister.into(), method.into(), Value::Null), ctx.call_position())
        },
    );
    let s = state.clone();
    FuncRegistration::new("candid").with_volatility(true).set_into_module(
        &mut icp,
        move |ctx: NativeCallContext, canister: ImmutableString| {
            request(&s, Request::Candid(canister.into()), ctx.call_position())
        },
    );
    FuncRegistration::new("principal")
        .set_into_module(&mut icp, move || -> ImmutableString { principal.as_str().into() });
    engine.register_static_module("icp", icp.into());
    engine
}

pub async fn run<B: HostBridge>(code: &str, bridge: &B) -> Result<Value, String> {
    #[cfg(test)]
    let _turn = crate::counting_alloc::test_serial::Turn::take();
    #[cfg(test)]
    let _count = crate::counting_alloc::test_serial::CountThisThread::on();
    let state: Shared = Arc::default();
    let engine = build_engine(&state, bridge.principal(), bridge.instruction_deadline());
    let ast = engine.compile(code).map_err(|e| match e.err_type() {
        rhai::ParseErrorType::ExprTooDeep => format!(
            "syntax error: {e} (expressions nest at most {MAX_EXPR_DEPTH} levels, {MAX_FN_EXPR_DEPTH} inside a function, \
             and each operator in a chain like `a + b + c` is a level: split long expressions into `let` bindings)"
        ),
        _ => format!("syntax error: {e}"),
    })?;
    // Compiling runs before any operation is counted; its cost is bounded by the script
    // size limit, and checked here so a slow compile is a policy error, not a surprise.
    crate::bridge::check_budget(bridge)?;
    let result = loop {
        {
            let mut s = state.lock().unwrap();
            s.seq = 0;
            s.logs.clear();
            s.log_bytes = 0;
            // Eval is synchronous: nothing else allocates on this thread until it returns.
            s.mem_base = crate::counting_alloc::live();
        }
        // ponytail: each request replays the script from the top, so cost is O(requests²)
        // in the worst case; bounded by the policy call cap. Upgrade: snapshot/continuations.
        let out = engine.eval_ast::<Dynamic>(&ast);
        let (pending, diverged) = {
            let mut s = state.lock().unwrap();
            (s.pending.take(), s.diverged.take())
        };
        if let Some(e) = diverged {
            break Err(e);
        }
        let Some(req) = pending else {
            break match out {
                Ok(v) => to_json_value(&v, 0, &mut ValueBudget::default()).map_err(|e| format!("result is not JSON-serializable: {e}")),
                Err(e) => Err(e.to_string()),
            };
        };
        if state.lock().unwrap().answers.len() >= MAX_REQUESTS {
            break Err(format!("script exceeded {MAX_REQUESTS} icp requests"));
        }
        let k = key(&req);
        let answer = match req {
            Request::Candid(id) => bridge.candid(&id).await.map(Value::String),
            Request::Call(id, m, a) => bridge.call(&id, &m, a).await,
        }
        .map(|v| v.to_string());
        let mut s = state.lock().unwrap();
        s.cached_bytes += k.len() + answer.as_ref().map_or_else(String::len, String::len);
        s.answers.push((k, answer));
    };
    for line in std::mem::take(&mut state.lock().unwrap().logs) {
        bridge.log(line);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::mock::MockBridge;
    use futures::executor::block_on;
    use serde_json::json;

    const L: &str = "ryjl3-tyaaa-aaaaa-aaaba-cai";

    #[test]
    fn replay_issues_each_call_once_and_keeps_final_logs() {
        let b = MockBridge::default();
        let code = format!(
            r#"print("start");
               let a = icp::call_canister("{L}", "icrc1_balance_of", #{{owner: "aaaaa-aa"}});
               let b = icp::call_canister("{L}", "icrc1_balance_of", #{{owner: "aaaaa-aa"}});
               print("done");
               [a.balance, b.balance]"#
        );
        assert_eq!(block_on(run(&code, &b)).unwrap(), json!([100, 200]));
        assert_eq!(b.calls.borrow().len(), 2);
        assert_eq!(b.logs.take(), vec!["start", "done"]);
    }

    #[test]
    fn terminate_marker_cannot_be_swallowed() {
        let b = MockBridge::default();
        // Closures wrap errors into catchable ones; the on_progress guard must still suspend.
        let code = format!(
            r#"let r = ();
               try {{ r = [1].map(|x| icp::call_canister("{L}", "icrc1_balance_of", #{{owner: "aaaaa-aa"}})); }}
               catch {{ r = "swallowed"; }}
               r"#
        );
        let out = block_on(run(&code, &b)).unwrap();
        assert_eq!(out, json!([{ "balance": 100, "owner": "aaaaa-aa" }]));
    }

    #[test]
    fn limits_and_sandbox() {
        let b = MockBridge::default();
        assert!(block_on(run("loop {}", &b)).unwrap_err().contains("Too many operations"));
        assert!(block_on(run("eval(\"1\")", &b)).is_err());
        assert!(block_on(run("timestamp()", &b)).is_err());
        assert!(block_on(run("import \"x\" as y; 1", &b)).is_err());
        assert_eq!(block_on(run("icp::principal()", &b)).unwrap(), json!("aaaaa-aa"));
        assert_eq!(block_on(run("#{a: (), b: 1.5}", &b)).unwrap(), json!({ "a": null, "b": 1.5 }));
        assert_eq!(block_on(run("let b = blob(2, 7); b", &b)).unwrap(), json!([7, 7]));
        // Strings are shared in Rhai: 300 references to one 1 MB string would copy out as 300 MB.
        let big = block_on(run("let s = \"x\"; s.pad(1000000, 'x'); let a = []; for i in 0..300 { a.push(s); } a", &b));
        assert!(big.is_err(), "refused by Rhai's own string-size accounting or by the value budget");
    }

    #[test]
    fn request_count_bounds_replays() {
        let b = MockBridge::default();
        let code = "let n = 0; for i in 0..100000 { try { icp::candid(\"nope\"); } catch (e) { n += 1; } } n";
        assert!(block_on(run(code, &b)).unwrap_err().contains("exceeded 64 icp requests"));
    }

    #[test]
    fn print_is_capped_per_run() {
        let b = MockBridge::default();
        block_on(run("let s = \"x\"; s.pad(100000, 'x'); for i in 0..1000 { print(s); } 1", &b)).unwrap();
        let total: usize = b.logs.take().iter().map(|l| crate::bridge::log_cost(l)).sum();
        assert!(total <= crate::bridge::MAX_LOG_BYTES, "{total}");
    }

    #[test]
    fn sleep_is_disabled_in_sandbox() {
        let b = MockBridge::default();
        assert!(block_on(run("sleep(0.001); 1", &b)).is_err());
    }

    #[test]
    fn sort_comparator_errors_are_not_swallowed() {
        let b = MockBridge::default();
        let err = block_on(run("[1, 2, 3].sort(|a, b| throw \"boom\"); 1", &b)).unwrap_err();
        assert!(err.contains("boom"), "{err}");
        let err = block_on(run("[1, 1].dedup(|a, b| throw \"dup\"); 1", &b)).unwrap_err();
        assert!(err.contains("dup"), "{err}");
        // Same results as Rhai's own sorts, including a comparer that is not a total order.
        let ok = |code: &str| block_on(run(code, &b)).unwrap();
        assert_eq!(ok("let a = [3, 1, 2]; a.sort(); a"), json!([1, 2, 3]));
        assert_eq!(ok("let a = [3, 1, 2]; a.sort_desc(); a"), json!([3, 2, 1]));
        assert_eq!(ok("let a = [1, 3, 2]; a.sort(|x, y| y - x); a"), json!([3, 2, 1]));
        assert_eq!(ok("[\"b\", \"a\"].order()"), json!(["a", "b"]));
        assert_eq!(ok("[2, 1].order_by(|x, y| x < y)"), json!([1, 2]));
        assert_eq!(ok("let a = [1, 1, 2, 1]; a.dedup(|x, y| x == y); a"), json!([1, 2, 1]));
        assert_eq!(ok("let a = [2.0, 0.0 / 0.0, 1.0, -1.0]; a.sort(); a.len()"), json!(4));
        assert_eq!(ok("let a = []; for i in 0..200 { a.push(i % 7); } a.sort(|x, y| if x == 3 { 1 } else { -1 }); a.len()"), json!(200));
        assert!(block_on(run("[1, \"a\"].sort(); 1", &b)).is_err());
    }

    #[test]
    fn audit_round_two_rhai_fixes() {
        let b = MockBridge::default();
        let run = |code: &str| block_on(run(code, &b));
        // `curry` is gone; closures still capture.
        assert!(run("fn add(a, b) { a + b } let f = Fn(\"add\").curry(1); f.call(2)").is_err());
        assert_eq!(run("let y = 2; let f = |x| x + y; f.call(3)").unwrap(), json!(5));
        // parse_json is serde_json: linear, and nested at most 128 levels.
        assert_eq!(run("let m = parse_json(\"{\\\"a\\\": [1, 2]}\"); m.a[1]").unwrap(), json!(2));
        let deep = format!("parse_json(\"{}{}\")", "[".repeat(200), "]".repeat(200));
        assert!(run(&deep).is_err());
        // A data race on a shared value is a clean error, not a spin-and-sleep trap.
        let race = run("let a = [1]; a.for_each(|| a.push(2)); a");
        assert!(race.is_err() || race.is_ok());
        // Arrays up to the app's value limit fit (Rhai's old 10,000 refused big replies).
        assert_eq!(run("let a = []; for i in 0..20000 { a.push(i); } a.len()").unwrap(), json!(20000));
    }

    #[test]
    fn replace_checks_the_size_before_building() {
        let b = MockBridge::default();
        let e = block_on(run("let s = \"a\"; s.pad(1000, 'a'); let t = \"b\"; t.pad(10000, 'b'); s.replace(\"a\", t); 1", &b)).unwrap_err();
        assert!(e.contains("over the"), "{e}");
        let e = block_on(run("let s = \"a\"; s.pad(1000, 'a'); let t = \"b\"; t.pad(10000, 'b'); s.replace('a', t); 1", &b)).unwrap_err();
        assert!(e.contains("over the"), "{e}");
        assert_eq!(block_on(run("let s = \"abca\"; s.replace(\"a\", \"xy\"); s", &b)).unwrap(), json!("xybcxy"));
    }

    #[test]
    fn two_arg_call_deep_value_and_non_deterministic_replay() {
        let b = MockBridge::default();
        assert!(block_on(run(&format!(r#"icp::call_canister("{L}", "icrc1_balance_of")"#), &b)).is_ok());

        // Nested > MAX_VALUE_DEPTH triggers value conversion error.
        let err = block_on(run(
            "let x = 1; for i in 0..135 { x = [x]; } x",
            &b,
        ))
        .unwrap_err();
        assert!(err.contains("nested deeper"), "{err}");

        // Trigger request divergence branch directly between cached request and replay request.
        let st: Shared = Arc::default();
        st.lock().unwrap().answers.push((key(&Request::Candid("a".into())), Ok("null".into())));
        let e = request(&st, Request::Candid("b".into()), Position::NONE);
        assert!(e.is_err());
        assert!(st.lock().unwrap().diverged.as_ref().unwrap().contains("non-deterministic"));
        assert!(request(&st, Request::Candid("c".into()), Position::NONE).is_err());
    }

    /// Answers every call with the same reply.
    struct Fixed(Value);
    impl HostBridge for Fixed {
        async fn candid(&self, _: &str) -> Result<String, String> {
            Err("no interface".into())
        }
        async fn call(&self, _: &str, _: &str, _: Value) -> Result<Value, String> {
            Ok(self.0.clone())
        }
        fn principal(&self) -> String {
            "aaaaa-aa".into()
        }
        fn log(&self, _: String) {}
    }

    #[test]
    fn replies_within_the_value_limits_fit_rhai() {
        use crate::bridge::{MAX_VALUE_BYTES, MAX_VALUE_NODES, check_value};
        let order = json!({ "id": 1, "items": [{ "sku": "a", "qty": 1 }] });
        let fits = [
            json!(vec![order; 1_200]),
            json!(vec![7; 20_000]),
            // 199,997 values: 49,999 records of three fields.
            json!(vec![json!({ "a": 1, "b": true, "c": "x" }); 49_999]),
            // The costliest in Rhai: one-field records, each its own map.
            json!(vec![json!({ "a": 1 }); 99_999]),
            json!(vec![json!({ "a": "" }); 99_999]),
            json!(vec![json!({ "a": 1, "b": 2 }); 66_666]),
            json!(vec![true; MAX_VALUE_NODES - 1]),
            json!("x".repeat(MAX_VALUE_BYTES)),
            // As deep as the bridge allows (one more than serde_json's default limit).
            json!([(0..127).fold(json!(1), |v, _| json!([v]))]),
        ];
        for reply in fits {
            check_value(&reply).unwrap();
            let n = reply.as_array().map_or_else(|| reply.as_str().unwrap().len(), Vec::len);
            let call = format!(r#"let r = icp::call_canister("{L}", "m");"#);
            // `for` and assignment copy the whole array in Rhai: two copies must fit.
            for tail in ["r.len()", "let r2 = r; r2.len()", "let n = 0; for x in r { n += 1; } n"] {
                let tail = if reply.is_string() { "r.len()" } else { tail };
                let out = block_on(run(&format!("{call} {tail}"), &Fixed(reply.clone())));
                assert_eq!(out.unwrap(), json!(n), "{tail}");
            }
        }
        // The bridge refuses larger replies for every runtime (the call has run by then).
        let deep = (0..129).fold(json!(1), |v, _| json!([v]));
        for over in [
            json!(vec![1; 250_000]),
            json!(vec![json!({ "a": 1, "b": true, "c": "x" }); 100_000]),
            json!("x".repeat(MAX_VALUE_BYTES + 1)),
            deep,
        ] {
            assert!(check_value(&over).is_err());
        }
    }

    #[test]
    fn memory_cap_counts_map_keys_and_captures() {
        use crate::bridge::MAX_VALUE_BYTES;
        let b = MockBridge::default();
        let mem = |code: &str| block_on(run(code, &b)).unwrap_err();
        // Rhai's size limits skip map keys: 300 keys of 1 MB each.
        let e = mem(r#"let k = "x"; k.pad(1000000, 'x'); let m = #{}; for i in 0..300 { m[k + i] = i; } m.len()"#);
        assert!(e.contains("MiB of memory"), "{e}");
        // And values captured by closures: 2,000 copies of a 9,000-element array.
        let e = mem("let t = []; for i in 0..9000 { t.push(i); } let fs = []; for i in 0..2000 { let a = t; fs.push(|| a.len()); } fs.len()");
        assert!(e.contains("MiB of memory"), "{e}");
        // One `pad` copying a map with a 1 MB key: refused before it allocates.
        let e = mem(r#"let k = "x"; k.pad(1000000, 'x'); let m = #{}; m[k] = 1; let a = []; a.pad(300, m); a.len()"#);
        assert!(e.contains("pad to 300"), "{e}");
        assert_eq!(block_on(run("let a = [1]; a.pad(3, #{x: [1, 2]}); a", &b)).unwrap(), json!([1, { "x": [1, 2] }, { "x": [1, 2] }]));
        assert_eq!(block_on(run("let a = [1, 2]; a.pad(1, 0); a.pad(-1, 0); a", &b)).unwrap(), json!([1, 2]));
        // Cached replies outlive every replay, so they count: 24 MiB of JSON each.
        let reply = json!("\u{1}".repeat(MAX_VALUE_BYTES));
        let e = block_on(run(&format!(r#"for i in 0..10 {{ icp::call_canister("{L}", "m"); }} 1"#), &Fixed(reply))).unwrap_err();
        assert!(e.contains("MiB of memory"), "{e}");
        // Values at Rhai's own size limits are well under the cap.
        let ok = "let a = []; a.pad(200000, 0); let s = \"\"; s.pad(4000000, 'x'); a.len() + s.len()";
        assert_eq!(block_on(run(ok, &b)).unwrap(), json!(4_200_000));
    }

    #[test]
    fn long_expressions_parse_and_too_deep_ones_say_how_to_split() {
        let b = MockBridge::default();
        let chain = |n| vec!["s"; n].join(" + ");
        assert_eq!(block_on(run(&format!("fn f(s) {{ {} }} f(1)", chain(40)), &b)).unwrap(), json!(40));
        assert_eq!(block_on(run(&format!("let s = 1; {}", chain(100)), &b)).unwrap(), json!(100));
        // Nesting up to the limit runs on a 2 MiB test thread; one more is a clean error.
        for (open, close) in [("(", ")"), ("[", "]"), ("#{a: ", "}"), ("abs(", ")")] {
            for in_fn in [false, true] {
                let mut n = 1;
                let err = loop {
                    let e = format!("{}1{}", open.repeat(n), close.repeat(n));
                    let code = if in_fn { format!("fn f() {{ {e} }} f()") } else { e };
                    match block_on(run(&code, &b)) {
                        Ok(_) => n += 1,
                        Err(e) => break e,
                    }
                };
                assert!(n > 10 && err.contains("split long expressions"), "{open} {in_fn} {n}: {err}");
            }
        }
    }
}
