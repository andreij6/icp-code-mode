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
    /// SHA-256 hex digests of bearer tokens: `printf %s "$TOKEN" | shasum -a 256 | cut -d' ' -f1`.
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
    /// Run slots taken (at most `MAX_CONCURRENT_RUNS`): scripts and searches that fetch,
    /// in flight or lingering after a trap.
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

/// Install, upgrade and admin arguments must match their declared types exactly.
/// Candid's subtyping would otherwise decode a value under `opt` that does not fit (an
/// untyped `5` for a `nat32`, a misspelled field, `opt blob` for `opt text`) as `null`,
/// which these methods read as "not supplied" or "clear". Unknown record fields are refused
/// too, so a misspelled optional field is not skipped.
fn strict_args(bytes: &[u8], expected: impl FnOnce(&mut candid::types::internal::TypeContainer) -> Vec<candid::types::Type>) -> Result<(), String> {
    use binrw::BinRead;
    use candid::types::subtype::{Gamma, OptReport, subtype_with_config};
    use candid::types::{Field, Label, Type, TypeInner};
    let bytes = if bytes.is_empty() { b"DIDL\x00\x00".as_slice() } else { bytes };
    let bad = |e: &dyn std::fmt::Display| format!("arguments do not match the method's Candid types: {e}");
    // binrw's error is a multi-line backtrace (with build paths); one line is enough here.
    let header = candid::binary_parser::Header::read_le_args(&mut std::io::Cursor::new(bytes), (None,))
        .map_err(|_| "arguments are not valid Candid".to_string())?;
    let (wire_env, wire) = header.to_types().map_err(|e| bad(&e))?;
    let tuple = |ts: Vec<Type>| -> Type {
        TypeInner::Record(ts.into_iter().enumerate().map(|(i, ty)| Field { id: Label::Unnamed(i as u32).into(), ty }).collect()).into()
    };
    let mut c = candid::types::internal::TypeContainer::new();
    let want = tuple(expected(&mut c));
    let mut env = c.env;
    let wire = env.merge_type(wire_env, tuple(wire));
    subtype_with_config(OptReport::Error, &mut Gamma::default(), &env, &wire, &want).map_err(|e| bad(&e))?;
    // Subtyping lets extra record fields through; ours are not recursive, so this ends.
    fn no_extra_fields(env: &candid::TypeEnv, w: &Type, t: &Type) -> Result<(), String> {
        let (w, t) = (env.trace_type(w).map_err(|e| e.to_string())?, env.trace_type(t).map_err(|e| e.to_string())?);
        match (w.as_ref(), t.as_ref()) {
            (TypeInner::Opt(a) | TypeInner::Vec(a), TypeInner::Opt(b) | TypeInner::Vec(b)) => no_extra_fields(env, a, b),
            (_, TypeInner::Opt(b)) => no_extra_fields(env, &w, b),
            (TypeInner::Record(wf), TypeInner::Record(tf)) | (TypeInner::Variant(wf), TypeInner::Variant(tf)) => wf.iter().try_for_each(|f| {
                match tf.iter().find(|g| g.id == f.id) {
                    Some(g) => no_extra_fields(env, &f.ty, &g.ty),
                    None => Err(format!("unknown field {}", f.id)),
                }
            }),
            _ => Ok(()),
        }
    }
    no_extra_fields(&env, &wire, &want).map_err(|e| bad(&e))
}

fn strict_limits_args() -> Result<(), String> {
    strict_args(&ic_cdk::api::msg_arg_data(), |c| vec![c.add::<Limits>()])
}

fn strict_override_args() -> Result<(), String> {
    strict_args(&ic_cdk::api::msg_arg_data(), |c| vec![c.add::<Principal>(), c.add::<Option<String>>()])
}

// All state lives in stable structures, so no pre_upgrade serialization is needed.
#[ic_cdk::init]
fn init(args: Option<InitArgs>) {
    strict_args(&ic_cdk::api::msg_arg_data(), |c| vec![c.add::<Option<InitArgs>>()]).unwrap_or_else(|e| ic_cdk::trap(e));
    apply(args);
    set_certified_data();
}

