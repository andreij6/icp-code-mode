//! Python via Monty (pydantic's sandboxed interpreter). `icp.*` are external
//! functions: the VM suspends on each call, the host awaits the bridge and
//! resumes it with the answer.
//!
//! * `icp.call(...)` looks synchronous: one suspension, one inter-canister call.
//! * `icp.call_async(...)` is awaitable, so `await asyncio.gather(...)` puts several
//!   calls in flight; the host dispatches them together when the VM blocks.

use crate::bridge::{HostBridge, ValueBudget, check_budget, instruction_counter, now_ms};
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

/// Collects `print` output up to `MAX_PRINT_BYTES`, then cuts it with the marker the
/// other runtimes use (`bridge::truncate`) and drops the rest. Monty's own capped
/// collector raises `MemoryError` instead.
#[derive(Default)]
struct Printed {
    text: String,
    cut: bool,
}

const CUT_MARK: &str = "…[truncated]";

impl Printed {
    fn append(&mut self, s: &str) {
        if self.cut {
            return;
        }
        if self.text.len() + s.len() <= MAX_PRINT_BYTES {
            self.text.push_str(s);
            return;
        }
        let keep = MAX_PRINT_BYTES - CUT_MARK.len();
        self.text.push_str(&s[..s.floor_char_boundary(keep.saturating_sub(self.text.len()))]);
        self.text.truncate(self.text.floor_char_boundary(keep));
        self.text.push_str(CUT_MARK);
        self.cut = true;
    }
}

impl monty_types::PrintWriterCallback for Printed {
    fn stdout_write(&mut self, output: std::borrow::Cow<'_, str>) -> Result<(), MontyException> {
        // Once cut, the vendored `print` probes with an empty write before formatting its
        // arguments and skips them on Err (vendor/monty/PATCHES.md): no formatting cost
        // for output nobody sees. The probe is its only empty write (empty `sep`, `end`
        // and argument text are not written), and any other write still succeeds.
        if self.cut && output.is_empty() {
            return Err(exc(ExcType::RuntimeError, "print output is full"));
        }
        self.append(&output);
        Ok(())
    }
    fn stdout_push(&mut self, end: char) -> Result<(), MontyException> {
        self.append(end.encode_utf8(&mut [0; 4]));
        Ok(())
    }
}

/// Wall-time VM budget for native tests. On-chain the budget is what is left of the
/// policy's instruction budget (see `vm_budget`).
pub const EXECUTION_BUDGET: Duration = Duration::from_secs(2);
/// Longest single stretch between suspensions, in instructions. The VM checks it between
/// steps, so a turn can end one step past it: 20B leaves 20B of headroom under the IC's
/// 40B per-message limit for that step (the patched Monty polls before every expensive
/// one, and the largest linear step measured costs ~13B).
const MAX_TURN_INSTRUCTIONS: u64 = 20_000_000_000;
const MAX_VALUE_DEPTH: usize = crate::candid_bridge::MAX_DEPTH;

/// What is left of this run's VM budget. On-chain it is what is left of the policy's
/// instruction budget (`set_limits`), so Python obeys the same limit as JavaScript and
/// Rhai; the web-time shim makes one "nanosecond" one instruction. Off-chain it is what
/// is left of `EXECUTION_BUDGET` of wall time since `started`.
fn vm_budget<B: HostBridge>(bridge: &B, started: Option<std::time::Instant>) -> Duration {
    if cfg!(target_arch = "wasm32") {
        Duration::from_nanos(bridge.instruction_deadline().saturating_sub(instruction_counter()))
    } else {
        EXECUTION_BUDGET.saturating_sub(started.map_or(Duration::ZERO, |s| s.elapsed()))
    }
}

/// Gives the VM a fresh allowance from the live counter before it starts or resumes.
/// Monty's clock stops while the run is suspended, so host work (argument conversion,
/// Candid, the awaited call) would otherwise never be deducted from a relative budget.
fn allow<B: HostBridge>(t: &mut ResourceTracker, bridge: &B, started: Option<std::time::Instant>) {
    let left = vm_budget(bridge, started);
    t.set_max_feed_duration(left);
    t.set_max_turn_duration(left.min(Duration::from_nanos(MAX_TURN_INSTRUCTIONS)));
}

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

