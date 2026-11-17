//! On-chain guardrails: canister/method allowlist, per-script execution caps,
//! SHA-256 hashed bearer tokens, and an append-only audit log.
//!
//! Everything lives in stable memory via `ic-stable-structures`, so upgrades
//! keep it without any pre/post-upgrade serialization.

use candid::{CandidType, Decode, Deserialize, Encode, Principal};
use ic_stable_structures::memory_manager::{MemoryId, MemoryManager, VirtualMemory};
use ic_stable_structures::storable::Bound;
use ic_stable_structures::{DefaultMemoryImpl, StableBTreeMap, StableCell, StableLog, Storable};
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

/// Methods a script may call on one canister. `["*"]` allows every method.
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
    /// during cleanup, in place of the final entry that the trap rolled back.
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
    pub instructions: u64,
    pub status: AuditStatus,
    pub violations: Vec<String>,
    pub error: Option<String>,
    /// Links all entries of one run: the id of the run's `Started` entry, or the entry's
    /// own id if the run never sent a message. Script hashes alone can't tell apart two
    /// runs of the same script (one finished, one not), and runs interleave across awaits.
    /// `None` only on entries written before this field existed.
    pub run_id: Option<u64>,
    /// Outbound messages as `canister.method`, in order. Interface fetches appear as
    /// `aaaaa-aa.canister_metadata(<target>)` and `<target>.__get_candid_interface_tmp_hack`. On a `Started`/`Call` entry: the one message about to
    /// be sent. On a final entry: all of them. `None` on entries from before this field.
    pub methods: Option<Vec<String>>,
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

pub fn limits() -> Limits {
    LIMITS.with(|c| c.borrow().get().clone())
}

/// Highest `max_instructions` accepted. Budgets are checked between steps, not on every
/// instruction, and one step can be large; this leaves 10B of headroom under the IC's 40B
/// per-message limit, so the budget error fires before the IC traps. It is also the
/// Python VM's per-turn limit, so no single turn can run past it.
pub const MAX_INSTRUCTIONS_CAP: u64 = 30_000_000_000;

/// Stores `l` with out-of-range values clamped: `max_instructions` to
/// `MAX_INSTRUCTIONS_CAP`, `call_timeout_seconds` to 1..=300 (the IC's range).
/// `get_policy` shows the values in effect.
pub fn set_limits(mut l: Limits) {
    l.max_instructions = l.max_instructions.min(MAX_INSTRUCTIONS_CAP);
    l.call_timeout_seconds = l.call_timeout_seconds.map(|s| s.clamp(1, 300));
    LIMITS.with(|c| c.borrow_mut().set(l));
}

// ---------- allowlist ----------

pub fn set_allowed(canister: Principal, methods: Vec<String>) {
    ALLOWLIST.with(|m| m.borrow_mut().insert(canister, AllowedMethods(methods)));
}

pub fn remove_allowed(canister: Principal) -> bool {
    ALLOWLIST.with(|m| m.borrow_mut().remove(&canister).is_some())
}

pub fn list_allowed() -> Vec<(Principal, Vec<String>)> {
    ALLOWLIST.with(|m| m.borrow().iter().map(|e| (*e.key(), e.value().0)).collect())
}

