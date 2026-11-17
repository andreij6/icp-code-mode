//! Python via Monty (pydantic's sandboxed interpreter). `icp.*` are external
//! functions: the VM suspends on each call, the host awaits the bridge and
//! resumes it with the answer.
//!
//! * `icp.call(...)` looks synchronous: one suspension, one inter-canister call.
//! * `icp.call_async(...)` is awaitable, so `await asyncio.gather(...)` puts several
//!   calls in flight; the host dispatches them together when the VM blocks.

use crate::bridge::{HostBridge, now_ms};
use monty::{MontyRun, RunProgress};
use monty_types::{
    CompileOptions, DateTimeSource, ExcType, ExtFunctionResult, MontyException, MontyObject, MontyUuid,
    NameLookupResult, ObjectRef, OsFunctionCall, OsPolicy, PrintWriter, RandomSeed, RandomStart, ResourceLimits,
    ResourceTracker, SleepMode,
};
use serde_json::{Map, Number, Value};
use std::sync::atomic::Ordering;
use std::time::Duration;

pub const MAX_RECURSION_DEPTH: usize = 200;
pub const MAX_SUSPENSIONS: usize = 256;
pub const MAX_MEMORY_BYTES: usize = 64 << 20;
const MAX_PRINT_BYTES: usize = crate::bridge::MAX_LOG_BYTES;
/// Total VM budget per script. On-chain the web-time shim makes Monty's clock an
/// instruction counter (1 ns == 1 instruction), so this is 10B instructions —
/// a clean `TimeoutError` well before the IC's 40B per-message trap. Natively it
/// is wall time.
pub const EXECUTION_BUDGET: Duration = Duration::from_secs(if cfg!(target_arch = "wasm32") { 10 } else { 2 });

fn icp_module() -> MontyObject {
    let s = MontyObject::string;
    MontyObject::module(
        "icp",
        MontyUuid::from_u128(0x1C9),
        [
            (s("call"), MontyObject::function("icp.call", Some("call(canister_id, method, args=None)".into()))),
            (s("call_async"), MontyObject::async_function("icp.call_async", Some("awaitable icp.call".into()))),
            (s("candid"), MontyObject::function("icp.candid", Some("candid(canister_id) -> .did text".into()))),
            (s("principal"), MontyObject::function("icp.principal", Some("this canister's principal".into()))),
        ],
    )
}

fn to_json(o: ObjectRef<'_>) -> Value {
    match o.type_name() {
        "NoneType" => Value::Null,
        "bool" => Value::Bool(o.as_bool().unwrap_or_default()),
        // Beyond i64: exact decimal string (matches how nat/int come back from candid).
        "int" => o.as_int().map_or_else(|| Value::String(o.py_repr()), Value::from),
        "float" => o.as_float().and_then(Number::from_f64).map_or(Value::Null, Value::Number),
        "str" => Value::String(o.as_str().unwrap_or_default().to_owned()),
        // bytes / bytearray -> byte array (what candid `blob` accepts).
        "bytes" | "bytearray" => match monty_types::unstable::node(o) {
            monty_types::unstable::MontyNode::Bytes(b) => Value::Array(b.iter().map(|x| Value::from(*x)).collect()),
            _ => Value::String(o.py_repr()),
        },
        "list" | "tuple" | "set" | "frozenset" => {
            Value::Array(o.items().unwrap_or_default().into_iter().map(to_json).collect())
        }
        "dict" => Value::Object(
            o.pairs()
                .unwrap_or_default()
                .into_iter()
                .map(|(k, v)| (k.as_str().map_or_else(|| k.py_repr(), str::to_owned), to_json(v)))
                .collect::<Map<_, _>>(),
        ),
        _ => Value::String(o.py_repr()),
    }
}

