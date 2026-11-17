# Grid: L13 Upgrade and compatibility, round 1

Run `2026-11-17-295d64b`. Question: does old data, an old client or a half-finished operation
survive a new version?

Evidence: **R-native** = reproduced with probe tests added to a private copy of the tree
(`cargo test --lib l13_`, module `l13_probe` appended to `src/lib.rs`, calling the production
`apply`, `policy::set_limits`, `policy::limits`, `Storable` impls, and
`candid_parser::utils::service_compatible` against every committed `icp_code_mode.did`
(`git cat-file -p <c>:icp_code_mode.did` for 9afe403, f5305be, d508ece, 5d856e3, 13abe24,
cdfa3d9)). **code** = the code path produces it unambiguously. No live deployment was
touched (not the Live phase).

History used: stored types at every commit (`git cat-file -p <c>:src/policy.rs`):
`Limits` gained `call_timeout_seconds`, `certified_reads` (opt) in d508ece; `AuditStatus` was
`{Started, Ok, Error, PolicyViolation}` at 9afe403, gained `Call` in d508ece and `Trapped` in
cdfa3d9; `AuditEntry` gained `run_id` (opt) in f5305be and `methods` (opt) in d508ece.
`AllowedMethods`, tokens and overrides never changed shape. `ic-stable-structures` 0.7.2,
`candid` 0.10.40 and `ic-cdk` 0.20.3 are the same at every commit (Cargo.lock history), so the
stable layout never changed.

Candidate findings L13-1..L13-3 and the #51 re-check are described at the end.

## Rows