/// Python value -> JSON, bounded in depth, element count and text size. Dispatch is on
/// the node kind, never on `type_name()` (which a script picks: `class list: ...`).
/// Containers and scalars convert; a few leaf kinds whose repr holds no nested value
/// (Decimal, datetime, ...) export as that repr; everything else (class instances,
/// exceptions, `range`, and Monty's markers for data it cut: a cycle or `<deeply
/// nested>`) is an error, never text standing in for the data.
fn to_json(o: ObjectRef<'_>, depth: usize, budget: &mut ValueBudget) -> Result<Value, String> {
    use monty_types::unstable::{MontyNode as N, node};
    if depth > MAX_VALUE_DEPTH {
        return Err(format!("value nested deeper than {MAX_VALUE_DEPTH} levels"));
    }
    budget.node()?;
    Ok(match node(o) {
        N::None => Value::Null,
        N::Bool(b) => Value::Bool(*b),
        N::Int(i) => Value::from(*i),
        // Beyond i64: exact decimal string (matches how nat/int come back from candid).
        N::BigInt(_) => Value::String(int_text(o, budget)?),
        N::Float(f) => Number::from_f64(*f).map_or(Value::Null, Value::Number),
        N::String(s) => {
            budget.bytes(s.len())?;
            Value::String(s.clone())
        }
        // bytes / bytearray -> byte array (what candid `blob` accepts), one node per byte.
        N::Bytes(b) => {
            budget.bytes(b.len())?;
            for _ in 0..b.len() {
                budget.node()?;
            }
            Value::Array(b.iter().map(|x| Value::from(*x)).collect())
        }
        N::List(_) | N::Tuple(_) | N::Set(_) | N::FrozenSet(_) | N::NamedTuple { .. } => Value::Array(
            o.items().unwrap_or_default().into_iter().map(|x| to_json(x, depth + 1, budget)).collect::<Result<_, _>>()?,
        ),
        N::Dict(_) => {
            let mut out = Map::new();
            for (k, v) in o.pairs().unwrap_or_default() {
                let key = match node(k) {
                    N::String(s) => s.clone(),
                    N::Int(_) | N::BigInt(_) => int_text(k, budget)?,
                    // As `json.dumps` renders them.
                    N::Bool(b) => b.to_string(),
                    _ => return Err(format!("dict keys must be str, int or bool to leave the sandbox, not {}", k.type_name())),
                };
                budget.bytes(key.len())?;
                let value = to_json(v, depth + 1, budget)?;
                if out.contains_key(&key) {
                    return Err(format!("dict has two keys that are both {key:?} as text"));
                }
                out.insert(key, value);
            }
            Value::Object(out)
        }
        N::Cycle(_) => return Err("value contains a reference cycle".into()),
        N::Repr(t) if t.is_empty() || t == "<deeply nested>" => {
            return Err(format!("value nested deeper than {MAX_VALUE_DEPTH} levels"));
        }
        N::Repr(t) => {
            let t: String = t.chars().take(60).collect();
            return Err(format!("{t} cannot leave the sandbox; convert it to a dict, list or str"));
        }
        N::Decimal(_)
        | N::Date(_)
        | N::DateTime(_)
        | N::Time(_)
        | N::TimeDelta(_)
        | N::TimeZone(_)
        | N::Complex(_)
        | N::Ellipsis => {
            let s = o.py_repr();
            budget.bytes(s.len())?;
            Value::String(s)
        }
        _ => return Err(format!("a {} value cannot leave the sandbox; convert it to a dict, list or str", o.type_name())),
    })
}

/// Largest int exported: what Monty's arithmetic may produce (`MAX_INT_BITS` in
/// vendor/monty/src/resource_checks.rs). Refused before its decimal text, which costs
/// time quadratic in the size, is built.
const MAX_INT_BITS: u64 = 1 << 15;

/// An `int` node's decimal text.
fn int_text(o: ObjectRef<'_>, budget: &mut ValueBudget) -> Result<String, String> {
    let s = match monty_types::unstable::node(o) {
        monty_types::unstable::MontyNode::BigInt(b) if b.bits() > MAX_INT_BITS => {
            return Err(format!("int of {} bits is too large to export (limit {MAX_INT_BITS} bits)", b.bits()));
        }
        monty_types::unstable::MontyNode::BigInt(b) => b.to_string(),
        _ => o.as_int().map(|i| i.to_string()).unwrap_or_default(),
    };
    budget.bytes(s.len())?;
    Ok(s)
}

// Input here is always JSON decoded from a Candid reply, which `candid_bridge` already
// limits to `MAX_DEPTH` levels and the reply-size quota.
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

/// A reply as a Python value. The JSON reply was allocated inside the `rebased` window,
/// so it is not charged to the run; dropping it must not credit the run either.
fn reply(v: Value) -> MontyObject {
    let o = from_json(&v);
    drop_uncharged(v);
    o
}

fn ext(r: Result<Value, MontyException>) -> ExtFunctionResult {
    match r {
        Ok(v) => ExtFunctionResult::Return(reply(v)),
        Err(e) => ExtFunctionResult::Error(e),
    }
}

