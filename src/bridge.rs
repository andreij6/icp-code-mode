//! The language-agnostic host surface every runtime calls into.
//!
//! All three engines (Boa, Monty, Rhai) see exactly the same `icp` SDK, so policy
//! enforcement, Candid encoding and audit bookkeeping live here once.
//! JSON <-> Candid value conversion lives in `candid_bridge`.

use crate::candid_bridge::Service;
use serde_json::Value;

/// Cap on captured `print`/`console.log` output per script.
pub const MAX_LOG_BYTES: usize = 64 * 1024;

#[allow(async_fn_in_trait)] // single-threaded canister executor; futures need not be Send
pub trait HostBridge {
    /// The target canister's Candid interface (`.did` text).
    async fn candid(&self, canister_id: &str) -> Result<String, String>;
    /// Policy-checked, Candid-encoded inter-canister call. Args/reply are JSON.
    async fn call(&self, canister_id: &str, method: &str, args: Value) -> Result<Value, String>;
    /// This canister's own principal.
    fn principal(&self) -> String;
    /// Captured script output (`print`, `console.log`).
    fn log(&self, line: String);
    /// Absolute `instruction_counter()` value past which a runtime must stop the script
    /// with a clean error (before the IC's own 40B trap discards the audit entry).
    fn instruction_deadline(&self) -> u64 {
        u64::MAX
    }
}

/// Runtimes call this between steps; `Err` once the script's instruction budget is spent.
pub fn check_budget<B: HostBridge + ?Sized>(bridge: &B) -> Result<(), String> {
    budget_error(bridge.instruction_deadline())
}

/// Same check from a captured deadline (for engines whose callbacks cannot hold `&B`).
pub fn budget_error(deadline: u64) -> Result<(), String> {
    match instruction_counter() {
        n if n > deadline => Err("policy: instruction budget exceeded".into()),
        _ => Ok(()),
    }
}

/// Limits on any value crossing between a script and the host (results, call arguments,
/// printed values), shared by all three runtimes. Depth and element count alone are not
/// enough: one 1 MB string referenced 5,000 times is a small value inside the engine and a
/// 5 GB one once copied out.
pub const MAX_VALUE_NODES: usize = 200_000;
pub const MAX_VALUE_BYTES: usize = 4 << 20;

/// Running totals for one conversion.
#[derive(Default)]
pub struct ValueBudget {
    nodes: usize,
    bytes: usize,
}

impl ValueBudget {
    /// Count one value (of any kind).
    pub fn node(&mut self) -> Result<(), String> {
        self.nodes += 1;
        if self.nodes > MAX_VALUE_NODES {
            return Err(format!("value has more than {MAX_VALUE_NODES} elements"));
        }
        Ok(())
    }
    /// Count string/key/blob payload.
    pub fn bytes(&mut self, n: usize) -> Result<(), String> {
        self.bytes += n;
        if self.bytes > MAX_VALUE_BYTES {
            return Err(format!("value holds more than {MAX_VALUE_BYTES} bytes of text"));
        }
        Ok(())
    }
}

/// Deterministic "now" for script clocks: block time on-chain, 0 off-chain (tests).
pub fn now_ms() -> u64 {
    #[cfg(target_arch = "wasm32")]
    return ic_cdk::api::time() / 1_000_000;
    #[cfg(not(target_arch = "wasm32"))]
    0
}

/// JS engines hand back every number as f64; turn integral values in the
/// JS-safe range back into JSON integers so `3` doesn't become `3.0`.
pub fn normalize_numbers(v: Value) -> Value {
    match v {
        Value::Number(n) if !n.is_i64() && !n.is_u64() => match n.as_f64() {
            Some(f) if f.fract() == 0.0 && f.abs() <= 9_007_199_254_740_991.0 => Value::from(f as i64),
            _ => Value::Number(n),
        },
        Value::Array(a) => Value::Array(a.into_iter().map(normalize_numbers).collect()),
        Value::Object(o) => Value::Object(o.into_iter().map(|(k, v)| (k, normalize_numbers(v))).collect()),
        v => v,
    }
}

/// Append-only, size-capped log buffer shared by bridge implementations.
#[derive(Default)]
pub struct LogBuf {
    lines: std::cell::RefCell<Vec<String>>,
    bytes: std::cell::Cell<usize>,
}

impl LogBuf {
    pub fn push(&self, line: String) {
        let used = self.bytes.get();
        if used >= MAX_LOG_BYTES {
            return;
        }
        let mut lines = self.lines.borrow_mut();
        if used + line.len() > MAX_LOG_BYTES {
            lines.push("…[output truncated]".into());
            self.bytes.set(MAX_LOG_BYTES);
        } else {
            self.bytes.set(used + line.len());
            lines.push(line);
        }
    }
    pub fn take(&self) -> Vec<String> {
        std::mem::take(&mut self.lines.borrow_mut())
    }
}

/// Longest method name a script may pass (keeps policy messages small).
pub const MAX_METHOD_LEN: usize = 256;
/// Largest encoded argument sent (the IC's limit for a message to another subnet).
pub const MAX_ARG_BYTES: usize = 2_000_000;

/// UTF-8 bytes of text in `v` (strings and object keys), the measure `MAX_VALUE_BYTES` uses.
pub fn text_bytes(v: &Value) -> usize {
    match v {
        Value::String(s) => s.len(),
        Value::Array(a) => a.iter().map(text_bytes).sum(),
        Value::Object(o) => o.iter().map(|(k, v)| k.len() + text_bytes(v)).sum(),
        _ => 0,
    }
}

pub use ic::{IcBridge, MAX_DID_BYTES, MAX_RUN_SECONDS, cached_service, evict_interface, pending_run_ids, remember_override};

/// Whether a refusal from an interface fetch would refuse every further fetch of the run too
/// (call cap, budget, stopping, revoked token, time limit, in-flight or cycles limit), as
/// opposed to one about a single target (not allowlisted). A search stops fetching on the
/// former only.
pub fn refuses_whole_run(e: &str) -> bool {
    e.starts_with("policy:") && !e.contains("is not allowlisted")
}

/// Instructions executed so far in this call context (0 off-chain).
pub fn instruction_counter() -> u64 {
    #[cfg(target_arch = "wasm32")]
    return ic_cdk::api::call_context_instruction_counter();
    #[cfg(not(target_arch = "wasm32"))]
    0
}