/// Decide whether a script may issue one more call. Pure function of its inputs
/// so it is unit-testable; `check_call` feeds it live state.
pub fn authorize(
    allowed: Option<&AllowedMethods>,
    canister: &str,
    method: &str,
    calls_made: u32,
    instructions_used: u64,
    limits: &Limits,
) -> Result<(), String> {
    let methods = allowed.ok_or_else(|| format!("policy: canister {canister} is not allowlisted"))?;
    // Method names come from scripts: escaped, so one cannot read as another sentence.
    let shown = method.escape_debug();
    if !methods.0.iter().any(|m| m == "*" || m == method) {
        return Err(format!("policy: method `{shown}` on {canister} is not allowlisted"));
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

pub fn check_call(canister: &Principal, method: &str, calls_made: u32, instructions_used: u64) -> Result<(), String> {
    let allowed = ALLOWLIST.with(|m| m.borrow().get(canister));
    authorize(allowed.as_ref(), &canister.to_text(), method, calls_made, instructions_used, &limits())
}

/// Interface fetches (`icp.candid`) may target any allowlisted canister and count
/// against the same per-script cap and budget as calls.
pub fn check_interface_fetch(canister: &Principal, calls_made: u32, instructions_used: u64) -> Result<(), String> {
    let any = ALLOWLIST.with(|m| m.borrow().contains_key(canister)).then(|| AllowedMethods(vec!["*".into()]));
    authorize(any.as_ref(), &canister.to_text(), "<candid interface>", calls_made, instructions_used, &limits())
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
    TOKENS.with(|t| t.borrow_mut().insert(h, label));
    Ok(())
}

pub fn revoke_token_hash(sha256_hex: &str) -> bool {
    TOKENS.with(|t| t.borrow_mut().remove(&sha256_hex.trim().to_ascii_lowercase()).is_some())
}

pub fn list_tokens() -> Vec<(String, String)> {
    TOKENS.with(|t| t.borrow().iter().map(|e| (e.key()[..16].to_string(), e.value())).collect())
}

/// Returns the token's hash if it is registered.
pub fn verify_token(token: &str) -> Option<String> {
    let h = sha256_hex(token.as_bytes());
    TOKENS.with(|t| t.borrow().contains_key(&h)).then_some(h)
}

// ---------- candid overrides ----------

pub fn set_candid_override(canister: Principal, did: Option<String>) {
    CANDID_OVERRIDES.with(|m| match did {
        Some(d) => m.borrow_mut().insert(canister.to_text(), d),
        None => m.borrow_mut().remove(&canister.to_text()),
    });
}

/// (canister, interface size in bytes, SHA-256 of the interface text) for every override.
pub fn list_candid_overrides() -> Vec<(String, u64, String)> {
    CANDID_OVERRIDES.with(|m| m.borrow().iter().map(|e| (e.key().clone(), e.value().len() as u64, sha256_hex(e.value().as_bytes()))).collect())
}

pub fn candid_override(canister: &str) -> Option<String> {
    CANDID_OVERRIDES.with(|m| m.borrow().get(&canister.to_string()))
}

// ---------- audit log ----------

/// Appends an entry and returns its id. An entry without a `run_id` starts its own run.
pub fn append_audit(mut entry: AuditEntry) -> u64 {
    AUDIT.with(|l| {
        let l = l.borrow();
        entry.id = l.len();
        entry.run_id.get_or_insert(entry.id);
        l.append(&entry).expect("audit log out of stable memory")
    })
}

pub fn audit_len() -> u64 {
    AUDIT.with(|l| l.borrow().len())
}

/// Newest first: `offset` 0 is the most recent entry.
/// Bytes of entries one page may return; the IC refuses replies over 2 MiB.
const MAX_AUDIT_PAGE_BYTES: usize = 1_500_000;
/// Text kept per field when reading an oversized entry (written before fields were capped).
const MAX_READ_FIELD_BYTES: usize = 4096;

pub fn audit_page(offset: u64, limit: u64) -> Vec<AuditEntry> {
    AUDIT.with(|l| {
        let l = l.borrow();
        let len = l.len();
        let mut bytes = 0;
        (0..limit.min(MAX_AUDIT_PAGE))
            .map_while(|i| len.checked_sub(offset.checked_add(i)?.checked_add(1)?))
            .filter_map(|idx| l.get(idx))
            .map(|mut e| {
                let cut = |s: &mut String| crate::bridge::truncate(s, MAX_READ_FIELD_BYTES);
                e.error.as_mut().map(cut);
                e.violations.iter_mut().for_each(cut);
                e
            })
            .take_while(|e| {
                bytes += Encode!(e).map_or(0, |b| b.len());
                bytes <= MAX_AUDIT_PAGE_BYTES
            })
            .collect()
    })
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
        assert!(authorize(Some(&some), LEDGER, "icrc1_transfer", 0, 0, &lim).unwrap_err().contains("method `icrc1_transfer`"));
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
        assert!(check_call(&ledger, "icrc1_balance_of", 0, 0).is_err());
        set_allowed(ledger, vec!["icrc1_balance_of".into()]);
        assert!(check_call(&ledger, "icrc1_balance_of", 0, 0).is_ok());
        assert!(check_call(&ledger, "icrc1_transfer", 0, 0).is_err());
        assert!(check_call(&Principal::management_canister(), "raw_rand", 0, 0).is_err());
        set_limits(Limits { max_calls_per_script: 0, max_instructions: 1, ..Limits::default() });
        assert!(check_call(&ledger, "icrc1_balance_of", 0, 0).unwrap_err().contains("call limit"));
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
            run_id: None, methods: None,
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
            canisters: vec![], calls: 0, instructions: 0, status, violations: vec![], error: None, run_id, methods: None,
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
        assert!(check_interface_fetch(&p, 0, 0).is_err());
        set_allowed(p, vec![]);
        assert!(check_interface_fetch(&p, 0, 0).is_ok());

        set_candid_override(p, Some("service : { ping : () -> () }".into()));
        assert_eq!(candid_override(&p.to_text()).as_deref(), Some("service : { ping : () -> () }"));
        set_candid_override(p, None);
        assert!(candid_override(&p.to_text()).is_none());

        assert!(add_token_hash(&"0".repeat(63), "short".into()).is_err());
        assert!(add_token_hash(&"g".repeat(64), "badhex".into()).is_err());
        assert!(!revoke_token_hash(&"f".repeat(64)));
        remove_allowed(p);
    }
}