/// Drops `t` and lowers the baseline by what that freed, so freeing memory the run was
/// never charged for does not count as room for more.
fn drop_uncharged<T>(t: T) {
    use monty_types::{BASELINE_MEMORY, LIVE_MEMORY};
    let before = LIVE_MEMORY.load(Ordering::Relaxed);
    drop(t);
    let freed = before.saturating_sub(LIVE_MEMORY.load(Ordering::Relaxed));
    let _ = BASELINE_MEMORY.try_update(Ordering::Relaxed, Ordering::Relaxed, |b| Some(b.saturating_sub(freed)));
}

/// Await `f` without letting other requests' allocations (made while we are
/// suspended) count against this script's memory limit, or ours against theirs.
///
/// The baseline is reset once, when `f` completes: everything allocated or freed while
/// it ran (other runs' work, the IC's reply buffer, its decoding) is left out, and what
/// this run held before is kept. The reply's Python copy, built afterwards, is charged;
/// the JSON reply is dropped with `drop_uncharged`. Monty checks memory only while the
/// VM runs, so the baseline does not matter in between.
async fn rebased<T>(f: impl std::future::Future<Output = T>) -> T {
    use monty_types::{BASELINE_MEMORY, LIVE_MEMORY};
    let own = LIVE_MEMORY.load(Ordering::Relaxed).saturating_sub(BASELINE_MEMORY.load(Ordering::Relaxed));
    let v = f.await;
    BASELINE_MEMORY.store(LIVE_MEMORY.load(Ordering::Relaxed).saturating_sub(own), Ordering::Relaxed);
    v
}

/// `eval` and `exec` compile source built at run time. There is no `builtins` module or
/// `compile` in Monty, so the names are the only direct way to reach them; this scan is
/// not a boundary, though: a string annotation is evaluated as an expression, so
/// `typing.get_type_hints` can still run `eval` from run-time text. What bounds that is
/// the patched Monty's 16 KiB cap and time poll on every snippet it compiles.
///
/// Python folds identifiers with NFKC (`ｅval` is `eval`), so the check runs on the folded text.
fn uses_dynamic_code(code: &str) -> Option<&'static str> {
    use unicode_normalization::UnicodeNormalization;
    let folded: String;
    let code = if code.is_ascii() {
        code
    } else {
        folded = code.nfkc().collect();
        &folded
    };
    let ident = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    ["eval", "exec"].into_iter().find(|w| {
        code.match_indices(w).any(|(i, _)| {
            let before = code[..i].chars().next_back();
            !ident(before) && before != Some('.') && !ident(code[i + w.len()..].chars().next())
        })
    })
}

/// Monty's memory counter is one process-wide number. The canister has a single thread,
/// but `cargo test` runs tests on several, so Python runs on different test threads would
/// see each other's allocations. Tests therefore take turns: a thread holds this lock while
/// it has any Python run in flight (several may interleave on one thread).
#[cfg(test)]
pub(crate) mod test_serial {
    use std::cell::RefCell;
    use std::sync::{Mutex, MutexGuard};