/// Await `fut`, calling `f` once at the first moment it is found pending (never, if it is
/// ready on its first poll).
pub async fn on_first_pending<T>(fut: impl std::future::Future<Output = T>, f: impl FnOnce()) -> T {
    let mut fut = std::pin::pin!(fut);
    let mut f = Some(f);
    std::future::poll_fn(|cx| {
        let r = fut.as_mut().poll(cx);
        if r.is_pending()
            && let Some(f) = f.take() {
                f();
            }
        r
    })
    .await
}

/// Truncate to at most `max` bytes on a char boundary, marking the cut.
pub fn truncate(s: &mut String, max: usize) {
    if s.len() > max {
        let mut cut = max;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
        s.push_str("…[truncated]");
    }
}

/// Production bridge. Compiles natively too (ic0 stubs panic off-chain; the few system
/// reads below have native stand-ins), which lets `export_candid!` and tests build without
/// a replica.
mod ic {
    use super::*;
    use crate::candid_bridge::EncodedCall;
    use crate::policy::{self, AuditEntry, AuditStatus, Limits};
    use candid::types::FuncMode;
    use candid::{CandidType, Deserialize, Principal};
    use ic_cdk::call::{Call, CallErrorExt, CallFailed, RejectCode};
    use std::cell::{Cell, RefCell};
    use std::collections::{BTreeMap, BTreeSet};
    use std::future::IntoFuture;
    use std::rc::Rc;
    use std::task::{Poll, Waker};

    /// How long a fetched interface is trusted before it is fetched again.
    const DID_CACHE_TTL_NS: u64 = 3600 * 1_000_000_000;
    /// How long a failed fetch is remembered. Without this, every search or script would
    /// repeat the same outbound calls to a target that exposes no interface.
    const DID_FAIL_TTL_NS: u64 = 300 * 1_000_000_000;
    /// Largest interface accepted from a target or as an override (the ICP ledger's is
    /// 17 KB). Parsing megabytes of hostile Candid would only burn instructions.
    pub const MAX_DID_BYTES: usize = 512 * 1024;
    /// Violations kept per script (the first and the latest half each); the rest are only
    /// counted.
    const MAX_VIOLATIONS: usize = 16;
    const MAX_VIOLATION_BYTES: usize = 512;
    /// Outbound messages awaiting a reply, over all runs and searches. Each one holds cycles
    /// reserved for its reply until it completes.
    const MAX_IN_FLIGHT: u32 = 64;
    /// Liquid cycles below which no new message is sent, so callers get a clear refusal
    /// instead of "call perform failed" while the canister still reports Running.
    // ponytail: fixed floor of about two calls' reply reservation; make it a Limits field
    // if operators need to tune it.
    const MIN_LIQUID_CYCLES: u128 = 100_000_000_000;
    /// Wall-clock limit for one run: no message is sent after it, and each message's
    /// timeout is cut to what is left. Bounds how long a run holds its slot.
    pub const MAX_RUN_SECONDS: u64 = 600;

    /// (.did text, parsed service, fetch time in ns).
    type Fetched = (String, Rc<Service>, u64);

    thread_local! {
        // ponytail: heap cache keyed by canonical principal text, emptied on upgrade
        // (re-fetched on demand); bounded by the allowlist size.
        static DID_CACHE: RefCell<BTreeMap<String, Fetched>> = RefCell::default();
        // Runs that wrote a `Started` entry and have not ended. Lets an auditor tell a run
        // still waiting on a reply from one that ended without a final entry.
        static PENDING_RUNS: RefCell<BTreeSet<u64>> = RefCell::default();
        // Recent fetch failures: (error, time in ns).
        static DID_FAILED: RefCell<BTreeMap<String, (String, u64)>> = RefCell::default();
        // Parsed admin overrides: (.did text it was parsed from, service).
        static OVERRIDES: RefCell<BTreeMap<String, (String, Rc<Service>)>> = RefCell::default();
        // Messages sent and not completed (see MAX_IN_FLIGHT).
        static IN_FLIGHT: Cell<u32> = const { Cell::new(0) };
        #[cfg(not(target_arch = "wasm32"))]
        static TEST_NOW: Cell<u64> = const { Cell::new(0) };
    }

    /// Block time on-chain; a settable clock in native tests.
    fn now_ns() -> u64 {
        #[cfg(target_arch = "wasm32")]
        return ic_cdk::api::time();
        #[cfg(not(target_arch = "wasm32"))]
        TEST_NOW.with(Cell::get)
    }

    fn running() -> bool {
        #[cfg(target_arch = "wasm32")]
        return ic_cdk::api::canister_status() == ic_cdk::api::CanisterStatusCode::Running;
        #[cfg(not(target_arch = "wasm32"))]
        true
    }

    fn liquid_cycles() -> u128 {
        #[cfg(target_arch = "wasm32")]
        return ic_cdk::api::canister_liquid_cycle_balance();
        #[cfg(not(target_arch = "wasm32"))]
        u128::MAX
    }

    /// What is known about `p`'s interface without any outbound message: an admin
    /// override, an earlier fetch, or a recent failure (`Some(Err)`). `None` means it has
    /// to be fetched, or (with `parse` false, on the query path, which must not spend its
    /// smaller instruction limit parsing) that an override still has to be parsed.
    pub fn cached_service(p: &Principal, parse: bool) -> Option<Result<Rc<Service>, String>> {
        let key = p.to_text();
        match override_service(&key, parse) {
            Ok(Some(r)) => return Some(r),
            Ok(None) => {}
            Err(()) => return None,
        }
        if let Some((_, s)) = cache_get(&key, DID_CACHE_TTL_NS) {
            return Some(Ok(s));
        }
        failed_get(&key).map(Err)
    }

    /// Keep an override that was just parsed (by `set_candid_override`, in an update).
    pub fn remember_override(p: &Principal, did: String, svc: Service) {
        OVERRIDES.with(|c| c.borrow_mut().insert(p.to_text(), (did, Rc::new(svc))));
    }

    /// The admin override for `key`, parsed once and reused until its text changes.
    /// `Ok(None)`: no override. `Err(())`: one exists but is not parsed and `parse` is off.
    fn override_service(key: &str, parse: bool) -> Result<Option<Result<Rc<Service>, String>>, ()> {
        let Some(did) = policy::candid_override(key) else { return Ok(None) };
        let hit = OVERRIDES.with(|c| c.borrow().get(key).filter(|(d, _)| *d == did).map(|(_, s)| s.clone()));
        Ok(Some(match hit {
            Some(s) => Ok(s),
            None if !parse => return Err(()),
            None => Service::parse(&did).map(|s| {
                let s = Rc::new(s);
                OVERRIDES.with(|c| c.borrow_mut().insert(key.to_string(), (did, s.clone())));
                s
            }),
        }))
    }

