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
    /// Interfaces set with `set_candid_override` (canister, bytes, SHA-256). Each one is
    /// used instead of fetching that canister's own interface.
    pub candid_overrides: Vec<(String, u64, String)>,
    /// Audit entries that could not be written (stable memory full) since the last upgrade.
    pub lost_audit_entries: u64,
}

fn apply(args: Option<InitArgs>) {
    let Some(a) = args else { return };
    for h in a.token_sha256 {
        policy::add_token_hash(&h, "init".into()).unwrap_or_else(|e| ic_cdk::trap(e));
    }
    let mut seen = std::collections::BTreeSet::new();
    for (c, m) in a.allowlist {
        if !seen.insert(c) {
            ic_cdk::trap(format!("canister {c} appears twice in the allowlist"));
        }
        policy::set_allowed(c, m).unwrap_or_else(|e| ic_cdk::trap(e));
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
    apply(args);
    set_certified_data();
}

/// With `certified_reads` off, tell the gateway to accept uncertified query responses
/// (initialize, tools/list, cached searches). With it on, certify nothing: every query
/// response then fails the gateway's check except an upgrade, so a single node cannot
/// answer in the canister's name.
fn set_certified_data() {
    let certified = policy::limits().certified_reads == Some(true);
    ic_cdk::api::certified_data_set(if certified { [0; 32] } else { SKIP_CERT_DATA });
}

/// Upgrade arguments are accepted (the same `opt InitArgs` type keeps old deploy scripts
/// working) and ignored: re-applying them would bring back revoked tokens, re-add removed
/// allowlist entries and replace the limits. Change state with the admin methods.
#[ic_cdk::post_upgrade]
fn post_upgrade(_: Option<InitArgs>) {
    set_certified_data(); // certified data resets on upgrade
}

/// A run waiting on a reply would get it in the new module, where its callback traps and
/// it never gets a closing entry. Stopping the canister first lets those replies arrive.
#[ic_cdk::pre_upgrade]
fn pre_upgrade() {
    let pending = bridge::pending_run_ids();
    if !pending.is_empty() {
        ic_cdk::trap(format!(
            "{} script run(s) are waiting on replies (run ids {pending:?}); stop the canister before upgrading",
            pending.len()
        ));
    }
}

// ---------- HTTP gateway ----------

#[ic_cdk::query]
fn http_request(req: HttpRequest) -> HttpResponse {
    use base64::Engine;
    let certified = policy::limits().certified_reads == Some(true);
    let mut r = mcp::http_query(&req, certified, search_cached);
    // Answer an over-limit token here, instead of letting the gateway upgrade the request
    // only for `inspect_message` to refuse it with an opaque error.
    if r.upgrade == Some(true)
        && let Some(e) = mcp::authenticate(&req.method, &req.url, &req.headers).ok().and_then(|h| rate_limited(&h)) {
            r = mcp::HttpResponse::too_many_requests(&e);
        }
    // Only available in non-replicated queries. With certified_reads the canister
    // certifies nothing, so these headers would not help: the response fails the check.
    if let (None, false, Some(cert)) = (r.upgrade, certified, ic_cdk::api::data_certificate()) {
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
    let minute = this_minute();
    if let Some(e) = rate_check(caller_hash, minute).or_else(|| cycles_check(ic_cdk::api::canister_liquid_cycle_balance())) {
        return mcp::tool_result(Err(e), vec![]);
    }
    rate_count(caller_hash, minute);
    match req {
        ToolRequest::Execute(call) => run_script(caller_hash, call).await,
        ToolRequest::Search(args) => run_search(caller_hash, args).await,
    }
}

/// Search from cached interfaces only; `None` if one must be fetched (needs an update).
fn search_cached(args: &SearchArgs) -> Option<serde_json::Value> {
    let entries = search::scope(args, policy::list_allowed())
        .into_iter()
        .map(|(canister, allowed)| Some(Entry { service: bridge::cached_service(&canister, false)?.map(Some), canister, allowed }))
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
        let service = match (bridge::cached_service(&canister, true), &halted) {
            (Some(s), _) => s.map(Some),
            // After a policy refusal (call cap, budget) every further fetch would be refused
            // too; don't try, and don't log one violation per remaining canister.
            (None, Some(why)) => Err(format!("not fetched: {why}; search with canister_id")),
            (None, None) => {
                use bridge::HostBridge;
                let r = bridge.candid(&canister.to_text()).await.map(|_| bridge::cached_service(&canister, true).and_then(Result::ok));
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
            args: None,
        });
    }
    bridge.finish();
    mcp::tool_result(result, vec![])
}

// Tool calls per token in the current minute: (minute, count). Heap only: an upgrade
// resets it, which is harmless. One entry per token that has called, so it stays small.
thread_local!(static RATE: std::cell::RefCell<std::collections::BTreeMap<String, (u64, u32)>> = Default::default());

fn this_minute() -> u64 {
    ic_cdk::api::time() / 60_000_000_000
}

/// The error for a token that used up `tool_calls_per_minute`, if it has.
fn rate_limited(token_hash: &str) -> Option<String> {
    rate_check(token_hash, this_minute())
}

fn rate_check(token_hash: &str, minute: u64) -> Option<String> {
    let limit = policy::limits().tool_calls_per_minute.unwrap_or(policy::DEFAULT_TOOL_CALLS_PER_MINUTE);
    let used = RATE.with(|r| r.borrow().get(token_hash).filter(|(m, _)| *m == minute).map_or(0, |(_, n)| *n));
    (used >= limit).then(|| format!("rate limited: at most {limit} tool calls per minute per token; retry in a minute"))
}

/// Liquid cycles (above the freezing threshold) below which no run is started. A run
/// grows the audit log; on the IC, growing stable memory without the cycles to reserve
/// storage traps rather than failing, which would lose the run's closing entry.
const MIN_LIQUID_CYCLES: u128 = 100_000_000_000;

fn cycles_check(liquid: u128) -> Option<String> {
    (liquid < MIN_LIQUID_CYCLES).then(|| "canister is low on cycles: no new runs until a controller tops it up".into())
}

fn rate_count(token_hash: &str, minute: u64) {
    RATE.with(|r| {
        let mut r = r.borrow_mut();
        let e = r.entry(token_hash.to_string()).or_insert((minute, 0));
        *e = if e.0 == minute { (minute, e.1 + 1) } else { (minute, 1) };
    });
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
struct RunSlot(String);

/// Slots one token may hold, so one client cannot keep every other one "busy".
pub const MAX_RUNS_PER_TOKEN: u32 = 4;

thread_local!(static RUNS_BY_TOKEN: std::cell::RefCell<std::collections::BTreeMap<String, u32>> = Default::default());

impl RunSlot {
    fn take(token: &str) -> Result<Self, String> {
        let mine = RUNS_BY_TOKEN.with(|m| m.borrow().get(token).copied().unwrap_or(0));
        if mine >= MAX_RUNS_PER_TOKEN {
            return Err(format!("busy: this token already has {MAX_RUNS_PER_TOKEN} scripts running; retry shortly"));
        }
        if ACTIVE_RUNS.with(|a| a.get()) >= MAX_CONCURRENT_RUNS {
            return Err(format!("busy: {MAX_CONCURRENT_RUNS} scripts are already running; retry shortly"));
        }
        ACTIVE_RUNS.with(|a| a.set(a.get() + 1));
        RUNS_BY_TOKEN.with(|m| *m.borrow_mut().entry(token.to_string()).or_default() += 1);
        Ok(RunSlot(token.to_string()))
    }
}

impl Drop for RunSlot {
    fn drop(&mut self) {
        ACTIVE_RUNS.with(|a| a.set(a.get().saturating_sub(1)));
        RUNS_BY_TOKEN.with(|m| {
            let mut m = m.borrow_mut();
            if let Some(n) = m.get_mut(&self.0) {
                *n -= 1;
                if *n == 0 {
                    m.remove(&self.0);
                }
            }
        });
    }
}

async fn run_script(caller_hash: &str, call: ToolCall) -> serde_json::Value {
    let _slot = match RunSlot::take(caller_hash) {
        Ok(s) => s,
        Err(e) => return mcp::tool_result(Err(e), vec![]),
    };
    let caller = caller_hash.get(..16).unwrap_or(caller_hash).to_string();
    let script_sha256 = policy::sha256_hex(call.code.as_bytes());
    let bridge = IcBridge::new(caller.clone(), call.language.as_str(), script_sha256.clone());
    let mut out = LanguageRouter::execute(call.language, &call.code, &bridge).await;
    // A result too big to send is an error, and recorded as one.
    if let Ok(v) = &out {
        if let Err(e) = mcp::result_text(v.clone()) {
            out = Err(e);
        }
    }
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
        // Script-authored: escaped so it cannot forge lines or hide text in the log.
        let mut e = policy::escape_invisible(e);
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
        args: None,
    });
    bridge.finish();
    mcp::tool_result(out, bridge.logs.take())
}

