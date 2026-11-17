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

pub use ic::{IcBridge, MAX_DID_BYTES, cached_service, evict_interface, pending_run_ids};

/// Instructions executed so far in this call context (0 off-chain).
pub fn instruction_counter() -> u64 {
    #[cfg(target_arch = "wasm32")]
    return ic_cdk::api::call_context_instruction_counter();
    #[cfg(not(target_arch = "wasm32"))]
    0
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

/// Production bridge. Compiles natively too (ic0 stubs panic off-chain), which lets
/// `export_candid!` and tests build without a replica.
mod ic {
    use super::*;
    use crate::policy::{self, AuditEntry, AuditStatus};
    use candid::types::FuncMode;
    use candid::{CandidType, Deserialize, Principal};
    use ic_cdk::call::{Call, CallErrorExt};
    use std::cell::{Cell, RefCell};
    use std::collections::{BTreeMap, BTreeSet};
    use std::rc::Rc;

    /// Longest method name a script may pass (keeps policy messages small).
    const MAX_METHOD_LEN: usize = 256;
    /// How long a fetched interface is trusted before it is fetched again.
    const DID_CACHE_TTL_NS: u64 = 3600 * 1_000_000_000;
    /// How long a failed fetch is remembered. Without this, every search or script would
    /// repeat the same outbound calls to a target that exposes no interface.
    const DID_FAIL_TTL_NS: u64 = 300 * 1_000_000_000;
    /// Largest interface accepted from a target or as an override (the ICP ledger's is
    /// 17 KB). Parsing megabytes of hostile Candid would only burn instructions.
    pub const MAX_DID_BYTES: usize = 512 * 1024;
    /// Violations kept per script; the rest are only counted.
    const MAX_VIOLATIONS: usize = 16;
    const MAX_VIOLATION_BYTES: usize = 512;

    thread_local! {
        // ponytail: heap cache keyed by canonical principal text, emptied on upgrade
        // (re-fetched on demand); bounded by the allowlist size.
        // Value: (.did text, parsed service, fetch time in ns).
        static DID_CACHE: RefCell<BTreeMap<String, (String, Rc<Service>, u64)>> = RefCell::default();
        // Runs that wrote a `Started` entry and have not ended. Lets an auditor tell a run
        // still waiting on a reply from one that ended without a final entry.
        static PENDING_RUNS: RefCell<BTreeSet<u64>> = RefCell::default();
        // Recent fetch failures: (error, time in ns).
        static DID_FAILED: RefCell<BTreeMap<String, (String, u64)>> = RefCell::default();
        // Parsed admin overrides: (.did text it was parsed from, service).
        static OVERRIDES: RefCell<BTreeMap<String, (String, Rc<Service>)>> = RefCell::default();
    }

    /// The interface for `p` without any outbound message: an admin override, or the
    /// cache filled by an earlier fetch. `None` means it has to be fetched.
    pub fn cached_service(p: &Principal) -> Option<Rc<Service>> {
        let key = p.to_text();
        if let Some(r) = override_service(&key) {
            return r.ok();
        }
        cache_get(&key).map(|(_, s)| s)
    }

    /// The admin override for `key`, parsed once and reused until its text changes.
    fn override_service(key: &str) -> Option<Result<Rc<Service>, String>> {
        let did = policy::candid_override(key)?;
        let hit = OVERRIDES.with(|c| c.borrow().get(key).filter(|(d, _)| *d == did).map(|(_, s)| s.clone()));
        Some(match hit {
            Some(s) => Ok(s),
            None => Service::parse(&did).map(|s| {
                let s = Rc::new(s);
                OVERRIDES.with(|c| c.borrow_mut().insert(key.to_string(), (did, s.clone())));
                s
            }),
        })
    }

    /// Run ids (see `AuditEntry::run_id`) with outbound messages sent and no end yet. Kept
    /// in memory: a run leaves the list when it finishes or when its trap is cleaned up,
    /// and the list is empty after an upgrade.
    pub fn pending_run_ids() -> Vec<u64> {
        PENDING_RUNS.with(|r| r.borrow().iter().copied().collect())
    }

    /// The run is over, however it ended: normally, or by ic-cdk dropping the future while
    /// cleaning up after a trap.
    impl Drop for IcBridge {
        fn drop(&mut self) {
            if let Some(id) = self.started.get() {
                PENDING_RUNS.with(|r| r.borrow_mut().remove(&id));
            }
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
        let now = ic_cdk::api::time();
        DID_FAILED.with(|c| c.borrow().get(key).filter(|(_, at)| now.saturating_sub(*at) <= DID_FAIL_TTL_NS).map(|(e, _)| e.clone()))
    }

    /// A cached interface, unless it is older than `DID_CACHE_TTL_NS` (then it is
    /// re-fetched, so a target's interface change is picked up without an upgrade here).
    fn cache_get(key: &str) -> Option<(String, Rc<Service>)> {
        let now = ic_cdk::api::time();
        DID_CACHE.with(|c| {
            c.borrow().get(key).filter(|(_, _, at)| now.saturating_sub(*at) <= DID_CACHE_TTL_NS).map(|(d, s, _)| (d.clone(), s.clone()))
        })
    }

    /// Production bridge: one instance per `tools/call`, so counters start at zero.
    pub struct IcBridge {
        calls: Cell<u32>,
        base_instructions: u64,
        max_instructions: u64,
        /// (caller, language, script sha) for the provisional audit entry.
        audit_key: (String, String, String),
        /// Id of this run's `Started` audit entry, once one is written.
        started: Cell<Option<u64>>,
        /// Every outbound message so far, as `canister.method`, in order.
        messages: RefCell<Vec<String>>,
        call_timeout: Option<u32>,
        pub touched: RefCell<BTreeSet<String>>,
        pub violations: RefCell<Vec<String>>,
        pub violations_dropped: Cell<u32>,
        pub logs: LogBuf,
    }

    impl IcBridge {
        pub fn new(caller: String, language: &str, script_sha256: String) -> Self {
            Self {
                calls: Cell::new(0),
                base_instructions: instruction_counter(),
                max_instructions: policy::limits().max_instructions,
                audit_key: (caller, language.to_string(), script_sha256),
                started: Cell::new(None),
                messages: RefCell::default(),
                call_timeout: policy::limits().call_timeout_seconds,
                touched: RefCell::default(),
                violations: RefCell::default(),
                violations_dropped: Cell::new(0),
                logs: LogBuf::default(),
            }
        }

        pub fn calls(&self) -> u32 {
            self.calls.get()
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

        pub fn deny(&self, e: String) -> String {
            let mut v = self.violations.borrow_mut();
            if v.len() < MAX_VIOLATIONS {
                let mut short = e.clone();
                truncate(&mut short, MAX_VIOLATION_BYTES);
                v.push(short);
            } else {
                self.violations_dropped.set(self.violations_dropped.get() + 1);
            }
            e
        }

        /// Policy-check and count one outbound message, before any await so
        /// concurrent requests (Promise.all / gather) are all counted.
        fn reserve(&self, p: &Principal, method: Option<&str>) -> Result<(), String> {
            let n = self.calls.get();
            let used = self.instructions_used();
            match method {
                Some(m) => policy::check_call(p, m, n, used),
                None => policy::check_interface_fetch(p, n, used),
            }
            .map_err(|e| self.deny(e))?;
            self.calls.set(n + 1);
            self.touched.borrow_mut().insert(p.to_text());
            self.record_message(p, format!("{}.{}", p.to_text(), method.unwrap_or("<candid interface>")));
            Ok(())
        }

        /// Write-ahead audit record for one outbound message: `Started` for the run's first,
        /// `Call` for each later one. State commits when the message is sent, so even if the
        /// script never finishes (a trap rolls back the final entry), the log still shows
        /// every message that went out, with its target and method.
        fn record_message(&self, p: &Principal, label: String) {
            let (caller, language, sha) = self.audit_key.clone();
            let run = self.started.get();
            let id = policy::append_audit(AuditEntry {
                id: 0,
                timestamp_ns: ic_cdk::api::time(),
                caller,
                language,
                script_sha256: sha,
                canisters: vec![p.to_text()],
                calls: self.calls.get(),
                instructions: self.instructions_used(),
                status: if run.is_none() { AuditStatus::Started } else { AuditStatus::Call },
                violations: vec![],
                error: None,
                run_id: run, // None: a Started entry opens its own run
                methods: Some(vec![label.clone()]),
            });
            if run.is_none() {
                self.started.set(Some(id));
                PENDING_RUNS.with(|r| r.borrow_mut().insert(id));
            }
            self.messages.borrow_mut().push(label);
        }

        async fn service(&self, p: &Principal) -> Result<Rc<Service>, String> {
            let key = p.to_text();
            if let Some(r) = override_service(&key) {
                return r;
            }
            if let Some((_, s)) = cache_get(&key) {
                return Ok(s);
            }
            self.candid(&key).await?; // populates DID_CACHE
            cache_get(&key).map(|(_, s)| s).ok_or_else(|| "candid cache miss".into())
        }
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

    /// 1. management `canister_metadata` (reads the public `candid:service` section; not on
    ///    every replica yet), 2. the `__get_candid_interface_tmp_hack` query that Motoko and
    ///    many Rust canisters export.
    async fn fetch_did(p: Principal) -> Result<String, String> {
        let meta = Call::bounded_wait(Principal::management_canister(), "canister_metadata")
            .with_arg(MetadataArgs { canister_id: p, name: "candid:service".into() })
            .await;
        if let Ok(r) = meta {
            if let Ok(m) = r.candid::<MetadataResult>() {
                return String::from_utf8(m.value).map_err(|_| "candid:service is not UTF-8".to_string());
            }
        }
        let reply = Call::bounded_wait(p, "__get_candid_interface_tmp_hack").await.map_err(|e| e.to_string())?;
        reply.candid::<String>().map_err(|e| e.to_string())
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
            if let Some((did, _)) = cache_get(&key) {
                return Ok(did);
            }
            // Fetching an interface sends messages: allowlisted canisters only, and it
            // counts against the per-script call cap.
            if let Some(e) = failed_get(&key) {
                return Err(e); // failed moments ago: don't send the same messages again
            }
            self.reserve(&p, None)?;
            let fetched = fetch_did(p)
                .await
                .and_then(|did| match did.len() {
                    n if n > MAX_DID_BYTES => Err(format!("interface is {n} bytes, over the {MAX_DID_BYTES}-byte limit")),
                    _ => Ok(did),
                })
                .and_then(|did| Service::parse(&did).map(|svc| (did, Rc::new(svc))));
            let now = ic_cdk::api::time();
            match fetched {
                Ok((did, svc)) => {
                    DID_FAILED.with(|c| c.borrow_mut().remove(&key));
                    DID_CACHE.with(|c| c.borrow_mut().insert(key, (did.clone(), svc, now)));
                    Ok(did)
                }
                Err(e) => {
                    let msg = format!("could not fetch candid for {key} ({e}); a controller can register it with set_candid_override");
                    DID_FAILED.with(|c| c.borrow_mut().insert(key, (msg.clone(), now)));
                    Err(msg)
                }
            }
        }

        async fn call(&self, canister_id: &str, method: &str, args: Value) -> Result<Value, String> {
            if method.len() > MAX_METHOD_LEN {
                return Err(format!("method name longer than {MAX_METHOD_LEN} bytes"));
            }
            let p = parse_principal(canister_id)?;
            self.reserve(&p, Some(method))?;
            let svc = self.service(&p).await?;
            let enc = svc.encode_args(method, &args)?;
            if enc.func.modes.contains(&FuncMode::CompositeQuery) {
                return Err(format!("`{method}` is a composite_query; it cannot be called from an update"));
            }
            // Default: wait for a guaranteed reply. With a timeout configured the call is
            // best-effort: it cannot block this canister from stopping, but a timed-out
            // call may still have executed on the target.
            let call = match self.call_timeout {
                Some(secs) => Call::bounded_wait(p, method).change_timeout(secs),
                None => Call::unbounded_wait(p, method),
            };
            let reply = call.with_raw_args(&enc.bytes).await.map_err(|e| {
                let unknown = self.call_timeout.is_some() && !e.is_clean_reject();
                format!("call {p}.{method} failed: {e}{}", if unknown { " (outcome unknown: the call may have executed)" } else { "" })
            })?;
            svc.decode_reply(&enc.func, &reply.into_bytes())
        }

        fn principal(&self) -> String {
            ic_cdk::api::canister_self().to_text()
        }

        fn log(&self, line: String) {
            self.logs.push(line);
        }

        fn instruction_deadline(&self) -> u64 {
            self.base_instructions.saturating_add(self.max_instructions)
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
        pub logs: LogBuf,
        /// Only this canister is "allowlisted".
        pub allowed: &'static str,
    }

    impl Default for MockBridge {
        fn default() -> Self {
            Self { calls: RefCell::default(), logs: LogBuf::default(), allowed: "ryjl3-tyaaa-aaaaa-aaaba-cai" }
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
            self.logs.push(line);
        }
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
}