    /// Run ids (see `AuditEntry::run_id`) with outbound messages sent and no end yet. Kept
    /// in memory: a run leaves the list when it finishes or when its trap is cleaned up,
    /// and the list is empty after an upgrade.
    pub fn pending_run_ids() -> Vec<u64> {
        PENDING_RUNS.with(|r| r.borrow().iter().copied().collect())
    }

    /// The run is over, however it ended: normally, or by ic-cdk dropping the future while
    /// cleaning up after a trap.
    ///
    /// In the trap case this runs in the cleanup callback, whose state changes are kept. The
    /// final entry the run would have written was rolled back with the trapped execution,
    /// so a `Trapped` entry closes the run instead. Memory is back to how it was when the
    /// last message was sent, so `methods` and `violations` are complete up to that point.
    impl Drop for IcBridge {
        fn drop(&mut self) {
            let Some(id) = self.started.get() else { return };
            PENDING_RUNS.with(|r| r.borrow_mut().remove(&id));
            if self.finished.get() || !ic_cdk::futures::is_recovering_from_trap() {
                return;
            }
            let (caller, language, sha) = self.audit_key.clone();
            policy::append_audit(AuditEntry {
                id: 0,
                timestamp_ns: now_ns(),
                caller,
                language,
                script_sha256: sha,
                canisters: self.touched.borrow().iter().cloned().collect(),
                calls: self.calls(),
                instructions: self.instructions_used(),
                status: AuditStatus::Trapped,
                violations: self.closing_violations(),
                error: Some("the canister trapped while handling a reply (instruction or memory limit, or a bug); the script's result was discarded".into()),
                run_id: Some(id),
                methods: Some(self.messages()),
            });
        }
    }

    /// Forget everything cached about a canister (called when it leaves the allowlist).
    pub fn evict_interface(p: &Principal) {
        let key = p.to_text();
        DID_CACHE.with(|c| c.borrow_mut().remove(&key));
        DID_FAILED.with(|c| c.borrow_mut().remove(&key));
        OVERRIDES.with(|c| c.borrow_mut().remove(&key));
    }

    fn failed_get(key: &str) -> Option<String> {
        let now = now_ns();
        DID_FAILED.with(|c| c.borrow().get(key).filter(|(_, at)| now.saturating_sub(*at) <= DID_FAIL_TTL_NS).map(|(e, _)| e.clone()))
    }

    /// A cached interface at most `max_age_ns` old. Callers pass `DID_CACHE_TTL_NS` (an
    /// older one is re-fetched, so a target's interface change is picked up without an
    /// upgrade here), or `u64::MAX` for the copy kept when a re-fetch failed.
    fn cache_get(key: &str, max_age_ns: u64) -> Option<(String, Rc<Service>)> {
        let now = now_ns();
        DID_CACHE.with(|c| c.borrow().get(key).filter(|(_, _, at)| now.saturating_sub(*at) <= max_age_ns).map(|(d, s, _)| (d.clone(), s.clone())))
    }

    /// Seconds the next message may wait for its reply: `want`, cut to what is left of the
    /// run's `MAX_RUN_SECONDS`. `Err` once nothing is left.
    fn timeout_left(started_ns: u64, now_ns: u64, want: u32) -> Result<u32, String> {
        let end = started_ns.saturating_add(MAX_RUN_SECONDS * 1_000_000_000);
        match end.saturating_sub(now_ns) / 1_000_000_000 {
            0 => Err(format!("policy: the run reached its {MAX_RUN_SECONDS}-second time limit; no new calls")),
            left => Ok(want.min(left as u32)),
        }
    }

    /// Whether a rejected interface request says something lasting about the target (no
    /// such method, no such canister), so the failure may be remembered. A stopped or
    /// out-of-cycles target, a transient or unknown outcome (timeout) does not.
    fn definitive(code: Option<RejectCode>, msg: &str) -> bool {
        match code {
            Some(RejectCode::CanisterReject | RejectCode::DestinationInvalid) => true,
            Some(RejectCode::CanisterError) => !["stopped", "stopping", "out of cycles"].iter().any(|w| msg.contains(w)),
            _ => false,
        }
    }

    fn definitive_failure(e: &CallFailed) -> bool {
        match e {
            CallFailed::CallRejected(r) => definitive(r.reject_code().ok(), r.reject_message()),
            // Refused here before sending (cycles, queue): nothing about the target.
            _ => false,
        }
    }

    /// The error for a reply that arrived but could not be decoded: the call ran on the
    /// target, so the script must not treat it as not sent.
    fn reply_decode_error(p: &Principal, method: &str, e: &str) -> String {
        format!(
            "call {p}.{} executed (the target replied), but its reply could not be decoded: {e}; the cached interface was dropped, so the next call fetches it again",
            method.escape_debug()
        )
    }

    /// Encode a call, refusing a composite_query before any argument work.
    fn encode_call(svc: &Service, method: &str, args: &Value) -> Result<EncodedCall, String> {
        if svc.method(method)?.modes.contains(&FuncMode::CompositeQuery) {
            return Err(format!("`{}` is a composite_query; it cannot be called from an update", method.escape_debug()));
        }
        svc.encode_args(method, args)
    }

    /// A fetch that ended without an interface. A failure that says something lasting
    /// about the target is remembered; otherwise (a refusal here, a transient reject, a
    /// timeout) an expired copy is served if there is one.
    fn fetch_failed(key: &str, e: String, remember: bool) -> Result<String, String> {
        let msg = format!("could not fetch candid for {key} ({e}); a controller can register it with set_candid_override");
        if remember {
            DID_FAILED.with(|c| c.borrow_mut().insert(key.to_string(), (msg.clone(), now_ns())));
            return Err(msg);
        }
        if let Some((did, _)) = cache_get(key, u64::MAX) {
            return Ok(did);
        }
        Err(if e.starts_with("policy:") { e } else { msg })
    }