/// Runs on one replica before an ingress update call is accepted, at no charge. Without
/// it, anyone could call `http_request_update` directly (bypassing the gateway's query)
/// and make the canister pay for receiving and running each call, valid token or not.
/// Only requests the query path can upgrade (valid token within its rate limit,
/// well-formed, containing a `tools/call`), and controller calls to other methods, are
/// let in.
///
/// This is a cost filter, not the security boundary: a dishonest replica can skip it and
/// calls from other canisters never pass through it (they pay for themselves), so every
/// method still does its own check.
#[ic_cdk::inspect_message]
fn inspect_message() {
    let allowed = match ic_cdk::api::msg_method_name().as_str() {
        "http_request_update" => candid::decode_one::<HttpUpdateRequest>(&ic_cdk::api::msg_arg_data())
            .ok()
            .and_then(|r| mcp::may_upgrade(&r, policy::limits().certified_reads == Some(true)))
            .is_some_and(|hash| rate_limited(&hash).is_none()),
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

/// Refused (the call traps) for the management canister, this canister, or past the caps
/// on methods per canister, method name length and allowlist size. `"*"` allows every
/// method except delegating ones such as `icrc2_approve`, which must be listed by name.
#[ic_cdk::update(guard = "controller")]
fn set_allowlist(canister: Principal, methods: Vec<String>) {
    // A controller re-allowlisting a canister usually means its interface changed or a
    // fetch failed: start from a fresh fetch.
    bridge::evict_interface(&canister);
    policy::set_allowed(canister, methods).unwrap_or_else(|e| ic_cdk::trap(e));
}

#[ic_cdk::update(guard = "controller")]
fn remove_allowlist(canister: Principal) -> bool {
    bridge::evict_interface(&canister);
    policy::remove_allowed(canister)
}

/// Replaces every field (an omitted `opt` field is `null`, which means its default) and
/// returns the previous limits.
#[ic_cdk::update(guard = "controller")]
fn set_limits(limits: Limits) -> Limits {
    let old = policy::limits();
    policy::set_limits(limits);
    set_certified_data();
    old
}

#[ic_cdk::update(guard = "controller")]
fn add_api_token_hash(sha256_hex: String, label: String) -> Result<(), String> {
    policy::add_token_hash(&sha256_hex, label)
}

/// Takes the full hash or the 16-character id `get_policy` shows (any unique prefix of at
/// least 16 hex characters). `false`: no token, or the prefix matches more than one.
#[ic_cdk::update(guard = "controller")]
fn revoke_api_token_hash(sha256_hex: String) -> bool {
    policy::revoke_token_hash(&sha256_hex)
}

/// Register (or clear, with `null`) a `.did` for a target. While set it takes precedence:
/// the target's own interface is not fetched or refreshed. `remove_allowlist` does not
/// clear it.
#[ic_cdk::update(guard = "controller")]
fn set_candid_override(canister: Principal, did: Option<String>) -> Result<(), String> {
    if let Some(d) = &did {
        if d.len() > bridge::MAX_DID_BYTES {
            return Err(format!("interface is {} bytes, over the {}-byte limit", d.len(), bridge::MAX_DID_BYTES));
        }
        // Parsed here, in an update, and kept: the query path never parses an override.
        let svc = candid_bridge::Service::parse(d)?;
        policy::set_candid_override(canister, did.clone())?;
        bridge::remember_override(&canister, d.clone(), svc);
        return Ok(());
    }
    policy::set_candid_override(canister, None)
}

#[ic_cdk::query(guard = "controller")]
fn get_policy() -> PolicyView {
    PolicyView {
        limits: policy::limits(),
        allowlist: policy::list_allowed(),
        tokens: policy::list_tokens(),
        active_runs: ACTIVE_RUNS.with(|a| a.get()),
        pending_run_ids: bridge::pending_run_ids(),
        candid_overrides: policy::list_candid_overrides(),
        lost_audit_entries: policy::lost_audit_entries(),
    }
}

/// Newest first; `limit` is capped at 100.
#[ic_cdk::query(guard = "controller")]
fn get_audit_logs(offset: u64, limit: u64) -> Vec<AuditEntry> {
    policy::audit_page(offset, limit)
}

/// Newest first, below id `before` (`null`: from the newest); `limit` is capped at 100.
/// Pass the last id of a page to get the next one: unlike offsets, ids do not shift as
/// entries are appended.
#[ic_cdk::query(guard = "controller")]
fn get_audit_logs_before(before: Option<u64>, limit: u64) -> Vec<AuditEntry> {
    policy::audit_page_before(before, limit)
}

/// The entries of one run (`run_id`), oldest first, from id `from` (`null`: the run's
/// start). Looks at 5,000 ids per call at most: pass the last id returned plus one, or
/// `from` + 5,000 after an empty page, to continue. The run is complete once an entry with
/// a status other than `Started`/`Call` is returned.
#[ic_cdk::query(guard = "controller")]
fn get_audit_run(run_id: u64, from: Option<u64>) -> Vec<AuditEntry> {
    policy::audit_run(run_id, from)
}

#[ic_cdk::query(guard = "controller")]
fn get_audit_log_len() -> u64 {
    policy::audit_len()
}

/// Counts live heap bytes, for the heap ceiling (every build) and for Monty's
/// `ResourceLimits::max_memory`, which reads `monty_types::LIVE_MEMORY`.
///
/// The ceiling also keeps linear memory well below 4 GiB. The 8 MiB stack sits below the
/// heap; at a full 4 GiB a stack overflow would wrap to the top of memory instead of
/// trapping.
pub(crate) mod counting_alloc {
    #[cfg(feature = "python")]
    use monty_types::LIVE_MEMORY;
    #[cfg(not(feature = "python"))]
    static LIVE_MEMORY: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
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

    /// Most heap the canister may hold. Python's own limit is checked between steps, and
    /// JavaScript has none, so one builtin call (`json.dumps(x, indent=10**8)`,
    /// `"x".repeat(n)`) could otherwise grow the heap toward 4 GiB, and Wasm memory never
    /// shrinks. Past the ceiling an allocation fails, which traps the message and rolls
    /// the growth back. Room for `MAX_CONCURRENT_RUNS` Python heaps and then some.
    // ponytail: one global ceiling, not per run; per-run accounting if runs ever need more.
    pub const HEAP_CEILING: usize = 1 << 30;

    pub fn over_ceiling(live: usize, extra: usize) -> bool {
        live.saturating_add(extra) > HEAP_CEILING
    }

    /// Native tests share the process heap with the test harness: enforce only on-chain.
    fn refuse(extra: usize) -> bool {
        cfg!(target_arch = "wasm32") && over_ceiling(LIVE_MEMORY.load(Relaxed), extra)
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
            if refuse(l.size()) {
                return std::ptr::null_mut();
            }
            let p = unsafe { System.alloc(l) };
            if !p.is_null() {
                add(l.size());
            }
            p
        }
        unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
            if refuse(l.size()) {
                return std::ptr::null_mut();
            }
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
            if refuse(new.saturating_sub(l.size())) {
                return std::ptr::null_mut();
            }
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
        use super::{MAX_CONCURRENT_RUNS, MAX_RUNS_PER_TOKEN, RunSlot};
        // Per token first: a fifth run of one token is refused while others still get slots.
        let mut held: Vec<RunSlot> = (0..MAX_RUNS_PER_TOKEN).map(|_| RunSlot::take("a").expect("free slot")).collect();
        assert!(RunSlot::take("a").err().unwrap().contains("this token"));
        held.extend((0..MAX_CONCURRENT_RUNS - MAX_RUNS_PER_TOKEN).map(|i| RunSlot::take(&format!("t{i}")).unwrap()));
        assert!(RunSlot::take("z").is_err(), "no slot beyond the cap");
        held.pop(); // a run that ended (normally, or by a cleaned-up trap) gives its slot back
        let again = RunSlot::take("z");
        assert!(again.is_ok() && RunSlot::take("y").is_err());
        drop(held);
        drop(again);
        assert!(RunSlot::take("a").is_ok(), "per-token counts are returned too");
    }

    /// #180: runs are refused before the canister is too low on cycles to grow the log.
    #[test]
    fn runs_need_a_cycles_margin() {
        assert!(super::cycles_check(super::MIN_LIQUID_CYCLES - 1).unwrap().contains("low on cycles"));
        assert!(super::cycles_check(super::MIN_LIQUID_CYCLES).is_none());
    }

    #[test]
    fn tool_calls_are_rate_limited_per_token_and_minute() {
        use super::{rate_check, rate_count};
        crate::policy::set_limits(super::Limits { tool_calls_per_minute: Some(2), ..Default::default() });
        for _ in 0..2 {
            assert!(rate_check("a", 5).is_none());
            rate_count("a", 5);
        }
        assert!(rate_check("a", 5).unwrap().contains("rate limited"));
        assert!(rate_check("b", 5).is_none(), "per token");
        assert!(rate_check("a", 6).is_none(), "per minute");
        rate_count("a", 6);
        assert!(rate_check("a", 6).is_none());
        crate::policy::set_limits(super::Limits::default());
        assert!(rate_check("a", 5).is_none(), "default is 120");
    }

    /// #170: a principal listed twice in the init allowlist is refused, not silently merged.
    #[test]
    #[should_panic]
    fn duplicate_allowlist_principal_is_refused() {
        let p = candid::Principal::from_text("ryjl3-tyaaa-aaaaa-aaaba-cai").unwrap();
        super::apply(Some(super::InitArgs {
            token_sha256: vec![],
            allowlist: vec![(p, vec!["a".into()]), (p, vec!["b".into()])],
            limits: None,
        }));
    }

    #[test]
    fn did_file_matches_exported_interface() {
        let expected = super::__export_service();
        let actual = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/icp_code_mode.did")).unwrap();
        assert_eq!(actual.trim(), expected.trim(), "regenerate icp_code_mode.did (see README)");
    }

    #[test]
    fn set_limits_rejects_unsafe_instruction_and_timeout_bounds() {
        crate::policy::set_limits(super::Limits {
            max_calls_per_script: 1,
            max_instructions: 40_000_000_000,
            call_timeout_seconds: Some(999),
            ..Default::default()
        });
        let stored = crate::policy::limits();
        assert!(stored.max_instructions < 40_000_000_000);
        assert!(stored.call_timeout_seconds.is_none_or(|s| (1..=300).contains(&s)));
    }

    #[test]
    fn admin_helpers_and_search_cached_edge_cases() {
        let p = candid::Principal::from_text("ryjl3-tyaaa-aaaaa-aaaba-cai").unwrap();
        let h = crate::policy::sha256_hex(b"init-token");
        super::apply(None);
        super::apply(Some(super::InitArgs {
            token_sha256: vec![h.clone()],
            allowlist: vec![(p, vec!["ping".into()])],
            limits: Some(super::Limits::default()),
        }));
        assert!(crate::policy::verify_token("init-token").is_some());

        // search_cached returns Some once an override is registered (without touching `ic0::time`).
        assert!(super::set_candid_override(p, Some("service : { ping : () -> () }".into())).is_ok());
        assert!(super::search_cached(&crate::search::SearchArgs::default()).is_some());

        // Override validation rejects oversized and malformed interfaces.
        let big = "a".repeat(crate::bridge::MAX_DID_BYTES + 1);
        assert!(super::set_candid_override(p, Some(big)).is_err());
        assert!(super::set_candid_override(p, Some("broken".into())).is_err());
        assert!(super::set_candid_override(p, None).is_ok());

        // Token & allowlist helpers + counting_alloc fallback.
        let h2 = crate::policy::sha256_hex(b"other-token");
        assert!(super::add_api_token_hash(h2.clone(), "t2".into()).is_ok());
        assert!(super::revoke_api_token_hash(h2));
        assert!(super::revoke_api_token_hash(h));
        super::set_allowlist(p, vec!["*".into()]);
        assert!(super::remove_allowlist(p));
        assert_eq!(super::get_audit_log_len(), crate::policy::audit_len());
    }
}
