//! On-chain guardrails: canister/method allowlist, per-script execution caps,
//! SHA-256 hashed bearer tokens, and an append-only audit log.
//!
//! Everything lives in stable memory via `ic-stable-structures`, so upgrades
//! keep it without any pre/post-upgrade serialization.

use candid::{CandidType, Decode, Deserialize, Encode, Principal};
use ic_stable_structures::memory_manager::{MemoryId, MemoryManager, VirtualMemory};
use ic_stable_structures::storable::Bound;
use ic_stable_structures::{DefaultMemoryImpl, Memory, StableBTreeMap, StableCell, StableLog, Storable};
use sha2::{Digest, Sha256};
use std::borrow::Cow;
use std::cell::RefCell;

type Mem = VirtualMemory<DefaultMemoryImpl>;

/// Default cap on inter-canister calls per script.
pub const DEFAULT_MAX_CALLS: u32 = 25;
/// Default instruction budget per script (IC hard limit for an update is 40B).
pub const DEFAULT_MAX_INSTRUCTIONS: u64 = 20_000_000_000;
/// Call timeout when `Limits::call_timeout_seconds` is `null`.
pub const DEFAULT_CALL_TIMEOUT_SECONDS: u32 = 300;
/// Default for `Limits::tool_calls_per_minute`.
pub const DEFAULT_TOOL_CALLS_PER_MINUTE: u32 = 120;
/// Hard cap on entries returned by one `get_audit_logs` page.
pub const MAX_AUDIT_PAGE: u64 = 100;

#[derive(CandidType, Deserialize, Clone, Debug, PartialEq)]
pub struct Limits {
    pub max_calls_per_script: u32,
    /// Checked against `call_context_instruction_counter()` before every call and after the run.
    /// At most 30B (larger values are stored as 30B).
    pub max_instructions: u64,
    /// Seconds a call waits for its reply (1-300, the IC's range; other values are stored
    /// clamped). `null` (default): 300. A timed-out call may or may not have executed on
    /// the target; the error says so.
    pub call_timeout_seconds: Option<u32>,
    /// `true`: every authenticated /mcp request goes through an update call, so responses
    /// are certified by consensus (about 1-2 s each) and the uncertified demo page is disabled.
    /// `null`/`false` (default): initialize, tools/list and cached searches are answered
    /// by fast query calls, which a single malicious node could alter.
    pub certified_reads: Option<bool>,
    /// Update calls each bearer token may make per minute (each runs at most one tool
    /// call; with `certified_reads` every MCP request is one). `null` (default): 120.
    /// Over the limit, requests are refused before they run, so nothing is
    /// spent and nothing is logged. This bounds how fast one token can spend cycles and
    /// grow the audit log.
    pub tool_calls_per_minute: Option<u32>,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_calls_per_script: DEFAULT_MAX_CALLS,
            max_instructions: DEFAULT_MAX_INSTRUCTIONS,
            call_timeout_seconds: None,
            certified_reads: None,
            tool_calls_per_minute: None,
        }
    }
}

/// Methods a script may call on one canister. `["*"]` allows every method except the
/// delegating ones (`DELEGATING_METHODS`), which must be listed by name. `"*"` is always
/// the wildcard: a method literally named `*` is reachable only through it.
#[derive(CandidType, Deserialize, Clone, Debug, PartialEq)]
pub struct AllowedMethods(pub Vec<String>);

#[derive(CandidType, Deserialize, Clone, Debug, PartialEq)]
pub enum AuditStatus {
    /// Written as a script's first outbound message is handed to the system. A run whose
    /// entries are only `Started`/`Call` has no closing entry (yet). Normally that means it
    /// is still waiting on a reply (`get_policy().pending_run_ids` lists those). A run
    /// that trapped gets a `Trapped` entry. If neither applies, treat it as unknown.
    Started,
    /// Written for each later outbound message of the run. Like `Started`, it is
    /// committed together with the message it describes, so it stays even if the run
    /// never finishes.
    Call,
    /// The canister trapped while handling a reply, after messages had gone out. Written
    /// during cleanup, in place of the final entry that the trap rolled back. Its
    /// `instructions` leaves out the trapped execution (rolled back with the trap), so it
    /// is a lower bound.
    Trapped,
    Ok,
    Error,
    PolicyViolation,
}

#[derive(CandidType, Deserialize, Clone, Debug, PartialEq)]
pub struct AuditEntry {
    pub id: u64,
    pub timestamp_ns: u64,
    /// First 16 hex chars of the bearer token's SHA-256 (never the token itself).
    pub caller: String,
    pub language: String,
    pub script_sha256: String,
    pub canisters: Vec<String>,
    pub calls: u32,
    /// Instructions the run used so far. On a `Trapped` entry, a lower bound (it leaves out
    /// the trapped execution).
    pub instructions: u64,
    pub status: AuditStatus,
    /// Policy refusals. They may quote script- or target-authored text, which is escaped
    /// and quoted (as are labels in `methods`).
    pub violations: Vec<String>,
    /// May hold script- or target-authored text (a script's own error, a target's reject),
    /// with control and bidi characters escaped.
    pub error: Option<String>,
    /// Links all entries of one run: the id of the run's `Started` entry, or the entry's
    /// own id if the run never sent a message. Script hashes alone can't tell apart two
    /// runs of the same script (one finished, one not), and runs interleave across awaits.
    /// `None` only on entries written before this field existed.
    pub run_id: Option<u64>,
    /// Outbound messages as `canister.method`, in order. Interface fetches appear as
    /// `aaaaa-aa.canister_metadata(<target>)` and `<target>.__get_candid_interface_tmp_hack`. On a `Started`/`Call` entry: the one message about to
    /// be sent. On a final entry: all of them. `None` on entries from before this field.
    /// Pages cut long lists (see `audit_page`); the `Started`/`Call` entries have each one.
    pub methods: Option<Vec<String>>,
    /// On a `Started`/`Call` entry for a script's call: its Candid-encoded argument, as hex
    /// (decode with `didc decode`), or past 1 KiB as `sha256:<hex> prefix:<first 512 bytes
    /// as hex>`. `None` on interface fetches, final entries and entries from before this field.
    pub args: Option<Vec<String>>,
}