    /// Production bridge: one instance per `tools/call`, so counters start at zero.
    pub struct IcBridge {
        /// Call-cap slots taken (attempts that passed policy), whether or not a message
        /// ended up being sent.
        slots: Cell<u32>,
        /// Set once the run's final audit entry is written.
        finished: Cell<bool>,
        base_instructions: u64,
        /// The limits in force when the run started, used for the whole run.
        limits: Limits,
        /// Full hash of the run's bearer token, checked again before every message.
        token: String,
        started_ns: u64,
        /// (caller, language, script sha) for the provisional audit entry.
        audit_key: (String, String, String),
        /// Id of this run's `Started` audit entry, once one is written.
        started: Cell<Option<u64>>,
        /// Every outbound message so far, as `canister.method`, in order.
        messages: RefCell<Vec<String>>,
        /// Interfaces this run is fetching, with the run's requests waiting for each.
        fetching: RefCell<BTreeMap<String, Vec<Waker>>>,
        pub touched: RefCell<BTreeSet<String>>,
        pub violations: RefCell<Vec<String>>,
        pub violations_dropped: Cell<u32>,
        pub logs: LogBuf,
    }

    /// Marks an interface as being fetched by this run until dropped. Dropped normally, it
    /// wakes the run's other requests for it. Dropped in trap cleanup (for example decoding
    /// or parsing the reply ran out of instructions), it records a failure, which the trap
    /// would otherwise have rolled back, so the next request does not send the same
    /// messages and trap again. Any trap of the run while the fetch is out counts.
    struct Fetching<'a> {
        bridge: &'a IcBridge,
        key: String,
    }

    impl<'a> Fetching<'a> {
        fn start(bridge: &'a IcBridge, key: &str) -> Self {
            bridge.fetching.borrow_mut().insert(key.to_string(), vec![]);
            Self { bridge, key: key.to_string() }
        }
    }

    impl Drop for Fetching<'_> {
        fn drop(&mut self) {
            let waiting = self.bridge.fetching.borrow_mut().remove(&self.key);
            waiting.into_iter().flatten().for_each(Waker::wake);
            if ic_cdk::futures::is_recovering_from_trap() {
                let msg = format!("could not fetch candid for {} (handling its reply trapped); a controller can register it with set_candid_override", self.key);
                DID_FAILED.with(|c| c.borrow_mut().insert(self.key.clone(), (msg, now_ns())));
            }
        }
    }

    impl IcBridge {
        /// `token_hash`: the full SHA-256 of the run's bearer token (its first 16
        /// characters go in the audit log).
        pub fn new(token_hash: &str, language: &str, script_sha256: String) -> Self {
            Self {
                slots: Cell::new(0),
                finished: Cell::new(false),
                base_instructions: instruction_counter(),
                limits: policy::limits(),
                token: token_hash.to_string(),
                started_ns: now_ns(),
                audit_key: (token_hash.get(..16).unwrap_or(token_hash).to_string(), language.to_string(), script_sha256),
                started: Cell::new(None),
                messages: RefCell::default(),
                fetching: RefCell::default(),
                touched: RefCell::default(),
                violations: RefCell::default(),
                violations_dropped: Cell::new(0),
                logs: LogBuf::default(),
            }
        }

        /// The limits this run started with; a `set_limits` during the run does not apply to it.
        pub fn limits(&self) -> &Limits {
            &self.limits
        }

        /// Messages actually sent so far.
        pub fn calls(&self) -> u32 {
            self.messages.borrow().len() as u32
        }

        /// The final audit entry is written; nothing more to record on drop.
        pub fn finish(&self) {
            self.finished.set(true);
        }

        /// Outbound messages so far (see `AuditEntry::methods`).
        pub fn messages(&self) -> Vec<String> {
            self.messages.borrow().clone()
        }

        /// The run this script's final audit entry belongs to (see `AuditEntry::run_id`).
        pub fn run_id(&self) -> Option<u64> {
            self.started.get()
        }

        /// Instructions spent by *this* script (batched requests share a call context).
        pub fn instructions_used(&self) -> u64 {
            instruction_counter().saturating_sub(self.base_instructions)
        }

        /// Record a refusal. Past `MAX_VIOLATIONS`, the first half is kept and the second
        /// half slides, so a script cannot hide a late refusal behind early junk ones.
        pub fn deny(&self, e: String) -> String {
            let mut short = e.clone();
            truncate(&mut short, MAX_VIOLATION_BYTES);
            let mut v = self.violations.borrow_mut();
            if v.len() >= MAX_VIOLATIONS {
                v.remove(MAX_VIOLATIONS / 2);
                self.violations_dropped.set(self.violations_dropped.get() + 1);
            }
            v.push(short);
            e
        }

        /// Violations for a closing audit entry, with a count of those not kept.
        pub fn closing_violations(&self) -> Vec<String> {
            let mut v = self.violations.borrow().clone();
            if let n @ 1.. = self.violations_dropped.get() {
                v.push(format!("…and {n} more"));
            }
            v
        }

        /// Checks made when a message is reserved and again right before it is sent: the
        /// await in between (an interface fetch, another message) can take minutes, during
        /// which the canister may begin stopping or the token or target may be removed.
        /// Returns the message's timeout.
        fn gate(&self, p: &Principal, method: Option<&str>, calls_made: u32) -> Result<u32, String> {
            // A stop waits for every outstanding call; a run must not keep adding new ones.
            let refusal = if !running() {
                Some("policy: the canister is stopping; no new calls".to_string())
            } else if !policy::token_registered(&self.token) {
                Some("policy: the API token was revoked".to_string())
            } else if IN_FLIGHT.get() >= MAX_IN_FLIGHT {
                Some(format!("policy: {MAX_IN_FLIGHT} messages are already awaiting replies; retry shortly"))
            } else if liquid_cycles() < MIN_LIQUID_CYCLES {
                Some(format!("policy: the canister has under {MIN_LIQUID_CYCLES} liquid cycles; a controller must top it up"))
            } else {
                None
            };
            if let Some(e) = refusal {
                return Err(self.deny(e));
            }
            let want = self.limits.call_timeout_seconds.unwrap_or(policy::DEFAULT_CALL_TIMEOUT_SECONDS);
            let secs = timeout_left(self.started_ns, now_ns(), want).map_err(|e| self.deny(e))?;
            let used = self.instructions_used();
            match method {
                Some(m) => policy::check_call(p, m, calls_made, used, &self.limits),
                None => policy::check_interface_fetch(p, calls_made, used, &self.limits),
            }
            .map_err(|e| self.deny(e))?;
            Ok(secs)
        }

        /// Policy-check one outbound request and take a call-cap slot, before any await so
        /// concurrent requests (Promise.all / gather) are all counted.
        fn reserve(&self, p: &Principal, method: Option<&str>) -> Result<(), String> {
            let n = self.slots.get();
            self.gate(p, method, n)?;
            self.slots.set(n + 1);
            Ok(())
        }

        /// The `gate` again right before sending a message reserved earlier (no new slot).
        fn recheck(&self, p: &Principal, method: Option<&str>) -> Result<u32, String> {
            self.gate(p, method, self.slots.get().saturating_sub(1))
        }

        /// Await an inter-canister call, writing its audit entry at the moment the system
        /// accepts the message: ic-cdk's call future is pending after its first poll only
        /// if `call_perform` succeeded, and resolves immediately with an error if the
        /// message was refused (no cycles, queue full). So the log has an entry exactly for
        /// the messages that were handed over, never for one that was not sent. The message
        /// counts in `IN_FLIGHT` until it completes or its run is dropped.
        ///
        /// `about` is the canister the message concerns (the target, or for a management
        /// request the canister it is about); `label` names the message.
        async fn sent<T>(&self, about: &Principal, label: String, call: impl std::future::Future<Output = T>) -> T {
            struct InFlight(bool);
            impl Drop for InFlight {
                fn drop(&mut self) {
                    if self.0 {
                        IN_FLIGHT.with(|n| n.set(n.get().saturating_sub(1)));
                    }
                }
            }
            let mut held = InFlight(false);
            on_first_pending(call, || {
                self.record_message(about, label);
                IN_FLIGHT.with(|n| n.set(n.get() + 1));
                held.0 = true;
            })
            .await
        }

        /// Audit record for one outbound message: `Started` for the run's first, `Call` for
        /// each later one. It is appended in the same message execution that hands the
        /// message to the system, so the two are committed together when that execution
        /// ends, or discarded together if it traps first. Later traps cannot remove it.
        fn record_message(&self, p: &Principal, label: String) {
            let (caller, language, sha) = self.audit_key.clone();
            let run = self.started.get();
            self.touched.borrow_mut().insert(p.to_text());
            self.messages.borrow_mut().push(label.clone());
            let id = policy::append_audit(AuditEntry {
                id: 0,
                timestamp_ns: now_ns(),
                caller,
                language,
                script_sha256: sha,
                canisters: vec![p.to_text()],
                calls: self.calls(),
                instructions: self.instructions_used(),
                status: if run.is_none() { AuditStatus::Started } else { AuditStatus::Call },
                violations: vec![],
                error: None,
                run_id: run, // None: a Started entry opens its own run
                methods: Some(vec![label]),
            });
            if run.is_none() {
                self.started.set(Some(id));
                PENDING_RUNS.with(|r| r.borrow_mut().insert(id));
            }
        }

        /// 1. management `canister_info`: `canister_metadata` returns private sections to a
        ///    controller, so when this canister controls the target it is skipped (a private
        ///    interface must not reach token holders). 2. management `canister_metadata`
        ///    (the `candid:service` section; not on every replica yet). 3. the
        ///    `__get_candid_interface_tmp_hack` query that Motoko and many Rust canisters
        ///    export. Each message is logged as it is sent.
        ///
        /// Each message takes its own call-cap slot (the caller reserved the first). The
        /// error's `bool` says whether the failure says something lasting about the target,
        /// so it may be remembered; a policy refusal, a message the system refused on the
        /// spot, or a transient or unknown outcome does not.
        async fn fetch_did(&self, p: Principal) -> Result<String, (String, bool)> {
            let mgmt = Principal::management_canister();
            let refused = |e: String| (e, false);
            let secs = self.recheck(&p, None).map_err(refused)?;
            let info = Call::bounded_wait(mgmt, "canister_info")
                .change_timeout(secs)
                .with_arg(InfoArgs { canister_id: p, num_requested_changes: Some(0) })
                .into_future();
            let info = self
                .sent(&p, format!("{mgmt}.canister_info({p})"), info)
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| candid::decode_one_with_config::<InfoResult>(&r.into_bytes(), &fetch_decoder()).map_err(|e| e.to_string()));
            let meta_err = match info {
                Err(e) => format!("skipped, canister_info failed: {e}"),
                Ok(i) if i.controllers.contains(&ic_cdk::api::canister_self()) => {
                    "skipped: this canister controls the target, so it could read a private section".to_string()
                }
                Ok(_) => {
                    self.reserve(&p, None).map_err(refused)?;
                    let secs = self.recheck(&p, None).map_err(refused)?;
                    let meta = Call::bounded_wait(mgmt, "canister_metadata")
                        .change_timeout(secs)
                        .with_arg(MetadataArgs { canister_id: p, name: "candid:service".into() })
                        .into_future();
                    match self.sent(&p, format!("{mgmt}.canister_metadata({p})"), meta).await {
                        Ok(r) => match candid::decode_one_with_config::<MetadataResult>(&r.into_bytes(), &fetch_decoder()) {
                            Ok(m) => return String::from_utf8(m.value).map_err(|_| ("candid:service is not UTF-8".to_string(), true)),
                            Err(e) => e.to_string(),
                        },
                        Err(e) => e.to_string(),
                    }
                }
            };
            self.reserve(&p, None).map_err(refused)?;
            let secs = self.recheck(&p, None).map_err(refused)?;
            let both = |e: String| format!("canister_metadata: {meta_err}; __get_candid_interface_tmp_hack: {e}");
            let hack = Call::bounded_wait(p, "__get_candid_interface_tmp_hack").change_timeout(secs).into_future();
            let reply = self
                .sent(&p, format!("{p}.__get_candid_interface_tmp_hack"), hack)
                .await
                .map_err(|e| (both(e.to_string()), definitive_failure(&e)))?;
            candid::decode_one_with_config::<String>(&reply.into_bytes(), &fetch_decoder()).map_err(|e| (both(e.to_string()), true))
        }

        /// Resolves once no request of this run is fetching `key` (at once if none is).
        fn fetch_done<'a>(&'a self, key: &'a str) -> impl std::future::Future<Output = ()> + 'a {
            std::future::poll_fn(move |cx| match self.fetching.borrow_mut().get_mut(key) {
                Some(waiting) => {
                    if !waiting.iter().any(|w| w.will_wake(cx.waker())) {
                        waiting.push(cx.waker().clone());
                    }
                    Poll::Pending
                }
                None => Poll::Ready(()),
            })
        }

        /// Cache a fetched interface, unless the target left the allowlist while it was
        /// being fetched (`remove_allowlist` evicts first; a fill after it must not undo that).
        fn keep(&self, p: &Principal, did: String, svc: Rc<Service>) -> Result<String, String> {
            // Default limits: only the allowlist decides.
            policy::check_interface_fetch(p, 0, 0, &Limits::default()).map_err(|e| self.deny(e))?;
            let key = p.to_text();
            DID_FAILED.with(|c| c.borrow_mut().remove(&key));
            DID_CACHE.with(|c| c.borrow_mut().insert(key, (did.clone(), svc, now_ns())));
            Ok(did)
        }

        async fn service(&self, p: &Principal) -> Result<Rc<Service>, String> {
            let key = p.to_text();
            if let Ok(Some(r)) = override_service(&key, true) {
                return r;
            }
            if let Some((_, s)) = cache_get(&key, DID_CACHE_TTL_NS) {
                return Ok(s);
            }
            self.candid(&key).await?; // populates DID_CACHE
            // Any age: candid() serves an expired copy when a re-fetch failed transiently.
            cache_get(&key, u64::MAX).map(|(_, s)| s).ok_or_else(|| "candid cache miss".into())
        }
    }

    /// Interface replies are text up to a few MiB; the quotas bound what a hostile reply
    /// can make the decoder do.
    fn fetch_decoder() -> candid::DecoderConfig {
        let mut cfg = candid::DecoderConfig::new();
        cfg.set_decoding_quota(8 << 20).set_skipping_quota(8 << 20).set_full_error_message(false);
        cfg
    }

    #[derive(CandidType)]
    struct MetadataArgs {
        canister_id: Principal,
        name: String,
    }

    #[derive(CandidType, Deserialize)]
    struct MetadataResult {
        #[serde(with = "serde_bytes")]
        value: Vec<u8>,
    }

    #[derive(CandidType)]
    struct InfoArgs {
        canister_id: Principal,
        num_requested_changes: Option<u64>,
    }

    /// The part of `canister_info`'s reply used here (other fields are skipped).
    #[derive(CandidType, Deserialize)]
    struct InfoResult {
        controllers: Vec<Principal>,
    }

    fn parse_principal(id: &str) -> Result<Principal, String> {
        let mut shown = id.to_string();
        truncate(&mut shown, 80);
        Principal::from_text(id).map_err(|e| format!("invalid canister id `{shown}`: {e}"))
    }

    impl HostBridge for IcBridge {
        async fn candid(&self, canister_id: &str) -> Result<String, String> {
            // Canonical text: `RYJL3-…` and `ryjl3-…` are the same principal.
            let p = parse_principal(canister_id)?;
            let key = p.to_text();
            if let Some(did) = policy::candid_override(&key) {
                return Ok(did);
            }
            // Another request of this run (Promise.all / gather) is fetching it: use that
            // fetch's result instead of sending the same messages again.
            self.fetch_done(&key).await;
            if let Some((did, _)) = cache_get(&key, DID_CACHE_TTL_NS) {
                return Ok(did);
            }
            // Fetching an interface sends messages: allowlisted canisters only, and it
            // counts against the per-script call cap.
            if let Some(e) = failed_get(&key) {
                return Err(e); // failed moments ago: don't send the same messages again
            }
            self.reserve(&p, None)?;
            let _fetching = Fetching::start(self, &key);
            let fetched = self
                .fetch_did(p)
                .await
                .and_then(|did| match did.len() {
                    n if n > MAX_DID_BYTES => Err((format!("interface is {n} bytes, over the {MAX_DID_BYTES}-byte limit"), true)),
                    _ => Ok(did),
                })
                .and_then(|did| Service::parse(&did).map(|svc| (did, Rc::new(svc))).map_err(|e| (e, true)));
            match fetched {
                Ok((did, svc)) => self.keep(&p, did, svc),
                Err((e, remember)) => fetch_failed(&key, e, remember),
            }
        }

        async fn call(&self, canister_id: &str, method: &str, args: Value) -> Result<Value, String> {
            if method.len() > MAX_METHOD_LEN {
                return Err(format!("method name longer than {MAX_METHOD_LEN} bytes"));
            }
            let p = parse_principal(canister_id)?;
            self.reserve(&p, Some(method))?;
            let svc = self.service(&p).await?;
            let enc = encode_call(&svc, method, &args)?;
            if enc.bytes.len() > MAX_ARG_BYTES {
                return Err(format!("arguments are {} bytes encoded, over the {MAX_ARG_BYTES}-byte message limit", enc.bytes.len()));
            }
            // The interface fetch above may have awaited: check again right before sending,
            // so nothing goes out past a stop, a removal or a revocation. Always a bounded
            // wait: a target that never answers holds a run slot, or this canister's stop,
            // for at most the timeout. A timed-out call may still have executed on the
            // target; the error says so.
            let secs = self.recheck(&p, Some(method))?;
            let call = Call::bounded_wait(p, method).change_timeout(secs);
            let label = format!("{p}.{}", method.escape_debug());
            let reply = self.sent(&p, label, call.with_raw_args(&enc.bytes).into_future()).await.map_err(|e| {
                let unknown = !e.is_clean_reject();
                format!("call {p}.{} failed: {e}{}", method.escape_debug(), if unknown { " (outcome unknown: the call may have executed)" } else { "" })
            })?;
            svc.decode_reply(&enc.func, &reply.into_bytes()).map_err(|e| {
                // Most likely the target was upgraded and our copy of its interface is stale.
                evict_interface(&p);
                reply_decode_error(&p, method, &e)
            })
        }

        fn principal(&self) -> String {
            ic_cdk::api::canister_self().to_text()
        }

        fn log(&self, line: String) {
            self.logs.push(line);
        }

        fn instruction_deadline(&self) -> u64 {
            self.base_instructions.saturating_add(self.limits.max_instructions)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use futures::executor::block_on;
        use serde_json::json;
        use std::future::Future;

        const FX: &str = "ryjl3-tyaaa-aaaaa-aaaba-cai";

        fn bridge() -> IcBridge {
            let h = policy::sha256_hex(b"bridge-test-token");
            policy::add_token_hash(&h, "t".into()).unwrap();
            IcBridge::new(&h, "javascript", "sha".into())
        }

        fn fx() -> Principal {
            Principal::from_text(FX).unwrap()
        }

        /// 115: the error for an undecodable reply names the call and says it ran.
        #[test]
        fn undecodable_reply_names_the_call_and_says_it_executed() {
            let svc = Service::parse("service : { r2 : () -> (record { a : nat64 }) }").unwrap();
            let func = svc.method("r2").unwrap().clone();
            let reply = candid::encode_one(7u64).unwrap();
            let e = svc.decode_reply(&func, &reply).unwrap_err();
            let msg = reply_decode_error(&fx(), "r2", &e);
            assert!(msg.contains(FX) && msg.contains(".r2") && msg.contains("executed"), "{msg}");
        }

        /// 191: a composite_query is refused before its arguments are encoded.
        #[test]
        fn composite_query_refused_before_encoding() {
            let svc = Service::parse("service : { c : (nat) -> (nat) composite_query }").unwrap();
            let e = encode_call(&svc, "c", &json!("not-a-number")).err().unwrap();
            assert!(e.contains("composite_query"), "{e}");
        }

        /// 75 / R44: every message's timeout is cut to the run's time limit, and nothing
        /// is sent after it.
        #[test]
        fn run_time_limit_cuts_timeouts_then_refuses() {
            let s = 1_000_000_000;
            assert_eq!(timeout_left(0, 0, 300), Ok(300));
            assert_eq!(timeout_left(0, (MAX_RUN_SECONDS - 10) * s, 300), Ok(10));
            assert!(timeout_left(0, MAX_RUN_SECONDS * s, 300).unwrap_err().starts_with("policy:"));
            assert!(timeout_left(5 * s, 5 * s + (MAX_RUN_SECONDS * s) - 1, 300).is_err());

            // Through the gate: an allowlisted call is refused once the run is too old.
            policy::set_allowed(fx(), vec!["*".into()]);
            let b = bridge();
            assert!(b.reserve(&fx(), Some("m")).is_ok());
            TEST_NOW.set(MAX_RUN_SECONDS * s);
            let e = b.recheck(&fx(), Some("m")).unwrap_err();
            assert!(e.contains("time limit") && refuses_whole_run(&e), "{e}");
            TEST_NOW.set(0);
            policy::remove_allowed(fx());
        }

        /// 136: only failures that say something lasting about the target are remembered.
        #[test]
        fn only_definitive_fetch_failures_are_remembered() {
            assert!(definitive(Some(RejectCode::CanisterReject), "no"));
            assert!(definitive(Some(RejectCode::DestinationInvalid), "no such canister"));
            assert!(definitive(Some(RejectCode::CanisterError), "Canister x has no query method '__get_candid_interface_tmp_hack'"));
            assert!(!definitive(Some(RejectCode::CanisterError), "Canister x is stopped"));
            assert!(!definitive(Some(RejectCode::CanisterError), "Canister x is stopping"));
            assert!(!definitive(Some(RejectCode::SysTransient), "queue full"));
            assert!(!definitive(Some(RejectCode::SysUnknown), "deadline expired"));
            assert!(!definitive(None, "unknown code"));

            // Not remembered: an expired copy is served instead of the error.
            let key = "aaaaa-aa";
            let svc = Rc::new(Service::parse("service : { ping : () -> () }").unwrap());
            DID_CACHE.with(|c| c.borrow_mut().insert(key.into(), ("old".into(), svc, 0)));
            TEST_NOW.set(DID_CACHE_TTL_NS + 1);
            assert!(cache_get(key, DID_CACHE_TTL_NS).is_none());
            assert_eq!(fetch_failed(key, "SysTransient".into(), false), Ok("old".into()));
            assert!(failed_get(key).is_none(), "a transient failure is not cached");
            assert!(fetch_failed(key, "no such method".into(), true).is_err());
            assert!(failed_get(key).is_some(), "a definitive one is");
            TEST_NOW.set(0);
            DID_CACHE.with(|c| c.borrow_mut().clear());
            DID_FAILED.with(|c| c.borrow_mut().clear());
        }

        /// 141: an interface fetched while its canister left the allowlist is not cached;
        /// the token is checked again before every message.
        #[test]
        fn removal_and_revocation_during_an_await_are_honoured() {
            let b = bridge();
            let svc = Rc::new(Service::parse("service : { tick : () -> () }").unwrap());
            policy::set_allowed(fx(), vec!["*".into()]);
            assert!(b.reserve(&fx(), Some("tick")).is_ok());
            policy::remove_allowed(fx()); // during the fetch
            assert!(b.keep(&fx(), "did".into(), svc.clone()).unwrap_err().contains("not allowlisted"));
            assert!(cached_service(&fx(), true).is_none(), "nothing cached for a removed canister");
            policy::set_allowed(fx(), vec!["*".into()]);
            assert!(b.keep(&fx(), "did".into(), svc).is_ok());

            assert!(b.recheck(&fx(), Some("tick")).is_ok());
            policy::revoke_token_hash(&b.token);
            let e = b.recheck(&fx(), Some("tick")).unwrap_err();
            assert!(e.contains("revoked"), "{e}");
            policy::remove_allowed(fx());
            evict_interface(&fx());
        }

        /// 163: the run keeps the limits it started with.
        #[test]
        fn run_keeps_the_limits_it_started_with() {
            policy::set_limits(Limits::default());
            policy::set_allowed(fx(), vec!["*".into()]);
            let b = bridge();
            policy::set_limits(Limits { max_calls_per_script: 0, max_instructions: 1, call_timeout_seconds: Some(5), ..Limits::default() });
            assert_eq!(b.recheck(&fx(), Some("m")), Ok(policy::DEFAULT_CALL_TIMEOUT_SECONDS));
            assert_eq!(b.instruction_deadline(), policy::DEFAULT_MAX_INSTRUCTIONS);
            policy::set_limits(Limits::default());
            policy::remove_allowed(fx());
        }

        /// 125: no new message once MAX_IN_FLIGHT are awaiting replies.
        #[test]
        fn in_flight_messages_are_capped() {
            policy::set_allowed(fx(), vec!["*".into()]);
            let b = bridge();
            IN_FLIGHT.set(MAX_IN_FLIGHT);
            assert!(b.reserve(&fx(), Some("m")).unwrap_err().contains("awaiting replies"));
            IN_FLIGHT.set(MAX_IN_FLIGHT - 1);
            assert!(b.reserve(&fx(), Some("m")).is_ok());
            IN_FLIGHT.set(0);
            policy::remove_allowed(fx());
        }

        /// 149: a late refusal stays visible after many early ones, in every closing entry.
        #[test]
        fn late_violations_are_kept() {
            let b = bridge();
            for i in 0..MAX_VIOLATIONS {
                b.deny(format!("policy: junk {i}"));
            }
            b.deny("policy: method `icrc1_transfer` on x is not allowlisted".into());
            let v = b.closing_violations();
            assert_eq!(v.len(), MAX_VIOLATIONS + 1);
            assert!(v[0].contains("junk 0"));
            assert!(v[MAX_VIOLATIONS - 1].contains("icrc1_transfer"), "{v:?}");
            assert_eq!(v.last().unwrap(), "…and 1 more");
        }

        /// 88: a request for an interface this run is already fetching waits for that
        /// fetch, takes no slot and sends nothing.
        #[test]
        fn concurrent_requests_share_one_fetch() {
            policy::set_allowed(fx(), vec!["*".into()]);
            let b = bridge();
            let fetching = Fetching::start(&b, FX);
            let mut waiter = Box::pin(b.candid(FX));
            let waker = futures::task::noop_waker();
            assert!(waiter.as_mut().poll(&mut std::task::Context::from_waker(&waker)).is_pending());
            // The first fetch completes.
            b.keep(&fx(), "service : { tick : () -> () }".into(), Rc::new(Service::parse("service : { tick : () -> () }").unwrap())).unwrap();
            drop(fetching);
            assert_eq!(block_on(waiter).unwrap(), "service : { tick : () -> () }");
            assert_eq!((b.slots.get(), b.calls()), (0, 0));
            policy::remove_allowed(fx());
            evict_interface(&fx());
        }

        /// 141: a search stops fetching on run-wide refusals only.
        #[test]
        fn search_continues_past_a_removed_canister() {
            assert!(!refuses_whole_run(&format!("policy: canister {FX} is not allowlisted")));
            assert!(refuses_whole_run("policy: call limit of 25 per script reached"));
            assert!(refuses_whole_run("policy: the canister is stopping; no new calls"));
            assert!(!refuses_whole_run("could not fetch candid for x (y)"));
        }
    }
}

