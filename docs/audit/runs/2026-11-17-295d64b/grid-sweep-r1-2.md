# Variant sweep r1-2

Pattern: *a configuration surface that accepts identifiers (self, platform services) that turn
an ordinary feature into a privileged or budget-multiplying one, without refusal or warning.*

Source finding (known, not re-reported): `set_allowlist` accepts this canister's own principal
and `aaaaa-aa` (L1-3 / L14-5). Method: every inventory row was asked "does this row accept an
identifier (principal, method name, path, language, tool name) from configuration or input, and
can some value of it reach a privileged or budget-multiplying effect without refusal or warning?"
Evidence is code reading at `295d64b` (CP); nothing was compiled or run, nothing live was touched.

Result: **no new candidate.** Every hit is either the known root cause reached by another
door, a known finding, or holds.

## Cells

| Row(s) | Identifier accepted | Verdict | Reason |
|---|---|---|---|
| AD1 / E1 / E2 `InitArgs.allowlist` | principal + method list at install/upgrade | known root cause | `apply` (`src/lib.rs:43`) calls the same `policy::set_allowed` as `set_allowlist` (`src/policy.rs:155-157`); one root cause, one finding (L1-3). The fix must sit in `set_allowed` (or a shared validator) so both doors are closed |
| AD2 / E6 `set_allowlist` | principal, method names, `"*"` | known (L1-3, L14-5) | source finding |
| AD3 / E7 `remove_allowlist` | principal | holds | only removes; canonical `Principal` from Candid |
| AD7 / E11 `set_candid_override` | any principal incl. `aaaaa-aa` and self | known | "Candid overrides are not tied to the allowlist: accepted for any principal"; an override for `aaaaa-aa` is the enabler that L14-5 already names (without it the `aaaaa-aa` interface fetch fails: `canister_metadata(aaaaa-aa)` and `aaaaa-aa.__get_candid_interface_tmp_hack` both reject) |
| AD4 / E8 `set_limits` | no identifiers | n/a | numeric values; unvalidated ranges are known findings |
| AD5 / AD6 / E9 / E10 tokens | hash + label | n/a | not identifiers of a target; empty-token hash is a known finding |
| AD8 canister settings (controllers) | principals as controllers | holds (repo) | if an operator made the canister its own controller, a self-allowlisted script could reach admin methods; that chain needs L1-3 first, and nothing in the repo or docs sets it (R-live in L1 grid: controllers = one user principal) |
| AD9 build choices / C1a per-language workers | feature set | holds | README:112-120 and `icp.yaml` comments describe standalone workers; there is no router that allowlists workers, so no budget-multiplying hop is configured or recommended |
| C6 / E27 `scripts/demo-setup.sh` | demo_store id from `icp canister status demo_store` | holds | allowlists only demo_store's 7 reads + `create_order`; cannot resolve to self or `aaaaa-aa`. `create_order` taking any customer is the known demo_store finding |
| C7 `icp.yaml` | `init_args: "(null)"` | holds | no allowlist entries at install |
| C14 / A12 per-customer doc | factory as co-controller | n/a | not built; the doc makes the customer a controller of an instance, not the instance a controller or target of itself |
| E5b / U5 search `canister_id` | principal text | holds | `search::scope` (`src/search.rs:38-47`) only filters the allowlist; an id not allowlisted yields no entries and no fetch |
| E16-E19 / U7 script `icp.call` / `icp.candid` ids and methods | principal text, method name | holds / known | `call` runs `reserve` -> `check_call` on the canonical principal before any send (`src/bridge.rs:510-512`); `icp.candid` serving overrides without an allowlist check is known |
| O2 / A7 interface fetch to `aaaaa-aa` | target principal (allowlisted) | holds (by design) | fixed method `canister_metadata`, fixed arg `{canister_id: p, name: "candid:service"}` (`src/bridge.rs:424-437`); the target must be allowlisted. Unverified side note below |
| E3 `inspect_message` | method name | holds | `http_request_update` needs a token; every other method needs a controller (`src/lib.rs:273-283`) |
| E4/E5 routes, tool names, `language` (B13) | path, tool name, language alias | n/a | no value maps to a privileged path; unknown names are refused |
| E20-E23 demo_store | `customer` principal | known | `create_order(customer, ...)` for any customer is the known demo_store finding; no principal in demo_store grants privilege (`demo_store/src/lib.rs` has no caller or admin check at all) |
| VF4 assets held by this principal via allowlisted methods (e.g. `icrc1_transfer`) | target + method | accepted (register #49) | one on-chain identity for all token holders; README:321 shows the ledger example with `icrc1_transfer` refused |
| E15, D1-D10, V1-V11, K1-K12, LIM*, W*, F*, B1-B12, B14-B17, S*, CL*, T* | none | n/a | these rows do not accept a target identifier from configuration; CL6/CL24 claims about the allowlist are covered by the known README findings |

## Unverified notes (not candidates; for the fix of L1-3 and for the Live phase)

1. **Subnet id as a management alias.** If the fix for L1-3 blocks `aaaaa-aa` by value, it
   should also consider whether this subnet's id is accepted by the replica as a receiver for
   management messages. Not tested: it needs a live call with the canister's authority, which
   this phase may not make.
2. **Targets this canister controls.** `canister_metadata` may return a *private*
   `candid:service` section when the caller is a controller of the target, and every call to
   such a target carries controller authority. No canister in the repo is controlled by
   `icp_code_mode`, so this is the same "one identity" accepted limit (#49) plus L1-3's
   warning gap, not a separate code path.
3. **Proxy targets** (wallet-style `call(canister, method, args)` methods) would bypass the
   per-method allowlist the same way `aaaaa-aa` does. No such target exists in the repo;
   this belongs in the README warning that L1-3's fix should add, not in a new finding.