macro_rules! candid_storable {
    ($($t:ty),*) => {$(
        impl Storable for $t {
            fn to_bytes(&self) -> Cow<'_, [u8]> {
                Cow::Owned(Encode!(self).expect("candid encode"))
            }
            fn into_bytes(self) -> Vec<u8> {
                Encode!(&self).expect("candid encode")
            }
            fn from_bytes(bytes: Cow<[u8]>) -> Self {
                Decode!(&bytes, Self).expect("candid decode")
            }
            const BOUND: Bound = Bound::Unbounded;
        }
    )*};
}
candid_storable!(Limits, AllowedMethods, AuditEntry);

thread_local! {
    static MM: RefCell<MemoryManager<DefaultMemoryImpl>> =
        RefCell::new(MemoryManager::init(DefaultMemoryImpl::default()));
    static LIMITS: RefCell<StableCell<Limits, Mem>> =
        RefCell::new(StableCell::init(mem(0), Limits::default()));
    static ALLOWLIST: RefCell<StableBTreeMap<Principal, AllowedMethods, Mem>> =
        RefCell::new(StableBTreeMap::init(mem(1)));
    /// token sha256 hex -> label
    static TOKENS: RefCell<StableBTreeMap<String, String, Mem>> =
        RefCell::new(StableBTreeMap::init(mem(2)));
    static AUDIT: RefCell<StableLog<AuditEntry, Mem, Mem>> =
        RefCell::new(StableLog::init(mem(3), mem(4)));
    /// canister id text -> admin-supplied .did (for targets that expose no interface)
    static CANDID_OVERRIDES: RefCell<StableBTreeMap<String, String, Mem>> =
        RefCell::new(StableBTreeMap::init(mem(5)));
}

fn mem(id: u8) -> Mem {
    MM.with(|m| m.borrow().get(MemoryId::new(id)))
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

// ---------- limits ----------

/// The limits in effect: normalized on read too, so a record stored before the clamp
/// existed (or by an older version) cannot bring back an unsafe value.
pub fn limits() -> Limits {
    normalized(LIMITS.with(|c| c.borrow().get().clone()))
}

fn normalized(mut l: Limits) -> Limits {
    l.max_instructions = l.max_instructions.min(MAX_INSTRUCTIONS_CAP);
    l.call_timeout_seconds = l.call_timeout_seconds.map(|s| s.clamp(1, 300));
    l
}

/// Highest `max_instructions` accepted. Budgets are checked between steps, not on every
/// instruction, and one step can be large; this leaves 10B of headroom under the IC's 40B
/// per-message limit, so the budget error fires before the IC traps. Python's VM caps each
/// turn lower (`MAX_TURN_INSTRUCTIONS` in runtimes/python.rs).
pub const MAX_INSTRUCTIONS_CAP: u64 = 30_000_000_000;

/// Stores `l` with out-of-range values clamped: `max_instructions` to
/// `MAX_INSTRUCTIONS_CAP`, `call_timeout_seconds` to 1..=300 (the IC's range).
/// `get_policy` shows the values in effect.
pub fn set_limits(l: Limits) {
    LIMITS.with(|c| c.borrow_mut().set(normalized(l)));
}

// ---------- allowlist ----------

/// Caps on admin input, so `get_policy` (one reply, at most 2 MiB) always fits: about
/// 0.8 MB of allowlist, 0.15 MB of tokens and 0.15 MB of override rows at most.
pub const MAX_ALLOWLISTED: u64 = 1000;
pub const MAX_METHODS_PER_CANISTER: usize = 256;
/// Method name bytes over the whole allowlist.
pub const MAX_ALLOWLIST_BYTES: usize = 512 * 1024;
pub const MAX_TOKENS: u64 = 1000;
pub const MAX_CANDID_OVERRIDES: u64 = 1000;
pub const MAX_LABEL_BYTES: usize = 128;

/// Methods that hand authority to someone else: an approval outlives token revocation
/// and `remove_allowlist`. `"*"` does not include them; they must be listed by name.
pub const DELEGATING_METHODS: &[&str] = &["icrc2_approve", "icrc37_approve_tokens", "icrc37_approve_collection", "approve"];

/// Whether an allowlist entry admits `method` (see `DELEGATING_METHODS` for `"*"`).
pub fn method_allowed(allowed: &[String], method: &str) -> bool {
    allowed.iter().any(|m| m == method || (m == "*" && !DELEGATING_METHODS.contains(&method)))
}

/// Targets no script may call, whatever the allowlist says: the management canister (the
/// canister a call acts on is an argument, which the allowlist and the audit log cannot
/// see) and this canister (a self-call would start a new run, not linked to this one,
/// with a fresh budget and call cap).
fn refused_target(canister: &Principal, self_id: Option<Principal>) -> Option<String> {
    if *canister == Principal::management_canister() {
        return Some("policy: the management canister (aaaaa-aa) cannot be called: the canister a call acts on is an argument, which the allowlist and audit log cannot see".into());
    }
    (Some(*canister) == self_id).then(|| "policy: this canister cannot call itself".into())
}

fn self_id() -> Option<Principal> {
    #[cfg(target_arch = "wasm32")]
    return Some(ic_cdk::api::canister_self());
    #[cfg(not(target_arch = "wasm32"))]
    None
}

pub fn set_allowed(canister: Principal, methods: Vec<String>) -> Result<(), String> {
    allowed_entry_ok(&canister, &methods, self_id())?;
    ALLOWLIST.with(|m| m.borrow_mut().insert(canister, AllowedMethods(methods)));
    Ok(())
}

fn allowed_entry_ok(canister: &Principal, methods: &[String], self_id: Option<Principal>) -> Result<(), String> {
    if let Some(e) = refused_target(canister, self_id) {
        return Err(e);
    }
    if methods.len() > MAX_METHODS_PER_CANISTER {
        return Err(format!("at most {MAX_METHODS_PER_CANISTER} methods per canister"));
    }
    if methods.iter().any(|m| m.len() > crate::bridge::MAX_METHOD_LEN) {
        return Err(format!("method names are at most {} bytes", crate::bridge::MAX_METHOD_LEN));
    }
    let (count, bytes) = ALLOWLIST.with(|m| {
        m.borrow().iter().filter(|e| e.key() != canister).fold((0, 0), |(n, b), e| (n + 1, b + e.value().0.iter().map(String::len).sum::<usize>()))
    });
    if count >= MAX_ALLOWLISTED {
        return Err(format!("at most {MAX_ALLOWLISTED} canisters can be allowlisted"));
    }
    if bytes + methods.iter().map(String::len).sum::<usize>() > MAX_ALLOWLIST_BYTES {
        return Err(format!("the allowlist's method names may total at most {MAX_ALLOWLIST_BYTES} bytes"));
    }
    Ok(())
}

pub fn remove_allowed(canister: Principal) -> bool {
    ALLOWLIST.with(|m| m.borrow_mut().remove(&canister).is_some())
}

pub fn list_allowed() -> Vec<(Principal, Vec<String>)> {
    ALLOWLIST.with(|m| m.borrow().iter().map(|e| (*e.key(), e.value().0)).collect())
}

/// Decide whether a script may issue one more call. Pure function of its inputs
/// so it is unit-testable; `check_call` feeds it the allowlist.
pub fn authorize(
    allowed: Option<&AllowedMethods>,
    canister: &str,
    method: &str,
    calls_made: u32,
    instructions_used: u64,
    limits: &Limits,
) -> Result<(), String> {
    let methods = allowed.ok_or_else(|| format!("policy: canister {canister} is not allowlisted"))?;
    if !method_allowed(&methods.0, method) {
        let hint = if DELEGATING_METHODS.contains(&method) && methods.0.iter().any(|m| m == "*") {
            "; `*` does not include methods that delegate authority, list it by name"
        } else {
            ""
        };
        return Err(format!("policy: method {} on {canister} is not allowlisted{hint}", shown(method)));
    }
    if calls_made >= limits.max_calls_per_script {
        return Err(format!("policy: call limit of {} per script reached", limits.max_calls_per_script));
    }
    if instructions_used > limits.max_instructions {
        return Err(format!(
            "policy: instruction budget exceeded ({instructions_used} > {})",
            limits.max_instructions
        ));
    }
    Ok(())
}

/// `s` as is when it is plainly a name (ASCII letters, digits, `_`, `-`), otherwise
/// Debug-quoted. For script- or target-authored text put into audit sentences and
/// labels: quotes, control and bidi characters are escaped and the quotes mark where
/// the text starts and ends, so it cannot pass for another sentence or label.
pub fn shown(s: &str) -> Cow<'_, str> {
    if !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
        Cow::Borrowed(s)
    } else {
        Cow::Owned(format!("{s:?}"))
    }
}