/// With `certified_reads` off, tell the gateway to accept uncertified query responses
/// (initialize, tools/list, cached searches). With it on, certify nothing: every query
/// response then fails the gateway's check except an upgrade, so a single node cannot
/// answer in the canister's name.
fn set_certified_data() {
    let certified = policy::limits().certified_reads == Some(true);
    // Off-chain (native tests) there is no certified data to set.
    if cfg!(target_family = "wasm") {
        ic_cdk::api::certified_data_set(if certified { [0; 32] } else { SKIP_CERT_DATA });
    }
}

/// Upgrade arguments are accepted (the same `opt InitArgs` type keeps old deploy scripts
/// working) and ignored: re-applying them would bring back revoked tokens, re-add removed
/// allowlist entries and replace the limits. Change state with the admin methods.
/// An upgrade is recorded in the audit log (language `upgrade`, caller `controller`).
#[ic_cdk::post_upgrade]
fn post_upgrade(_: Option<InitArgs>) {
    set_certified_data(); // certified data resets on upgrade
    policy::append_audit(AuditEntry {
        id: 0,
        timestamp_ns: ic_cdk::api::time(),
        caller: "controller".into(),
        language: "upgrade".into(),
        script_sha256: String::new(),
        canisters: vec![],
        calls: 0,
        instructions: 0,
        status: AuditStatus::Ok,
        violations: vec![],
        error: None,
        run_id: None,
        methods: None,
        args: None,
    });
}

