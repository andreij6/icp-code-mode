//! ICP canister exposing a single MCP "Code Mode" tool, `execute_icp_script`,
//! over the HTTP gateway. See README.md for deployment and curl examples.

pub mod bridge;
pub mod candid_bridge;
pub mod mcp;
pub mod policy;
pub mod runtimes;
pub mod search;

use bridge::IcBridge;
use candid::{CandidType, Deserialize, Principal};
use mcp::{HttpRequest, HttpResponse, HttpUpdateRequest, ToolCall, ToolRequest};
use search::{Entry, SearchArgs};
use policy::{AuditEntry, AuditStatus, Limits};
use runtimes::LanguageRouter;

#[derive(CandidType, Deserialize, Default)]
pub struct InitArgs {
    /// SHA-256 hex digests of bearer tokens: `echo -n "$TOKEN" | shasum -a 256`.
    pub token_sha256: Vec<String>,
    pub allowlist: Vec<(Principal, Vec<String>)>,
    pub limits: Option<Limits>,
}

#[derive(CandidType, Deserialize)]
pub struct PolicyView {
    pub limits: Limits,
    pub allowlist: Vec<(Principal, Vec<String>)>,
    /// (token hash prefix, label)
    pub tokens: Vec<(String, String)>,
    /// Scripts running right now (at most `MAX_CONCURRENT_RUNS`).
    pub active_runs: u32,
    /// Runs (by `run_id`) that have sent messages and not ended: still waiting on a reply.
    pub pending_run_ids: Vec<u64>,
}

fn apply(args: Option<InitArgs>) {
    let Some(a) = args else { return };
    for h in a.token_sha256 {
        policy::add_token_hash(&h, "init".into()).unwrap_or_else(|e| ic_cdk::trap(e));
    }
    for (c, m) in a.allowlist {
        policy::set_allowed(c, m);
    }
    if let Some(l) = a.limits {
        policy::set_limits(l);
    }
}

/// `ic_http_certification::utils::skip_certification_certified_data()`: lets the
/// gateway accept uncertified *query* responses (initialize, tools/list) on the
/// certified domain. Script results come from `http_request_update`, which is
/// certified by consensus anyway.
const SKIP_CERT_DATA: [u8; 32] = [
    85, 236, 195, 28, 62, 128, 71, 252, 21, 143, 32, 234, 10, 160, 96, 154, 172, 199, 181, 126, 6, 234, 64, 220, 65, 134,
    2, 114, 167, 214, 66, 145,
];

// All state lives in stable structures, so no pre_upgrade serialization is needed;
// post_upgrade accepts the same (optional) args to adjust policy during an upgrade.
#[ic_cdk::init]
fn init(args: Option<InitArgs>) {
    ic_cdk::api::certified_data_set(SKIP_CERT_DATA);
    apply(args);
}

#[ic_cdk::post_upgrade]
fn post_upgrade(args: Option<InitArgs>) {
    ic_cdk::api::certified_data_set(SKIP_CERT_DATA); // certified data resets on upgrade
    apply(args);
}

// ---------- HTTP gateway ----------

#[ic_cdk::query]
fn http_request(req: HttpRequest) -> HttpResponse {
    use base64::Engine;
    let certified = policy::limits().certified_reads == Some(true);
    let mut r = mcp::http_query(&req, certified, search_cached);
    // Only available in non-replicated queries.
    if let (None, Some(cert)) = (r.upgrade, ic_cdk::api::data_certificate()) {
        let cert = base64::engine::general_purpose::STANDARD.encode(cert);
        r.headers.push((
            "IC-Certificate".into(),
            format!(
                "certificate=:{cert}:, tree=:2dn3gwJJaHR0cF9leHBygwJDPCo+gwJYIMMautvQsFn51GT9bfTani3Ah659C0BGjTNyJtQTszcjggNA:, expr_path=:2dn3gmlodHRwX2V4cHJjPCo+:, version=2"
            ),
        ));
        r.headers.push((
            "IC-CertificateExpression".into(),
            "default_certification(ValidationArgs{no_certification:Empty{}})".into(),
        ));
    }
    r
}

#[ic_cdk::update]
async fn http_request_update(req: HttpUpdateRequest) -> HttpResponse {
    mcp::http_update(&req, run_tool).await
}

async fn run_tool(caller_hash: &str, req: ToolRequest) -> serde_json::Value {
    match req {
        ToolRequest::Execute(call) => run_script(caller_hash, call).await,
        ToolRequest::Search(args) => run_search(caller_hash, args).await,
    }
}

