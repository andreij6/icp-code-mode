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
    wide_keys: usize,
}

/// Objects with more keys than this count toward `MAX_WIDE_KEYS`.
pub const WIDE_OBJECT: usize = 64;
/// Keys summed over wide objects. Engines build objects in time quadratic in their key count
/// when the keys collide (hash seeds are public here): 194 objects of 1,024 colliding keys took
/// 1 s natively in Boa, and this allows 16 (about 80 ms).
pub const MAX_WIDE_KEYS: usize = 16 * 1024;

impl ValueBudget {
    /// Count an object's keys toward the wide-object bound.
    pub fn object(&mut self, keys: usize) -> Result<(), String> {
        if keys > WIDE_OBJECT {
            self.wide_keys += keys;
            if self.wide_keys > MAX_WIDE_KEYS {
                return Err(format!("value has more than {MAX_WIDE_KEYS} keys in objects of over {WIDE_OBJECT} keys"));
            }
        }
        Ok(())
    }
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

/// The depth rule every runtime and the bridge share: at most `MAX_DEPTH` levels of arrays
/// and objects (bytes and blobs count as arrays); scalars add no level. Call it for a
/// container at `depth` (the root is at 0).
pub fn container_depth(depth: usize) -> Result<(), String> {
    const MAX: usize = crate::candid_bridge::MAX_DEPTH;
    match depth {
        d if d >= MAX => Err(format!("value nested deeper than {MAX} levels")),
        _ => Ok(()),
    }
}

/// A float as it leaves a script or a reply: non-finite values as the strings "NaN",
/// "inf" and "-inf" (which the Candid bridge parses back), in every runtime.
pub fn float_json(f: f64) -> Value {
    match serde_json::Number::from_f64(f) {
        Some(n) => Value::Number(n),
        None if f.is_nan() => Value::String("NaN".into()),
        None if f > 0.0 => Value::String("inf".into()),
        None => Value::String("-inf".into()),
    }
}

/// A script's result with integers beyond 2^53 as exact decimal strings, as Candid replies
/// give them, whatever the language (Python and Rhai integers are exact up to 2^63).
pub fn big_ints_to_strings(v: Value) -> Value {
    const SAFE: u64 = crate::candid_bridge::MAX_SAFE_INT;
    match v {
        Value::Number(n) if n.as_i64().map_or(n.is_u64(), |i| i.unsigned_abs() > SAFE) => Value::String(n.to_string()),
        Value::Array(a) => Value::Array(a.into_iter().map(big_ints_to_strings).collect()),
        Value::Object(o) => Value::Object(o.into_iter().map(|(k, v)| (k, big_ints_to_strings(v))).collect()),
        v => v,
    }
}

/// Hold a host reply to the same limits as values leaving a script, so every runtime
/// accepts the same replies (Rhai's engine limits equal these and never count more).
pub fn check_value(v: &Value) -> Result<(), String> {
    fn walk(v: &Value, depth: usize, b: &mut ValueBudget) -> Result<(), String> {
        b.node()?;
        match v {
            Value::String(s) => b.bytes(s.len()),
            Value::Array(a) => container_depth(depth).and_then(|_| a.iter().try_for_each(|x| walk(x, depth + 1, b))),
            Value::Object(o) => container_depth(depth)
                .and_then(|_| o.iter().try_for_each(|(k, x)| b.bytes(k.len()).and_then(|_| walk(x, depth + 1, b)))),
            _ => Ok(()),
        }
    }
    walk(v, 0, &mut ValueBudget::default())
}

/// Decode a call's reply and hold it to the value limits. `Err((message, stale))`: `stale`
/// when the reply did not decode, most likely because the target was upgraded and our copy
/// of its interface is out of date.
pub fn decode_checked(svc: &Service, func: &candid::types::Function, bytes: &[u8], label: &str) -> Result<Value, (String, bool)> {
    let done = format!("call {label} executed (the target replied), but its reply could not be decoded");
    let v = svc.decode_reply(func, bytes).map_err(|e| {
        if e.stale {
            // Most likely the target was upgraded and our copy of its interface is stale.
            (format!("{done}: {}; the cached interface was dropped, so the next call fetches it again", e.msg), true)
        } else {
            (format!("{done}: {}", e.msg), false)
        }
    })?;
    check_value(&v).map_err(|e| (format!("reply from {label} is over the value limits ({e}); the call was executed"), false))?;
    Ok(v)
}

/// Deterministic "now" for script clocks: block time on-chain, the settable test clock
/// off-chain (0 unless a test moves it).
pub fn now_ms() -> u64 {
    ic::now_ns() / 1_000_000
}

/// JS engines hand back every number as f64; turn integral values in the
/// JS-safe range back into JSON integers so `3` doesn't become `3.0`.
pub fn normalize_numbers(v: Value) -> Value {
    match v {
        Value::Number(n) if !n.is_i64() && !n.is_u64() => match n.as_f64() {
            // -0 stays a float, as it does in Python and Rhai.
            Some(f) if f.fract() == 0.0 && f.abs() <= 9_007_199_254_740_991.0 && !(f == 0.0 && f.is_sign_negative()) => {
                Value::from(f as i64)
            }
            _ => Value::Number(n),
        },
        Value::Array(a) => Value::Array(a.into_iter().map(normalize_numbers).collect()),
        Value::Object(o) => Value::Object(o.into_iter().map(|(k, v)| (k, normalize_numbers(v))).collect()),
        v => v,
    }
}

/// Append-only, size-capped log buffer shared by bridge implementations, with the
/// runtimes' measure (`log_cost`), so it keeps whatever a runtime captured.
#[derive(Default)]
pub struct LogBuf {
    lines: std::cell::RefCell<Vec<String>>,
    bytes: std::cell::Cell<usize>,
}

/// What one captured output line costs against `MAX_LOG_BYTES`: its JSON-escaped size (the
/// size it is sent at) plus its escaped newline, so empty lines are not free.
pub fn log_cost(line: &str) -> usize {
    escaped_len(line) + 2
}

/// Room a log keeps for the line that ends it with the marker (`log_line`).
pub const LOG_RESERVE: usize = CUT_MARK.len() + 2;

/// Charge `line` to a log that has `used` of `MAX_LOG_BYTES`: the line to keep (cut with
/// the marker if it does not fit) and the new total, or `None` once the log is full. Whole
/// lines fill the log up to `LOG_RESERVE` short of the cap, so whatever is dropped is
/// always marked. `bounded`: the line is the runtime's own output, already held to this
/// rule (the line it cut may use the reserve), so only the hard cap applies; never set it
/// for script text, which could end with the marker itself.
pub fn log_line(used: usize, mut line: String, bounded: bool) -> Option<(String, usize)> {
    if used >= MAX_LOG_BYTES {
        return None;
    }
    let room = if bounded { MAX_LOG_BYTES } else { MAX_LOG_BYTES - LOG_RESERVE };
    match used + log_cost(&line) {
        n if n <= room => Some((line, n)),
        _ => {
            cut_escaped(&mut line, (MAX_LOG_BYTES - used).saturating_sub(2));
            Some((line, MAX_LOG_BYTES))
        }
    }
}

impl LogBuf {
    pub fn push(&self, line: String) {
        // Lines arrive from a runtime that already capped them (`log_line`).
        if let Some((line, used)) = log_line(self.bytes.get(), line, true) {
            self.bytes.set(used);
            self.lines.borrow_mut().push(line);
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

/// Whether a refusal from an interface fetch would refuse every further fetch of the run too
/// (call cap, budget, stopping, revoked token, time limit, in-flight or cycles limit), as
/// opposed to one about a single target (not allowlisted). A search stops fetching on the
/// former only.
pub fn refuses_whole_run(e: &str) -> bool {
    e.starts_with("policy:") && !e.contains("is not allowlisted")
}

/// Wall-clock limit for one run (script or search). Each call's timeout is cut to the time
/// left, so no run holds a slot, or this canister's stop, for longer.
pub const MAX_RUN_SECONDS: u64 = 600;
/// How long a run's bookkeeping (slot, pending id) is kept after the run trapped: its other
/// calls may still be outstanding, and every call is bounded-wait (at most 300 s, plus margin).
pub const TRAP_LINGER_SECONDS: u64 = 360;
/// Past this age a run's bookkeeping is stale (left by a snapshot load, for instance): no run
/// lasts longer than `MAX_RUN_SECONDS`, then lingers at most `TRAP_LINGER_SECONDS`.
pub const STALE_RUN_MS: u64 = (MAX_RUN_SECONDS + TRAP_LINGER_SECONDS) * 1000;

/// Instructions executed so far in this call context. Off-chain, a stand-in that advances by
/// one at every read (each read is one budget check), so native tests can set a deadline
/// and see every runtime stop at it.
pub fn instruction_counter() -> u64 {
    #[cfg(target_arch = "wasm32")]
    return ic_cdk::api::call_context_instruction_counter();
    #[cfg(not(target_arch = "wasm32"))]
    TEST_INSTRUCTIONS.with(|c| {
        c.set(c.get() + 1);
        c.get()
    })
}

#[cfg(not(target_arch = "wasm32"))]
thread_local!(static TEST_INSTRUCTIONS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) });

/// The mark left where text was cut.
pub const CUT_MARK: &str = "…[truncated]";

/// Truncate to at most `max` bytes in all on a char boundary, marking the cut (the mark is
/// left out when `max` has no room for it).
pub fn truncate(s: &mut String, max: usize) {
    if s.len() > max {
        let keep = if max >= CUT_MARK.len() { max - CUT_MARK.len() } else { max };
        s.truncate(s.floor_char_boundary(keep));
        if max >= CUT_MARK.len() {
            s.push_str(CUT_MARK);
        }
    }
}

/// Bytes `c` takes once JSON-escaped.
fn escaped_char(c: char) -> usize {
    match c {
        '"' | '\\' | '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2,
        c if (c as u32) < 0x20 => 6,
        c => c.len_utf8(),
    }
}

/// Bytes `s` takes once JSON-escaped (control characters become `\u00XX`, six bytes each).
pub fn escaped_len(s: &str) -> usize {
    s.chars().map(escaped_char).sum()
}

/// Length in bytes of the longest prefix of `s` whose escaped form fits in `max`.
pub fn escaped_prefix(s: &str, max: usize) -> usize {
    let mut used = 0;
    for (i, c) in s.char_indices() {
        used += escaped_char(c);
        if used > max {
            return i;
        }
    }
    s.len()
}

/// Cut `s` so that, with the marker, its escaped form fits in `max`.
pub fn cut_escaped(s: &mut String, max: usize) {
    let fits = max >= CUT_MARK.len();
    s.truncate(escaped_prefix(s, if fits { max - CUT_MARK.len() } else { max }));
    if fits {
        s.push_str(CUT_MARK);
    }
}

/// Truncate so the escaped form of `s`, marker included, fits in `max`.
pub fn truncate_escaped(s: &mut String, max: usize) {
    if escaped_prefix(s, max) < s.len() {
        cut_escaped(s, max);
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
    /// One token's share of `MAX_IN_FLIGHT`, in the same proportion as its share of run
    /// slots, so one client's slow or silent targets cannot refuse every other client's calls.
    const MAX_IN_FLIGHT_PER_TOKEN: u32 = MAX_IN_FLIGHT * crate::MAX_RUNS_PER_TOKEN / crate::MAX_CONCURRENT_RUNS;
    /// Liquid cycles below which no new message is sent, so callers get a clear refusal
    /// instead of "call perform failed" while the canister still reports Running.
    // ponytail: fixed floor of about two calls' reply reservation; make it a Limits field
    // if operators need to tune it.
    const MIN_LIQUID_CYCLES: u128 = 100_000_000_000;

    /// (.did text, parsed service, fetch time in ns).
    type Fetched = (String, Rc<Service>, u64);

    /// One outbound message, resolving to its raw reply.
    type Msg<'a> = std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<u8>, CallFailed>> + 'a>>;

    thread_local! {
        // ponytail: heap cache keyed by canonical principal text, emptied on upgrade
        // (re-fetched on demand); bounded by the allowlist size.
        static DID_CACHE: RefCell<BTreeMap<String, Fetched>> = RefCell::default();
        // Runs that wrote a `Started` entry and have not ended, with the time (ms) after which
        // the entry is dropped as stale. Lets an auditor tell a run still waiting on a reply
        // from one that ended without a final entry.
        static PENDING_RUNS: RefCell<BTreeMap<u64, u64>> = RefCell::default();
        // Recent fetch failures: (error, time in ns).
        static DID_FAILED: RefCell<BTreeMap<String, (String, u64)>> = RefCell::default();
        // Parsed admin overrides: (.did text it was parsed from, service).
        static OVERRIDES: RefCell<BTreeMap<String, (String, Rc<Service>)>> = RefCell::default();
        // Messages sent and not completed, per token hash (see MAX_IN_FLIGHT).
        static IN_FLIGHT: RefCell<BTreeMap<String, u32>> = RefCell::default();
        #[cfg(not(target_arch = "wasm32"))]
        static TEST_NOW: Cell<u64> = const { Cell::new(0) };
    }

    /// Block time on-chain; a settable clock in native tests.
    pub(super) fn now_ns() -> u64 {
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

    fn canister_self() -> Principal {
        #[cfg(target_arch = "wasm32")]
        return ic_cdk::api::canister_self();
        #[cfg(not(target_arch = "wasm32"))]
        Principal::anonymous()
    }

    fn recovering_from_trap() -> bool {
        #[cfg(test)]
        if tests::RECOVERING.get() {
            return true;
        }
        ic_cdk::futures::is_recovering_from_trap()
    }

    /// Count a message of `token`'s as sent (`up`) or completed.
    fn in_flight(token: &str, up: bool) {
        IN_FLIGHT.with(|m| {
            let mut m = m.borrow_mut();
            let n = m.entry(token.to_string()).or_default();
            *n = if up { *n + 1 } else { n.saturating_sub(1) };
            if *n == 0 {
                m.remove(token);
            }
        })
    }

    /// A bounded-wait message (native tests stand in a fake network).
    fn message<'a>(to: Principal, method: &'a str, args: &'a [u8], secs: u32) -> Msg<'a> {
        #[cfg(test)]
        if let Some(net) = tests::NET.with(|n| n.borrow().clone()) {
            return net(to, method);
        }
        let call = Call::bounded_wait(to, method).change_timeout(secs).with_raw_args(args).into_future();
        Box::pin(async move { call.await.map(|r| r.into_bytes()) })
    }

    /// A message `sent` handed to the system and not yet seen complete.
    struct Held<'a> {
        msg: Msg<'a>,
        token: &'a str,
        /// The interface this message fetches, if it does.
        fetch_of: Option<&'a str>,
        sent: bool,
    }

    impl Drop for Held<'_> {
        fn drop(&mut self) {
            if !self.sent {
                return;
            }
            in_flight(self.token, false);
            // In trap cleanup ic-cdk (0.20) completes the message whose reply callback
            // trapped, then drops the run. So a fetch message that is ready now is the one
            // whose reply handling trapped (decoding or parsing ran out of instructions, say):
            // remember the failure, which the trap rolled back, so the next request does not
            // send the same messages and trap again. A trap in another reply's callback says
            // nothing about this target. (`sent` means the message is past `Prepared`, so this
            // poll never sends anything.)
            if let Some(key) = self.fetch_of
                && recovering_from_trap()
                && self.msg.as_mut().poll(&mut std::task::Context::from_waker(Waker::noop())).is_ready()
            {
                let msg = format!("could not fetch candid for {key} (handling its reply trapped); a controller can register it with set_candid_override");
                DID_FAILED.with(|c| c.borrow_mut().insert(key.to_string(), (msg, now_ns())));
            }
        }
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
    /// in memory: a run leaves the list when it finishes, `TRAP_LINGER_SECONDS` after its
    /// trap is cleaned up (its other calls may still be outstanding), or once stale; the
    /// list is empty after an upgrade.
    pub fn pending_run_ids() -> Vec<u64> {
        let now = now_ms();
        PENDING_RUNS.with(|r| {
            let mut r = r.borrow_mut();
            r.retain(|_, until| *until > now);
            r.keys().copied().collect()
        })
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
            let trapped = ic_cdk::futures::is_recovering_from_trap();
            PENDING_RUNS.with(|r| match trapped {
                // Sibling calls of a batch may still be outstanding: keep the run listed
                // until they must have ended.
                true => r.borrow_mut().insert(id, now_ms() + TRAP_LINGER_SECONDS * 1000).map(|_| ()),
                false => r.borrow_mut().remove(&id).map(|_| ()),
            });
            if self.finished.get() || !trapped {
                return;
            }
            // Must not trap: that would also undo the slot and pending-id release above.
            let (caller, language, script_sha256) = std::mem::take(&mut self.audit_key);
            let calls = self.calls();
            policy::try_append_audit(AuditEntry {
                id: 0,
                timestamp_ns: now_ns(),
                caller,
                language,
                script_sha256,
                canisters: self.touched.take().into_iter().collect(),
                calls,
                instructions: self.instructions_used(),
                status: AuditStatus::Trapped,
                violations: self.closing_violations(),
                error: Some("the canister trapped while handling a reply (instruction or memory limit, or a bug); the script's result was discarded; `instructions` excludes the trapped execution".into()),
                run_id: Some(id),
                methods: Some(self.messages.take()),
                args: None,
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

    /// Held across an interface fetch. If the canister traps before the fetch finishes (say,
    /// parsing a hostile interface), the rollback loses everything else, but ic-cdk drops the
    /// future in the cleanup callback, whose changes are kept: the failure is remembered, so
    /// later requests don't send the same messages again. Concurrent fetches of the same
    /// interface are not blocked (an in-flight marker would fail them).
    struct TrapMarksFailed(String);

    impl Drop for TrapMarksFailed {
        fn drop(&mut self) {
            if ic_cdk::futures::is_recovering_from_trap() {
                let msg = format!("could not fetch candid for {} (the canister trapped while the fetch was in progress)", self.0);
                DID_FAILED.with(|c| c.borrow_mut().insert(std::mem::take(&mut self.0), (msg, now_ns())));
            }
        }
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
        // `e` may be a target's reject message: quoted and escaped.
        let mut shown_e = e.clone();
        truncate(&mut shown_e, 300);
        let msg = format!("could not fetch candid for {key} ({}); a controller can register it with set_candid_override", policy::shown(&shown_e));
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
        /// Interfaces this run is fetching.
        fetching: RefCell<BTreeMap<String, Rc<RefCell<FetchState>>>>,
        pub touched: RefCell<BTreeSet<String>>,
        pub violations: RefCell<Vec<String>>,
        pub violations_dropped: Cell<u32>,
        pub logs: LogBuf,
    }

    /// A fetch in progress: the run's requests waiting for it, then its outcome.
    #[derive(Default)]
    struct FetchState {
        waiting: Vec<Waker>,
        result: Option<Result<String, String>>,
    }

    /// Marks an interface as being fetched by this run until dropped, then hands the
    /// fetch's outcome, whatever it is, to the run's other requests for it.
    struct Fetching<'a> {
        bridge: &'a IcBridge,
        key: String,
        state: Rc<RefCell<FetchState>>,
    }

    impl<'a> Fetching<'a> {
        fn start(bridge: &'a IcBridge, key: &str) -> Self {
            let state = Rc::new(RefCell::new(FetchState::default()));
            bridge.fetching.borrow_mut().insert(key.to_string(), state.clone());
            Self { bridge, key: key.to_string(), state }
        }

        fn finish(&self, r: Result<String, String>) -> Result<String, String> {
            self.state.borrow_mut().result = Some(r.clone());
            r
        }
    }

    impl Drop for Fetching<'_> {
        fn drop(&mut self) {
            self.bridge.fetching.borrow_mut().remove(&self.key);
            let mut s = self.state.borrow_mut();
            s.result.get_or_insert_with(|| Err("the interface fetch was cancelled".into()));
            s.waiting.drain(..).for_each(Waker::wake);
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
        /// Running out of budget once messages went out is not a violation: the run closes
        /// as `Error` (see `close_over_budget` in lib.rs).
        pub fn deny(&self, e: String) -> String {
            if self.calls() > 0 && e.starts_with("policy: instruction budget exceeded") {
                return e;
            }
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
            } else if let Some(e) = IN_FLIGHT.with(|m| {
                let m = m.borrow();
                if m.values().sum::<u32>() >= MAX_IN_FLIGHT {
                    Some(format!("policy: {MAX_IN_FLIGHT} messages are already awaiting replies; retry shortly"))
                } else if m.get(&self.token).is_some_and(|n| *n >= MAX_IN_FLIGHT_PER_TOKEN) {
                    Some(format!("policy: this token already has {MAX_IN_FLIGHT_PER_TOKEN} messages awaiting replies; retry shortly"))
                } else {
                    None
                }
            }) {
                Some(e)
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
        /// request the canister it is about); `label` names the message; `fetch_of` is the
        /// interface it fetches, if it does (see `Held`).
        /// `args`: the encoded arguments of a call, whose digest goes in the audit entry.
        async fn sent(&self, about: &Principal, mut label: String, fetch_of: Option<&str>, args: Option<&[u8]>, msg: Msg<'_>) -> Result<Vec<u8>, CallFailed> {
            let mut held = Held { msg, token: &self.token, fetch_of, sent: false };
            std::future::poll_fn(|cx| {
                let r = held.msg.as_mut().poll(cx);
                match (r.is_pending(), held.sent) {
                    (true, false) => {
                        self.record_message(about, std::mem::take(&mut label), args);
                        in_flight(&self.token, true);
                        held.sent = true;
                    }
                    (false, true) => {
                        in_flight(&self.token, false);
                        held.sent = false;
                    }
                    _ => {}
                }
                r
            })
            .await
        }

        /// Audit record for one outbound message: `Started` for the run's first, `Call` for
        /// each later one. It is appended in the same message execution that hands the
        /// message to the system, so the two are committed together when that execution
        /// ends, or discarded together if it traps first. Later traps cannot remove it.
        fn record_message(&self, p: &Principal, label: String, args: Option<&[u8]>) {
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
                args: args.map(|a| vec![policy::arg_digest(a)]),
            });
            if run.is_none() {
                self.started.set(Some(id));
                PENDING_RUNS.with(|r| r.borrow_mut().insert(id, now_ms() + STALE_RUN_MS));
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
            let key = p.to_text();
            let fetch_of = Some(key.as_str());
            let refused = |e: String| (e, false);
            let secs = self.recheck(&p, None).map_err(refused)?;
            let args = candid::encode_one(InfoArgs { canister_id: p, num_requested_changes: Some(0) }).map_err(|e| (e.to_string(), false))?;
            let info = self.sent(&p, format!("{mgmt}.canister_info({p})"), fetch_of, None, message(mgmt, "canister_info", &args, secs))
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| candid::decode_one_with_config::<InfoResult>(&r, &fetch_decoder()).map_err(|e| e.to_string()));
            // Whether metadata's failure (or skipping it) is lasting, so a failure of the
            // fallback may be remembered: not when it was skipped for a failed canister_info,
            // which may have been transient.
            let (meta_err, meta_lasting) = match info {
                Err(e) => (format!("skipped, canister_info failed: {e}"), false),
                Ok(i) if i.controllers.contains(&canister_self()) => {
                    ("skipped: this canister controls the target, so it could read a private section".to_string(), true)
                }
                Ok(_) => {
                    self.reserve(&p, None).map_err(refused)?;
                    let secs = self.recheck(&p, None).map_err(refused)?;
                    let args = candid::encode_one(MetadataArgs { canister_id: p, name: "candid:service".into() }).map_err(|e| (e.to_string(), false))?;
                    match self.sent(&p, format!("{mgmt}.canister_metadata({p})"), fetch_of, None, message(mgmt, "canister_metadata", &args, secs)).await {
                        Ok(r) => match candid::decode_one_with_config::<MetadataResult>(&r, &fetch_decoder()) {
                            Ok(m) => return String::from_utf8(m.value).map_err(|_| ("candid:service is not UTF-8".to_string(), true)),
                            Err(e) => (e.to_string(), true),
                        },
                        Err(e) => (e.to_string(), definitive_failure(&e)),
                    }
                }
            };
            self.reserve(&p, None).map_err(refused)?;
            let secs = self.recheck(&p, None).map_err(refused)?;
            let both = |e: String| format!("canister_metadata: {meta_err}; __get_candid_interface_tmp_hack: {e}");
            let hack = message(p, "__get_candid_interface_tmp_hack", &[], secs);
            let reply = self
                .sent(&p, format!("{p}.__get_candid_interface_tmp_hack"), fetch_of, None, hack)
                .await
                .map_err(|e| (both(e.to_string()), meta_lasting && definitive_failure(&e)))?;
            candid::decode_one_with_config::<String>(&reply, &fetch_decoder()).map_err(|e| (both(e.to_string()), meta_lasting))
        }

        /// The outcome of this run's fetch of `key` already in progress (Promise.all /
        /// gather), once it ends; `None` if there is none.
        async fn joined_fetch(&self, key: &str) -> Option<Result<String, String>> {
            let state = self.fetching.borrow().get(key).cloned()?;
            Some(
                std::future::poll_fn(|cx| {
                    let mut s = state.borrow_mut();
                    if let Some(r) = &s.result {
                        return Poll::Ready(r.clone());
                    }
                    if !s.waiting.iter().any(|w| w.will_wake(cx.waker())) {
                        s.waiting.push(cx.waker().clone());
                    }
                    Poll::Pending
                })
                .await,
            )
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

        /// `p`'s interface: an override, a cached copy, or fetched (an expired copy when a
        /// re-fetch failed transiently).
        pub async fn service(&self, p: &Principal) -> Result<Rc<Service>, String> {
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
        Principal::from_text(id).map_err(|e| format!("invalid canister id {}: {e}", policy::shown(&shown)))
    }

    impl HostBridge for IcBridge {
        async fn candid(&self, canister_id: &str) -> Result<String, String> {
            // Canonical text: `RYJL3-…` and `ryjl3-…` are the same principal.
            let p = parse_principal(canister_id)?;
            let key = p.to_text();
            // Allowlisted canisters only, whether the text comes from an override or a fetch.
            if let Some(did) = policy::served_override(&p).map_err(|e| self.deny(e))? {
                return Ok(did);
            }
            // Another request of this run (Promise.all / gather) is fetching it: use that
            // fetch's outcome, success or failure, instead of sending the same messages again.
            if let Some(r) = self.joined_fetch(&key).await {
                return r;
            }
            if let Some((did, _)) = cache_get(&key, DID_CACHE_TTL_NS) {
                return Ok(did);
            }
            // Fetching an interface sends messages: allowlisted canisters only, and it
            // counts against the per-script call cap.
            if let Some(e) = failed_get(&key) {
                return Err(e); // failed moments ago: don't send the same messages again
            }
            self.reserve(&p, None)?;
            let _trap = TrapMarksFailed(key.clone());
            let fetching = Fetching::start(self, &key);
            let fetched = self
                .fetch_did(p)
                .await
                .and_then(|did| match did.len() {
                    n if n > MAX_DID_BYTES => Err((format!("interface is {n} bytes, over the {MAX_DID_BYTES}-byte limit"), true)),
                    _ => Ok(did),
                })
                .and_then(|did| Service::parse(&did).map(|svc| (did, Rc::new(svc))).map_err(|e| (e, true)));
            fetching.finish(match fetched {
                Ok((did, svc)) => self.keep(&p, did, svc),
                Err((e, remember)) => fetch_failed(&key, e, remember),
            })
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
            let label = format!("{p}.{}", policy::shown(method));
            let reply = self.sent(&p, label.clone(), None, Some(&enc.bytes), message(p, method, &enc.bytes, secs)).await.map_err(|e| {
                let unknown = !e.is_clean_reject();
                // The reject message is the target's text: quoted and escaped.
                let mut e = e.to_string();
                truncate(&mut e, 300);
                format!("call {p}.{} failed: {}{}", policy::shown(method), policy::shown(&e), if unknown { " (outcome unknown: the call may have executed)" } else { "" })
            })?;
            decode_checked(&svc, &enc.func, &reply, &label).map_err(|(e, stale)| {
                if stale {
                    evict_interface(&p);
                }
                e
            })
        }

        fn principal(&self) -> String {
            canister_self().to_text()
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
        const TMP_HACK: &str = "__get_candid_interface_tmp_hack";
        const DID: &str = "service : { tick : () -> () }";

        /// Fake network for `message`: (target, method) -> reply.
        type Net = Rc<dyn Fn(Principal, &str) -> Msg<'static>>;

        thread_local! {
            pub(super) static NET: RefCell<Option<Net>> = RefCell::default();
            /// Stands in for ic-cdk's trap-cleanup flag.
            pub(super) static RECOVERING: Cell<bool> = const { Cell::new(false) };
        }

        fn bridge() -> IcBridge {
            bridge_for(b"bridge-test-token")
        }

        fn bridge_for(token: &[u8]) -> IcBridge {
            let h = policy::sha256_hex(token);
            policy::add_token_hash(&h, "t".into()).unwrap();
            IcBridge::new(&h, "javascript", "sha".into())
        }

        /// A reply that arrives on the second poll, as one to a sent message does.
        fn later(r: Result<Vec<u8>, CallFailed>) -> Msg<'static> {
            let mut r = Some(r);
            let mut first = true;
            Box::pin(std::future::poll_fn(move |cx| {
                if std::mem::take(&mut first) {
                    cx.waker().wake_by_ref();
                    return Poll::Pending;
                }
                Poll::Ready(r.take().unwrap())
            }))
        }

        fn reject(code: RejectCode, msg: &str) -> Result<Vec<u8>, CallFailed> {
            Err(CallFailed::CallRejected(ic_cdk::call::CallRejected::with_rejection(code as u32, msg.into())))
        }

        /// Install a fake network answering every message through `f` (after one poll).
        fn net(f: impl Fn(Principal, &str) -> Result<Vec<u8>, CallFailed> + 'static) {
            NET.set(Some(Rc::new(move |to, m| later(f(to, m)))));
        }

        fn poll_once<F: Future + ?Sized>(f: std::pin::Pin<&mut F>) -> Poll<F::Output> {
            f.poll(&mut std::task::Context::from_waker(Waker::noop()))
        }

        fn reset() {
            NET.set(None);
            RECOVERING.set(false);
            TEST_NOW.set(0);
            policy::remove_allowed(fx());
            evict_interface(&fx());
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
            let (msg, stale) = decode_checked(&svc, &func, &reply, &format!("{}.r2", fx())).unwrap_err();
            assert!(stale);
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
            policy::set_allowed(fx(), vec!["*".into()]).unwrap();
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
            policy::set_allowed(fx(), vec!["*".into()]).unwrap();
            assert!(b.reserve(&fx(), Some("tick")).is_ok());
            policy::remove_allowed(fx()); // during the fetch
            assert!(b.keep(&fx(), "did".into(), svc.clone()).unwrap_err().contains("not allowlisted"));
            assert!(cached_service(&fx(), true).is_none(), "nothing cached for a removed canister");
            policy::set_allowed(fx(), vec!["*".into()]).unwrap();
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
            policy::set_allowed(fx(), vec!["*".into()]).unwrap();
            let b = bridge();
            policy::set_limits(Limits { max_calls_per_script: 0, max_instructions: 1, call_timeout_seconds: Some(5), ..Limits::default() });
            assert_eq!(b.recheck(&fx(), Some("m")), Ok(policy::DEFAULT_CALL_TIMEOUT_SECONDS));
            assert_eq!(b.instruction_deadline() - b.base_instructions, policy::DEFAULT_MAX_INSTRUCTIONS);
            policy::set_limits(Limits::default());
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

        /// 125: messages count per token while awaiting a reply; one token may hold only
        /// its share, the rest stay open to other tokens, and the global cap still holds.
        #[test]
        fn in_flight_messages_are_capped_per_token_and_overall() {
            policy::set_allowed(fx(), vec!["*".into()]).unwrap();
            let (a, b, c) = (bridge_for(b"a"), bridge_for(b"b"), bridge_for(b"c"));
            let pending = || -> Msg<'static> { Box::pin(std::future::pending()) };
            let x = fx();
            let mut held = vec![];
            for _ in 0..MAX_IN_FLIGHT_PER_TOKEN {
                let mut f = Box::pin(a.sent(&x, "m".into(), None, None, pending()));
                assert!(poll_once(f.as_mut()).is_pending());
                held.push(f);
            }
            assert_eq!(a.calls(), MAX_IN_FLIGHT_PER_TOKEN, "each sent message is recorded");
            let e = a.reserve(&fx(), Some("m")).unwrap_err();
            assert!(e.contains("this token") && refuses_whole_run(&e), "{e}");
            assert!(b.reserve(&fx(), Some("m")).is_ok(), "another token is not locked out");
            for _ in 0..MAX_IN_FLIGHT - MAX_IN_FLIGHT_PER_TOKEN {
                let mut f = Box::pin(b.sent(&x, "m".into(), None, None, pending()));
                assert!(poll_once(f.as_mut()).is_pending());
                held.push(f);
            }
            assert!(c.reserve(&fx(), Some("m")).unwrap_err().contains("64 messages"));
            drop(held);
            assert!(IN_FLIGHT.with(|m| m.borrow().is_empty()), "dropping releases every message");
            // A completed message is released at once; a refused one (ready on the first
            // poll) is never counted or recorded.
            net(|_, _| Ok(vec![]));
            assert!(block_on(a.sent(&fx(), "m".into(), None, None, message(fx(), "m", &[], 1))).is_ok());
            assert!(block_on(c.sent(&fx(), "m".into(), None, None, Box::pin(async { Ok(vec![]) }))).is_ok());
            assert_eq!(c.calls(), 0);
            assert!(IN_FLIGHT.with(|m| m.borrow().is_empty()));
            reset();
        }

        /// 88: concurrent requests for an uncached interface share one fetch and its
        /// outcome, also a failure that is not remembered (a stopped target).
        #[test]
        fn concurrent_requests_share_one_fetch_and_its_failure() {
            policy::set_allowed(fx(), vec!["*".into()]).unwrap();
            net(|_, _| reject(RejectCode::CanisterError, "Canister x is stopped"));
            let b = bridge();
            let (r1, r2) = block_on(futures::future::join(b.candid(FX), b.candid(FX)));
            assert!(r1.as_ref().unwrap_err().contains("is stopped"), "{r1:?}");
            assert_eq!(r1, r2);
            assert_eq!(b.messages().len(), 2, "one canister_info and one tmp_hack: {:?}", b.messages());
            assert!(failed_get(FX).is_none(), "a stopped target is not remembered");

            // Success is shared too.
            net(|_, m| match m {
                // Controlled: metadata is skipped.
                "canister_info" => Ok(candid::encode_one(InfoResult { controllers: vec![canister_self()] }).unwrap()),
                _ => Ok(candid::encode_one(DID).unwrap()),
            });
            let b = bridge();
            let (r1, r2) = block_on(futures::future::join(b.candid(FX), b.candid(FX)));
            assert_eq!((r1.as_deref(), r2.as_deref()), (Ok(DID), Ok(DID)));
            assert_eq!(b.messages().len(), 2, "{:?}", b.messages());
            reset();
        }

        /// 196: a failed canister_info skips metadata, so a failure of the fallback is not
        /// remembered, however lasting it looks; after a good canister_info it is.
        #[test]
        fn fallback_failure_after_a_failed_canister_info_is_not_remembered() {
            policy::set_allowed(fx(), vec!["*".into()]).unwrap();
            let no_method = || reject(RejectCode::CanisterError, "Canister x has no query method '__get_candid_interface_tmp_hack'");
            net(move |_, m| match m {
                "canister_info" => reject(RejectCode::SysUnknown, "deadline expired"),
                TMP_HACK => no_method(),
                other => panic!("unexpected {other}"),
            });
            assert!(block_on(bridge().candid(FX)).is_err());
            assert!(failed_get(FX).is_none(), "metadata was never tried");

            // canister_info answered; metadata has no such section: remembered.
            net(move |_, m| match m {
                "canister_info" => Ok(candid::encode_one(InfoResult { controllers: vec![] }).unwrap()),
                "canister_metadata" => reject(RejectCode::CanisterReject, "no metadata section"),
                TMP_HACK => no_method(),
                other => panic!("unexpected {other}"),
            });
            assert!(block_on(bridge().candid(FX)).is_err());
            assert!(failed_get(FX).is_some());
            reset();
        }

        /// 136: an expired interface is served (also to a search, through `service`) when a
        /// re-fetch fails transiently.
        #[test]
        fn expired_interface_is_served_when_the_refetch_fails_transiently() {
            policy::set_allowed(fx(), vec!["*".into()]).unwrap();
            let svc = Rc::new(Service::parse(DID).unwrap());
            DID_CACHE.with(|c| c.borrow_mut().insert(FX.into(), (DID.into(), svc, 0)));
            TEST_NOW.set(DID_CACHE_TTL_NS + 1);
            assert!(cached_service(&fx(), true).is_none(), "expired");
            net(|_, _| reject(RejectCode::SysTransient, "queue full"));
            let b = bridge();
            assert!(block_on(b.service(&fx())).unwrap().method("tick").is_ok());
            assert_eq!(b.calls(), 2, "it was re-fetched: {:?}", b.messages());
            reset();
        }

        /// 73: a trap marks the interface failed only when it came from handling one of this
        /// fetch's replies, not from another message's reply in the same run.
        #[test]
        fn only_a_trap_in_this_fetchs_reply_is_remembered() {
            policy::set_allowed(fx(), vec!["*".into()]).unwrap();
            for replied in [false, true] {
                let ready = Rc::new(Cell::new(false));
                let r = ready.clone();
                NET.set(Some(Rc::new(move |_, _| {
                    let r = r.clone();
                    Box::pin(std::future::poll_fn(move |_| if r.get() { Poll::Ready(Ok(vec![])) } else { Poll::Pending }))
                })));
                let b = bridge();
                let mut f = Box::pin(b.candid(FX));
                assert!(poll_once(f.as_mut()).is_pending(), "canister_info sent");
                // Trap cleanup: ic-cdk completes the message whose reply callback trapped.
                ready.set(replied);
                RECOVERING.set(true);
                drop(f);
                RECOVERING.set(false);
                assert_eq!(failed_get(FX).is_some(), replied, "replied = {replied}");
                DID_FAILED.with(|c| c.borrow_mut().clear());
                assert!(IN_FLIGHT.with(|m| m.borrow().is_empty()));
            }
            reset();
        }

        /// 141(1): an interface whose canister left the allowlist while it was being fetched
        /// is not cached by `candid`.
        #[test]
        fn candid_does_not_cache_for_a_canister_removed_during_the_fetch() {
            policy::set_allowed(fx(), vec!["*".into()]).unwrap();
            net(|_, m| match m {
                "canister_info" => Ok(candid::encode_one(InfoResult { controllers: vec![] }).unwrap()),
                _ => {
                    policy::remove_allowed(fx()); // while the reply is on its way
                    Ok(candid::encode_one(MetadataResult { value: DID.into() }).unwrap())
                }
            });
            let e = block_on(bridge().candid(FX)).unwrap_err();
            assert!(e.contains("not allowlisted"), "{e}");
            assert!(DID_CACHE.with(|c| c.borrow().is_empty()));
            reset();
        }

        /// 191: `call` refuses a composite_query before encoding its arguments.
        #[test]
        fn call_refuses_composite_query_before_encoding() {
            policy::set_allowed(fx(), vec!["*".into()]).unwrap();
            policy::set_candid_override(fx(), Some("service : { c : (nat) -> (nat) composite_query }".into())).unwrap();
            let b = bridge();
            let e = block_on(b.call(FX, "c", json!("not-a-number"))).unwrap_err();
            assert!(e.contains("composite_query"), "{e}");
            assert_eq!(b.calls(), 0);
            policy::set_candid_override(fx(), None).unwrap();
            reset();
        }

        /// 110: interface TTLs at their boundaries, on the settable clock.
        #[test]
        fn interface_ttls_hold_to_the_nanosecond() {
            let svc = Rc::new(Service::parse(DID).unwrap());
            DID_CACHE.with(|c| c.borrow_mut().insert(FX.into(), (DID.into(), svc, 5)));
            TEST_NOW.set(5 + DID_CACHE_TTL_NS);
            assert!(cache_get(FX, DID_CACHE_TTL_NS).is_some());
            TEST_NOW.set(5 + DID_CACHE_TTL_NS + 1);
            assert!(cache_get(FX, DID_CACHE_TTL_NS).is_none());
            TEST_NOW.set(7);
            assert!(fetch_failed(FX, "no such method".into(), true).is_err());
            TEST_NOW.set(7 + DID_FAIL_TTL_NS);
            assert!(failed_get(FX).is_some());
            TEST_NOW.set(7 + DID_FAIL_TTL_NS + 1);
            assert!(failed_get(FX).is_none());
            reset();
            DID_CACHE.with(|c| c.borrow_mut().clear());
            DID_FAILED.with(|c| c.borrow_mut().clear());
        }

        /// 110: a call through the fake network writes one `Started` entry and lists the
        /// run as pending until it ends; the run's clock is the test clock.
        #[test]
        fn a_sent_call_opens_one_started_entry_and_a_pending_run() {
            policy::set_allowed(fx(), vec!["*".into()]).unwrap();
            policy::set_candid_override(fx(), Some(DID.into())).unwrap();
            net(|_, m| match m {
                "tick" => Ok(candid::encode_args(()).unwrap()),
                other => panic!("unexpected {other}"),
            });
            TEST_NOW.set(3_000_000_000);
            let before = policy::audit_len();
            let b = bridge();
            assert_eq!(block_on(b.call(FX, "tick", Value::Null)), Ok(Value::Null));
            assert_eq!(policy::audit_len(), before + 1);
            let id = b.run_id().unwrap();
            let entry = policy::audit_run(id, None).remove(0);
            assert!(matches!(entry.status, AuditStatus::Started), "{:?}", entry.status);
            assert_eq!(entry.timestamp_ns, 3_000_000_000);
            assert!(pending_run_ids().contains(&id));
            assert_eq!(b.principal(), Principal::anonymous().to_text(), "the native stand-in, no platform read off-chain");
            drop(b);
            assert!(!pending_run_ids().contains(&id));
            policy::set_candid_override(fx(), None).unwrap();
            reset();
        }

        /// 134: an over-long method name is refused before anything else.
        #[test]
        fn over_long_method_names_are_refused() {
            let b = bridge();
            let e = block_on(b.call(FX, &"m".repeat(MAX_METHOD_LEN + 1), Value::Null)).unwrap_err();
            assert!(e.contains("longer than"), "{e}");
            policy::set_allowed(fx(), vec!["*".into()]).unwrap();
            policy::set_candid_override(fx(), Some(DID.into())).unwrap();
            let e = block_on(b.call(FX, &"m".repeat(MAX_METHOD_LEN), Value::Null)).unwrap_err();
            assert!(!e.contains("longer than"), "{e}");
            policy::set_candid_override(fx(), None).unwrap();
            reset();
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

    /// 77: a budget refusal at send time is a violation only while nothing was sent.
    #[cfg(test)]
    #[test]
    fn budget_refusal_after_sends_is_not_a_violation() {
        let b = IcBridge::new("c", "javascript", "h".into());
        let over = || "policy: instruction budget exceeded (11 > 10)".to_string();
        b.deny(over());
        assert_eq!(b.violations.borrow().len(), 1);
        b.messages.borrow_mut().push("x.icrc1_transfer".into());
        b.deny(over());
        assert_eq!(b.violations.borrow().len(), 1, "not recorded once a message went out");
        b.deny("policy: method `m` on x is not allowlisted".into());
        assert_eq!(b.violations.borrow().len(), 2, "other refusals still are");
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
        /// `instruction_deadline` (see `instruction_counter`'s off-chain stand-in).
        pub deadline: u64,
    }

    impl Default for MockBridge {
        fn default() -> Self {
            Self { calls: RefCell::default(), logs: RefCell::default(), allowed: "ryjl3-tyaaa-aaaaa-aaaba-cai", deadline: u64::MAX }
        }
    }

    impl MockBridge {
        /// A bridge whose instruction budget ends after `checks` more budget checks.
        pub fn with_budget(checks: u64) -> Self {
            Self { deadline: instruction_counter() + checks, ..Self::default() }
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
        fn instruction_deadline(&self) -> u64 {
            self.deadline
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

    /// 175: a cut text, marker included, is never longer than asked, in raw or escaped bytes.
    #[test]
    fn truncation_stays_within_max() {
        for text in ["x".repeat(5000), "🦀é".repeat(2000), "\u{1}\"".repeat(3000)] {
            for max in [3, 14, 15, 300, 512, 4096] {
                let mut s = text.clone();
                truncate(&mut s, max);
                assert!(s.len() <= max, "{max}: {}", s.len());
                assert_eq!(s.ends_with(CUT_MARK), max >= CUT_MARK.len());
                let mut s = text.clone();
                truncate_escaped(&mut s, max);
                assert!(escaped_len(&s) <= max, "{max}: {}", escaped_len(&s));
                assert_eq!(s.ends_with(CUT_MARK), max >= CUT_MARK.len());
            }
        }
        let mut s = "short".to_string();
        truncate(&mut s, 5);
        truncate_escaped(&mut s, 5);
        assert_eq!(s, "short");
    }

    /// 175 / 176: a line that does not fit keeps its prefix (it is not replaced by a marker),
    /// lines are charged escaped, newline included, and whatever is dropped is marked, even
    /// when script text ends with the marker itself.
    #[test]
    fn log_lines_keep_the_prefix_charge_newlines_and_mark_drops() {
        let log = |lines: Vec<String>| {
            let mut used = 0;
            lines.into_iter().filter_map(|l| log_line(used, l, false).inspect(|(_, u)| used = *u).map(|(l, _)| l)).collect::<Vec<_>>()
        };
        let lines = log(vec!["a".repeat(MAX_LOG_BYTES - 1000), "b".repeat(5000), "c".into()]);
        assert_eq!(lines.len(), 2);
        assert!(lines[1].starts_with('b') && lines[1].ends_with(CUT_MARK), "{}", &lines[1][..10]);
        assert!(lines.iter().map(|l| log_cost(l)).sum::<usize>() <= MAX_LOG_BYTES);
        let lines = log(vec![String::new(); MAX_LOG_BYTES]);
        assert_eq!(lines.len(), (MAX_LOG_BYTES - LOG_RESERVE) / 2 + 1, "an empty line costs its escaped newline");
        assert_eq!(lines.last().unwrap(), CUT_MARK, "what was dropped is marked");
        assert_eq!(log_cost("\u{1}"), 8);
        // Script text ending with the marker gets no room from the reserve.
        let fake = "x".repeat(MAX_LOG_BYTES - LOG_RESERVE - CUT_MARK.len()) + CUT_MARK;
        let lines = log([fake, "secret".into()].into_iter().chain((0..10).map(|i| i.to_string())).collect());
        assert!(lines.last().unwrap().ends_with(CUT_MARK), "{:?}", lines.last());
        assert!(lines.len() <= 2 && lines.iter().map(|l| log_cost(l)).sum::<usize>() <= MAX_LOG_BYTES);
        // The transport keeps what a runtime produced, its cut line included.
        let buf = LogBuf::default();
        let produced = log(vec!["a".repeat(MAX_LOG_BYTES - 1000), "b".repeat(5000)]);
        produced.iter().for_each(|l| buf.push(l.clone()));
        assert_eq!(buf.take(), produced);
    }

    /// 114 / 155 / 156: the value rules every runtime shares.
    #[test]
    fn shared_value_rules() {
        let safe = (1u64 << 53) - 1;
        let v = big_ints_to_strings(serde_json::json!([safe, -(safe as i64), safe + 1, -(safe as i64) - 1, u64::MAX, 1.5e300, { "k": 1u64 << 60 }]));
        assert_eq!(v, serde_json::json!([safe, -(safe as i64), "9007199254740992", "-9007199254740992", "18446744073709551615", 1.5e300, { "k": "1152921504606846976" }]));
        assert_eq!([f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0.5].map(float_json), [serde_json::json!("NaN"), "inf".into(), "-inf".into(), 0.5.into()]);
        assert_eq!(normalize_numbers(serde_json::json!([-0.0, 0.0])), serde_json::json!([-0.0, 0]));
        let max = crate::candid_bridge::MAX_DEPTH;
        assert!(container_depth(max - 1).is_ok() && container_depth(max).is_err());
        // Replies: MAX levels of arrays fit, scalars inside them included; one more does not.
        let nest = |n: usize, inner: Value| (0..n).fold(inner, |v, _| serde_json::json!([v]));
        for inner in [serde_json::json!([]), serde_json::json!(1)] {
            let levels = if inner.is_array() { 1 } else { 0 };
            assert!(check_value(&nest(max - levels, inner.clone())).is_ok());
            assert!(check_value(&nest(max + 1 - levels, inner)).is_err());
        }
    }

    #[test]
    fn value_budget_truncate_normalize_logbuf_and_overrides() {
        // LogBuf cap short-circuits once MAX_LOG_BYTES is reached.
        let buf = LogBuf::default();
        buf.push("x".repeat(MAX_LOG_BYTES));
        buf.push("overflow".into());
        assert_eq!(buf.take().len(), 1);

        // Multi-byte UTF-8 boundary truncation; no room for the marker.
        let mut s = "a🦀🦀".to_string();
        truncate(&mut s, 3);
        assert_eq!(s, "a");

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
        crate::policy::set_candid_override(p, Some("service : { ping : () -> () }".into())).unwrap();
        assert!(cached_service(&p, true).is_some());
        assert!(cached_service(&p, true).is_some()); // override cache hit
        evict_interface(&p);
        crate::policy::set_candid_override(p, None).unwrap();
    }
}