/// `s` with control, bidi and other invisible characters escaped (as Rust's Debug does)
/// and printable text, quotes included, kept. For free text such as a script's own error:
/// it stays on one line and cannot hide or reorder what follows.
pub fn escape_invisible(s: &str) -> String {
    s.chars().map(|c| if matches!(c, '"' | '\'' | '\\') { c.to_string() } else { c.escape_debug().to_string() }).collect()
}

/// `limits`: the run's snapshot (see `IcBridge::limits`), so a run keeps the limits it started with.
pub fn check_call(canister: &Principal, method: &str, calls_made: u32, instructions_used: u64, limits: &Limits) -> Result<(), String> {
    // Also for entries stored before `set_allowed` refused these targets.
    if let Some(e) = refused_target(canister, self_id()) {
        return Err(e);
    }
    let allowed = ALLOWLIST.with(|m| m.borrow().get(canister));
    authorize(allowed.as_ref(), &canister.to_text(), method, calls_made, instructions_used, limits)
}

/// Interface fetches (`icp.candid`) may target any allowlisted canister and count
/// against the same per-script cap and budget as calls.
pub fn check_interface_fetch(canister: &Principal, calls_made: u32, instructions_used: u64, limits: &Limits) -> Result<(), String> {
    let any = ALLOWLIST.with(|m| m.borrow().contains_key(canister)).then(|| AllowedMethods(vec!["*".into()]));
    authorize(any.as_ref(), &canister.to_text(), "<candid interface>", calls_made, instructions_used, limits)
}

// ---------- bearer tokens ----------

/// Register a token by its SHA-256 hex digest (the plaintext never touches the chain).
pub fn add_token_hash(sha256_hex: &str, label: String) -> Result<(), String> {
    let h = sha256_hex.trim().to_ascii_lowercase();
    if h.len() != 64 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("token hash must be 64 hex characters (SHA-256)".into());
    }
    if h == self::sha256_hex(b"") {
        return Err("that is the hash of an empty token".into());
    }
    if label.len() > MAX_LABEL_BYTES {
        return Err(format!("labels are at most {MAX_LABEL_BYTES} bytes"));
    }
    TOKENS.with(|t| {
        let mut t = t.borrow_mut();
        if !t.contains_key(&h) && t.len() >= MAX_TOKENS {
            return Err(format!("at most {MAX_TOKENS} tokens can be registered"));
        }
        t.insert(h, label);
        Ok(())
    })
}