/// Search from cached interfaces only; `None` if one must be fetched (needs an update).
fn search_cached(args: &SearchArgs) -> Option<serde_json::Value> {
    let entries = search::scope(args, policy::list_allowed())
        .into_iter()
        .map(|(canister, allowed)| Some(Entry { service: Ok(Some(bridge::cached_service(&canister)?)), canister, allowed }))
        .collect::<Option<Vec<_>>>()?;
    Some(search::render(args, &entries))
}

/// Update-path search: fetch the interfaces that are not cached yet (counted and
/// audited like script calls), then render.
async fn run_search(caller_hash: &str, args: SearchArgs) -> serde_json::Value {
    let caller = caller_hash.get(..16).unwrap_or(caller_hash).to_string();
    let key = format!("search:{}:{}", args.query.as_deref().unwrap_or(""), args.canister_id.as_deref().unwrap_or(""));
    let script_sha256 = policy::sha256_hex(key.as_bytes());
    let bridge = IcBridge::new(caller.clone(), "search", script_sha256.clone());
    let mut entries = vec![];
    let mut fetch_errors = vec![];
    let mut halted: Option<String> = None;
    for (canister, allowed) in search::scope(&args, policy::list_allowed()) {
        let service = match (bridge::cached_service(&canister), &halted) {
            (Some(s), _) => Ok(Some(s)),
            // After a policy refusal (call cap, budget) every further fetch would be refused
            // too; don't try, and don't log one violation per remaining canister.
            (None, Some(why)) => Err(format!("not fetched: {why}; search with canister_id")),
            (None, None) => {
                use bridge::HostBridge;
                let r = bridge.candid(&canister.to_text()).await.map(|_| bridge::cached_service(&canister));
                if let Err(e) = &r {
                    if e.starts_with("policy:") {
                        halted = Some(e.clone());
                    } else {
                        fetch_errors.push(e.clone());
                    }
                }
                r
            }
        };
        entries.push(Entry { canister, allowed, service });
    }
    let mut result = Ok(search::render(&args, &entries));
    let (used, max) = (bridge.instructions_used(), policy::limits().max_instructions);
    if used > max {
        result = Err(bridge.deny(format!("policy: instruction budget exceeded ({used} > {max})")));
    }
    let mut violations = bridge.violations.take();
    if let n @ 1.. = bridge.violations_dropped.get() {
        violations.push(format!("…and {n} more"));
    }
    // Audit whenever a fetch was attempted: sent messages (a `Started` entry is open) or
    // was refused by policy (refusals must be logged even though nothing was sent).
    if bridge.calls() > 0 || !violations.is_empty() {
        let status = match (violations.is_empty(), fetch_errors.is_empty()) {
            (false, _) => AuditStatus::PolicyViolation,
            (true, false) => AuditStatus::Error,
            (true, true) => AuditStatus::Ok,
        };
        let error = (!fetch_errors.is_empty()).then(|| {
            let mut e = fetch_errors.join("; ");
            bridge::truncate(&mut e, MAX_AUDIT_ERROR_BYTES);
            e
        });
        policy::append_audit(AuditEntry {
            id: 0,
            timestamp_ns: ic_cdk::api::time(),
            caller,
            language: "search".into(),
            script_sha256,
            canisters: bridge.touched.take().into_iter().collect(),
            calls: bridge.calls(),
            instructions: bridge.instructions_used(),
            status,
            violations,
            error,
            run_id: bridge.run_id(),
            methods: Some(bridge.messages()),
        });
    }
    bridge.finish();
    mcp::tool_result(result, vec![])
}

/// Audit `error` cap; the full text already went to the client (truncated there too).
const MAX_AUDIT_ERROR_BYTES: usize = 4096;

/// Scripts allowed in flight at once. Each one suspended on a call keeps its VM heap
/// alive (up to 64 MiB for Python), so an unlimited number could exhaust the canister's
/// memory and trap every request.
pub const MAX_CONCURRENT_RUNS: u32 = 8;

thread_local!(static ACTIVE_RUNS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) });

/// One of the `MAX_CONCURRENT_RUNS` slots, given back on drop. If a run traps after an
/// await, ic-cdk drops its future in the cleanup callback, so the slot is still returned.
struct RunSlot;

impl RunSlot {
    fn take() -> Option<Self> {
        ACTIVE_RUNS.with(|a| (a.get() < MAX_CONCURRENT_RUNS).then(|| a.set(a.get() + 1))).map(|()| RunSlot)
    }
}

