# Coverage grid: L18 Operations, round 1

Run `2026-11-17-295d64b`. Lens question: can this be run safely for a year? (running out of
funds or storage without warning, no way to observe failures, logs that grow without bound,
no recovery path). Rows are from `inventory.md`; `file:line` is commit `295d64b`.

Evidence keys: **R** reproduced (native scratch tests in a private copy built with
`CARGO_TARGET_DIR=/private/tmp/claude-501/audit-target`, copy deleted afterwards; or
read-only `icp canister status` / `logs` / controller queries against the local network on
port 8010, nothing deployed or changed), **C** the code path produces it unambiguously.
Candidate findings are L18-1 ... L18-9 (end of this file).

## Measurements used below

| What | Value | How |
|---|---|---|
| Audit entry size, `Started`/`Call` (one method) | 332 B Candid | R, `Encode!` of a realistic entry |
| Closing entry with 25 methods | 1,340 B | R |
| Largest entry (25 x 256-byte method names, 17 violations x 512 B, 4 KiB error) | 20,276 B | R |
| Stored size per message entry | 332 B/entry (200,000 entries = 66.4 MB `log_size_bytes`) | R, native `StableLog` scale test |
| Paging at 200,000 entries | first and last pages correct (`audit_page(n-1,100)` returns entry 0) | R |
| Stable memory ceiling | `MemoryManager` 32,768 buckets x 128 pages = 256 GiB for all six memories together; ic-stable-structures 0.7.2 has no API to release buckets, `StableLog` has no truncate/remove | C (`memory_manager.rs:59-61, 348`; `log.rs` public API) |
| Live `icp_code_mode` | memory_size 1,246,514,630 B; idle burn 32,710,006,708 cycles/day; cycles 19.06T; freezing threshold 2,592,000 s; reserved cycles limit 5T; wasm_memory_threshold 0; log_memory_limit 4,096 B | R, `icp canister status --json` |
| Live `demo_store` | memory_size 9,103,596 B; idle burn 1,096,579,042 cycles/day | R |
| Local storage rate (fit of the two canisters) | about 25.5 cycles/byte/day (~317k cycles/GiB/s) + ~0.86B cycles/day per canister | R + arithmetic; mainnet 13-node published rate is 127k cycles/GiB/s |
| Live audit log | 839 entries; JS runs median 19-20M instructions; 7 runs between 20.0B and 29.3B instructions; 6 runs (ids 465, 469, 470, 471, 488, 654) have `Started` and no closing entry while `pending_run_ids = vec {}` | R, `get_audit_logs` pages + `get_policy` |
| Live canister log | 8 lines, all `[TRAP]: heap out of bounds`; older lines rotated out of the 4 KiB buffer | R, `icp canister logs` |

## Cells