/// In-memory bridge for native tests: canned candid + replies, records every call.
#[cfg(test)]
pub mod mock {
    use super::*;
    use std::cell::RefCell;

    pub struct MockBridge {
        pub calls: RefCell<Vec<(String, String, Value)>>,
        /// Every line the runtime logged, uncapped: the runtimes' own output caps are
        /// what tests measure here, not `LogBuf`'s.
        pub logs: RefCell<Vec<String>>,
        /// Only this canister is "allowlisted".
        pub allowed: &'static str,
    }

    impl Default for MockBridge {
        fn default() -> Self {
            Self { calls: RefCell::default(), logs: RefCell::default(), allowed: "ryjl3-tyaaa-aaaaa-aaaba-cai" }
        }
    }

    pub const LEDGER_DID: &str = "service : { icrc1_balance_of : (record { owner : principal }) -> (nat) query }";

    impl HostBridge for MockBridge {
        async fn candid(&self, id: &str) -> Result<String, String> {
            if id == self.allowed { Ok(LEDGER_DID.into()) } else { Err(format!("policy: canister {id} is not allowlisted")) }
        }
        async fn call(&self, id: &str, method: &str, args: Value) -> Result<Value, String> {
            if id != self.allowed {
                return Err(format!("policy: canister {id} is not allowlisted"));
            }
            self.calls.borrow_mut().push((id.into(), method.into(), args.clone()));
            // Echo-ish: balance = 100 * number of calls so far, owner echoed back.
            let n = self.calls.borrow().len() as u64;
            Ok(serde_json::json!({ "balance": n * 100, "owner": args["owner"] }))
        }
        fn principal(&self) -> String {
            "aaaaa-aa".into()
        }
        fn log(&self, line: String) {
            self.logs.borrow_mut().push(line);
        }
    }

