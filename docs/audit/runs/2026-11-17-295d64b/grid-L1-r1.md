# Coverage grid: L1 Access control, round 1

Run `2026-11-17-295d64b`. Lens question: who can reach this, and should they? Row ids are
from `inventory.md`. Each cell is **finding** (id below), **holds** (with the reason) or
**n/a** (with the reason).

Evidence key: **R-native** = reproduced by a native test that drives the production
`IcBridge` / `policy` / `mcp` code in a private copy (probe source in the appendix);
**R-live** = reproduced on the local icp-cli network (port 8010) with read-only or no-op
requests (anonymous identity, non-existent hash/principal as arguments); **CP** = the code
path produces it unambiguously. No settings were changed on the live canister.

## Candidate findings

| Id | Title | Severity | Evidence |
|---|---|---|---|
| L1-1 | `icp.candid` serves admin overrides (and cached interfaces) without an allowlist check; overrides outlive `remove_allowlist` | Low | R-native |
| L1-2 | Allowlist revocation is not re-checked after the interface-fetch await: a call authorized before `remove_allowlist`/`set_allowlist` narrowing is still sent after it | Medium | CP |
| L1-3 | `set_allowlist` accepts this canister's own principal and `aaaaa-aa` with `"*"`: nested self-runs with fresh budgets, management calls with the canister's authority | Low | R-native (acceptance) + CP (consequence) |
| L1-4 | README says the override is the third interface source; code uses it first, before any fetch (observed under L1; class L16) | Medium | R-native |
| L1-5 | demo_store has no access control: anyone can `cancel_order`/`restock`/`create_order` for any customer, while the README presents them as "blocked" | Low | CP |

Details are in the StructuredOutput of this lens; summary of each:

- **L1-1** `src/bridge.rs:471-480`: `HostBridge::candid` returns `policy::candid_override(&key)`
  and then `cache_get(&key)` before `reserve(&p, None)`, so neither is gated by
  `check_interface_fetch`. `remove_allowlist` (`src/lib.rs:300-304`) evicts the heap caches
  but not `CANDID_OVERRIDES` (D5), and `set_candid_override` accepts any principal (AD7).
  The doc comment at `src/policy.rs:198` says `icp.candid` "may target any allowlisted
  canister". Probe `override_is_served_to_scripts_without_allowlist`: a canister never
  allowlisted, with an override, returns the full override text through `IcBridge::candid`
  and through JS, Python and Rhai scripts, while `icp.call` on it is refused with "not
  allowlisted"; no violation is recorded for the read. Probe `override_survives_remove_allowlist`:
  same after the `remove_allowlist` sequence. Variant (CP): a fetch in flight when
  `remove_allowlist` runs re-inserts `DID_CACHE` after its await (`src/bridge.rs:497-499`),
  so the removed canister's interface is served by `icp.candid` for up to 1 h.
- **L1-2** `src/bridge.rs:510-528`: `call` runs `reserve(&p, Some(method))`, then
  `self.service(&p).await`, which on a cache miss calls `candid` -> `fetch_did`, two
  `bounded_wait` awaits (up to ~300 s each). Other messages (e.g. a controller's
  `remove_allowlist`) execute during those awaits. After them the call is encoded and sent
  with no second policy check. Same inside `fetch_did` (`src/bridge.rs:424-436`): the
  fallback `__get_candid_interface_tmp_hack` is sent to `p` after the
  `canister_metadata` await without re-checking `p`. A run's bearer token is checked once
  per HTTP request (`src/mcp.rs:375`), so `revoke_api_token_hash` does not stop a run in
  flight either (up to `max_calls_per_script` more messages, possibly much later with
  unbounded waits).
- **L1-3** `src/lib.rs:295-298`, `src/policy.rs:155-157`: no validation of the principal.
  Probe `any_principal_can_be_allowlisted_including_management_and_self_like_ids`: with
  `"*"`, `check_call` admits `aaaaa-aa.update_settings`, `aaaaa-aa.delete_canister`, and
  `<self>.http_request_update`. Consequences by code path: a self-call to
  `http_request_update` carrying the caller's token passes `mcp::authenticate` (inter-canister
  calls skip `inspect_message`, `src/lib.rs:270-272`) and starts a nested run with its own
  `IcBridge` (fresh 25-call cap, fresh instruction budget in a new call context, its own
  `run_id`, not linked to the outer run), bounded only by the 8 run slots for depth;
  sequential fan-out is up to 25 per level. Admin endpoints stay closed: the canister is
  not its own controller (R-live: controllers = one user principal), so the guard refuses.
  The README warns about none of this.