| Row | Verdict | Reason / evidence |
|---|---|---|
| C1 `icp_code_mode` canister | **finding L18-1, L18-2** | Runs indefinitely only while funded and while its stable memory can grow; neither is observable or recoverable from inside the app (see D4, D7) |
| C1a feature subsets | not applicable | Operations do not differ by engine set; size/idle-cost aspect is under C14 |
| C2 `demo_store` | holds | Orders grow without bound (W15) but state is heap-only and re-seeded on every upgrade (E25), so `icp deploy` is a recovery path; it is a demo target, not run for a year |
| C5 demo page | not applicable | Static page; no state or cost of its own |
| C6 `scripts/demo-setup.sh` | **finding L18-7** | `sed -n 's/^Canister Id: *//p'` over human-readable status text. Differential test (R): `icp canister status demo_store -e local --id-only` and the sed pipeline both yield `4fbx2-kt777-77775-aaabq-cai\n` today, so it works with icp-cli 1.7.0; icp-cli is not pinned (S7), and a format change makes the script exit with the wrong reason ("demo_store is not deployed") |
| C7 `icp.yaml` | holds (with L18-2) | Build steps fail loudly (`set -e` semantics, `--locked`). No canister settings are declared (freezing threshold, `wasm_memory_threshold`, log memory, reserved-cycles limit stay platform defaults); that is part of L18-2, not a separate defect |
| C9 toolchain pin | holds | `rust-toolchain.toml` pins 1.100.0, so a rebuild for recovery uses the same compiler |
| C12 video, C13 audit tooling | not applicable | Not deployed, not operated |
| C14 `docs/per-customer-canisters.md` | **finding L18-8** | Idle and per-run cost figures do not match measurements (see CL37) |
| A5 controller (operator) | **finding L18-2, L18-3, L18-5** | The operator's only views are `get_policy`, `get_audit_logs`, `get_audit_log_len`, the platform's `canister status` and a 4 KiB canister log; no cycles/memory figure, failure counter or runbook in the repo |
| A3 token holders | holds for this lens | Their cost to the operator (no rate limit) is register #31 (open); not repeated here |
| A9 infrastructure | not applicable | Trust lens (L10) |
| E1/E2 `init`/`post_upgrade` | holds; see L18-4 | Install/upgrade is atomic: a bad token hash traps (F1) and the IC keeps the old module and state, so a failed upgrade is recoverable by fixing the argument (C, `src/lib.rs:38-49`). A future non-optional schema change (F3) makes every read of `LIMITS` trap, including `set_limits` and `get_policy`; `post_upgrade` does not touch `LIMITS` unless `limits` is passed, so a second upgrade with fixed code still works: recovery path exists (C). No snapshot step is documented before upgrades (part of L18-2). Upgrades leave no mark in the audit log: L18-4 |
| E3 `inspect_message` | not applicable | Cost filter; no operational state |
| E5 `http_request_update` | **finding L18-2** | A trap before the first send leaves no entry (register #11, accepted); after that the only trace is the 4 KiB canister log (R: 8 lines kept, all `heap out of bounds`, no run id or token prefix). No counter of traps, busy refusals or policy refusals exists, so a degrading service is not visible to the operator |
| E5d trap cleanup | holds | `Trapped` entry written in cleanup (`src/bridge.rs:243-267`); 1 present in the live log (R) |
| E6/E7/E9/E10/E11 admin writes | holds | Each is a single replicated execution; a wrong value is undone by calling the same endpoint again (allowlist, tokens, override `null`) |
| E8 `set_limits` | **finding L18-3** | Whole-record replace; omitted options reset to `null` (R) |
| E12 `get_policy` | **finding L18-4**; `active_runs` holds | `pending_run_ids` lives in heap (`src/bridge.rs:198`) and is empty after an upgrade (documented, README.md:418-421). Live: 6 runs with `Started` and no closing entry and an empty pending list (R): the operator cannot tell "cut by an upgrade" from "cleanup trapped". Searches are not in `active_runs` (L4-4, not repeated) |
| E13/E14 audit reads | **finding L18-5**; boundaries hold | Paging is by offset from the newest entry, so pages shift while the log grows (R: 10 appends between two page reads repeat 10 entries). No cursor by id and no filter (run, status, caller). Limit 100 and far-end paging hold (R at 200,000 entries; `policy::audit_log_pages_newest_first`) |
| D1 `LIMITS` | **finding L18-3** | See E8 |
| D2/D3/D5 allowlist, tokens, overrides | holds | Written only by controllers; size is operator-chosen; removable |
| D4 audit log | **finding L18-1** | No retention: never pruned, no export, no reclaim (C: no endpoint; `StableLog` has no remove; `MemoryManager` 0.7.2 cannot release buckets). Growth: 332 B per message entry, ~1.3 KB per closing entry (R); one 25-call run ~9.6 KB, so one such run per minute is ~5 GB/year. Who can grow it is register #31 (not repeated) |
| D6 certified data | holds | Re-set in `post_upgrade` |
| D7 canister settings / cycles | **finding L18-2** | No cycles or memory figure in `get_policy`, `wasm_memory_threshold` 0 (no low-memory hook), no README section on cycles, top-ups, freezing or monitoring (R: grep of README finds none). The idea doc itself lists "Running dry: how does a customer learn ... before it freezes?" as open |
| D8 demo_store state | holds | See C2 |
| D9/D10 browser token, `.env` | not applicable | Not operated state |
| V1 `ACTIVE_RUNS` | holds; see L18-9 | Reset on upgrade; a run stuck in an unbounded wait holds its slot (L4-5, not repeated) |
| V2/V4/V5 interface caches | holds | Bounded by allowlist size, emptied on upgrade, refetched on demand |
| V3 `PENDING_RUNS` | **finding L18-4** | See E12 |
| V6-V10 heap state (engines, executor) | **finding L18-8** (cost aspect) | Wasm memory never shrinks: live memory_size 1.25 GB against an 11.4 MB module and at most ~48 MiB of stable buckets for this small log (C: six memories x 8 MiB buckets; 839 entries), so the idle bill is set by the heap high-water mark of past runs and drops only on an upgrade (R/C) |
| K6/K7 closing entry / cleanup | **finding L18-1** (F4 path) | If `append_audit` cannot grow memory it traps (`expect`, `src/policy.rs:252`) in K6, and again in K7 |
| K11 upgrade while runs wait | **finding L18-4** | See E12 |
| LIM3 `call_timeout_seconds` | **finding L18-9**; base issue is register #44 | The timeout is read once per run (`src/bridge.rs:323`), so setting it after a call is already waiting does not release that call (C); no documented way out of a canister stuck in Stopping |
| LIM5 `MAX_AUDIT_PAGE` | holds; see L18-5 | 100 per page; a year at the rate above is millions of entries, i.e. tens of thousands of page queries with no cursor |
| LIM29 Wasm memory limit 3 GiB | **finding L18-2** | Default limit, no `wasm_memory_threshold`; the live canister's log shows only `heap out of bounds` traps with no other signal |
| LIM30 IC platform limits | not applicable | Platform |
| W14 audit/pending growth | **finding L18-1** | See D4 |
| W15 demo_store orders | holds | See C2 |
| F3 decode `expect` | holds (recovery) | See E1/E2 |
| F4 audit append `expect` | **finding L18-1** | R: a `StableLog` over memory that refuses to grow returns `GrowFailed` after 445 appends at 2 pages, and `.expect("audit log out of stable memory")` panics. On-chain this traps every audited request; no endpoint can free space |
| F13 out of memory | **finding L18-2** (observability) | Memory traps are visible only as unlabelled `heap out of bounds` lines in a 4 KiB rotating log (R) |
| F14 instruction trap | not applicable | Resource lens (register #23) |
| VF1 cycles paid by the canister | **finding L18-2** | Nothing reports spend; per run up to the 20B budget and beyond (R: 7 runs of 20.0-29.3B instructions in the live log), ~1,000x the median run |
| VF5 work multiplied over time | holds for this lens | Register #31 |
| VF7 factory fees | **finding L18-8** | Cost basis in the doc is off (C14) |
| AD4 `set_limits` ranges | **finding L18-3** | Recovery from a bad value holds: every admin endpoint works whatever `Limits` holds (`max_instructions` 0, `max_calls_per_script` 0, `certified_reads` true are all undone by another `set_limits`, C). The defect is the silent reset of omitted options |
| AD8 canister settings, reinstall | **finding L18-2** | Reinstall is the only way to reclaim audit-log memory and wipes D1-D5; snapshots exist in icp-cli (`icp canister snapshot`, none taken on the live canister, R) but are not mentioned in the README |
| AD9 `icp deploy --yes` | holds | Documented in README.md:471-472 for Candid-breaking upgrades |
| S7 icp-cli/ic-wasm unpinned | **finding L18-7** (script side) | Reproducibility side belongs to L15 |
| T tests | **finding L18-6** | No CI file of any kind (`git ls-files` finds only `.claude/workflows/audit.js`, an audit runner); the 59 tests, the `.did` match test, clippy, the feature-subset builds and the `ic0`-only import check run only when someone runs them. Register guards named as tests guard nothing unless run. Related: register #54 (no integration tests) |
| CL4 "immutable audit log" | **finding L18-2** | A canister that runs out of cycles is frozen and later uninstalled, losing the log; the README says nothing about this (C, platform behaviour) |
| CL5 survives upgrades | holds | Stable structures; `policy::old_audit_entries_still_decode` |
| CL20 / CL31 pending vs unknown | **finding L18-4** | The claim is accurate, but "unknown" covers two different causes the operator cannot separate |
| CL21 "read the audit log with an update call" | holds | Query methods can be called as updates |
| CL32 audit paging | holds; see L18-5 | Newest first, capped at 100 (R) |
| CL37 cost figures | **finding L18-8** | See C14 |
| History #31 | related | Not repeated: #31 is who can grow the log (no rate limit); L18-1 is that nothing can shrink it or recover when it can no longer grow |
| History #44 | related | Not repeated; L18-9 adds the missing recovery path |

## Candidate findings

### L18-1 Audit log has no retention or reclaim path; when it cannot grow, every audited request traps (Medium)

- Invariant: I4 (bounded storage), I5 (clean failure), I9 (durability over time).
- Where: `src/policy.rs:128-129, 247-254`; no admin endpoint in `src/lib.rs:295-355`; ic-stable-structures 0.7.2 `memory_manager.rs:59-61`, `log.rs` API.
- Evidence: R (native). 332 B per message entry, 1,340 B per 25-method closing entry, up to 20,276 B; 200,000 entries = 66.4 MB. `StableLog::append` returns `GrowFailed` when memory cannot grow and `append_audit`'s `expect` panics. C: no prune/export endpoint; `StableLog` cannot remove entries and `MemoryManager` cannot release buckets, so even a code upgrade cannot free the space without migrating to new memories; reinstall wipes policy and tokens too.
- Scenario: normal use of one 25-call run per minute stores ~5 GB/year, paid every second from then on and rising linearly; when growth fails (256 GiB manager cap, reserved-cycles limit 5T on a busy subnet, insufficient cycles) every script and search that would write an entry traps, including the `Started` entry at the first send, so the service stops for every request except queries.
- Fix: an archive/prune path (export pages to another canister or off-chain, then drop a prefix by rotating to a fresh `StableLog` in reserved memory ids), a configurable retention, and a clean refusal instead of a trap when the log cannot grow.

### L18-2 No signal before the canister runs out of cycles or memory, and no operations runbook (Medium)

- Invariant: I10 (honest interface: "immutable audit log"), I3 (failures observable).
- Where: `src/lib.rs:26-36` (`PolicyView` has no cycles/memory fields); canister settings (`wasm_memory_threshold` 0, `log_memory_limit` 4,096); README (no section on cycles, top-up, freezing threshold, monitoring, snapshots, log visibility).
- Evidence: R. `icp canister status` shows the settings above; README grep finds no mention of cycles/freezing/top-up/snapshot; `icp canister logs` keeps 8 lines, all `[TRAP]: heap out of bounds`, with no run or token context. The idea doc lists running dry as an open question.
- Scenario: a token holder or a busy client spends cycles at up to the per-run budget (live log: runs of 20-29B instructions); nothing in the app reports balance, burn or failures, so the first signal is a frozen canister (all MCP requests fail); left frozen, the canister is uninstalled at zero cycles and the "immutable" audit log, policy and tokens are gone.
- Fix: expose `canister_cycle_balance`, memory size and failure counters in `get_policy` (or a metrics query), set `wasm_memory_threshold` with an `on_low_wasm_memory` hook, document top-up, freezing threshold, log memory and snapshot-before-upgrade.

### L18-3 `set_limits` silently resets options the operator leaves out (Medium)

- Invariant: I10; L14 configuration safety.
- Where: `src/lib.rs:306-309`, `src/policy.rs:149-151`; README.md:160-161 calls the two options "optional".
- Evidence: R (native). With `call_timeout_seconds = opt 60` and `certified_reads = opt true` stored, decoding the README-shaped call `(record { max_calls_per_script = 50 : nat32; max_instructions = 20_000_000_000 : nat64 })` and passing it to `set_limits` leaves both options `None`. Existing test `policy::old_audit_entries_still_decode` shows the same decode.
- Scenario: an operator raises the call cap and, without being told, turns the call timeout off (back to the unbounded wait of register #44) and certified reads off (uncertified query path and token-asking demo page back on, register #47).
- Fix: make each field `opt` with "keep current" semantics, or return the previous `Limits` and document that the record is replaced in full.

### L18-4 Upgrades leave no trace in the audit log; runs cut by an upgrade cannot be told apart (Low)

- Invariant: I3 (records match reality), observability.
- Where: `src/bridge.rs:195-198, 229-235`; `src/lib.rs:68-72` (no entry written); README.md:418-421.
- Evidence: R (live, read-only). 6 runs (ids 465, 469, 470, 471, 488, 654) have `Started` and no closing entry; `get_policy` shows `pending_run_ids = vec {}`. The log cannot say whether each was cut by an upgrade or by a failed cleanup.
- Fix: keep the pending set in stable memory and, in `post_upgrade`, close each pending run with an entry naming the upgrade (as an `Error` with a fixed message, to avoid another Candid-breaking status, register #51).

### L18-5 Audit paging shifts as the log grows; no cursor or filter (Low)

- Where: `src/policy.rs:261-270`, `src/lib.rs:346-350`.
- Evidence: R (native). Reading page 0, appending 10 entries, reading page 1 repeats 10 entries. At 200,000 entries paging still works.
- Scenario: an operator or export job walking the log repeats entries (detectable by id) and cannot ask for a run, a status or "everything after id X"; a year at moderate use is tens of thousands of page queries.
- Fix: `get_audit_logs_before(id, limit)` (or after), plus filters by `run_id` and status.

### L18-6 No CI: nothing runs the tests, the interface check or the import check automatically (Low, testability)

- Evidence: R. No pipeline file in `git ls-files`; baseline confirms. Every register guard named as a test depends on a person running `cargo test`.
- Fix: a CI job running `cargo test --workspace --locked`, the feature-subset checks, clippy, a release wasm build with the `ic0`-only import check, and the `.did` match test.

### L18-7 `scripts/demo-setup.sh` parses human-readable CLI output (Low)

- Where: `scripts/demo-setup.sh:7`.
- Evidence: R. Works with icp-cli 1.7.0 (same id as `--id-only`); icp-cli is unpinned and provides `--id-only` and `--json`. On a format change the script stops with "demo_store is not deployed", the wrong diagnosis.
- Fix: `icp canister status demo_store -e "$ENV" --id-only`.

### L18-8 Cost figures in the per-customer idea doc do not match measurements (Low)

- Where: `docs/per-customer-canisters.md` ("What an instance costs the customer").
- Evidence: R (live) + arithmetic. Idle burn is 32.7B cycles/day (11.9T/year) at memory_size 1.25 GB; the 11.4 MB module accounts for about 0.29B/day of it (about 1%), so "mostly storage for the 12 MB Wasm" is wrong and "a build without Python is about half that size" does not halve idle cost. Idle cost follows the heap high-water mark (never shrinks until an upgrade, JS uncapped up to 3 GiB) and the audit log (L18-1), so it grows over an instance's life. The local rate (~317k cycles/GiB/s by fit) is about 2.5x the published 13-node rate (127k), so local figures are not mainnet prices. "15-20 million cycles per run" matches the median run (19-20M instructions) but a run may use the whole 20B budget and more (live: up to 29.3B instructions), about 1,000x.
- Fix: restate idle cost as a function of memory size with mainnet rates, give a worst-case per-run cost, and note that memory and the log grow.

### L18-9 No recovery for a call already stuck in an unbounded wait (Low)

- Where: `src/bridge.rs:323` (timeout read once at run start), `src/bridge.rs:524-527`; README.md:428-433.
- Evidence: C. Setting `call_timeout_seconds` after the fact does not affect calls already waiting; the README describes the option but not what to do once a stop is stuck (start the canister again; the run keeps its slot until the target replies).
- Fix: document the recovery (`icp canister start`, then wait, or reinstall with the data loss that implies); consider making the timeout the default (register #44).
