# Coverage grid: L1 Access control, round 4: durable authority through allowlisted methods

Run `2026-11-17-295d64b`. Question: can one audited call hand an outside principal standing
authority over what this canister's principal holds, so that later effects escape the call
cap, the policy check, the audit log, token revocation and `remove_allowlist`?

Evidence key: **R-native** = probe in a private copy (`src/l1r4_probe.rs`, 4 tests, all pass,
source in the appendix); **R-live-ro** = read-only queries on the local network (port 8010),
nothing changed; **CP** = the code path produces it unambiguously.

## Candidate findings

| Id | Title | Severity | Evidence |
|---|---|---|---|
| L1-r4-1 | The allowlist and the README treat authority-granting methods like any other; `vec {"*"}` is offered with no caveat. One `icrc2_approve` (or ICRC-37 approval, neuron hotkey, controller change) gives an outside principal standing authority that is exercised outside every guardrail and survives `revoke_api_token_hash` and `remove_allowlist` | High | R-native (policy and encoding), R-live-ro (ledger supports ICRC-2), CP (consequence) |
| L1-r4-2 | Audit entries record `canister.method` only, never the arguments, so the log cannot tell who was approved, who was paid or how much | Medium | R-live-ro (real `icrc1_transfer` entry), R-native (field list) |
| L1-r4-3 | An empty method list is an undocumented third state: the canister is listed by search, its interface is fetched (messages sent to `aaaaa-aa` and to the target) and returned by `icp.candid`, while every call is refused | Low | R-native |
| L1-r4-4 | Interface fetch asks for `candid:service` without a visibility check: if this canister controls the target, `canister_metadata` can return the private section, which is then cached and handed to every token holder; the code comment and README say "public" | Low | CP (platform behaviour for private sections not reproduced) |

### L1-r4-1

- `src/policy.rs:178` admits any method when the list contains `"*"`; there is no notion of a
  method that delegates authority. `README.md:159` "Use `vec {"*"}` to allow every method of a
  canister", directly after the ICP ledger example, with no caveat. `README.md:460-462`
  advises revoking tokens "when in doubt", which does not undo an approval.
- R-native `l1r4_wildcard_admits_authority_granting_methods`: with `["*"]` on the ledger id,
  `check_call` admits `icrc2_approve`, `icrc37_approve_tokens`, `icrc37_approve_collection`,
  `manage_neuron`, `add_controller`, `set_admin`, and a method a future upgrade of the target
  might add. After `remove_allowed`, `icrc2_approve` is refused: the canister then has no path
  at all to zero an allowance it granted (only re-allowlisting plus a token-holder script can).
- R-native `l1r4_approve_encodes_unbounded_non_expiring_allowance_to_any_spender`: the Candid
  bridge encodes `icrc2_approve { spender = 2vxsx-fae; amount = 2^128-1; expires_at = null }`
  intact (decoded bytes show both). One cap slot, one audit entry `ledger.icrc2_approve`.
- R-live-ro: the local ledger `ryjl3-...` reports ICRC-2 in `icrc1_supported_standards`;
  `icrc2_allowance(4caro-hl777-77775-aaaba-cai -> 2vxsx-fae)` = 0 (no approval exists today);
  the live allowlist holds the five read methods only. No `icrc2_*` entry in 9,799 audit
  entries scanned.
- Consequence (CP, ICRC-2 standard semantics): the spender calls `icrc2_transfer_from` on the
  ledger directly. That message never passes through this canister: no call cap, no
  instruction budget, no allowlist, no audit entry, and neither `revoke_api_token_hash`
  (stops new requests only) nor `remove_allowlist` (stops this canister's own calls only)
  affects it. Same shape: ICRC-37 approvals, NNS/SNS `manage_neuron` (hotkeys, follow,
  disburse to another account), any `add_controller`/`set_owner` method; `aaaaa-aa.update_settings`
  is the already-reported L1-3 variant.
- Refutation tried: register #49 (one shared identity, accepted, Low) says tokens act as one
  principal; it does not say that a grant outlives the token or escapes caps and audit. The
  ledger's own block log does record the approve and every `transfer_from` (mitigates
  detection for ledgers, not for neurons or arbitrary targets); it does not stop the drain.
- Fix: document that allowlisting a delegating method gives token holders the power to
  create authority that outlives them; exclude a known list of delegating methods from `"*"`
  (must be named explicitly) or refuse `"*"` on canisters whose interface declares them;
  give controllers a way to undo grants (e.g. a controller-only passthrough) after revocation.

### L1-r4-2

- `src/policy.rs:76-98` (`AuditEntry`): fields `id, timestamp_ns, caller, language,
  script_sha256, canisters, calls, instructions, status, violations, error, run_id, methods`
  (R-native `l1r4_audit_entry_carries_no_call_arguments`). Label built at
  `src/bridge.rs:388` as `format!("{}.{method}", p.to_text())`. The script text is not stored,
  only its hash.
- R-live-ro: entry id 1160 (`Ok`, python, methods `ryjl3-....icrc1_balance_of`,
  `ryjl3-....icrc1_transfer`) has no recipient, amount or memo; nothing in the canister can
  recover them.
- Fix: for calls (not fetches), store a bounded copy or a digest of the encoded arguments, or
  the decoded record of a known value/authority method (spender, to, amount, expires_at).

### L1-r4-3