- **L1-4** `README.md:493-496` vs `src/bridge.rs:207-212, 439-443, 475-477`. Probe
  `override_is_used_before_any_fetch_even_when_allowlisted`: an allowlisted canister with
  an override returns the override without attempting a send (a send would hit the ic0
  stub and panic natively).
- **L1-5** `demo_store/src/lib.rs:277-337` has no caller check and no `inspect_message`;
  `create_order` takes `customer` as an argument. `README.md:168-171` and
  `scripts/demo-setup.sh:3` say `cancel_order` and `restock` "stay blocked"; that holds only
  through `icp_code_mode`.

## Grid

### Actors

| Row | Cell |
|---|---|
| A1 Anonymous web user | holds: only `GET /`, `/demo` are served without a token (`mcp::demo_page`); every other path or method on `/mcp` goes through `authenticate` (404/405/401). Unit tests `mcp::auth_and_routing`, `demo_page_is_served_without_auth`; probe `token_header_forms` |
| A2 Anonymous direct ingress | holds (R-live): anonymous ingress to `http_request_update` with no token, a wrong token, and a `GET /` request was refused by `inspect_message` ("Canister rejected the message"); anonymous `http_request` as an update, `get_policy` as an update, `revoke_api_token_hash` (all-zero hash) and `remove_allowlist(2vxsx-fae)` were refused the same way; `get_policy` and `get_audit_log_len` as queries were refused by the guard ("caller is not a controller") |
| A3 Token holder | holds for the request path (`authenticate` in E4 and E5, same function). Finding L1-1 (reads overrides of non-allowlisted canisters), L1-2 (in-flight runs survive token revocation). All holders share one principal: register #49, accepted |
| A4 AI model | n/a for L1: consumes content, has no access of its own beyond A3's token (content steering is L10/L17) |
| A5 Controller | holds: every admin method has `guard = "controller"` (`src/lib.rs:295-353`), and `inspect_message` admits non-`http_request_update` ingress only from controllers (R-live). Finding L1-3: a controller can grant management/self access without warning |
| A6 Allowlisted target | holds: it receives only allowlisted methods (policy before send), except L1-2 (a method revoked during an interface fetch can still be called). It cannot call back into C1 with privilege (not a controller; `http_request_update` still needs a token) |
| A7 Management canister | holds for the fetch path: `canister_metadata` is sent for any allowlisted target without `aaaaa-aa` itself being allowlisted (by design, fixed method, fixed argument). Finding L1-3 for script calls when a controller allowlists it |
| A8 Other canisters | holds: no `inspect_message` for them (they pay), but `http_request_update` still runs `authenticate`, and admin methods still run the guard. Self as A8: L1-3 |
| A9 Infrastructure | n/a for L1: can read tokens (register #48, accepted) and alter queries (register #47, opt-in fix); that is L10 |
| A10 Anyone on demo_store | finding L1-5 |

### Entry points

| Row | Cell |
|---|---|
| E1 `init` | holds: run by the installer only (platform) |
| E2 `post_upgrade` | holds: only controllers can upgrade (platform); args can add tokens/allowlist, which is the controller's right |
| E3 `inspect_message` | holds (R-live, six refusals above). Checks the same `mcp::authenticate` as E5 on the same decoded type, so the filter and the method agree; non-token methods require `is_controller`. Not a security boundary by design (comment `src/lib.rs:270-272`), and E5/E6-E14 re-check. No automated test (I18 gap stays) |
| E4 `http_request` | holds: query, free; `/mcp` requires a token before anything is answered (`http_query` calls `authenticate` before parsing the body); called as an update by a non-controller it is refused (R-live) |
| E4a demo page | holds: public by design, no secrets (`CL30`, L17 owns content); disabled under `certified_reads` |
| E4b `POST /mcp` query routes | holds: token checked first; cached search shows only allowlisted canisters and allowed methods (`search::scope`, `render`) |
| E5 `http_request_update` | holds: `authenticate` first (`src/mcp.rs:375`); token is not re-checked during the run (L1-2) |
| E5a `execute_icp_script` | holds for entry (token); outbound policy: L1-1, L1-2 |
| E5b `search_icp_canisters` | holds: scope is the allowlist (`policy::list_allowed`), fetches go through `IcBridge::candid` -> `reserve(p, None)`; canonical principal (`search::scope` parses and compares `Principal`) |
| E5c reply callbacks | n/a: callable only by the system |
| E5d cleanup callback | n/a: callable only by the system |
| E6 `set_allowlist` | holds for who (guard, R-live for a no-op sibling). What it accepts: L1-3 |
| E7 `remove_allowlist` | holds for who (R-live: anonymous refused). Effect incomplete: L1-1 (override stays and is still served), L1-2 (in-flight calls) |
| E8 `set_limits` | holds for who (guard; not probed live because a refusal probe would need a value that changes state if the guard failed) |
| E9 `add_api_token_hash` | holds for who (guard, CP) |
| E10 `revoke_api_token_hash` | holds for who (R-live: anonymous refused). Effective immediately for new requests (probe `token_header_forms`); not for runs in flight (L1-2) |
| E11 `set_candid_override` | holds for who (guard). Accepts any principal: part of L1-1 |
| E12 `get_policy` | holds (R-live: anonymous query refused by guard, update refused by `inspect_message`) |
| E13 `get_audit_logs` | holds (guard, same macro as E12/E14) |
| E14 `get_audit_log_len` | holds (R-live: anonymous query refused) |
| E15 non-method exports | holds: not `canister_*` exports, so not callable as messages |
| E16 JS SDK | holds for `icp.call` (every request goes to `IcBridge::call` -> `reserve`; `icp` is read-only and non-enumerable); `icp.candid`: L1-1 (R-native through JS) |
| E17 JS console/print | n/a for L1: no access decision (output caps are L3) |
| E18 Python SDK | same as E16 (R-native through Python for `icp.candid`) |
| E19 Rhai SDK | same as E16 (R-native through Rhai). Replay answers are served only to the identical `Request` at the same sequence position (I27), so a cached answer is never handed to a request that policy did not see |
| E20 demo_store queries | holds: public read-only demo data by design |
| E21-E23 demo_store updates | finding L1-5 |
| E24 `__get_candid_interface_tmp_hack` | holds: public interface text by design |
| E25 demo_store lifecycle | n/a: no init args, nothing to gate |
| E26-E28 page, setup script, video | n/a for L1: page only calls `POST /mcp` with the user's token; setup script runs as the operator's identity; video tooling is local |

### Outbound effects

| Row | Cell |
|---|---|
| O1 script calls | holds: `reserve` (allowlist by `Principal` key, exact method or `"*"`, cap, budget) runs before any await of `call`, on the canonical principal (R-native: uppercase id passes and reaches the send stub `cost_call`; case-variant method name refused; whitespace, missing dashes, trailing dash refused). Finding L1-2: not re-checked after the interface-fetch awaits |
| O2 interface fetch | holds: `reserve(p, None)` -> `check_interface_fetch(p)` before the first message; the management call is fixed (`canister_metadata` of `p`). Finding L1-2 for the fallback message after the first await |
| O3 audit writes | holds: only via E5 (token) and cleanup |
| O4 admin writes | holds: guard |
| O5-O8 | n/a for L1: certified data, HTTP replies, `accept_message`, traps are not access decisions beyond E3/E4 above |
| O9 demo_store writes | finding L1-5 |
| O10 video tooling | n/a: local developer tooling |

### State, admin surface, conversions

| Row | Cell |
|---|---|
| D1 limits | holds: written by E1/E2/E8 only (guarded) |
| D2 allowlist | holds: keyed by `Principal` (bytes), so spelling cannot miss it; writers guarded. Accepts any principal: L1-3 |
| D3 tokens | holds: writers guarded; lowercase hex on insert, revoke and lookup (probe `token_header_forms`, unit `policy::tokens_are_hashed`) |
| D4 audit | holds: written only by E5 and cleanup; read only by controllers (R-live) |
| D5 overrides | finding L1-1 (read without allowlist check, survives `remove_allowlist`). Key is canonical (`Principal::to_text()` on write and read; R-native uppercase lookup) |
| AD1 init args | holds: controller-only, hash format checked |
| AD2 `set_allowlist` | finding L1-3; empty method list = interface reads only (R-native), `"*"` is the only wildcard (`"icrc1_*"` is literal, R-native) |
| AD3 `remove_allowlist` | finding L1-1 / L1-2 |
| AD4 `set_limits` | n/a for L1 (ranges are L14) |
| AD5 / AD6 token admin | holds (see D3) |
| AD7 `set_candid_override` | part of L1-1 (any principal, not tied to the allowlist) |
| AD8 canister settings | holds: platform controllers only; the live canister is not its own controller (R-live) |
| AD9 build choices | n/a for L1 |
| B3 token -> hash, caller id | holds: SHA-256 of the exact token after `trim()` of surrounding whitespace; scheme case-insensitive; first `Authorization` header wins (a second valid one does not help); the 16-hex audit prefix is a label, not an auth key |
| B4 principal text | holds on every path: bridge (`parse_principal` -> `Principal::from_text`, keys from `to_text()`), search (`Principal::from_text(trim)` compared as `Principal`), overrides (`to_text()`). R-native. Search trims whitespace and the bridge does not; the bridge fails closed, so this is not an access difference |

### Invariants, notes, register

| Row | Cell |
|---|---|
| I1(a) | violated in part: L1-2 (a message that the allowlist no longer permits can be sent). L1-1 sends nothing but contradicts the "allowlisted canisters only" comment for `icp.candid` |
| I1(b) | holds (R-live for token and controller paths); still no automated test of the guards (gap stands) |
| I18 | holds (R-live); no automated test (gap stands) |
| I19 | holds on the bridge call path (R-native, see O1/B4); register #1 bridge-path guard is still "none" in the repo, the probe exists only in the private copy (appendix) |
| Note 8 (`aaaaa-aa`/self allowlisted) | finding L1-3 |
| Note 9 (override/cached text without allowlist) | finding L1-1 |
| Register #1 (bridge path) | still fixed (R-native) |

## Not tested, and why

- L1-2 and the L1-3 consequences need `remove_allowlist`/`set_allowlist` on a live
  network during an in-flight fetch, or allowlisting the canister itself; both change
  settings, which this phase may not do. A Live-phase test: allowlist demo_store with
  `create_order`, start a script that calls it with a cold interface cache, call
  `remove_allowlist` while the fetch is pending, then check demo_store's order count and
  the audit `methods`.
- `IcBridge` paths that send messages cannot run natively (ic0 stubs panic, register #58);
  only the no-send paths and the policy decision before the send were exercised.
- Calls to `http_request_update` and admin methods from another canister (A8) were not
  made: no caller canister is deployed and creating one changes the network.
- `set_limits` and `set_candid_override` were not probed live as non-controllers: no
  argument is a guaranteed no-op if the guard were broken.
- `canister_metadata` returning private metadata when this canister is a controller of the
  target was not examined (needs a controlled target).

## Appendix: probe source (private copy, `src/l1_audit_tests.rs`, added as `#[cfg(test)] mod`)

Run: `cargo test --lib l1_audit` with `CARGO_TARGET_DIR` shared; 7 passed. The shared target
directory is reused by other agents with the same package hash, so the test binary was
copied out right after building before it was run.

```rust
const X: &str = "ryjl3-tyaaa-aaaaa-aaaba-cai"; // never allowlisted in these tests
const DID: &str = "service : { secret_admin_only : (text) -> (); icrc1_balance_of : (record { owner : principal }) -> (nat) query }";
fn bridge() -> IcBridge { IcBridge::new("c".into(), "javascript", "s".into()) }

#[test]
fn override_is_served_to_scripts_without_allowlist() {
    let p = Principal::from_text(X).unwrap();
    policy::set_candid_override(p, Some(DID.into()));
    assert!(policy::list_allowed().is_empty());
    let b = bridge();
    assert_eq!(block_on(b.candid(X)), Ok(DID.to_string()));
    assert_eq!(block_on(b.candid(&X.to_uppercase())), Ok(DID.to_string()));
    let e = block_on(b.call(X, "icrc1_balance_of", Value::Null)).unwrap_err();
    assert!(e.contains("not allowlisted"), "{e}");
    assert_eq!(b.violations.borrow().len(), 1); // the call only; the read records nothing
    // JS `return await icp.candid(X)`, Python `icp.candid(X)`, Rhai `icp::candid(X)` through
    // `Fwd(IcBridge)` (IcBridge with principal() stubbed): all return DID.
}

#[test]
fn override_survives_remove_allowlist() {
    let p = Principal::from_text(X).unwrap();
    policy::set_allowed(p, vec!["icrc1_balance_of".into()]);
    policy::set_candid_override(p, Some(DID.into()));
    evict_interface(&p);               // what remove_allowlist does
    assert!(policy::remove_allowed(p));
    assert_eq!(block_on(bridge().candid(X)), Ok(DID.to_string()));
}

#[test]
fn override_is_used_before_any_fetch_even_when_allowlisted() {
    let p = Principal::from_text(X).unwrap();
    policy::set_allowed(p, vec!["*".into()]);
    policy::set_candid_override(p, Some(DID.into()));
    assert_eq!(block_on(bridge().candid(X)), Ok(DID.to_string())); // no send attempted
}

#[test]
fn bridge_canonicalises_before_allowlist_on_call_path() {
    // uppercase id passes the allowlist and panics in ic0 `cost_call` (the send);
    // "ICRC1_BALANCE_OF" -> not allowlisted; " id", id without dashes, "id-", "aaaaa-aa-"
    // -> "invalid canister id".
}

#[test]
fn any_principal_can_be_allowlisted_including_management_and_self_like_ids() {
    // set_allowed(aaaaa-aa, ["*"]): check_call ok for update_settings, delete_canister;
    // set_allowed(other, ["*"]): check_call ok for http_request_update, set_allowlist.
}
```