    #[test]
    fn on_first_pending_fires_once_and_only_if_pending() {
        use futures::executor::block_on;
        use std::cell::Cell;
        let n = Cell::new(0);
        // Ready on the first poll (a message the system refused): the hook never runs.
        assert_eq!(block_on(on_first_pending(async { 7 }, || n.set(n.get() + 1))), 7);
        assert_eq!(n.get(), 0);
        // Pending first (the message was accepted): the hook runs exactly once.
        let twice = async {
            YieldOnce(false).await;
            YieldOnce(false).await;
            8
        };
        assert_eq!(block_on(on_first_pending(twice, || n.set(n.get() + 1))), 8);
        assert_eq!(n.get(), 1);
    }

    /// Like MockBridge, but every call suspends once, as an inter-canister await does,
    /// so two runs driven by `join` really interleave.
    pub struct YieldBridge(pub MockBridge);
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
    fn value_budget_truncate_normalize_logbuf_and_overrides() {
        // LogBuf cap short-circuits once MAX_LOG_BYTES is reached.
        let buf = LogBuf::default();
        buf.push("x".repeat(MAX_LOG_BYTES));
        buf.push("overflow".into());
        assert_eq!(buf.take().len(), 1);

        // Multi-byte UTF-8 boundary truncation.
        let mut s = "a🦀🦀".to_string();
        truncate(&mut s, 3);
        assert_eq!(s, "a…[truncated]");

        // ValueBudget node and byte caps.
        let mut vb = ValueBudget::default();
        for _ in 0..MAX_VALUE_NODES {
            vb.node().unwrap();
        }
        assert!(vb.node().is_err());
        let mut vb = ValueBudget::default();
        assert!(vb.bytes(MAX_VALUE_BYTES + 1).is_err());

        // normalize_numbers preserves non-integral, out-of-safe-range, arrays, objects.
        let out = normalize_numbers(serde_json::json!([1.5, 9_007_199_254_740_992.0, { "k": 3.0 }]));
        assert_eq!(out, serde_json::json!([1.5, 9_007_199_254_740_992.0, { "k": 3 }]));

        // Default HostBridge::instruction_deadline & evict_interface with override.
        assert_eq!(MockBridge::default().instruction_deadline(), u64::MAX);
        assert_eq!(now_ms(), 0);
        let p = candid::Principal::from_text("ryjl3-tyaaa-aaaaa-aaaba-cai").unwrap();
        crate::policy::set_candid_override(p, Some("service : { ping : () -> () }".into()));
        assert!(cached_service(&p, true).is_some());
        assert!(cached_service(&p, true).is_some()); // override cache hit
        evict_interface(&p);
        crate::policy::set_candid_override(p, None);
    }
}