- `src/policy.rs:200-201`: `check_interface_fetch` only tests `contains_key`. R-native
  `l1r4_empty_method_list_still_admits_messages_to_the_target`: `set_allowed(p, [])` ->
  fetch `Ok`, any call refused, `search::scope` lists `p` with `[]`. `README.md:365` documents
  only `["method" | "*"]`; nothing says an empty list still permits messages to the target.

### L1-r4-4

- `src/bridge.rs:424-432` sends `canister_metadata { name = "candid:service" }`; the platform
  type (`ic-management-canister-types-0.10.0/src/lib.rs:604-606`) addresses both
  `icp:public` and `icp:private` sections. The comment (`src/bridge.rs:421`) and
  `README.md:494` say "the public `candid:service` section". The result goes to `DID_CACHE`
  and out through `icp.candid` (`src/bridge.rs:478-479`) and search to every token holder.
  Live: this canister controls neither target (both controlled by one user principal), so
  the private case cannot arise there today.

## Grid

| Row | Cell |
|---|---|
| VF4 assets held by this principal | **finding** L1-r4-1: value can move after the token and the allowlist entry are gone, through authority granted in one audited call; L1-r4-2: the audit does not say where value went |
| D2 allowlist | **finding** L1-r4-1 (no delegating-method notion, wildcard covers future methods), L1-r4-3 (empty list semantics). Keying and canonical form hold (round 1, 2) |
| AD2 `set_allowlist` | **finding** L1-r4-1 / L1-r4-3: accepts `"*"` on any canister without warning and `[]` as a silent fetch-only state. Unvalidated principal itself: already reported (L1-3), not repeated |
| E6 `set_allowlist` | **holds** for who may call it (controller guard, R-live round 1). What it grants: L1-r4-1 |
| E9 / AD5 `add_api_token_hash` | **holds**: guard, 64-hex validation (`src/policy.rs:208-214`); a new token gets exactly the shared principal's current allowlist, nothing more. Label length is L3/L14, not L1 |
| E10 `revoke_api_token_hash` | **finding** (part of L1-r4-1): revocation stops new requests (holds, round 1) but not authority already delegated to outside principals. In-flight runs: already reported (L1-2), not repeated |
| AD6 token revocation | same as E10 |
| A7 management canister | **finding** L1-r4-4 (fetch can read private metadata when this canister is a controller). Script calls to `aaaaa-aa`: already reported (L1-3) |
| CL6 "deny by default; checked before any message is sent" | **holds** literally for messages this canister sends (R-native round 1, `reserve` before every send); the guarantee does not extend to effects of delegated authority, which the README never says (L1-r4-1). Empty-list nuance: L1-r4-3 |
| CL1 "every run is audited" | **holds** literally (each message of each run gets an entry). Misleading for delegated effects (not runs, not audited) and for arguments (L1-r4-2) |
| README:159 `vec {"*"}` | **finding** L1-r4-1 (no caveat; on a ledger it includes `icrc2_approve` and `icrc1_transfer`) |
| README:460-462 revocation advice | **finding** (part of L1-r4-1): "revoked ... when in doubt" implies revocation contains a leaked token; it does not undo its approvals |

## Phase 4 techniques applied

- Boundary: approve with `amount = 2^128-1`, `expires_at = null` encodes intact (no limit on
  the size of a grant; one cap slot regardless of amount).
- Differential: `check_call` vs `check_interface_fetch` on an empty list (fetch allowed, call
  refused).
- Live: read-only only (this task is not the Live phase).

## Not tested, and why

- The end-to-end grant: allowlist `icrc2_approve` on the local ledger, fund the canister,
  approve an outside identity from a script, revoke the token and `remove_allowlist`, then
  `icrc2_transfer_from` as that identity and diff the audit log. It changes settings and
  ledger state, which only the Live phase may do. Restore: allowlist back to the five read
  methods, allowance set to 0, balance returned.
- Private `candid:service` via `canister_metadata`: needs this canister as controller of a
  target built with `icp:private candid:service`; that changes canister settings.
- ICRC-37, NNS governance, SNS: none deployed on the local network.

## Appendix: probe source (private copy only, `src/l1r4_probe.rs`, `#[cfg(test)] mod`)

```rust
#[test] fn l1r4_wildcard_admits_authority_granting_methods() {
    let l = Principal::from_text(LEDGER).unwrap();
    policy::set_allowed(l, vec!["*".into()]);
    for m in ["icrc2_approve","icrc37_approve_tokens","icrc37_approve_collection","manage_neuron",
              "add_controller","set_admin","icrc1_transfer","some_method_added_by_a_future_upgrade"] {
        assert_eq!(policy::check_call(&l, m, 0, 0), Ok(()));
    }
    assert!(policy::remove_allowed(l));
    assert!(policy::check_call(&l, "icrc2_approve", 0, 0).unwrap_err().contains("not allowlisted"));
}
#[test] fn l1r4_approve_encodes_unbounded_non_expiring_allowance_to_any_spender() {
    // ICRC-2 ApproveArgs .did; spender 2vxsx-fae, amount 2^128-1, expires_at null:
    // Service::encode_args ok; IDLArgs::from_bytes shows the principal and the amount.
}
#[test] fn l1r4_audit_entry_carries_no_call_arguments() {
    // AuditEntry::ty() record fields: id, status, timestamp_ns, methods, run_id, calls,
    // violations, instructions, error, language, canisters, caller, script_sha256.
}
#[test] fn l1r4_empty_method_list_still_admits_messages_to_the_target() {
    // set_allowed(p, []): check_interface_fetch Ok; check_call Err "not allowlisted";
    // search::scope lists (p, []).
}
```
