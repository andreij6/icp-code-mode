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

use crate::bridge::{HostBridge, MAX_LOG_BYTES, budget_error};
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
fn to_json_value(d: &Dynamic, depth: usize) -> Result<Value, Box<EvalAltResult>> {
    if depth > MAX_VALUE_DEPTH {
        return Err(format!("value nested deeper than {MAX_VALUE_DEPTH} levels").into());
    }
    if d.is_blob() {
        let b = d.clone().into_blob().unwrap_or_default();
        return Ok(Value::Array(b.into_iter().map(Value::from).collect()));
    }
    if d.is_array() {
        let a = d.clone().into_array().unwrap_or_default();
        return a.iter().map(|x| to_json_value(x, depth + 1)).collect::<Result<_, _>>().map(Value::Array);
    }
    if d.is_map() {
        let m = d.clone().cast::<rhai::Map>();
        let mut out = serde_json::Map::new();
        for (k, v) in m.iter() {
            out.insert(k.to_string(), to_json_value(v, depth + 1)?);
        }
        return Ok(Value::Object(out));
    }
    rhai::serde::from_dynamic::<Value>(d)
}

fn build_engine(state: &Shared, principal: String, deadline: u64) -> Engine {
    let mut engine = Engine::new_raw();
    engine.register_global_module(StandardPackage::new().as_shared_module());
    engine
        .set_max_operations(MAX_OPERATIONS)
        .set_max_call_levels(MAX_CALL_LEVELS)
        .set_max_expr_depths(64, 32)
        .set_max_string_size(1 << 20)
        .set_max_array_size(10_000)
        .set_max_map_size(10_000)
        .set_max_variables(1_000)
        .set_max_functions(256)
        .set_max_modules(0);
    // `eval` would wrap our terminate marker into a catchable error.
    engine.disable_symbol("eval");

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
            let args = to_json_value(&args, 0)?;
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
                Ok(v) => to_json_value(&v, 0).map_err(|e| format!("result is not JSON-serializable: {e}")),
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
}
