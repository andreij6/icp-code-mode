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

pub use ic::{IcBridge, MAX_DID_BYTES, cached_service, evict_interface, pending_run_ids, remember_override};

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

/// Production bridge. Compiles natively too (ic0 stubs panic off-chain), which lets
/// `export_candid!` and tests build without a replica.
mod ic {
    use super::*;
    use crate::policy::{self, AuditEntry, AuditStatus};
    use candid::types::FuncMode;
    use candid::{CandidType, Deserialize, Principal};
    use ic_cdk::call::{Call, CallErrorExt};
    use std::future::IntoFuture;
    use std::cell::{Cell, RefCell};
    use std::collections::{BTreeMap, BTreeSet};
    use std::rc::Rc;

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
        if let Some((_, s)) = cache_get(&key) {
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
                timestamp_ns: ic_cdk::api::time(),
                caller,
                language,
                script_sha256: sha,
                canisters: self.touched.borrow().iter().cloned().collect(),
                calls: self.calls(),
                instructions: self.instructions_used(),
                status: AuditStatus::Trapped,
                violations: self.violations.borrow().clone(),
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
        /// Call-cap slots taken (attempts that passed policy), whether or not a message
        /// ended up being sent.
        slots: Cell<u32>,
        /// Set once the run's final audit entry is written.
        finished: Cell<bool>,
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
                slots: Cell::new(0),
                finished: Cell::new(false),
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

        /// Policy-check one outbound request and take a call-cap slot, before any await so
        /// concurrent requests (Promise.all / gather) are all counted.
        fn reserve(&self, p: &Principal, method: Option<&str>) -> Result<(), String> {
            // A stop waits for every outstanding call; a run must not keep adding new ones.
            if ic_cdk::api::canister_status() != ic_cdk::api::CanisterStatusCode::Running {
                return Err(self.deny("policy: the canister is stopping; no new calls".into()));
            }
            let n = self.slots.get();
            let used = self.instructions_used();
            match method {
                Some(m) => policy::check_call(p, m, n, used),
                None => policy::check_interface_fetch(p, n, used),
            }
            .map_err(|e| self.deny(e))?;
            self.slots.set(n + 1);
            Ok(())
        }

        /// Await an inter-canister call, writing its audit entry at the moment the system
        /// accepts the message: ic-cdk's call future is pending after its first poll only
        /// if `call_perform` succeeded, and resolves immediately with an error if the
        /// message was refused (no cycles, queue full). So the log has an entry exactly for
        /// the messages that were handed over, never for one that was not sent.
        ///
        /// `about` is the canister the message concerns (the target, or for a metadata
        /// request the canister whose interface is read); `label` names the message.
        async fn sent<T>(&self, about: &Principal, label: String, call: impl std::future::Future<Output = T>) -> T {
            on_first_pending(call, || self.record_message(about, label)).await
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
                timestamp_ns: ic_cdk::api::time(),
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

        /// 1. management `canister_metadata` (reads the public `candid:service` section; not
        ///    on every replica yet), 2. the `__get_candid_interface_tmp_hack` query that
        ///    Motoko and many Rust canisters export. Each message is logged as it is sent.
        ///
        /// Each message takes its own call-cap slot (the caller reserved the first). The
        /// error's `bool` says whether the target really answered with a failure, so the
        /// failure may be remembered; a policy refusal or a message the system refused on
        /// the spot says nothing about the target and is not.
        async fn fetch_did(&self, p: Principal) -> Result<String, (String, bool)> {
            let mgmt = Principal::management_canister();
            let meta = Call::bounded_wait(mgmt, "canister_metadata")
                .with_arg(MetadataArgs { canister_id: p, name: "candid:service".into() })
                .into_future();
            if let Ok(r) = self.sent(&p, format!("{mgmt}.canister_metadata({p})"), meta).await
                && let Ok(m) = candid::decode_one_with_config::<MetadataResult>(&r.into_bytes(), &fetch_decoder()) {
                    return String::from_utf8(m.value).map_err(|_| ("candid:service is not UTF-8".to_string(), true));
                }
            self.reserve(&p, None).map_err(|e| (e, false))?;
            let sent_before = self.calls();
            let hack = Call::bounded_wait(p, "__get_candid_interface_tmp_hack").into_future();
            let reply = self
                .sent(&p, format!("{p}.__get_candid_interface_tmp_hack"), hack)
                .await
                .map_err(|e| (e.to_string(), self.calls() > sent_before))?;
            candid::decode_one_with_config::<String>(&reply.into_bytes(), &fetch_decoder()).map_err(|e| (e.to_string(), true))
        }

        async fn service(&self, p: &Principal) -> Result<Rc<Service>, String> {
            let key = p.to_text();
            if let Ok(Some(r)) = override_service(&key, true) {
                return r;
            }
            if let Some((_, s)) = cache_get(&key) {
                return Ok(s);
            }
            self.candid(&key).await?; // populates DID_CACHE
            cache_get(&key).map(|(_, s)| s).ok_or_else(|| "candid cache miss".into())
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
            let fetched = self
                .fetch_did(p)
                .await
                .and_then(|did| match did.len() {
                    n if n > MAX_DID_BYTES => Err((format!("interface is {n} bytes, over the {MAX_DID_BYTES}-byte limit"), true)),
                    _ => Ok(did),
                })
                .and_then(|did| Service::parse(&did).map(|svc| (did, Rc::new(svc))).map_err(|e| (e, true)));
            let now = ic_cdk::api::time();
            match fetched {
                Ok((did, svc)) => {
                    DID_FAILED.with(|c| c.borrow_mut().remove(&key));
                    DID_CACHE.with(|c| c.borrow_mut().insert(key, (did.clone(), svc, now)));
                    Ok(did)
                }
                Err((e, remember)) => {
                    let msg = format!("could not fetch candid for {key} ({e}); a controller can register it with set_candid_override");
                    if remember {
                        DID_FAILED.with(|c| c.borrow_mut().insert(key, (msg.clone(), now)));
                    }
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
            // The interface fetch above may have awaited: check policy (allowlist, budget)
            // again right before sending, so nothing goes out past it.
            policy::check_call(&p, method, self.slots.get().saturating_sub(1), self.instructions_used()).map_err(|e| self.deny(e))?;
            if enc.bytes.len() > MAX_ARG_BYTES {
                return Err(format!("arguments are {} bytes encoded, over the {MAX_ARG_BYTES}-byte message limit", enc.bytes.len()));
            }
            // Always a bounded wait: a target that never answers cannot hold a run slot, or
            // this canister's stop, for longer than the timeout. A timed-out call may still
            // have executed on the target; the error says so.
            let secs = self.call_timeout.unwrap_or(policy::DEFAULT_CALL_TIMEOUT_SECONDS);
            let call = Call::bounded_wait(p, method).change_timeout(secs);
            let label = format!("{p}.{}", method.escape_debug());
            let reply = self.sent(&p, label, call.with_raw_args(&enc.bytes).into_future()).await.map_err(|e| {
                let unknown = !e.is_clean_reject();
                format!("call {p}.{} failed: {e}{}", method.escape_debug(), if unknown { " (outcome unknown: the call may have executed)" } else { "" })
            })?;
            svc.decode_reply(&enc.func, &reply.into_bytes()).map_err(|e| {
                // Most likely the target was upgraded and our copy of its interface is stale.
                evict_interface(&p);
                format!("{e}; the cached interface was dropped, so the next call fetches it again")
            })
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
