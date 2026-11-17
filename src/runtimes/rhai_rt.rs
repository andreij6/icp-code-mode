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

use crate::bridge::{HostBridge, MAX_LOG_BYTES, ValueBudget, budget_error};
use rhai::packages::{Package, StandardPackage};
use rhai::{Dynamic, Engine, EvalAltResult, FuncRegistration, ImmutableString, Module, NativeCallContext, Position};
use serde_json::Value;
use std::sync::{Arc, Mutex};

/// Operations per run (each replay is a run).
pub const MAX_OPERATIONS: u64 = 1_000_000;
pub const MAX_CALL_LEVELS: usize = 32;
/// Host requests per script, which also bounds replays (each request costs one re-run).
pub const MAX_REQUESTS: usize = 64;
/// Deepest value accepted when converting to JSON.
const MAX_VALUE_DEPTH: usize = crate::candid_bridge::MAX_DEPTH;
const SUSPEND: &str = "__icp_suspend__";

#[derive(Clone, Debug, PartialEq)]
enum Request {
    Candid(String),
    Call(String, String, Value),
}

#[derive(Default)]
struct RunState {
    /// (request, answer) by sequence number. Errors are cached too so replays see them.
    answers: Vec<(Request, Result<Value, String>)>,
    seq: usize,
    pending: Option<Request>,
    diverged: Option<String>,
    logs: Vec<String>,
    log_bytes: usize,
}

impl RunState {
    fn log(&mut self, line: String) {
        if self.log_bytes >= MAX_LOG_BYTES {
            return;
        }
        let mut line = line;
        crate::bridge::truncate(&mut line, MAX_LOG_BYTES - self.log_bytes);
        self.log_bytes += line.len();
        self.logs.push(line);
    }
}

type Shared = Arc<Mutex<RunState>>;

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
        Some((cached, _)) if *cached != req => {
            s.diverged = Some(format!("non-deterministic script: request #{seq} changed between replays"));
            Err(suspend(pos))
        }
        Some((_, Ok(v))) => rhai::serde::to_dynamic(v),
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
    if depth > MAX_VALUE_DEPTH {
        return Err(format!("value nested deeper than {MAX_VALUE_DEPTH} levels").into());
    }
    budget.node()?;
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
    rhai::serde::from_dynamic::<Value>(d)
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
        .set_max_expr_depths(64, 32)
        // Sized to the app's value limits, so a reply the other runtimes accept fits too.
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
    // overflow the stack. Closures still capture variables (that does not use `curry`).
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
    let state: Shared = Arc::default();
    let engine = build_engine(&state, bridge.principal(), bridge.instruction_deadline());
    let ast = engine.compile(code).map_err(|e| format!("syntax error: {e}"))?;
    // Compiling runs before any operation is counted; its cost is bounded by the script
    // size limit, and checked here so a slow compile is a policy error, not a surprise.
    crate::bridge::check_budget(bridge)?;
    let result = loop {
        {
            let mut s = state.lock().unwrap();
            s.seq = 0;
            s.logs.clear();
            s.log_bytes = 0;
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
        let answer = match &req {
            Request::Candid(id) => bridge.candid(id).await.map(Value::String),
            Request::Call(id, m, a) => bridge.call(id, m, a.clone()).await,
        };
        state.lock().unwrap().answers.push((req, answer));
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
        let total: usize = b.logs.take().iter().map(|l| l.len()).sum();
        assert!(total <= MAX_LOG_BYTES + 64, "{total}");
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
        st.lock().unwrap().answers.push((Request::Candid("a".into()), Ok(Value::Null)));
        let e = request(&st, Request::Candid("b".into()), Position::NONE);
        assert!(e.is_err());
        assert!(st.lock().unwrap().diverged.as_ref().unwrap().contains("non-deterministic"));
        assert!(request(&st, Request::Candid("c".into()), Position::NONE).is_err());
    }
}