| Row | Verdict | Why |
|---|---|---|
| C1 | finding | L13-1, L13-2, L13-3; register #51 still open |
| C1a | holds | Feature subsets share `policy.rs`, `lib.rs` and the `.did`; stable layout is feature-independent (memory ids 0-5 are unconditional). Swapping a worker's feature set on upgrade keeps D1-D5 |
| C2 | holds (by design) | demo_store: heap-only state, no `post_upgrade`; `demo_store/src/lib.rs:5` says "reseeded on every install/upgrade". See D8 |
| C3, C4 | not applicable | Shim and vendored chrono hold no state and no interface |
| C7 | holds | `init_args: "(null)"` decodes to `None` on install and upgrade, so `apply` is a no-op. icp-cli 1.7.0 stops the canister before an upgrade (binary strings `Failed to stop canister '…' before upgrade` / `Failed to start canister '…' after upgrade`) and runs a Candid compatibility check (`--yes` skips it). Not determined: whether icp-cli re-sends `init_args` on upgrade when no `upgrade_args` is set (it has both keys). Relevant to L13-2 |
| C8, C9, C10 | not applicable | Build flags; no state or interface evolution |
| C11 | finding | Register #51 re-check (R-native): `service_compatible(current, old)` fails on `get_audit_logs` for all five committed `.did`s before cdfa3d9; only cdfa3d9 is compatible. `demo_store.did`: no stored state, interface consumed via fetch (see B6 note) |
| C12, C13, C14 | not applicable | Not deployed, no stored state |
| C15 | not applicable | Not messages |
| A5 | finding | Operator is the actor of L13-1 (old-shape `set_limits` / upgrade args) and L13-2 (re-applied upgrade args) |
| A3, A4 | holds | MCP clients see no upgrade-sensitive interface (JSON-RPC over HTTP; L12) |
| A6 | holds (documented) | A target upgraded with a new interface is served from the stale cached interface for up to 1 h (V2, CL22). Candid typing at the target rejects incompatible args; replies decode with Candid subtyping. Documented TTL |
| E1 | holds | `init` traps on a malformed token hash (F1): install rolls back entirely. Empty arg bytes become `DIDL\0\0` in ic-cdk 0.20.3 (`ic-cdk-macros/src/export.rs:257`) and decode to `None` (R-native `l13_empty_upgrade_args`) |
| E2 | finding | Applies the same `apply` as `init` on top of existing state: L13-2 (revoked tokens re-registered, allowlist narrowing undone, labels overwritten) and L13-1 (`limits` replaced whole). Trap on bad hash rolls the whole upgrade back: holds. `certified_data_set` repeated: holds (D6) |
| E3 | holds | Decodes `HttpUpdateRequest` (unchanged since 9afe403); no stored state |
| E4, E4a, E4b | holds | Reads `limits()` (D1) on every query; all stored `Limits` shapes since 9afe403 decode (T6 + R-native round trip). Answers from heap caches that are empty after upgrade, so the first search after an upgrade upgrades to an update (expected) |
| E5, E5a, E5b | holds | Upgrade-relevant parts are K11 and V1-V5 below |
| E5c, E5d | holds (documented) | Callbacks of an in-flight run do not survive an upgrade without a stop: the IC deletes callbacks on upgrade, so neither the reply continuation nor the cleanup (`Trapped`) runs. Started/Call entries stay without a closing entry; README.md:419-421 calls this "unknown". With icp-cli's stop-before-upgrade they run to completion first (see K11) |
| E6, E7, E9, E10, E11 | holds | Argument types unchanged since 9afe403 |
| E8 | finding | L13-1: `set_limits(Limits)` replaces the whole record; a caller written against the pre-d508ece `.did`, or one that omits the "optional" fields as README.md:160-161 allows, resets `call_timeout_seconds` and `certified_reads` to null (R-native) |
| E12 | holds | `PolicyView` only gained output fields (`pending_run_ids`, Limits opts): old readers skip unknown record fields |
| E13 | finding | Register #51 (still open), and L13-3 on rollback |
| E14 | holds | Unchanged |
| E15 | not applicable | Not messages |
| E16-E19, E26-E28 | not applicable | Script SDK / tooling: no upgrade-time state |
| E20-E24 | holds (by design) | demo_store interface; state is re-seeded (E25) |
| E25 | holds (by design) | No init/post_upgrade; documented in the crate header |
| D1 | holds | `Limits` decodes from the 9afe403 shape (T6 `old_audit_entries_still_decode`) and round-trips at edge values (R-native `l13_storable_roundtrip`, 2,000 cases). Read on every request via `expect` (F3); no shape in history fails. Write path is L13-1 |
| D2 | holds | `AllowedMethods(Vec<String>)` unchanged since 9afe403; `Principal` key uses the library's bounded Storable (same 0.7.2 at every commit) |
| D3 | finding (L13-2) | Survives upgrades (holds), but a revoked hash is re-inserted by `apply` on upgrade with the install args, and an existing label is overwritten with `"init"` (`src/lib.rs:41`, `policy.rs:213` `insert` replaces) |
| D4 | holds for forward reads; finding for rollback (L13-3) | Every historical `AuditEntry` shape decodes with the current type (new fields are opt; variants only added). An older Wasm reading entries written by the current one traps on `Trapped` (and pre-d508ece on `Call`) |
| D5 | holds | Stored as text; parsed lazily into V5 after an upgrade. A stored override that a newer `candid_parser` rejects yields an error per run (`override_service` returns `Err`, `cached_service` returns `None`), not a trap |
| D6 | holds | Reset by the IC on upgrade, re-set first thing in `post_upgrade` (`src/lib.rs:70`) |
| D7 | not applicable | Platform settings, unchanged by upgrade |
| D8 | holds (by design) | demo_store `STORE` re-seeded on every install/upgrade; orders placed through scripts vanish while the icp_code_mode audit log still lists the `create_order` calls. Demo data, documented in the crate (`demo_store/src/lib.rs:5`), not in README |
| D9, D10 | not applicable | Browser / disk, not canister state |
| V1 | holds | `ACTIVE_RUNS` resets to 0. Correct: after a stopped upgrade no run is in flight; after an unstopped one the old runs' futures and callbacks are gone, so no slot can be returned twice or leak |
| V2 | holds (documented) | `DID_CACHE` empty after upgrade; re-fetched on demand (README.md:468); first fetches count against the caller's cap as any miss does |
| V3 | holds (documented) | `PENDING_RUNS` empty after upgrade; README.md:419-421 and `.did` say runs then read as "unknown". The `.did` comment on `pending_run_ids` ("still waiting on a reply") stays true: after an upgrade no code waits |
| V4 | holds | `DID_FAILED` empty after upgrade: failures are retried once more, bounded by the call cap |
| V5 | holds | Rebuilt from D5 on first use |
| V6 | holds | `LIVE_MEMORY` / `BASELINE_MEMORY` restart at 0 with a fresh heap |
| V7 | not applicable (L13) | Seed stream restarts at the constant after each upgrade; a determinism question (I7), not compatibility |
| V8, V9, V10 | holds | Heap objects; an upgrade discards them together with the runs that used them |
| V11 | see D8 | |
| K1-K10, K12 | not applicable | Commit points within one version; L2 |
| K11 | holds (documented) | `icp deploy` stops the canister first, so in-flight runs either finish (and close normally) or the stop hangs on a target that never answers with the default unbounded wait (register #44, opt-in fix). An upgrade without a stop leaves Started/Call entries with no closing entry, no `Trapped`, and nothing in `pending_run_ids`; README.md:419-421 documents this as "unknown". Not reproduced live (not the Live phase) |
| LIM1-LIM32 | not applicable | Constants in code, not stored; a new version simply changes them. `set_limits` storage of LIM1-LIM4 is E8/D1 |
| W1-W16 | not applicable | Work per request, no version dependence |
| F1 | holds | Trap rolls back install/upgrade atomically; `apply` runs after `certified_data_set`, both rolled back together |
| F2 | holds | Encoding a well-formed Rust value with candid 0.10.40 cannot fail for these types (R-native round trip) |
| F3 | holds (current data); finding for rollback (L13-3) | No stored shape in history fails to decode with the current types. The `expect` turns any future non-additive change, or a rollback, into a trap on every read of that record (D1 on every request, D4 on every audit page) |
| F4 | not applicable | Stable memory exhaustion, not version-dependent (L3/L18) |
| F5-F16 | not applicable | Not upgrade-related |
| B2 | holds | `HttpUpdateRequest` unchanged; inspected and decoded the same way in both places |
| B6 | holds (documented) | Replies decoded with a cached interface up to 1 h old after a target's upgrade (A6) |
| B12 | finding (L13-3); holds forward | See D1, D4, F3 |
| B1, B3-B5, B7-B11, B13-B17 | not applicable | No stored or versioned form |
| VF1-VF8 | not applicable | No upgrade-specific value flow |
| AD1 | finding | L13-2 (re-applied on upgrade), L13-1 (`limits` inside it) |
| AD2, AD3, AD5-AD7 | holds | Admin calls are single executions; nothing version-dependent besides D2/D3/D5 above |
| AD4 | finding | L13-1 |
| AD8 | holds | Reinstall wipes D1-D5 (platform); README does not promise otherwise beyond "survive upgrades". CL4 "immutable" vs reinstall is an L16 question |
| AD9 | holds (documented) | README.md:471-472: Candid-breaking upgrades needed `icp deploy --yes`; the Quickstart does not mention it (noted in baseline) |
| S1-S10 | holds | Stable-structure, candid and ic-cdk versions identical across the whole history; no dependency change has touched the stored format |
| CL4, CL5 | holds | "survive upgrades": D1-D5 do (stable structures, additive shapes). CL4 "immutable" is L16 |
| CL20 | holds | Upgrade case documented as "unknown" |
| CL24 | finding (#51 re-check) | The README limitation names `Started`, `Call` and `Trapped`; in committed history `Started` is present from the first commit (9afe403), so the committed breaks are `Call` (d508ece) and `Trapped` (cdfa3d9). Wording, not behaviour |
| CL31 | holds | `pending_run_ids` comment remains true after an upgrade (see V3) |
| Other CL rows | not applicable | Not about upgrades or versions |
| T2 | holds | `.did` equals `export_candid!` |
| T6 | holds, partial | `old_audit_entries_still_decode` covers the 9afe403 `AuditEntry` and `Limits` shapes; no test for downgrade reads, for a whole-record `set_limits` from an older client, or for `post_upgrade(Some(args))` on populated state |
| I9 | violated | #51 (open), L13-1, L13-2, L13-3 |
| Register #51 | still open | Re-checked, R-native (see below) |

## Candidate findings

### L13-1 Medium: an old-shape or terse `set_limits` silently turns the two newer options off

`Limits` gained `call_timeout_seconds : opt nat32` and `certified_reads : opt bool` in d508ece.
`set_limits` (`src/lib.rs:307`) and `InitArgs.limits` (`apply`, `src/lib.rs:46`) replace the
whole record. Candid decodes a missing `opt` field as `null`, so any caller written against the
pre-d508ece interface, or an operator following README.md:160-161 ("the last two are
optional"), who only means to change `max_calls_per_script` resets `certified_reads` to null
(certified mode off: queries uncertified again and the demo page back on) and
`call_timeout_seconds` to null (unbounded waits back on, register #44). No error, no warning.

Evidence (R-native, `l13_set_limits_old_shape_clears_new_settings`): with
`Limits { call_timeout_seconds: Some(60), certified_reads: Some(true), .. }` stored, decoding
`record { max_calls_per_script = 50; max_instructions = 20_000_000_000 }` the way the generated
entry point does and calling `policy::set_limits` leaves
`Limits { max_calls_per_script: 50, max_instructions: 20000000000, call_timeout_seconds: None, certified_reads: None }`.
Same through `apply(Some(InitArgs { limits: Some(<old shape>) }))`.

Refutation tried: a caller could re-read `get_policy` first; nothing in the README says so, and
it calls the fields optional. The `.did` is accurate about the types, but `opt` here means
"off", not "unchanged".

Fix: make the update partial (every field `opt`, `null` = keep), or give the options their own
setters; or refuse a `set_limits` that omits them when they are currently set.

### L13-2 Medium: upgrading with the install arguments resurrects revoked tokens and undoes allowlist changes

`post_upgrade` runs the same additive `apply` as `init` (`src/lib.rs:38-49, 68-72`). With the
args used at install, an upgrade re-registers every listed token hash (including ones revoked
since, with label overwritten to `"init"`), replaces each listed canister's method list with
the install-time one (undoing a narrowing, re-adding a removed canister), and replaces the
limits (L13-1). README.md:379 presents this as a feature ("`post_upgrade` accepts the same
optional `InitArgs` as `init`") without a warning.

Evidence (R-native, `l13_upgrade_args_reapply`, production `apply`): after install args
`{token h, ledger -> [icrc1_transfer]}`, then `add_token_hash(h, "my-laptop")`,
`revoke_token_hash(h)` and `set_allowlist(ledger, [icrc1_balance_of])`, a second
`apply(<install args>)` leaves `verify_token` succeeding for the revoked token, its label
`"init"`, and the ledger allowlisted for `icrc1_transfer` again.

Precondition: the operator upgrades with non-null args. The repo's `icp.yaml` uses `(null)`;
whether icp-cli re-sends `init_args` on upgrade when an operator fills them in (it also has an
`upgrade_args` key) was not determined.

Fix: give `post_upgrade` its own argument type (or ignore tokens and allowlist there), never
re-add a hash that was revoked, and document that upgrade args are applied on top of the
current state.

### L13-3 Low: rolling back to an earlier release makes the audit log unreadable

Stored values are decoded with `Decode!(..).expect("candid decode")` (`src/policy.rs:110`).
`AuditStatus` is a closed Candid variant, so a Wasm built before cdfa3d9 traps on any stored
`Trapped` entry, and one built before d508ece on any `Call` entry. After a rollback (for
example to revert a bad release), every `get_audit_logs` page that contains such an entry
traps. The data stays in stable memory and reads again after re-upgrading.

Evidence (R-native, `l13_old_readers_and_rollback`): bytes from the current
`Storable::to_bytes` of a `Trapped` entry, decoded with the 13abe24 type, panic with
`Subtyping error: Unknown variant field 1_576_376_546` (a trap on chain); `Ok`, `Started`,
`Call` decode.

Same root as #51 (a closed variant used for an evolving set of statuses), at the storage
layer instead of the interface. The verifier may merge.

Fix: store and expose the status in a form that tolerates unknown values (a text tag, or a
catch-all decoded with `opt` semantics), and decode stored records without `expect` on the
read path.

### Register #51 re-check: still open, description slightly off

R-native (`l13_old_did_compat`): `service_compatible(current, old)` reports
`Method get_audit_logs: func (nat64, nat64) -> (vec AuditEntry) query is not a subtype of …`
for 9afe403, f5305be, d508ece, 5d856e3 and 13abe24; cdfa3d9 is compatible. Decoding a reply
page with an older reader type fails for the whole page as soon as one entry has a status
the reader lacks (pre-d508ece readers: `Call` or `Trapped`; 13abe24 readers: `Trapped`), so old
readers lose every entry on that page, not just the new ones. The next status added breaks
readers again: nothing in the interface is forward-compatible. Wording: the register says
three breaks and README.md:471 names `Started`, but `Started` is in the first commit, so git
history shows two (`Call`, `Trapped`).

## Not tested

- Upgrade with runs in flight on a real replica (K11, E5c/E5d), with and without a stop: not
  the Live phase. Needs a misbehaving target that replies late (register #55) and an upgrade
  during the run.
- Whether `icp deploy` re-sends `init_args` on upgrade (L13-2 precondition), and how icp-cli
  encodes an omitted `opt` field in `icp canister call ... set_limits` text (L13-1 assumes the
  Candid rule, which the native decode follows): needs a live call that changes settings.
- Old `@dfinity/candid` (JS agent) readers: only the Rust decoder and `candid_parser`
  subtyping were exercised; the Candid spec gives the same result.