fn from_json(v: &Value) -> MontyObject {
    match v {
        Value::Null => MontyObject::none(),
        Value::Bool(b) => MontyObject::bool(*b),
        Value::Number(n) => n
            .as_i64()
            .map(MontyObject::int)
            .or_else(|| n.as_u64().map(|u| MontyObject::bigint(u.into())))
            .unwrap_or_else(|| MontyObject::float(n.as_f64().unwrap_or(f64::NAN))),
        Value::String(s) => MontyObject::string(s.as_str()),
        Value::Array(a) => MontyObject::list(a.iter().map(from_json)),
        Value::Object(m) => MontyObject::dict(m.iter().map(|(k, v)| (MontyObject::string(k.as_str()), from_json(v)))),
    }
}

fn exc(t: ExcType, msg: impl Into<String>) -> MontyException {
    MontyException::new(t, Some(msg.into()))
}

/// Perform one `icp.*` request.
async fn host<B: HostBridge>(bridge: &B, name: &str, args: &[Value]) -> Result<Value, MontyException> {
    let rt = |e: String| exc(ExcType::RuntimeError, e);
    match (name, args) {
        ("icp.call" | "icp.call_async", [Value::String(c), Value::String(m)]) => {
            bridge.call(c, m, Value::Null).await.map_err(rt)
        }
        ("icp.call" | "icp.call_async", [Value::String(c), Value::String(m), a]) => {
            bridge.call(c, m, a.clone()).await.map_err(rt)
        }
        ("icp.candid", [Value::String(c)]) => bridge.candid(c).await.map(Value::String).map_err(rt),
        ("icp.principal", []) => Ok(Value::String(bridge.principal())),
        ("icp.call" | "icp.call_async", _) => Err(exc(ExcType::TypeError, "icp.call(canister_id: str, method: str, args=None)")),
        ("icp.candid", _) => Err(exc(ExcType::TypeError, "icp.candid(canister_id: str)")),
        ("icp.principal", _) => Err(exc(ExcType::TypeError, "icp.principal() takes no arguments")),
        _ => Err(ExtFunctionResult::not_found_exception(name)),
    }
}

fn ext(r: Result<Value, MontyException>) -> ExtFunctionResult {
    match r {
        Ok(v) => ExtFunctionResult::Return(from_json(&v)),
        Err(e) => ExtFunctionResult::Error(e),
    }
}

/// Await `f` without letting other requests' allocations (made while we are
/// suspended) count against this script's memory limit, or ours against theirs.
async fn rebased<T>(f: impl std::future::Future<Output = T>) -> T {
    use monty_types::{BASELINE_MEMORY, LIVE_MEMORY};
    let own = LIVE_MEMORY.load(Ordering::Relaxed).saturating_sub(BASELINE_MEMORY.load(Ordering::Relaxed));
    let out = f.await;
    BASELINE_MEMORY.store(LIVE_MEMORY.load(Ordering::Relaxed).saturating_sub(own), Ordering::Relaxed);
    out
}

pub async fn run<B: HostBridge>(code: &str, bridge: &B) -> Result<Value, String> {
    #[cfg(test)]
    crate::counting_alloc::COUNT_THIS_THREAD.with(|c| c.set(true));
    let mut out = String::new();
    let result = drive(code, bridge, &mut out).await;
    for line in out.lines() {
        bridge.log(line.to_string());
    }
    result
}