    static LOCK: Mutex<()> = Mutex::new(());
    thread_local!(static HELD: RefCell<(usize, Option<MutexGuard<'static, ()>>)> = const { RefCell::new((0, None)) });

    pub struct Turn;
    impl Turn {
        pub fn take() -> Self {
            HELD.with(|h| {
                let mut h = h.borrow_mut();
                if h.0 == 0 {
                    h.1 = Some(LOCK.lock().unwrap_or_else(|e| e.into_inner()));
                }
                h.0 += 1;
            });
            Turn
        }
    }
    impl Drop for Turn {
        fn drop(&mut self) {
            HELD.with(|h| {
                let mut h = h.borrow_mut();
                h.0 -= 1;
                if h.0 == 0 {
                    h.1 = None;
                }
            });
        }
    }

    /// Counts this thread's allocations for the run, then restores the previous setting.
    pub struct CountThisThread(bool);
    impl CountThisThread {
        pub fn on() -> Self {
            CountThisThread(crate::counting_alloc::COUNT_THIS_THREAD.with(|c| c.replace(true)))
        }
    }
    impl Drop for CountThisThread {
        fn drop(&mut self) {
            crate::counting_alloc::COUNT_THIS_THREAD.with(|c| c.set(self.0));
        }
    }
}

pub async fn run<B: HostBridge>(code: &str, bridge: &B) -> Result<Value, String> {
    #[cfg(test)]
    let _turn = test_serial::Turn::take();
    #[cfg(test)]
    let _count = test_serial::CountThisThread::on();
    let mut out = Printed::default();
    let result = drive(code, bridge, &mut out).await;
    for line in out.text.lines() {
        bridge.log(line.to_string());
    }
    result
}

/// Block time for the script's clock, to the millisecond as JavaScript sees it.
fn fixed_datetime(ms: u64) -> DateTimeSource {
    DateTimeSource::Fixed { unix_seconds: (ms / 1000) as i64, microsecond: ((ms % 1000) * 1000) as u32 }
}

async fn drive<B: HostBridge>(code: &str, bridge: &B, out: &mut Printed) -> Result<Value, String> {
    // Bounded like JS error text: a raised message can be as large as the heap allows.
    let err = |e: MontyException| {
        let mut s = e.to_string();
        crate::bridge::truncate(&mut s, 8192);
        s
    };
    let started = (!cfg!(target_arch = "wasm32")).then(std::time::Instant::now);
    let policy = OsPolicy {
        // Determinism: block time, no sleeping, seeded `random`.
        datetime: fixed_datetime(now_ms()),
        sleep: SleepMode::Zero,
        random_start: RandomStart::Seed(RandomSeed::Int(0x1C9.into())),
        ..OsPolicy::default()
    };
    if let Some(w) = uses_dynamic_code(code) {
        return Err(format!("`{w}` is not available in the sandbox (dynamic code evaluation is disabled)"));
    }
    let limits = ResourceLimits::default()
        .max_recursion_depth(MAX_RECURSION_DEPTH)
        .max_suspensions(MAX_SUSPENSIONS)
        .max_memory(MAX_MEMORY_BYTES);
    let mut tracker = ResourceTracker::new(limits);
    allow(&mut tracker, bridge, started);
    // max_memory counts LIVE_MEMORY - BASELINE_MEMORY (counting allocator in lib.rs).
    // The baseline is process-global and other requests run while we await, so it is
    // re-based around every await to keep measuring only this script's heap.
    monty_types::BASELINE_MEMORY.store(monty_types::LIVE_MEMORY.load(Ordering::Relaxed), Ordering::Relaxed);

    let mut progress = MontyRun::new(code.to_owned(), "main.py", vec!["icp".to_owned()], CompileOptions::default())
        .map_err(err)?
        .with_os_policy(policy)
        .start(vec![icp_module()], tracker, PrintWriter::Callback(out))
        .map_err(err)?;

    // In-flight `icp.call_async` requests: (call_id, function, args).
    let mut queued: Vec<(u32, String, Vec<Value>)> = vec![];
    for i in 0..=MAX_SUSPENSIONS {
        // Host work between suspensions (conversions, policy checks) is outside the VM's
        // clock, so the absolute deadline is checked here, uncatchably, and every resume
        // below starts from a fresh allowance (`allow`).
        check_budget(bridge)?;
        let next = match progress {
            RunProgress::Complete(_) if !queued.is_empty() => {
                return Err(format!("{} icp request(s) were started but never awaited, so they were not sent", queued.len()));
            }
            RunProgress::Complete(v) => return to_json(v.as_ref(), 0, &mut ValueBudget::default()),
            // Refuse before the request that would pass the limit, so this error never
            // follows messages that went out.
            _ if i == MAX_SUSPENSIONS => break,
            RunProgress::FunctionCall(mut c) => {
                let mut vb = ValueBudget::default();
                let args: Vec<Value> = c.args.args().map(|a| to_json(a, 0, &mut vb)).collect::<Result<_, _>>()?;
                if c.args.kwargs().next().is_some() {
                    let e = exc(ExcType::TypeError, "icp functions take positional arguments only");
                    allow(c.tracker_mut(), bridge, started);
                    c.resume(ExtFunctionResult::Error(e), PrintWriter::Callback(out))
                } else if c.function_name == "icp.call_async" && !c.allow_eager_await {
                    queued.push((c.call_id, c.function_name.clone(), args));
                    allow(c.tracker_mut(), bridge, started);
                    c.resume_pending(PrintWriter::Callback(out))
                } else {
                    let r = rebased(host(bridge, &c.function_name, &args)).await;
                    allow(c.tracker_mut(), bridge, started);
                    let pw = PrintWriter::Callback(out);
                    if c.allow_eager_await { c.resume_eager(r.map(reply), pw) } else { c.resume(ext(r), pw) }
                }
            }
            RunProgress::ResolveFutures(mut rf) => {
                let batch = std::mem::take(&mut queued);
                if batch.is_empty() {
                    return Err("script is awaiting something that can never complete".into());
                }
                let replies = rebased(futures::future::join_all(batch.iter().map(|(_, n, a)| host(bridge, n, a)))).await;
                let results = batch.iter().zip(replies).map(|((id, ..), r)| (*id, ext(r))).collect();
                allow(rf.tracker_mut(), bridge, started);
                rf.resume(results, PrintWriter::Callback(out))
            }
            RunProgress::NameLookup(mut l) => {
                allow(l.tracker_mut(), bridge, started);
                l.resume(NameLookupResult::Undefined, PrintWriter::Callback(out))
            }
            RunProgress::OsCall(mut c) => {
                let r = match &c.function_call {
                    OsFunctionCall::Import(a) if a.module == "icp" => ExtFunctionResult::Return(icp_module()),
                    other => other.no_handler_result(),
                };
                allow(c.tracker_mut(), bridge, started);
                c.resume(r, PrintWriter::Callback(out))
            }
        };
        progress = next.map_err(err)?;
    }
    Err(format!("script exceeded {MAX_SUSPENSIONS} host suspensions"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::mock::{MockBridge, YieldBridge};
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

    #[test]
    fn values_are_bounded() {
        // Deep data is an error, not a silently truncated result with a marker inside.
        let e = py("x = 1\nfor _ in range(100000):\n    x = [x]\nx").0.unwrap_err();
        assert!(e.contains("nested deeper"), "{e}");
        assert!(py("x = 1\nfor _ in range(100):\n    x = [x]\nx").0.is_ok());
        // One big string referenced many times is small in the VM, huge when copied out.
        assert!(py("s = 'x' * 1000000\n[s] * 300").0.unwrap_err().contains("bytes of text"));
        assert!(py("[0] * 900000").0.unwrap_err().contains("elements"));
        // The same limits apply to call arguments.
        assert!(py(&format!("s = 'x' * 1000000\nicp.call('{L}', 'icrc1_balance_of', [s] * 300)")).0.unwrap_err().contains("bytes of text"));
    }

    #[test]
    fn memory_limit_survives_interleaved_runs() {
        // 2 x 40 MB > 64 MiB, but neither half alone is: it must fail even though run B
        // starts (and resets the shared baseline) while A awaits. Without the rebase in
        // `rebased`, A's first 40 MB would vanish into B's baseline and A would pass.
        let a = format!("x = 'y' * 40000000\nicp.call('{L}', 'icrc1_balance_of', {{'owner': 'aaaaa-aa'}})\nz = 'q' * 40000000\nlen(x) + len(z)");
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

    #[test]
    fn bigint_result_counts_against_value_byte_budget() {
        let e = py("[1 << 30000] * 500").0.unwrap_err();
        assert!(e.contains("bytes of text"), "{e}");
    }

    #[test]
    fn heap_ceiling_bounds_single_builtin_allocations() {
        // Monty checks its memory limit between steps, so one builtin call can overshoot
        // it (here by 70 MB). On-chain, the global heap ceiling bounds that overshoot.
        use crate::counting_alloc::{HEAP_CEILING, over_ceiling};
        assert!(py("import json\nlen(json.dumps({'a': 1}, indent=70_000_000))").0.is_ok());
        assert!(!over_ceiling(HEAP_CEILING - 100, 100));
        assert!(over_ceiling(HEAP_CEILING - 100, 101));
        assert!(over_ceiling(usize::MAX, 1));
    }

    #[test]
    fn audit_round_two_python_fixes() {
        // eval/exec are refused; names that only contain the words are fine.
        assert!(py("exec('x = 1')").0.unwrap_err().contains("not available"));
        assert!(py("eval ('1')").0.unwrap_err().contains("not available"));
        assert_eq!(py("evaluate = 2\nexecutor = 3\nevaluate + executor").0.unwrap(), json!(5));
        // Monty does not fold a fullwidth "eval" into the builtin.
        assert!(py("\u{ff45}val('1')").0.is_err());
        // Output past the cap is cut, not a MemoryError, as in JS and Rhai.
        let (r, logs) = py("for i in range(3000):\n    print('x' * 40)\n1");
        assert_eq!(r.unwrap(), json!(1));
        let logs = logs.logs.take();
        assert!(logs.iter().map(String::len).sum::<usize>() <= crate::bridge::MAX_LOG_BYTES);
        assert!(logs.last().unwrap().ends_with("[truncated]"), "{:?}", logs.last());
        // Huge ints are refused before their decimal text is built (additions can pass
        // the arithmetic cap by a bit per step).
        assert!(py("x = 1 << 32767\nfor _ in range(4):\n    x = x + x\nx").0.unwrap_err().contains("too large"));
        assert_eq!(py("1 << 100").0.unwrap(), json!("1267650600228229401496703205376"));
        // A call started with call_async and never awaited was not sent: an error.
        let e = py(&format!("import asyncio\nicp.call_async('{L}', 'icrc1_decimals')\n1")).0.unwrap_err();
        assert!(e.contains("never awaited") || e.contains("never"), "{e}");
    }

    /// Replies of 1 MB each, built during the call from buffers allocated before the run
    /// (so freeing a buffer credits the run, as freeing the IC's reply buffer does
    /// on-chain), each call finishing after a different number of polls.
    struct BigReplyBridge {
        bufs: std::cell::RefCell<Vec<Vec<u8>>>,
        n: std::cell::Cell<usize>,
    }
    impl BigReplyBridge {
        fn new(n: usize) -> Self {
            Self { bufs: std::cell::RefCell::new(vec![vec![b'r'; 1 << 20]; n]), n: Default::default() }
        }
    }
    struct Yield(usize);
    impl std::future::Future for Yield {
        type Output = ();
        fn poll(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<()> {
            if self.0 == 0 {
                return std::task::Poll::Ready(());
            }
            self.0 -= 1;
            cx.waker().wake_by_ref();
            std::task::Poll::Pending
        }
    }
    impl HostBridge for BigReplyBridge {
        async fn candid(&self, _: &str) -> Result<String, String> {
            Err("no".into())
        }
        async fn call(&self, _: &str, _: &str, _: Value) -> Result<Value, String> {
            let k = self.n.get();
            self.n.set(k + 1);
            Yield(k % 3 + 1).await;
            let buf = self.bufs.borrow_mut().pop().unwrap_or_default();
            let s = String::from_utf8_lossy(&buf).into_owned();
            drop(buf);
            Ok(Value::String(s))
        }
        fn principal(&self) -> String {
            "aaaaa-aa".into()
        }
        fn log(&self, _: String) {}
    }

    #[test]
    fn kept_replies_count_against_memory() {
        // The buffers are counted heap, as everything is on-chain; hold the test lock so
        // no other test's run sees them.
        let _turn = test_serial::Turn::take();
        let _count = test_serial::CountThisThread::on();
        let seq = format!("xs = []\nfor i in range(200):\n    xs.append(icp.call('{L}', 'm'))\nlen(xs)");
        let e = block_on(run(&seq, &BigReplyBridge::new(200))).unwrap_err();
        assert!(e.contains("MemoryError"), "{e}");
        let gather = format!(
            "import asyncio\nxs = await asyncio.gather(*[icp.call_async('{L}', 'm') for i in range(200)])\nlen(xs)"
        );
        let e = block_on(run(&gather, &BigReplyBridge::new(200))).unwrap_err();
        assert!(e.contains("MemoryError"), "{e}");
        // A few replies are fine.
        let few = format!("xs = [icp.call('{L}', 'm') for i in range(5)]\nlen(xs[4])");
        assert_eq!(block_on(run(&few, &BigReplyBridge::new(5))).unwrap(), json!(1 << 20));
    }

    /// A bridge whose calls spend 1.5 s of host time.
    struct SlowBridge(MockBridge);
    impl HostBridge for SlowBridge {
        async fn candid(&self, id: &str) -> Result<String, String> {
            self.0.candid(id).await
        }
        async fn call(&self, id: &str, m: &str, a: Value) -> Result<Value, String> {
            std::thread::sleep(Duration::from_millis(1500));
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
    fn host_time_is_deducted_from_the_vm_budget() {
        // Monty's clock stops while the run is suspended: with a one-time relative
        // allowance the loop below would get the full 2 s after the 1.5 s call.
        let _turn = test_serial::Turn::take(); // start the clock after the test lock
        let t0 = std::time::Instant::now();
        let r = block_on(run(&format!("icp.call('{L}', 'icrc1_balance_of')\nwhile True:\n    pass"), &SlowBridge(MockBridge::default())));
        assert!(r.unwrap_err().contains("TimeoutError"));
        assert!(t0.elapsed() < EXECUTION_BUDGET + Duration::from_millis(500), "{:?}", t0.elapsed());
    }

    #[test]
    fn block_time_keeps_milliseconds() {
        let DateTimeSource::Fixed { unix_seconds, microsecond } = fixed_datetime(1_794_911_052_517) else {
            panic!("not a fixed clock")
        };
        assert_eq!((unix_seconds, microsecond), (1_794_911_052, 517_000));
    }

    #[test]
    fn print_past_the_cap_is_not_formatted() {
        // Once output is cut, `print` skips formatting: 2,000 prints of a ~600 KB list
        // would otherwise run far past the 2 s budget.
        let (r, b) = py("print('x' * 70000)\nbig = list(range(100000))\nfor i in range(2000):\n    print(big)\n1");
        assert_eq!(r.unwrap(), json!(1));
        assert!(b.logs.take().last().unwrap().ends_with("[truncated]"));
        // Empty `end`, `sep` or argument text is not the probe: the print that crosses
        // the cap, and later ones, still succeed.
        for code in ["print('x' * 70000, end='')\n1", "print('x' * 65000)\nprint('y' * 1000, '')\nprint('', sep='', end='')\n1"] {
            let (r, b) = py(code);
            assert_eq!(r.unwrap(), json!(1), "{code}");
            assert!(b.logs.take().last().unwrap().ends_with("[truncated]"), "{code}");
        }
        let (r, b) = py("print('a', end='')\nprint('', 'b', sep='')\n1");
        assert_eq!(r.unwrap(), json!(1));
        assert_eq!(b.logs.take(), vec!["ab".to_string()]);
    }

    #[test]
    fn exports_dispatch_on_the_value_kind() {
        // Classes named after builtins are still class instances: refused, not exported
        // as "", false, null, [] or {}.
        for c in ["str", "bool", "float", "list", "dict", "NoneType"] {
            let e = py(&format!("class {c}:\n    pass\n{c}()")).0.unwrap_err();
            assert!(e.contains("cannot leave the sandbox"), "{c}: {e}");
        }
        let (r, b) = py(&format!("class bool:\n    pass\nicp.call('{L}', 'icrc1_balance_of', bool())"));
        assert!(r.is_err());
        assert!(b.calls.borrow().is_empty());
        assert!(py("range(3)").0.unwrap_err().contains("range(0, 3) cannot leave"));
        assert!(py("[ValueError('x' * 1000000)] * 20").0.unwrap_err().contains("cannot leave"));
        // Bytes are charged one element per byte.
        assert!(py("b'a' * 4000000").0.unwrap_err().contains("elements"));
        // Keys that collide once turned into text, and keys that are not str or int.
        assert!(py("{1: 'a', '1': 'b'}").0.unwrap_err().contains("two keys"));
        assert!(py("{(1, 2): 1}").0.unwrap_err().contains("dict keys"));
        assert!(py("{None: 1}").0.unwrap_err().contains("dict keys"));
        assert_eq!(py("{True: 1, 'a': 2}").0.unwrap(), json!({"true": 1, "a": 2}));
        assert!(py("{False: 1, 'false': 2}").0.unwrap_err().contains("two keys"));
        assert_eq!(py("{1: 'a', 'b': 2}").0.unwrap(), json!({"1": "a", "b": 2}));
        // A class instance over a DAG that doubles at each level is refused at once, not
        // rendered 2^26 times; so is one holding a cycle.
        let turn = test_serial::Turn::take(); // start the clock after the test lock
        let t0 = std::time::Instant::now();
        let dag = "a = [1]\nfor _ in range(26):\n    a = [a, a]\nclass C:\n    pass\nc = C()\nc.v = a\n";
        assert!(py(&format!("{dag}c")).0.is_err());
        let (r, b) = py(&format!("{dag}icp.call('{L}', 'icrc1_balance_of', c)"));
        assert!(r.is_err());
        assert!(b.calls.borrow().is_empty());
        assert!(t0.elapsed() < Duration::from_secs(1), "{:?}", t0.elapsed());
        drop(turn);
        assert!(py("l = ['x']\nl.append(l)\nclass C:\n    pass\nc = C()\nc.l = l\nc").0.is_err());
        // namedtuples are tuples.
        assert_eq!(py("from collections import namedtuple\nP = namedtuple('P', 'x y')\nP(1, 2)").0.unwrap(), json!([1, 2]));
    }

    #[test]
    fn deep_call_arguments_are_exact_or_refused() {
        // Monty cuts values exported from deep in the call stack and leaves markers; a
        // marker must never reach the host as text, in a key or in an object.
        for n in 185..=198 {
            for arg in ["{(1, (2, (3, (4, (5,))))): 1}", "C()"] {
                let code = format!(
                    "class C:\n    def __init__(self):\n        self.v = [[[[1]]]]\n\
                     def f(n):\n    if n == 0:\n        return icp.call('{L}', 'icrc1_balance_of', {arg})\n    return f(n - 1)\nf({n})"
                );
                let (r, b) = py(&code);
                let calls = b.calls.borrow();
                assert!(calls.iter().all(|c| !c.2.to_string().contains("nested")), "{n} {arg}: {calls:?}");
                if r.is_err() {
                    assert!(calls.is_empty(), "{n} {arg}: {calls:?}");
                }
            }
        }
    }

    #[test]
    fn engine_work_is_bounded() {
        // Big-integer growth is capped, and so are the operations Monty runs on
        // operands that never went through its growth checks.
        let cap = |code: &str| {
            let e = py(code).0.unwrap_err();
            assert!(e.contains("sandbox limit"), "{code}: {e}");
        };
        cap("3 ** 40000");
        cap("x = 1 << 40000");
        cap("(1 << 20000) * (1 << 20000)");
        cap("int('f' * 10000, 16)");
        cap("import math\nx = 1 << 32767\nfor _ in range(4):\n    x = x + x\nmath.gcd(x, 3)");
        cap("import math\nx = 1 << 32767\nfor _ in range(4):\n    x = x + x\nmath.isqrt(x)");
        cap("x = 1 << 32767\nfor _ in range(4):\n    x = x + x\nx % 7");
        cap("x = 1 << 32767\nfor _ in range(4):\n    x = x + x\npow(3, 5, x)");
        assert_eq!(py("len(str(2 ** 10000))").0.unwrap(), json!(3011));
        // Decimal precision and the digits one operation may need.
        assert!(py("import decimal\ndecimal.getcontext().prec = 100000").0.unwrap_err().contains("prec"));
        assert!(py("import decimal\nint(decimal.Decimal('1e100000'))").0.unwrap_err().contains("sandbox limit"));
        // Backtracking regexes take a bounded subject; linear ones any.
        let e = py("import re\nre.match('a*(?=[^x]*$)', 'a' * 20000 + 'x')").0.unwrap_err();
        assert!(e.contains("lookaround"), "{e}");
        assert_eq!(py("import re\nre.search('b', 'a' * 100000) is None").0.unwrap(), json!(true));
        // Text compiled at run time is capped; ForwardRef.evaluate is refused like eval.
        assert!(py("import typing\ntyping.ForwardRef('6*7').evaluate()").0.unwrap_err().contains("not available"));
        let e = py("import typing\ns = '1+' * 10000 + '1'\ndef f(x: typing.ForwardRef(s)): pass\ntyping.get_type_hints(f)").0.unwrap_err();
        assert!(e.contains("sandbox limit"), "{e}");
    }

    #[test]
    fn expensive_operations_poll_the_time_limit() {
        // The VM polls time every 255 bytecodes; a loop of costly native operations ran a
        // few dozen of them past the limit before the patched Monty polled in each one.
        for code in [
            "import math\nx = (1 << 32000) | 1\ny = (1 << 31000) | 3\nwhile True:\n    math.gcd(x, y)",
            "class K:\n    def __hash__(self):\n        return 0\nk = [K() for _ in range(3000)]\nwhile True:\n    set(k)",
            // Each match of a backtracking regex is a fresh VM run with a fresh backtrack
            // allowance: one `sub` on the 5,000-byte cap ran ~6 s natively unpolled, and
            // a loop of `findall` on short subjects as long.
            "import re\nwhile True:\n    re.sub('a*(?=[^x]*$)x|.', '', 'a' * 4999)",
            "import re\nwhile True:\n    re.findall('a*(?=[^x]*$)x|.', 'a' * 1000)",
        ] {
            let _turn = test_serial::Turn::take(); // start the clock after the test lock
            let t0 = std::time::Instant::now();
            let e = py(code).0.unwrap_err();
            assert!(e.contains("TimeoutError"), "{e}");
            assert!(t0.elapsed() < EXECUTION_BUDGET + Duration::from_millis(700), "{code}: {:?}", t0.elapsed());
        }
    }

    #[test]
    fn raised_error_text_is_bounded() {
        let e = py("raise ValueError('x' * (1 << 24))").0.unwrap_err();
        assert!(e.len() < 8300 && e.contains("ValueError"), "{}", e.len());
    }

    #[test]
    fn python_run_resets_test_thread_allocation_flag() {
        crate::counting_alloc::COUNT_THIS_THREAD.with(|c| c.set(false));
        assert_eq!(py("1").0.unwrap(), json!(1));
        assert!(!crate::counting_alloc::COUNT_THIS_THREAD.with(|c| c.get()));
    }

    #[test]
    fn arity_suspension_limit_and_json_conversion_edge_cases() {
        assert!(py("icp.candid()").0.unwrap_err().contains("TypeError"));
        assert!(py("icp.principal(1)").0.unwrap_err().contains("no arguments"));
        assert!(py(&format!("icp.call('{L}', 'icrc1_balance_of')")).0.is_ok());

        // Unresolvable async gather / non-object repr fallback.
        let e = py("slice(1, 2)").0.unwrap_err();
        assert!(e.contains("slice(1, 2, None) cannot leave"), "{e}");
        let too_many = format!(
            "for _ in range(257):\n    icp.call('{L}', 'icrc1_balance_of', {{'owner': 'aaaaa-aa'}})"
        );
        assert!(py(&too_many).0.unwrap_err().contains("suspensions"));

        // Exercise `from_json` with booleans, null, large u64, floats, arrays.
        assert_eq!(
            from_json(&json!([null, true, u64::MAX, 1.25])),
            MontyObject::list([
                MontyObject::none(),
                MontyObject::bool(true),
                MontyObject::bigint(u64::MAX.into()),
                MontyObject::float(1.25),
            ])
        );
    }
}