/// A run waiting on a reply would get it in the new module, where its callback traps and
/// it never gets a closing entry. Stopping the canister first lets those replies arrive.
/// (`skip_pre_upgrade` in `install_code` bypasses this.)
#[ic_cdk::pre_upgrade]
fn pre_upgrade() {
    let pending = bridge::pending_run_ids();
    // A stopped canister has no call outstanding, whatever the heap-only list still holds
    // (a run lingering after a trap, an entry restored by a snapshot).
    if !pending.is_empty() && ic_cdk::api::canister_status() != ic_cdk::api::CanisterStatusCode::Stopped {
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

/// How the `#[update]` macro decodes `http_request_update`'s argument; `inspect_message`
/// uses the same, so it admits exactly what the method can decode.
fn decode_update_req(bytes: &[u8]) -> Result<HttpUpdateRequest, candid::Error> {
    let mut c = candid::DecoderConfig::new();
    c.set_skipping_quota(10000);
    candid::utils::decode_one_with_config(bytes, &c)
}

#[ic_cdk::update]
async fn http_request_update(req: HttpUpdateRequest) -> HttpResponse {
    // Count every authenticated update, not only those that reach a tool: canister callers
    // skip `inspect_message`, and with certified_reads every request is an update.
    if let Ok(hash) = mcp::authenticate(&req.method, &req.url, &req.headers) {
        if let Some(e) = rate_limited(&hash) {
            return HttpResponse::too_many_requests(&e);
        }
        rate_count(&hash, this_minute());
    }
    mcp::http_update(&req, policy::limits().certified_reads == Some(true), run_tool).await
}

async fn run_tool(caller_hash: &str, req: ToolRequest) -> serde_json::Value {
    // Rate counting is done per HTTP request in `http_request_update`.
    if let Some(e) = cycles_check(ic_cdk::api::canister_liquid_cycle_balance()) {
        return mcp::tool_result(Err(e), vec![]);
    }
    match req {
        ToolRequest::Execute(call) => run_script(caller_hash, call).await,
        ToolRequest::Search(args) => run_search(caller_hash, args).await,
    }
}

/// Search from cached interfaces only; `None` if one must be fetched, or a remembered
/// fetch failure must be reported (both need an update, which writes the audit entry).
fn search_cached(args: &SearchArgs) -> Option<serde_json::Value> {
    let entries = search::scope(args, policy::list_allowed())
        .into_iter()
        .map(|(canister, allowed)| Some(Entry { service: Ok(Some(bridge::cached_service(&canister, false)?.ok()?)), canister, allowed }))
        .collect::<Option<Vec<_>>>()?;
    Some(search::render(args, &entries))
}

/// Update-path search: fetch the interfaces that are not cached yet (counted and
/// audited like script calls), then render.
async fn run_search(caller_hash: &str, args: SearchArgs) -> serde_json::Value {
    let scope = search::scope(&args, policy::list_allowed());
    // A search that has to fetch waits on replies like a script: it takes a run slot.
    let needs_fetch = scope.iter().any(|(c, _)| bridge::cached_service(c, true).is_none());
    let _slot = match search_slot(needs_fetch, caller_hash) {
        Ok(s) => s,
        Err(e) => return mcp::tool_result(Err(e), vec![]),
    };
    let caller = caller_hash.get(..16).unwrap_or(caller_hash).to_string();
    let script_sha256 = policy::sha256_hex(search_key(&args).as_bytes());
    let bridge = IcBridge::new(caller_hash, "search", script_sha256.clone());
    let mut entries = vec![];
    let mut fetch_errors = vec![];
    let mut halted: Option<String> = None;
    for (canister, allowed) in scope {
        let service = match (bridge::cached_service(&canister, true), &halted) {
            (Some(s), _) => {
                // A failure remembered from a recent fetch: report it in the audit log too.
                if let Err(e) = &s {
                    fetch_errors.push(e.clone());
                }
                s.map(Some)
            }
            // After a policy refusal (call cap, budget) every further fetch would be refused
            // too; don't try, and don't log one violation per remaining canister.
            (None, Some(why)) => Err(format!("not fetched: {why}; search with canister_id")),
            (None, None) => {
                // The service `candid` fetched, or the expired copy it served when a
                // re-fetch failed transiently (a TTL-checked cache read would miss that).
                let r = bridge.service(&canister).await.map(Some);
                if let Err(e) = &r {
                    if bridge::refuses_whole_run(e) {
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
    let (used, max) = (bridge.instructions_used(), bridge.limits().max_instructions);
    let (result, deny) = close_over_budget(Ok(search::render(&args, &entries)), used, max, &bridge.messages());
    let result = result.map_err(|e| if deny { bridge.deny(e) } else { e });
    let violations = bridge.closing_violations();
    // Audit whenever a fetch was attempted or failed: sent messages (a `Started` entry is
    // open), was refused by policy or by the system, or a remembered failure was served.
    if bridge.calls() > 0 || !violations.is_empty() || !fetch_errors.is_empty() {
        let status = match (violations.is_empty(), fetch_errors.is_empty() && result.is_ok()) {
            (false, _) => AuditStatus::PolicyViolation,
            (true, false) => AuditStatus::Error,
            (true, true) => AuditStatus::Ok,
        };
        let error = (!fetch_errors.is_empty() || result.is_err()).then(|| {
            let mut e = result.as_ref().err().into_iter().chain(&fetch_errors).cloned().collect::<Vec<_>>().join("; ");
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

/// The text whose SHA-256 is a search's `script_sha256`: `search:<query>:<canister id>`,
/// with the id in canonical principal text, so every spelling of one canister hashes alike.
fn search_key(args: &SearchArgs) -> String {
    let id = args.canister_id.as_deref().map(str::trim).unwrap_or("");
    let id = Principal::from_text(id).map_or_else(|_| id.to_string(), |p| p.to_text());
    format!("search:{}:{id}", args.query.as_deref().unwrap_or("").trim())
}

/// A run slot for a search that has to fetch an interface; none for one answered from cache.
fn search_slot(needs_fetch: bool, token: &str) -> Result<Option<RunSlot>, String> {
    needs_fetch.then(|| RunSlot::take(token)).transpose()
}

/// Applies the instruction budget to a run's outcome. Over budget with nothing sent, it is
/// a policy refusal (the `bool`: record a violation). Once messages went out it is not:
/// the error says so and lists them, since their effects may stand.
fn close_over_budget(out: Result<serde_json::Value, String>, used: u64, max: u64, sent: &[String]) -> (Result<serde_json::Value, String>, bool) {
    let over = used > max || out.as_ref().is_err_and(|e| e.starts_with("policy: instruction budget exceeded"));
    match (over, sent.len()) {
        (false, _) => (out, false),
        (true, 0) => (Err(format!("policy: instruction budget exceeded ({used} > {max})")), true),
        (true, n) => (Err(format!("instruction budget exceeded ({used} > {max}) after {n} message(s) were sent: {}", sent.join(", "))), false),
    }
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
    (used >= limit).then(|| format!("rate limited: at most {limit} update calls per minute per token; retry in a minute"))
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

/// Runs (scripts, and searches that fetch) allowed in flight at once. Each script
/// suspended on a call keeps its VM heap alive (up to 64 MiB for Python), so an unlimited
/// number could exhaust the canister's memory and trap every request.
pub const MAX_CONCURRENT_RUNS: u32 = 8;

/// Slots one token may hold, so one client cannot keep every other one "busy".
pub const MAX_RUNS_PER_TOKEN: u32 = 4;

// Slots taken: slot id -> (token, time in ms after which the slot is stale and freed).
// Heap only. A slot normally ends with its run; the expiry frees one whose run can no
// longer end (a snapshot restored with runs in flight) and ends the linger after a trap.
type Runs = (u64, std::collections::BTreeMap<u64, (String, u64)>);
thread_local!(static RUNS: std::cell::RefCell<Runs> = Default::default());

/// One of the `MAX_CONCURRENT_RUNS` slots, given back on drop.
struct RunSlot(u64);

impl RunSlot {
    fn take(token: &str) -> Result<Self, String> {
        Self::take_at(token, bridge::now_ms())
    }

    fn take_at(token: &str, now: u64) -> Result<Self, String> {
        RUNS.with(|r| {
            let (next, runs) = &mut *r.borrow_mut();
            runs.retain(|_, (_, until)| *until > now);
            if runs.values().filter(|(t, _)| t == token).count() as u32 >= MAX_RUNS_PER_TOKEN {
                return Err(format!("busy: this token already has {MAX_RUNS_PER_TOKEN} runs in flight; retry shortly"));
            }
            if runs.len() as u32 >= MAX_CONCURRENT_RUNS {
                return Err(format!("busy: {MAX_CONCURRENT_RUNS} runs are already in flight; retry shortly"));
            }
            if heap_busy() {
                return Err("busy: the canister heap is over half full; retry shortly".into());
            }
            *next += 1;
            runs.insert(*next, (token.to_string(), now + bridge::STALE_RUN_MS));
            Ok(RunSlot(*next))
        })
    }

    /// Give the slot back, or, after a trap (ic-cdk drops the run's future in the cleanup
    /// callback), keep it `TRAP_LINGER_SECONDS` more: calls the run sent alongside the one
    /// that trapped may still be outstanding, and each must end within its timeout.
    // ponytail: lingers even when no sibling call is outstanding (which one trapped is not
    // known here); a trapping run costs its token a slot for up to 6 minutes.
    fn release(id: u64, trapped: bool, now: u64) {
        RUNS.with(|r| {
            let runs = &mut r.borrow_mut().1;
            match (trapped, runs.get_mut(&id)) {
                (true, Some((_, until))) => *until = now + bridge::TRAP_LINGER_SECONDS * 1000,
                _ => _ = runs.remove(&id),
            }
        });
    }
}

/// Whether the heap is too full to start a run. A JS run that trapped can leave garbage in
/// the shared GC heap, so that is collected before refusing.
fn heap_busy() -> bool {
    let full = || counting_alloc::live() > counting_alloc::HEAP_CEILING / 2;
    #[cfg(feature = "js")]
    if full() {
        boa_gc::force_collect();
    }
    full()
}

/// Runs once free Wasm memory falls below `wasm_memory_threshold` (icp.yaml). Memory never
/// shrinks, but freeing the shared JS heap lets later runs reuse it. Also leaves a line in
/// the canister log, next to the traps that usually follow.
#[ic_cdk::on_low_wasm_memory]
fn on_low_wasm_memory() {
    #[cfg(feature = "js")]
    boa_gc::force_collect();
    ic_cdk::println!("low wasm memory: {} live heap bytes, {} runs in flight", counting_alloc::live(), active_runs());
}

impl Drop for RunSlot {
    fn drop(&mut self) {
        Self::release(self.0, ic_cdk::futures::is_recovering_from_trap(), bridge::now_ms());
    }
}

/// Slots taken now (running, or lingering after a trap).
fn active_runs() -> u32 {
    let now = bridge::now_ms();
    RUNS.with(|r| r.borrow().1.values().filter(|(_, until)| *until > now).count() as u32)
}

async fn run_script(caller_hash: &str, call: ToolCall) -> serde_json::Value {
    let _slot = match RunSlot::take(caller_hash) {
        Ok(s) => s,
        Err(e) => return mcp::tool_result(Err(e), vec![]),
    };
    let caller = caller_hash.get(..16).unwrap_or(caller_hash).to_string();
    let script_sha256 = policy::sha256_hex(call.code.as_bytes());
    let bridge = IcBridge::new(caller_hash, call.language.as_str(), script_sha256.clone());
    let mut out = LanguageRouter::execute(call.language, &call.code, &bridge).await;
    // A result too big to send is an error, and recorded as one.
    if let Ok(v) = &out
        && let Err(e) = mcp::result_text(v.clone())
    {
        out = Err(e);
    }
    let instructions = bridge.instructions_used();
    let (out, deny) = close_over_budget(out, instructions, bridge.limits().max_instructions, &bridge.messages());
    let out = out.map_err(|e| if deny { bridge.deny(e) } else { e });
    let violations = bridge.closing_violations();
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
/// This is a cost filter for ingress only, not the security boundary: a dishonest replica
/// can skip it, and calls from other canisters never pass through it. Such a call is paid
/// for by this canister (the base update fee plus decoding, about 5-6M cycles to refuse
/// one without a token), so every method still does its own check.
#[ic_cdk::inspect_message]
fn inspect_message() {
    let allowed = match ic_cdk::api::msg_method_name().as_str() {
        "http_request_update" => decode_update_req(&ic_cdk::api::msg_arg_data())
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
/// `vec {}` allowlists the interface only: scripts can read it with `icp.candid` (those
/// fetches are sent and logged) and search lists the canister with no methods, but every
/// call is refused.
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

/// Sets the limits and returns the previous ones. An omitted or `null` optional field
/// keeps its current value; give it explicitly to change it (`opt 300`, `opt false`,
/// `opt 120` are the defaults). Arguments that do not match the types are refused.
#[ic_cdk::update(guard = "controller", guard = "strict_limits_args")]
fn set_limits(limits: Limits) -> Limits {
    let old = policy::limits();
    policy::set_limits(merge_limits(limits, &old));
    set_certified_data();
    old
}

/// `new`, with each `null` optional field taken from `old`.
fn merge_limits(new: Limits, old: &Limits) -> Limits {
    Limits {
        call_timeout_seconds: new.call_timeout_seconds.or(old.call_timeout_seconds),
        certified_reads: new.certified_reads.or(old.certified_reads),
        tool_calls_per_minute: new.tool_calls_per_minute.or(old.tool_calls_per_minute),
        ..new
    }
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
/// clear it. Arguments that do not match the types (`opt blob`) are refused.
#[ic_cdk::update(guard = "controller", guard = "strict_override_args")]
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
        active_runs: active_runs(),
        pending_run_ids: bridge::pending_run_ids(),
        candid_overrides: policy::list_candid_overrides(),
        lost_audit_entries: policy::lost_audit_entries(),
    }
}

#[derive(CandidType, Deserialize)]
pub struct Metrics {
    pub cycle_balance: candid::Nat,
    /// Wasm (heap) memory in bytes, as the IC counts it: it grows and never shrinks.
    pub wasm_memory_bytes: u64,
    /// Heap bytes in use now; allocation fails past 1 GiB (see `HEAP_CEILING`).
    pub live_heap_bytes: u64,
    pub stable_memory_bytes: u64,
    pub audit_log_len: u64,
    pub active_runs: u32,
    pub pending_runs: u32,
}

/// Cycles, memory and run counts for operators. Like every query, it is refused while the
/// canister is frozen or stopping; `icp canister status` still works then.
#[ic_cdk::query(guard = "controller")]
fn get_metrics() -> Metrics {
    #[cfg(target_arch = "wasm32")]
    let wasm_memory_bytes = core::arch::wasm32::memory_size(0) as u64 * 65536;
    #[cfg(not(target_arch = "wasm32"))]
    let wasm_memory_bytes = 0;
    Metrics {
        cycle_balance: ic_cdk::api::canister_cycle_balance().into(),
        wasm_memory_bytes,
        live_heap_bytes: counting_alloc::live() as u64,
        stable_memory_bytes: ic_cdk::stable::stable_size() * 65536,
        audit_log_len: policy::audit_len(),
        active_runs: active_runs(),
        pending_runs: bridge::pending_run_ids().len() as u32,
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
    /// the growth back. Room for `MAX_CONCURRENT_RUNS` Python heaps and then some; eight Rhai
    /// runs all at their 128 MiB cap would reach it, and the message that crosses it traps.
    // ponytail: one global ceiling, not per run; per-run accounting if runs ever need more.
    pub const HEAP_CEILING: usize = 1 << 30;

    /// Heap bytes live now.
    pub fn live() -> usize {
        LIVE_MEMORY.load(Relaxed)
    }

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

    /// The memory counter is one process-wide number. The canister has a single thread,
    /// but `cargo test` runs tests on several, so runs with a memory cap (Python, Rhai, JS) on
    /// different test threads would see each other's allocations. Tests therefore take
    /// turns: a thread holds this lock while it has any such run in flight (several may
    /// interleave on one thread).
    #[cfg(test)]
    pub mod test_serial {
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
                CountThisThread(super::COUNT_THIS_THREAD.with(|c| c.replace(true)))
            }
        }
        impl Drop for CountThisThread {
            fn drop(&mut self) {
                super::COUNT_THIS_THREAD.with(|c| c.set(self.0));
            }
        }
    }
}

ic_cdk::export_candid!();

/// ICP replicas have no OS RNG. getrandom 0.3 and 0.4 (pulled in by ahash and
/// boa's `rand`) both resolve the "custom" backend (see .cargo/config.toml) to this
/// symbol. Replicas must agree bit-for-bit, so it is a fixed deterministic stream, and
/// public: hash seeds drawn from it are predictable and give no collision resistance
/// (input caps bound that work instead). Never use it for secrets.
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
    /// 104: a record without the optional fields keeps their stored values (through the
    /// endpoint itself, not only `merge_limits`).
    #[test]
    fn set_limits_keeps_omitted_options() {
        let kept = super::Limits { call_timeout_seconds: Some(60), certified_reads: Some(true), tool_calls_per_minute: Some(7), ..Default::default() };
        crate::policy::set_limits(kept);
        super::set_limits(super::Limits { max_calls_per_script: 3, ..Default::default() });
        let l = crate::policy::limits();
        crate::policy::set_limits(super::Limits::default());
        assert_eq!((l.max_calls_per_script, l.call_timeout_seconds, l.certified_reads, l.tool_calls_per_minute), (3, Some(60), Some(true), Some(7)));

        let old = super::Limits { call_timeout_seconds: Some(60), certified_reads: Some(true), tool_calls_per_minute: Some(7), ..Default::default() };
        let new = super::Limits { max_calls_per_script: 3, max_instructions: 9, call_timeout_seconds: None, certified_reads: None, tool_calls_per_minute: None };
        let m = super::merge_limits(new, &old);
        assert_eq!((m.max_calls_per_script, m.max_instructions), (3, 9));
        assert_eq!((m.call_timeout_seconds, m.certified_reads, m.tool_calls_per_minute), (Some(60), Some(true), Some(7)));
        // Defaults stay reachable when given explicitly.
        let m = super::merge_limits(super::Limits { call_timeout_seconds: Some(300), certified_reads: Some(false), ..Default::default() }, &old);
        assert_eq!((m.call_timeout_seconds, m.certified_reads), (Some(300), Some(false)));
    }

    /// 144: arguments that Candid would silently turn into `null` are refused.
    #[test]
    fn install_and_admin_args_are_decoded_strictly() {
        use super::{InitArgs, Limits, strict_args};
        use candid::Principal;
        let bytes = |t: &str| candid_parser::parse_idl_args(t).unwrap().to_bytes().unwrap();
        let init = |t: &str| strict_args(&bytes(t), |c| vec![c.add::<Option<InitArgs>>()]);
        let limits = |t: &str| strict_args(&bytes(t), |c| vec![c.add::<Limits>()]);
        let over = |t: &str| strict_args(&bytes(t), |c| vec![c.add::<Principal>(), c.add::<Option<String>>()]);
        // Untyped ints (as icp-cli passes init_args), a misspelled required field.
        assert!(init("(opt record { token_sha256 = vec {}; allowlist = vec {}; limits = opt record { max_calls_per_script = 5; max_instructions = 1_000_000_000 } })").is_err());
        assert!(init("(opt record { token_sha25 = vec {}; allowlist = vec {} })").is_err());
        let typed = "(opt record { token_sha256 = vec {}; allowlist = vec {}; limits = opt record { max_calls_per_script = 5 : nat32; max_instructions = 1_000_000_000 : nat64 } })";
        assert_eq!(init(typed), Ok(()));
        let args: Option<InitArgs> = candid::decode_one(&bytes(typed)).unwrap();
        assert_eq!(args.unwrap().limits.unwrap().max_calls_per_script, 5);
        assert_eq!(init("(null)"), Ok(()));
        assert_eq!(strict_args(&[], |c| vec![c.add::<Option<InitArgs>>()]), Ok(()), "no arguments at all");
        // An untyped `opt 60`, and a misspelled optional field.
        let base = "max_calls_per_script = 5 : nat32; max_instructions = 9 : nat64";
        assert!(limits(&format!("(record {{ {base}; call_timeout_seconds = opt 60 }})")).is_err());
        assert!(limits(&format!("(record {{ {base}; call_timeout_second = opt (60 : nat32) }})")).unwrap_err().contains("unknown field"));
        assert_eq!(limits(&format!("(record {{ {base}; call_timeout_seconds = opt (60 : nat32) }})")), Ok(()));
        assert_eq!(limits(&format!("(record {{ {base} }})")), Ok(()));
        // `opt blob` does not clear an override.
        assert!(over("(principal \"aaaaa-aa\", opt blob \"x\")").is_err());
        assert_eq!(over("(principal \"aaaaa-aa\", opt \"service : {}\")"), Ok(()));
        assert_eq!(over("(principal \"aaaaa-aa\", null)"), Ok(()));
        // Bytes that are not Candid get one short line, not binrw's backtrace.
        assert_eq!(strict_args(b"DIDLxx", |c| vec![c.add::<Limits>()]), Err("arguments are not valid Candid".into()));
    }

    /// 178: one canister, one hash, however its id is spelled.
    #[test]
    fn search_key_canonicalises_the_canister_id() {
        let key = |id: &str| super::search_key(&crate::search::SearchArgs { canister_id: Some(id.into()), query: Some(" icrc1 ".into()) });
        let k = key("ryjl3-tyaaa-aaaaa-aaaba-cai");
        assert_eq!(k, "search:icrc1:ryjl3-tyaaa-aaaaa-aaaba-cai");
        assert_eq!(key("  ryjl3-tyaaa-aaaaa-aaaba-cai\t"), k);
        assert_eq!(key("RYJL3-TYAAA-AAAAA-AAABA-CAI"), k);
    }

    /// 153: a search that must fetch takes a run slot; one answered from cache does not.
    #[test]
    fn fetching_searches_take_run_slots() {
        use super::{MAX_RUNS_PER_TOKEN, RunSlot, search_slot};
        let held: Vec<RunSlot> = (0..MAX_RUNS_PER_TOKEN).map(|_| RunSlot::take("s").unwrap()).collect();
        assert!(search_slot(true, "s").err().unwrap().contains("busy"));
        assert!(search_slot(false, "s").unwrap().is_none());
        // run_search itself refuses before fetching: an allowlisted canister with nothing cached.
        let p = candid::Principal::from_text("qoctq-giaaa-aaaaa-aaaea-cai").unwrap();
        crate::policy::set_allowed(p, vec!["*".into()]).unwrap();
        let args = crate::search::SearchArgs { canister_id: Some(p.to_text()), query: None };
        let out = futures::executor::block_on(super::run_search("s", args)).to_string();
        crate::policy::remove_allowed(p);
        assert!(out.contains("busy"), "{out}");
        drop(held);
        assert!(search_slot(true, "s").unwrap().is_some());
    }

    /// 181: the canister log is large enough to keep the low-memory line next to later traps.
    #[test]
    fn icp_yaml_sets_a_larger_canister_log() {
        assert!(include_str!("../icp.yaml").contains("log_memory_limit: 2097152"));
    }

    /// 77: once messages went out, running out of budget is not reported as a refusal.
    #[test]
    fn budget_overrun_after_sends_is_not_a_policy_refusal() {
        use super::close_over_budget;
        let ok = || Ok(serde_json::json!(1));
        assert_eq!(close_over_budget(ok(), 5, 10, &[]), (ok(), false));
        let (out, deny) = close_over_budget(ok(), 11, 10, &[]);
        assert!(deny && out.unwrap_err().starts_with("policy: instruction budget exceeded (11 > 10)"));
        let sent = ["ryjl3-tyaaa-aaaaa-aaaba-cai.icrc1_transfer".to_string()];
        let (out, deny) = close_over_budget(ok(), 11, 10, &sent);
        let e = out.unwrap_err();
        assert!(!deny && !e.starts_with("policy:") && e.contains("1 message(s)") && e.contains("icrc1_transfer"), "{e}");
        // A runtime's own budget stop after sends is rewritten the same way.
        let (out, deny) = close_over_budget(Err("policy: instruction budget exceeded".into()), 10, 10, &sent);
        assert!(!deny && out.unwrap_err().contains("icrc1_transfer"));
        assert_eq!(close_over_budget(Err("boom".into()), 1, 10, &sent), (Err("boom".into()), false));
    }

    /// 187: `inspect_message` decodes exactly what the method's macro decoder accepts.
    #[test]
    fn inspect_message_decoder_matches_the_method() {
        #[derive(candid::CandidType)]
        struct Padded {
            method: String,
            url: String,
            headers: Vec<(String, String)>,
            body: serde_bytes::ByteBuf,
            extra: serde_bytes::ByteBuf,
        }
        for n in [0, 9_999, 10_001, 20_000] {
            let req = Padded { method: "POST".into(), url: "/mcp".into(), headers: vec![], body: Default::default(), extra: vec![0; n].into() };
            let bytes = candid::encode_one(&req).unwrap();
            let mut cfg = candid::DecoderConfig::new();
            cfg.set_skipping_quota(10000);
            let method = candid::utils::decode_args_with_config::<(super::HttpUpdateRequest,)>(&bytes, &cfg).is_ok();
            assert_eq!(super::decode_update_req(&bytes).is_ok(), method, "{n} bytes");
            if n == 0 || n == 20_000 {
                assert_eq!(method, n == 0, "{n} bytes");
            }
        }
    }

    /// 120, 128: a trapped run keeps its slot for the linger time; a slot nobody can end
    /// (restored by a snapshot) is freed once stale.
    #[test]
    fn run_slots_linger_after_a_trap_and_expire_when_stale() {
        use super::{MAX_CONCURRENT_RUNS, RunSlot};
        use crate::bridge::{STALE_RUN_MS, TRAP_LINGER_SECONDS};
        let t = 1_000;
        let s = RunSlot::take_at("a", t).unwrap();
        RunSlot::release(s.0, true, t);
        std::mem::forget(s);
        let linger = t + TRAP_LINGER_SECONDS * 1000;
        let more: Vec<_> = (1..MAX_CONCURRENT_RUNS).map(|i| RunSlot::take_at(&format!("t{i}"), t).unwrap()).collect();
        assert!(RunSlot::take_at("z", linger - 1).is_err(), "the trapped run still holds its slot");
        let z = RunSlot::take_at("z", linger).unwrap();
        std::mem::forget(more); // never dropped, as after a snapshot load
        assert!(RunSlot::take_at("y", t + STALE_RUN_MS - 1).is_err());
        assert!(RunSlot::take_at("y", t + STALE_RUN_MS).is_ok(), "stale slots are freed");
        drop(z);
    }

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
        assert!(rate_check("a", 5).unwrap().contains("rate limited: at most 2 update calls"));
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