/// Revoke by the full hash, or by a prefix of at least 16 hex characters (the id that
/// `get_policy` and audit entries show) that matches exactly one token.
pub fn revoke_token_hash(sha256_hex: &str) -> bool {
    let h = sha256_hex.trim().to_ascii_lowercase();
    if h.len() < 16 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return false;
    }
    TOKENS.with(|t| {
        let mut t = t.borrow_mut();
        let found: Vec<String> = t.keys_range(h.clone()..).take_while(|k| k.starts_with(&h)).take(2).collect();
        match found.as_slice() {
            [k] => t.remove(k).is_some(),
            _ => false, // unknown or ambiguous
        }
    })
}

pub fn list_tokens() -> Vec<(String, String)> {
    TOKENS.with(|t| t.borrow().iter().map(|e| (e.key()[..16].to_string(), e.value())).collect())
}

/// Whether a token hash (from `verify_token`) is still registered: a run checks it again
/// before every message, so a revoked token's run sends nothing more.
pub fn token_registered(sha256_hex: &str) -> bool {
    TOKENS.with(|t| t.borrow().contains_key(&sha256_hex.to_string()))
}

/// Returns the token's hash if it is registered.
pub fn verify_token(token: &str) -> Option<String> {
    let h = sha256_hex(token.as_bytes());
    TOKENS.with(|t| t.borrow().contains_key(&h)).then_some(h)
}

// ---------- candid overrides ----------

pub fn set_candid_override(canister: Principal, did: Option<String>) -> Result<(), String> {
    CANDID_OVERRIDES.with(|m| {
        let mut m = m.borrow_mut();
        let key = canister.to_text();
        match did {
            Some(_) if !m.contains_key(&key) && m.len() >= MAX_CANDID_OVERRIDES => {
                return Err(format!("at most {MAX_CANDID_OVERRIDES} candid overrides can be registered"));
            }
            Some(d) => m.insert(key, d),
            None => m.remove(&key),
        };
        Ok(())
    })
}

/// (canister, interface size in bytes, SHA-256 of the interface text) for every override.
pub fn list_candid_overrides() -> Vec<(String, u64, String)> {
    CANDID_OVERRIDES.with(|m| m.borrow().iter().map(|e| (e.key().clone(), e.value().len() as u64, sha256_hex(e.value().as_bytes()))).collect())
}

/// The override for `canister`. One over `MAX_DID_BYTES` (stored before the endpoint
/// checked the size) is ignored; `get_policy` still lists it, with its size.
pub fn candid_override(canister: &str) -> Option<String> {
    CANDID_OVERRIDES.with(|m| m.borrow().get(&canister.to_string())).filter(|d| d.len() <= crate::bridge::MAX_DID_BYTES)
}

/// The override a script may read with `icp.candid`: only for an allowlisted canister,
/// like a fetched interface.
pub fn served_override(canister: &Principal) -> Result<Option<String>, String> {
    if !ALLOWLIST.with(|m| m.borrow().contains_key(canister)) {
        return Err(format!("policy: canister {canister} is not allowlisted"));
    }
    Ok(candid_override(&canister.to_text()))
}

// ---------- audit log ----------