async fn drive<B: HostBridge>(code: &str, bridge: &B, out: &mut String) -> Result<Value, String> {
    let err = |e: MontyException| e.to_string();
    let policy = OsPolicy {
        // Determinism: block time, no sleeping, seeded `random`.
        datetime: DateTimeSource::Fixed { unix_seconds: (now_ms() / 1000) as i64, microsecond: 0 },
        sleep: SleepMode::Zero,
        random_start: RandomStart::Seed(RandomSeed::Int(0x1C9.into())),
        ..OsPolicy::default()
    };
    let limits = ResourceLimits::default()
        .max_recursion_depth(MAX_RECURSION_DEPTH)
        .max_suspensions(MAX_SUSPENSIONS)
        .max_feed_duration(EXECUTION_BUDGET)
        .max_memory(MAX_MEMORY_BYTES);
    // max_memory counts LIVE_MEMORY - BASELINE_MEMORY (counting allocator in lib.rs).
    // The baseline is process-global and other requests run while we await, so it is
    // re-based around every await to keep measuring only this script's heap.
    monty_types::BASELINE_MEMORY.store(monty_types::LIVE_MEMORY.load(Ordering::Relaxed), Ordering::Relaxed);

    let mut progress = MontyRun::new(code.to_owned(), "main.py", vec!["icp".to_owned()], CompileOptions::default())
        .map_err(err)?
        .with_os_policy(policy)
        .start(vec![icp_module()], ResourceTracker::new(limits), PrintWriter::CollectString(out, Some(MAX_PRINT_BYTES)))
        .map_err(err)?;

    // In-flight `icp.call_async` requests: (call_id, function, args).
    let mut queued: Vec<(u32, String, Vec<Value>)> = vec![];
    for _ in 0..MAX_SUSPENSIONS {
        let next = match progress {
            RunProgress::Complete(v) => return Ok(to_json(v.as_ref())),
            RunProgress::FunctionCall(c) => {
                let args: Vec<Value> = c.args.args().map(to_json).collect();
                if c.args.kwargs().next().is_some() {
                    let e = exc(ExcType::TypeError, "icp functions take positional arguments only");
                    c.resume(ExtFunctionResult::Error(e), PrintWriter::CollectString(out, Some(MAX_PRINT_BYTES)))
                } else if c.function_name == "icp.call_async" && !c.allow_eager_await {
                    queued.push((c.call_id, c.function_name.clone(), args));
                    c.resume_pending(PrintWriter::CollectString(out, Some(MAX_PRINT_BYTES)))
                } else {
                    let r = rebased(host(bridge, &c.function_name, &args)).await;
                    let pw = PrintWriter::CollectString(out, Some(MAX_PRINT_BYTES));
                    if c.allow_eager_await { c.resume_eager(r.map(|v| from_json(&v)), pw) } else { c.resume(ext(r), pw) }
                }
            }
            RunProgress::ResolveFutures(rf) => {
                let batch = std::mem::take(&mut queued);
                if batch.is_empty() {
                    return Err("script is awaiting something that can never complete".into());
                }
                let replies = rebased(futures::future::join_all(batch.iter().map(|(_, n, a)| host(bridge, n, a)))).await;
                let results = batch.iter().zip(replies).map(|((id, ..), r)| (*id, ext(r))).collect();
                rf.resume(results, PrintWriter::CollectString(out, Some(MAX_PRINT_BYTES)))
            }
            RunProgress::NameLookup(l) => l.resume(NameLookupResult::Undefined, PrintWriter::CollectString(out, Some(MAX_PRINT_BYTES))),
            RunProgress::OsCall(c) => {
                let r = match &c.function_call {
                    OsFunctionCall::Import(a) if a.module == "icp" => ExtFunctionResult::Return(icp_module()),
                    other => other.no_handler_result(),
                };
                c.resume(r, PrintWriter::CollectString(out, Some(MAX_PRINT_BYTES)))
            }
        };
        progress = next.map_err(err)?;
    }
    Err(format!("script exceeded {MAX_SUSPENSIONS} host suspensions"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::mock::MockBridge;
    use futures::executor::block_on;
    use serde_json::json;

    const L: &str = "ryjl3-tyaaa-aaaaa-aaaba-cai";

    fn py(code: &str) -> (Result<Value, String>, MockBridge) {
        let b = MockBridge::default();
        let r = block_on(run(code, &b));
        (r, b)
    }

    #[test]
    fn gather_batches_async_calls() {
        let code = format!(
            "import asyncio\n\
             a, b = await asyncio.gather(\n\
                 icp.call_async('{L}', 'icrc1_balance_of', {{'owner': 'aaaaa-aa'}}),\n\
                 icp.call_async('{L}', 'icrc1_balance_of', {{'owner': '2vxsx-fae'}}))\n\
             c = await icp.call_async('{L}', 'icrc1_balance_of', {{'owner': 'x'}})\n\
             [a['balance'] + b['balance'], c['balance']]"
        );
        let (r, b) = py(&code);
        assert_eq!(r.unwrap(), json!([300, 300]));
        assert_eq!(b.calls.borrow().len(), 3);
    }

    #[test]
    fn determinism_and_sandbox() {
        assert_eq!(py("import time\ntime.time()").0.unwrap(), json!(0.0));
        let r1 = py("import random\nrandom.random()").0.unwrap();
        assert_eq!(r1, py("import random\nrandom.random()").0.unwrap());
        assert!(py("import os\nos.listdir('.')").0.is_err());
        assert!(py("open('/etc/passwd').read()").0.is_err());
        assert!(py("undefined_name").0.unwrap_err().contains("NameError"));
        assert_eq!(py("2**70").0.unwrap(), json!("1180591620717411303424"));
        assert_eq!(py("{'t': (1, 2), 'n': None, 'f': 0.5}").0.unwrap(), json!({ "t": [1, 2], "n": null, "f": 0.5 }));
        assert!(py("icp.call(1)").0.unwrap_err().contains("TypeError"));
        assert!(py("icp.call('a', 'b', args=1)").0.unwrap_err().contains("positional"));
        assert_eq!(py("from icp import principal\nprincipal()").0.unwrap(), json!("aaaaa-aa"));
        assert_eq!(py("b'\\x01\\x02'").0.unwrap(), json!([1, 2]));
    }

    /// Like MockBridge, but every call suspends once, as an inter-canister await does.
    struct YieldBridge(MockBridge);
    struct YieldOnce(bool);
    impl std::future::Future for YieldOnce {
        type Output = ();
        fn poll(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<()> {
            if self.0 {
                return std::task::Poll::Ready(());
            }
            self.0 = true;
            cx.waker().wake_by_ref();
            std::task::Poll::Pending
        }
    }
    impl HostBridge for YieldBridge {
        async fn candid(&self, id: &str) -> Result<String, String> {
            self.0.candid(id).await
        }
        async fn call(&self, id: &str, m: &str, a: Value) -> Result<Value, String> {
            YieldOnce(false).await;
            self.0.call(id, m, a).await
        }
        fn principal(&self) -> String {
            self.0.principal()
        }
        fn log(&self, l: String) {
            self.0.log(l)
        }
    }

    #[test]
    fn memory_limit_survives_interleaved_runs() {
        // 2 x 100 MB > 64 MiB: must fail even if another run starts while this one awaits.
        // (Wide margin: other test threads running Python share monty's global counter.)
        let a = format!("x = 'y' * 100000000\nicp.call('{L}', 'icrc1_balance_of', {{'owner': 'aaaaa-aa'}})\nz = 'q' * 100000000\nlen(x) + len(z)");
        let alone = block_on(run(&a, &YieldBridge(MockBridge::default())));
        assert!(alone.as_ref().unwrap_err().contains("MemoryError"), "alone: {alone:?}");
        let (ba, bb) = (YieldBridge(MockBridge::default()), YieldBridge(MockBridge::default()));
        let (ra, rb) = block_on(futures::future::join(run(&a, &ba), run("1", &bb)));
        assert!(ra.unwrap_err().contains("MemoryError"));
        assert_eq!(rb.unwrap(), json!(1));
    }

    #[test]
    fn memory_limit() {
        let r = py("x = []\nwhile True:\n    x.append('y' * 100000)").0;
        let e = r.unwrap_err();
        assert!(e.contains("MemoryError") || e.contains("TimeoutError"), "{e}");
    }
}