impl Drop for RunSlot {
    fn drop(&mut self) {
        ACTIVE_RUNS.with(|a| a.set(a.get().saturating_sub(1)));
    }
}

async fn run_script(caller_hash: &str, call: ToolCall) -> serde_json::Value {
    let Some(_slot) = RunSlot::take() else {
        return mcp::tool_result(Err(format!("busy: {MAX_CONCURRENT_RUNS} scripts are already running; retry shortly")), vec![]);
    };
    let caller = caller_hash.get(..16).unwrap_or(caller_hash).to_string();
    let script_sha256 = policy::sha256_hex(call.code.as_bytes());
    let bridge = IcBridge::new(caller.clone(), call.language.as_str(), script_sha256.clone());
    let mut out = LanguageRouter::execute(call.language, &call.code, &bridge).await;
    let instructions = bridge.instructions_used();
    let max = policy::limits().max_instructions;
    if instructions > max {
        let e = format!("policy: instruction budget exceeded ({instructions} > {max})");
        out = Err(bridge.deny(e));
    }
    let mut violations = bridge.violations.take();
    if let n @ 1.. = bridge.violations_dropped.get() {
        violations.push(format!("…and {n} more"));
    }
    let status = match (&out, violations.is_empty()) {
        (_, false) => AuditStatus::PolicyViolation,
        (Err(_), true) => AuditStatus::Error,
        (Ok(_), true) => AuditStatus::Ok,
    };
    let error = out.as_ref().err().map(|e| {
        let mut e = e.clone();
        bridge::truncate(&mut e, MAX_AUDIT_ERROR_BYTES);
        e
    });
    policy::append_audit(AuditEntry {
        id: 0,
        timestamp_ns: ic_cdk::api::time(),
        caller,
        language: call.language.as_str().into(),
        script_sha256,
        canisters: bridge.touched.take().into_iter().collect(),
        calls: bridge.calls(),
        instructions,
        status,
        violations,
        error,
        run_id: bridge.run_id(),
        methods: Some(bridge.messages()),
    });
    bridge.finish();
    mcp::tool_result(out, bridge.logs.take())
}

/// Runs on one replica before an ingress update call is accepted, at no charge. Without
/// it, anyone could call `http_request_update` directly (bypassing the gateway's query)
/// and make the canister pay for receiving and running each call, valid token or not.
/// Only calls with a registered bearer token, or from a controller, are let in.
///
/// This is a cost filter, not the security boundary: a dishonest replica can skip it and
/// calls from other canisters never pass through it (they pay for themselves), so every
/// method still does its own check.
#[ic_cdk::inspect_message]
fn inspect_message() {
    let allowed = match ic_cdk::api::msg_method_name().as_str() {
        "http_request_update" => candid::decode_one::<HttpUpdateRequest>(&ic_cdk::api::msg_arg_data())
            .is_ok_and(|r| mcp::authenticate(&r.method, &r.url, &r.headers).is_ok()),
        _ => ic_cdk::api::is_controller(&ic_cdk::api::msg_caller()),
    };
    if allowed {
        ic_cdk::api::accept_message();
    }
}

// ---------- admin (controllers only) ----------

fn controller() -> Result<(), String> {
    if ic_cdk::api::is_controller(&ic_cdk::api::msg_caller()) {
        Ok(())
    } else {
        Err("caller is not a controller".into())
    }
}

#[ic_cdk::update(guard = "controller")]
fn set_allowlist(canister: Principal, methods: Vec<String>) {
    policy::set_allowed(canister, methods);
}

#[ic_cdk::update(guard = "controller")]
fn remove_allowlist(canister: Principal) -> bool {
    bridge::evict_interface(&canister);
    policy::remove_allowed(canister)
}

#[ic_cdk::update(guard = "controller")]
fn set_limits(limits: Limits) {
    policy::set_limits(limits);
}

#[ic_cdk::update(guard = "controller")]
fn add_api_token_hash(sha256_hex: String, label: String) -> Result<(), String> {
    policy::add_token_hash(&sha256_hex, label)
}

#[ic_cdk::update(guard = "controller")]
fn revoke_api_token_hash(sha256_hex: String) -> bool {
    policy::revoke_token_hash(&sha256_hex)
}

/// Register (or clear, with `null`) a `.did` for a target that does not expose
/// `__get_candid_interface_tmp_hack`.
#[ic_cdk::update(guard = "controller")]
fn set_candid_override(canister: Principal, did: Option<String>) -> Result<(), String> {
    if let Some(d) = &did {
        if d.len() > bridge::MAX_DID_BYTES {
            return Err(format!("interface is {} bytes, over the {}-byte limit", d.len(), bridge::MAX_DID_BYTES));
        }
        candid_bridge::Service::parse(d)?;
    }
    policy::set_candid_override(canister, did);
    Ok(())
}