thread_local! {
    // ponytail: heap counter, reset by an upgrade; a stable cell if it must survive one.
    static LOST_AUDIT_ENTRIES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

fn append_to<M: Memory>(l: &StableLog<AuditEntry, M, M>, mut entry: AuditEntry) -> Result<u64, String> {
    entry.id = l.len();
    entry.run_id.get_or_insert(entry.id);
    l.append(&entry).map_err(|e| format!("audit log out of stable memory: {e:?}"))
}

/// Appends an entry and returns its id. An entry without a `run_id` starts its own run.
///
/// Traps if stable memory cannot grow. That is deliberate for the entry written as a
/// message is handed over: the trap also discards the message, so nothing is sent
/// unaudited.
pub fn append_audit(entry: AuditEntry) -> u64 {
    AUDIT.with(|l| append_to(&l.borrow(), entry)).unwrap_or_else(|e| panic!("{e}"))
}

/// Like `append_audit`, but never traps: for trap cleanup, where a trap would also undo
/// the release of the run's slot and pending id. A failed append is counted in
/// `lost_audit_entries` (`get_policy`).
pub fn try_append_audit(entry: AuditEntry) -> Option<u64> {
    let r = AUDIT.with(|l| append_to(&l.borrow(), entry)).ok();
    if r.is_none() {
        LOST_AUDIT_ENTRIES.with(|c| c.set(c.get() + 1));
    }
    r
}

/// Audit entries that could not be written since the last upgrade.
pub fn lost_audit_entries() -> u64 {
    LOST_AUDIT_ENTRIES.with(|c| c.get())
}

/// `AuditEntry::args` text for one encoded argument: bounded, and complete for the usual
/// ledger call (100-200 bytes).
pub fn arg_digest(bytes: &[u8]) -> String {
    if bytes.len() <= 1024 {
        hex::encode(bytes)
    } else {
        format!("sha256:{} prefix:{}", sha256_hex(bytes), hex::encode(&bytes[..512]))
    }
}

pub fn audit_len() -> u64 {
    AUDIT.with(|l| l.borrow().len())
}

/// Bytes of entries one page may return; the IC refuses replies over 2 MiB.
const MAX_AUDIT_PAGE_BYTES: usize = 1_500_000;
/// Text kept of `error` when reading (entries written before fields were capped).
const MAX_READ_FIELD_BYTES: usize = 4096;
/// Items kept per list, and bytes per item, when reading. With these, one entry encodes to
/// well under 2 MiB whatever was stored, so every entry can be read.
const MAX_READ_ITEMS: usize = 128;
const MAX_READ_ITEM_BYTES: usize = 2048;

fn capped(mut e: AuditEntry) -> AuditEntry {
    use crate::bridge::truncate;
    if let Some(s) = e.error.as_mut() {
        truncate(s, MAX_READ_FIELD_BYTES);
    }
    let cut_list = |v: &mut Vec<String>| {
        if v.len() > MAX_READ_ITEMS {
            let more = v.len() - MAX_READ_ITEMS;
            v.truncate(MAX_READ_ITEMS);
            v.push(format!("…and {more} more"));
        }
        v.iter_mut().for_each(|s| truncate(s, MAX_READ_ITEM_BYTES));
    };
    cut_list(&mut e.violations);
    cut_list(&mut e.canisters);
    e.methods.as_mut().map(cut_list);
    e.args.as_mut().map(cut_list);
    e
}

/// Entries at `idxs`, cut to size, until the page's byte budget is spent. The first entry
/// is always included, so no entry is unreadable.
fn page_of(idxs: impl Iterator<Item = u64>) -> Vec<AuditEntry> {
    AUDIT.with(|l| {
        let l = l.borrow();
        let mut bytes = 0;
        let mut page = vec![];
        for e in idxs.filter_map(|i| l.get(i)).map(capped) {
            bytes += Encode!(&e).map_or(0, |b| b.len());
            if !page.is_empty() && bytes > MAX_AUDIT_PAGE_BYTES {
                break;
            }
            page.push(e);
        }
        page
    })
}

/// Newest first: `offset` 0 is the most recent entry.
pub fn audit_page(offset: u64, limit: u64) -> Vec<AuditEntry> {
    let len = audit_len();
    page_of((0..limit.min(MAX_AUDIT_PAGE)).map_while(|i| len.checked_sub(offset.checked_add(i)?.checked_add(1)?)))
}

/// Newest first, starting below id `before` (`None`: the newest entry). Ids do not move as
/// entries are appended, so passing the last id of one page gives the next one, with no
/// gaps or repeats.
pub fn audit_page_before(before: Option<u64>, limit: u64) -> Vec<AuditEntry> {
    let end = before.unwrap_or(u64::MAX).min(audit_len());
    page_of((0..end).rev().take(limit.min(MAX_AUDIT_PAGE) as usize))
}

/// Ids `audit_run` looks at per call, so one query stays well inside its instruction limit.
pub const MAX_RUN_SCAN: u64 = 5_000;

/// The entries of run `run_id`, oldest first, from id `from` (`None`: the run's start).
/// A run's entries all lie at or after its id, interleaved with other runs'; each call
/// looks at the next `MAX_RUN_SCAN` ids at most. To continue, pass the last id returned
/// plus one, or `from + MAX_RUN_SCAN` after an empty page. The run is complete once an
/// entry with a status other than `Started`/`Call` is among them.
pub fn audit_run(run_id: u64, from: Option<u64>) -> Vec<AuditEntry> {
    let start = from.unwrap_or(run_id).max(run_id);
    let end = start.saturating_add(MAX_RUN_SCAN).min(audit_len());
    // ponytail: decodes every scanned entry; index by run_id if scans get slow.
    let ids: Vec<u64> = AUDIT.with(|l| {
        let l = l.borrow();
        (start..end).filter(|&i| l.get(i).is_some_and(|e| e.run_id == Some(run_id))).take(MAX_AUDIT_PAGE as usize).collect()
    });
    page_of(ids.into_iter())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEDGER: &str = "ryjl3-tyaaa-aaaaa-aaaba-cai";

    #[test]
    fn allowlist_denies_unknown_canisters_and_methods() {
        let lim = Limits::default();
        let any = AllowedMethods(vec!["*".into()]);
        let some = AllowedMethods(vec!["icrc1_balance_of".into()]);
        assert!(authorize(None, LEDGER, "icrc1_balance_of", 0, 0, &lim).unwrap_err().contains("not allowlisted"));
        assert!(authorize(Some(&some), LEDGER, "icrc1_transfer", 0, 0, &lim).unwrap_err().contains("method icrc1_transfer on"));
        assert!(authorize(Some(&some), LEDGER, "icrc1_balance_of", 0, 0, &lim).is_ok());
        assert!(authorize(Some(&any), LEDGER, "anything", 0, 0, &lim).is_ok());
    }

    #[test]
    fn execution_caps() {
        let lim = Limits { max_calls_per_script: 2, max_instructions: 100, ..Limits::default() };
        let any = AllowedMethods(vec!["*".into()]);
        assert!(authorize(Some(&any), LEDGER, "m", 1, 0, &lim).is_ok());
        assert!(authorize(Some(&any), LEDGER, "m", 2, 0, &lim).unwrap_err().contains("call limit"));
        assert!(authorize(Some(&any), LEDGER, "m", 0, 101, &lim).unwrap_err().contains("instruction budget"));
    }

    #[test]
    fn stable_state_check_call() {
        let ledger = Principal::from_text(LEDGER).unwrap();
        assert!(check_call(&ledger, "icrc1_balance_of", 0, 0, &limits()).is_err());
        set_allowed(ledger, vec!["icrc1_balance_of".into()]).unwrap();
        assert!(check_call(&ledger, "icrc1_balance_of", 0, 0, &limits()).is_ok());
        assert!(check_call(&ledger, "icrc1_transfer", 0, 0, &limits()).is_err());
        assert!(check_call(&Principal::management_canister(), "raw_rand", 0, 0, &limits()).is_err());
        set_limits(Limits { max_calls_per_script: 0, max_instructions: 1, ..Limits::default() });
        assert!(check_call(&ledger, "icrc1_balance_of", 0, 0, &limits()).unwrap_err().contains("call limit"));
        assert!(remove_allowed(ledger));
        assert_eq!(list_allowed(), vec![]);
    }

    #[test]
    fn tokens_are_hashed() {
        let h = sha256_hex(b"secret-token");
        assert!(verify_token("secret-token").is_none());
        assert!(add_token_hash("nothex", "x".into()).is_err());
        add_token_hash(&h.to_uppercase(), "ci".into()).unwrap();
        assert_eq!(verify_token("secret-token"), Some(h.clone()));
        assert!(verify_token("wrong").is_none());
        assert_eq!(list_tokens(), vec![(h[..16].to_string(), "ci".to_string())]);
        assert!(revoke_token_hash(&h));
        assert!(verify_token("secret-token").is_none());
    }

    #[test]
    fn audit_log_pages_newest_first() {
        let e = |lang: &str| AuditEntry {
            id: 0, timestamp_ns: 1, caller: "c".into(), language: lang.into(), script_sha256: "s".into(),
            canisters: vec![], calls: 0, instructions: 0, status: AuditStatus::Ok, violations: vec![], error: None,
            run_id: None, methods: None, args: None,
        };
        for l in ["a", "b", "c"] {
            append_audit(e(l));
        }
        assert_eq!(audit_len(), 3);
        let page: Vec<_> = audit_page(0, 2).into_iter().map(|e| (e.id, e.language)).collect();
        assert_eq!(page, vec![(2, "c".into()), (1, "b".into())]);
        assert_eq!(audit_page(2, 10).len(), 1);
        assert!(audit_page(3, 10).is_empty());
        assert!(audit_page(u64::MAX, u64::MAX).is_empty());
    }

    fn entry(sha: &str, status: AuditStatus, run_id: Option<u64>) -> AuditEntry {
        AuditEntry {
            id: 0, timestamp_ns: 1, caller: "c".into(), language: "javascript".into(), script_sha256: sha.into(),
            canisters: vec![], calls: 0, instructions: 0, status, violations: vec![], error: None, run_id, methods: None, args: None,
        }
    }

    /// The case from review: the same caller runs identical code twice; the first run traps
    /// after an outbound call (its final entry is rolled back, its Started entry stays), the
    /// second succeeds. The run id says which attempt finished; the script hash can't.
    #[test]
    fn run_id_links_started_and_final_entries() {
        let first = append_audit(entry("same", AuditStatus::Started, None)); // never finishes
        let second = append_audit(entry("same", AuditStatus::Started, None));
        let done = append_audit(entry("same", AuditStatus::Ok, Some(second)));
        let page = audit_page(0, 3);
        assert_eq!(page.iter().map(|e| (e.id, e.run_id)).collect::<Vec<_>>(),
            vec![(done, Some(second)), (second, Some(second)), (first, Some(first))]);
        // The unfinished run (pending, or ended by a trap) is the one with no final entry.
        let finished: Vec<_> = page.iter().filter(|e| e.status != AuditStatus::Started).map(|e| e.run_id).collect();
        assert!(!finished.contains(&Some(first)) && finished.contains(&Some(second)));
        // A run that never reached the network is its own run.
        let solo = append_audit(entry("other", AuditStatus::Error, None));
        assert_eq!(audit_page(0, 1)[0].run_id, Some(solo));
    }

    /// Entries stored before `run_id` existed still decode (the opt field reads as null).
    #[test]
    fn old_audit_entries_still_decode() {
        #[derive(CandidType)]
        struct OldEntry {
            id: u64, timestamp_ns: u64, caller: String, language: String, script_sha256: String, canisters: Vec<String>,
            calls: u32, instructions: u64, status: AuditStatus, violations: Vec<String>, error: Option<String>,
        }
        let old = OldEntry {
            id: 7, timestamp_ns: 1, caller: "c".into(), language: "rhai".into(), script_sha256: "s".into(), canisters: vec![],
            calls: 1, instructions: 2, status: AuditStatus::Ok, violations: vec![], error: None,
        };
        let decoded = <AuditEntry as Storable>::from_bytes(Cow::Owned(Encode!(&old).unwrap()));
        assert_eq!((decoded.id, decoded.run_id, decoded.language.as_str()), (7, None, "rhai"));
        assert_eq!(decoded.methods, None);

        // Same for the stored limits record, which gained optional settings.
        #[derive(CandidType)]
        struct OldLimits {
            max_calls_per_script: u32,
            max_instructions: u64,
        }
        let l = <Limits as Storable>::from_bytes(Cow::Owned(Encode!(&OldLimits { max_calls_per_script: 7, max_instructions: 9 }).unwrap()));
        assert_eq!(l, Limits { max_calls_per_script: 7, max_instructions: 9, ..Limits::default() });
    }

    #[test]
    fn interface_fetch_and_overrides_and_token_edge_cases() {
        let p = Principal::from_text(LEDGER).unwrap();
        set_limits(Limits::default());
        assert!(check_interface_fetch(&p, 0, 0, &limits()).is_err());
        set_allowed(p, vec![]).unwrap();
        assert!(check_interface_fetch(&p, 0, 0, &limits()).is_ok());

        set_candid_override(p, Some("service : { ping : () -> () }".into())).unwrap();
        assert_eq!(candid_override(&p.to_text()).as_deref(), Some("service : { ping : () -> () }"));
        set_candid_override(p, None).unwrap();
        assert!(candid_override(&p.to_text()).is_none());

        assert!(add_token_hash(&"0".repeat(63), "short".into()).is_err());
        assert!(add_token_hash(&"g".repeat(64), "badhex".into()).is_err());
        assert!(!revoke_token_hash(&"f".repeat(64)));
        remove_allowed(p);
    }

    /// #106: a record stored before set_limits clamped (or by an older version) is
    /// clamped when read.
    #[test]
    fn stored_unsafe_limits_are_clamped_on_read() {
        let raw = Limits { max_instructions: 60_000_000_000, call_timeout_seconds: Some(0), ..Limits::default() };
        LIMITS.with(|c| c.borrow_mut().set(raw));
        let l = limits();
        assert_eq!((l.max_instructions, l.call_timeout_seconds), (MAX_INSTRUCTIONS_CAP, Some(1)));
    }

    /// #80: an override is served to scripts only for an allowlisted canister, and one
    /// over the size cap (stored before the endpoint checked it) is ignored.
    #[test]
    fn overrides_need_the_allowlist_and_the_size_cap() {
        let p = Principal::from_text(LEDGER).unwrap();
        set_candid_override(p, Some("service : {}".into())).unwrap();
        assert!(served_override(&p).unwrap_err().contains("not allowlisted"));
        set_allowed(p, vec![]).unwrap();
        assert_eq!(served_override(&p).unwrap().as_deref(), Some("service : {}"));
        set_candid_override(p, Some("a".repeat(crate::bridge::MAX_DID_BYTES + 1))).unwrap();
        assert!(candid_override(&p.to_text()).is_none());
        assert_eq!(served_override(&p), Ok(None));
        assert_eq!(list_candid_overrides()[0].1, crate::bridge::MAX_DID_BYTES as u64 + 1, "still listed for the admin");
    }

    /// #82, #147: the management canister and this canister are refused, when allowlisted
    /// and for entries stored before that check.
    #[test]
    fn privileged_targets_are_refused() {
        let mgmt = Principal::management_canister();
        let me = Principal::from_text("rrkah-fqaaa-aaaaa-aaaaq-cai").unwrap();
        assert!(set_allowed(mgmt, vec!["*".into()]).unwrap_err().contains("management canister"));
        assert!(set_allowed(mgmt, vec!["canister_status".into()]).is_err());
        assert!(allowed_entry_ok(&me, &["ping".into()], Some(me)).unwrap_err().contains("itself"));
        assert!(allowed_entry_ok(&Principal::from_text(LEDGER).unwrap(), &["*".into()], Some(me)).is_ok());
        // An entry stored before the check: refused at call time.
        ALLOWLIST.with(|m| m.borrow_mut().insert(mgmt, AllowedMethods(vec!["*".into()])));
        assert!(check_call(&mgmt, "stop_canister", 0, 0, &limits()).unwrap_err().contains("management canister"));
        remove_allowed(mgmt);
    }

    /// #170: admin input is capped.
    #[test]
    fn admin_input_is_capped() {
        let p = Principal::from_text(LEDGER).unwrap();
        let h = sha256_hex(b"t");
        assert!(add_token_hash(&h, "x".repeat(MAX_LABEL_BYTES + 1)).unwrap_err().contains("labels"));
        assert!(add_token_hash(&h, "x".repeat(MAX_LABEL_BYTES)).is_ok());
        assert!(set_allowed(p, vec!["m".into(); MAX_METHODS_PER_CANISTER + 1]).is_err());
        assert!(set_allowed(p, vec!["m".repeat(crate::bridge::MAX_METHOD_LEN + 1)]).is_err());
        assert!(set_allowed(p, vec!["m".into(); MAX_METHODS_PER_CANISTER]).is_ok());
        // Replacing an entry does not count its old methods against the total.
        assert!(set_allowed(p, vec!["m".into(); MAX_METHODS_PER_CANISTER]).is_ok());
        remove_allowed(p);
        revoke_token_hash(&h);
    }

    /// #129: script-authored method names are quoted and escaped, so they cannot forge a
    /// sentence or a label.
    #[test]
    fn script_text_is_escaped_in_refusals_and_labels() {
        let some = AllowedMethods(vec!["a".into()]);
        let e = authorize(Some(&some), LEDGER, "x` on y\n\u{202e} z", 0, 0, &Limits::default()).unwrap_err();
        assert!(!e.contains('\n') && !e.contains('\u{202e}') && !e.contains("method `x"), "{e}");
        assert!(e.contains("method \"x` on y\\n\\u{202e} z\" on"), "{e}");
        assert!(shown("x ryjl3-tyaaa-aaaaa-aaaba-cai.icrc1_transfer").starts_with('"'));
        assert_eq!(shown("icrc1_transfer"), "icrc1_transfer");
        assert_eq!(shown(""), "\"\"");
        // A script's own error, as run_script stores it in `error`.
        let e = escape_invisible("Error: policy: method `x` on y is not allowlisted\n\u{202e}Z\u{1b}[31m \"q\"");
        assert!(!e.contains('\n') && !e.contains('\u{202e}') && !e.contains('\u{1b}'), "{e}");
        assert!(e.ends_with("allowlisted\\n\\u{202e}Z\\u{1b}[31m \"q\""), "{e}");
    }

    /// #138: the 16-character id shown by get_policy revokes, unless ambiguous.
    #[test]
    fn revoke_by_shown_prefix() {
        let (a, b) = (format!("{}{}", "ab".repeat(8), "0".repeat(48)), format!("{}{}", "ab".repeat(8), "1".repeat(48)));
        add_token_hash(&a, "a".into()).unwrap();
        add_token_hash(&b, "b".into()).unwrap();
        assert!(!revoke_token_hash(&a[..16]), "ambiguous");
        assert!(!revoke_token_hash(&a[..15]), "too short");
        assert_eq!(list_tokens().len(), 2);
        assert!(revoke_token_hash(&a[..17]));
        let h = sha256_hex(b"secret");
        add_token_hash(&h, "c".into()).unwrap();
        let id = list_tokens().into_iter().find(|(_, l)| l == "c").unwrap().0;
        assert!(revoke_token_hash(&id.to_uppercase()));
        assert!(verify_token("secret").is_none());
        assert!(revoke_token_hash(&b));
    }

    /// #193: `*` does not admit delegating methods unless they are listed by name.
    #[test]
    fn wildcard_excludes_delegating_methods() {
        let lim = Limits::default();
        let e = authorize(Some(&AllowedMethods(vec!["*".into()])), LEDGER, "icrc2_approve", 0, 0, &lim).unwrap_err();
        assert!(e.contains("list it by name"), "{e}");
        assert!(authorize(Some(&AllowedMethods(vec!["*".into(), "icrc2_approve".into()])), LEDGER, "icrc2_approve", 0, 0, &lim).is_ok());
        assert!(authorize(Some(&AllowedMethods(vec!["*".into()])), LEDGER, "icrc1_transfer", 0, 0, &lim).is_ok());
    }

    /// #96: a huge legacy entry is returned (cut) on its own page under 2 MiB, and pages
    /// stop by bytes.
    #[test]
    fn oversized_entries_are_readable_and_pages_bounded() {
        let mut big = entry("big", AuditStatus::Ok, None);
        big.error = Some("e".repeat(8 << 20));
        big.violations = vec!["v".repeat(100); 20_000];
        big.methods = Some(vec!["m".repeat(1500); 5_000]);
        big.canisters = vec!["c".repeat(3000); 2_000];
        let id = append_audit(big);
        let page = audit_page(0, 100);
        assert_eq!(page.iter().map(|e| e.id).collect::<Vec<_>>(), vec![id]);
        assert!(Encode!(&page).unwrap().len() < 2 << 20);
        assert!(page[0].error.as_ref().unwrap().len() <= MAX_READ_FIELD_BYTES + 16);
        assert_eq!(page[0].violations.len(), MAX_READ_ITEMS + 1);
        assert!(page[0].violations.last().unwrap().contains("more"));
        for _ in 0..100 {
            let mut e = entry("mid", AuditStatus::Ok, None);
            e.methods = Some(vec!["m".repeat(1000); 30]);
            append_audit(e);
        }
        let page = audit_page(0, 100);
        assert!(page.len() < 100 && Encode!(&page).unwrap().len() <= MAX_AUDIT_PAGE_BYTES + 100);
    }

    /// #182: id-anchored pages have no repeats or gaps while entries are appended.
    #[test]
    fn pages_before_an_id_do_not_shift() {
        for _ in 0..300 {
            append_audit(entry("s", AuditStatus::Ok, None));
        }
        let first = audit_page_before(None, 100);
        for _ in 0..10 {
            append_audit(entry("s", AuditStatus::Ok, None));
        }
        let second = audit_page_before(Some(first.last().unwrap().id), 100);
        let ids: Vec<u64> = first.iter().chain(&second).map(|e| e.id).collect();
        assert_eq!(ids, (100..300).rev().collect::<Vec<_>>());
        assert_eq!(audit_page_before(Some(0), 10), vec![]);
        assert_eq!(audit_page_before(Some(u64::MAX), 1)[0].id, 309);
    }

    /// #182: one run's entries, without paging through the runs interleaved with it.
    #[test]
    fn a_run_is_read_by_its_id() {
        let a = append_audit(entry("a", AuditStatus::Started, None));
        let b = append_audit(entry("b", AuditStatus::Started, None));
        append_audit(entry("a", AuditStatus::Call, Some(a)));
        append_audit(entry("b", AuditStatus::Ok, Some(b)));
        let done = append_audit(entry("a", AuditStatus::Ok, Some(a)));
        let ids = |v: Vec<AuditEntry>| v.iter().map(|e| e.id).collect::<Vec<_>>();
        assert_eq!(ids(audit_run(a, None)), vec![a, a + 2, done]);
        assert_eq!(ids(audit_run(a, Some(a + 3))), vec![done]);
        assert_eq!(ids(audit_run(b, Some(0))), vec![b, b + 2], "never before the run's id");
        assert_eq!(ids(audit_run(done + 1, None)), Vec::<u64>::new());
        // The scan is bounded: matches past MAX_RUN_SCAN ids need another call.
        for _ in 0..MAX_RUN_SCAN {
            append_audit(entry("x", AuditStatus::Ok, None));
        }
        let late = append_audit(entry("a", AuditStatus::Ok, Some(a)));
        assert!(!ids(audit_run(a, Some(done + 1))).contains(&late));
        assert_eq!(ids(audit_run(a, Some(done + 1 + MAX_RUN_SCAN))), vec![late]);
    }

    /// #170: the override count is capped like tokens, so get_policy stays under the
    /// reply limit. Replacing or clearing an existing one is still allowed at the cap.
    #[test]
    fn candid_overrides_are_capped() {
        let p = |i: u64| Principal::from_slice(&i.to_be_bytes());
        for i in 0..MAX_CANDID_OVERRIDES {
            set_candid_override(p(i), Some("service : {}".into())).unwrap();
        }
        assert!(set_candid_override(p(MAX_CANDID_OVERRIDES), Some("service : {}".into())).unwrap_err().contains("at most"));
        set_candid_override(p(0), Some("service : { a : () -> () }".into())).unwrap();
        set_candid_override(p(1), None).unwrap();
        set_candid_override(p(MAX_CANDID_OVERRIDES), Some("service : {}".into())).unwrap();
        assert_eq!(list_candid_overrides().len() as u64, MAX_CANDID_OVERRIDES);
    }

    /// #150, #180: a full stable memory makes the append fail without trapping.
    #[test]
    fn append_over_full_memory_is_an_error() {
        use ic_stable_structures::{RestrictedMemory, VectorMemory};
        let m = || RestrictedMemory::new(VectorMemory::default(), 0..1);
        let log = StableLog::init(m(), m());
        let mut e = entry("s", AuditStatus::Ok, None);
        e.error = Some("x".repeat(16 * 1024));
        let results: Vec<_> = (0..8).map(|_| append_to(&log, e.clone())).collect();
        assert!(results[0].is_ok());
        assert!(results.last().unwrap().as_ref().unwrap_err().contains("out of stable memory"));
    }

    /// #194: arguments are recorded, bounded.
    #[test]
    fn arg_digest_is_bounded() {
        assert_eq!(arg_digest(&[1u8; 100]), "01".repeat(100));
        let d = arg_digest(&[7u8; 64 * 1024]);
        assert!(d.starts_with("sha256:") && d.len() <= 1200, "{}", d.len());
    }
}