#[ic_cdk::query(guard = "controller")]
fn get_policy() -> PolicyView {
    PolicyView {
        limits: policy::limits(),
        allowlist: policy::list_allowed(),
        tokens: policy::list_tokens(),
        active_runs: ACTIVE_RUNS.with(|a| a.get()),
        pending_run_ids: bridge::pending_run_ids(),
    }
}

/// Newest first; `limit` is capped at 100.
#[ic_cdk::query(guard = "controller")]
fn get_audit_logs(offset: u64, limit: u64) -> Vec<AuditEntry> {
    policy::audit_page(offset, limit)
}

#[ic_cdk::query(guard = "controller")]
fn get_audit_log_len() -> u64 {
    policy::audit_len()
}

/// Monty's `ResourceLimits::max_memory` is only enforced when the global
/// allocator reports into `monty_types::LIVE_MEMORY`.
#[cfg(feature = "python")]
pub(crate) mod counting_alloc {
    use monty_types::LIVE_MEMORY;
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::sync::atomic::Ordering::Relaxed;

    // Native tests run in parallel threads; only count the thread running Python so
    // other tests' heap traffic cannot skew the limit. The canister is single-threaded.
    #[cfg(test)]
    thread_local!(pub static COUNT_THIS_THREAD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) });

    fn counted() -> bool {
        #[cfg(test)]
        return COUNT_THIS_THREAD.try_with(|c| c.get()).unwrap_or(false);
        #[cfg(not(test))]
        true
    }

    fn add(n: usize) {
        if counted() {
            LIVE_MEMORY.fetch_add(n, Relaxed);
        }
    }

    fn sub(n: usize) {
        if counted() {
            let _ = LIVE_MEMORY.try_update(Relaxed, Relaxed, |v| Some(v.saturating_sub(n)));
        }
    }

    struct Counting;

    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, l: Layout) -> *mut u8 {
            let p = unsafe { System.alloc(l) };
            if !p.is_null() {
                add(l.size());
            }
            p
        }
        unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
            let p = unsafe { System.alloc_zeroed(l) };
            if !p.is_null() {
                add(l.size());
            }
            p
        }
        unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
            unsafe { System.dealloc(p, l) };
            sub(l.size());
        }
        unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
            let q = unsafe { System.realloc(p, l, new) };
            if !q.is_null() {
                // Apply the net change, so the counter never shows old + new at once.
                match new.checked_sub(l.size()) {
                    Some(grown) => add(grown),
                    None => sub(l.size() - new),
                }
            }
            q
        }
    }

    #[global_allocator]
    static ALLOC: Counting = Counting;
}

ic_cdk::export_candid!();

/// ICP replicas have no OS RNG. getrandom 0.3 and 0.4 (pulled in by ahash and
/// boa's `rand`) both resolve the "custom" backend (see .cargo/config.toml) to this
/// symbol. Replicas must agree bit-for-bit, so it is a fixed deterministic stream:
/// fine for hash seeding, never use it for secrets.
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
unsafe extern "Rust" fn __getrandom_v03_custom(dest: *mut u8, len: usize) -> Result<(), getrandom::Error> {
    use std::cell::Cell;
    thread_local!(static STATE: Cell<u64> = const { Cell::new(0x9E37_79B9_7F4A_7C15) });
    STATE.with(|s| {
        for i in 0..len {
            // splitmix64
            let mut z = s.get().wrapping_add(0x9E37_79B9_7F4A_7C15);
            s.set(z);
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            unsafe { *dest.add(i) = (z ^ (z >> 31)) as u8 };
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn run_slots_are_capped_and_returned() {
        use super::{MAX_CONCURRENT_RUNS, RunSlot};
        let mut held: Vec<RunSlot> = (0..MAX_CONCURRENT_RUNS).map(|_| RunSlot::take().expect("free slot")).collect();
        assert!(RunSlot::take().is_none(), "no slot beyond the cap");
        held.pop(); // a run that ended (normally, or by a cleaned-up trap) gives its slot back
        let again = RunSlot::take();
        assert!(again.is_some() && RunSlot::take().is_none());
    }

    #[test]
    fn did_file_matches_exported_interface() {
        let expected = super::__export_service();
        let actual = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/icp_code_mode.did")).unwrap();
        assert_eq!(actual.trim(), expected.trim(), "regenerate icp_code_mode.did (see README)");
    }
}
