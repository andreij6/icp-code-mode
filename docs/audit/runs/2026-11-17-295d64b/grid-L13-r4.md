# Grid: L13 Upgrade and compatibility, round 4 (upgrade without a stop, PocketIC)

Run `2026-11-17-295d64b`. Round 2 (`grid-L13-r2.md`) and L2-r3 could only upgrade with icp-cli, which
always stops first, so the documented "unknown" outcome (README.md:419-421: `Started`/`Call`
entries with no closing entry, slots and pending ids reset) was never observed, and only
JavaScript was taken across an upgrade. This round issues `install_code` in upgrade mode on a
**running** canister while JavaScript, Python and Rhai runs wait on a target that never replies,
and forces traps in continuations (E5d).

## Method

- **Harness:** PocketIC server 15.0.0 (`~/.cache/hive/pocket-ic`) driven by the `pocket-ic` 15.0.0
  crate, one fresh instance per experiment, explicit `tick()` rounds. Nothing was sent to the
  shared icp-cli network on port 8010; no setting there was touched.
- **Build:** `git archive HEAD` into a private temp copy (uncommitted `design/` and the modified
  plan doc excluded), `CARGO_TARGET_DIR` = the shared directory, then icp.yaml's own `ic-wasm`
  steps. Artifact sha256 `224cfa7a…c769` (11,402,905 B), byte-identical to the module round 2
  deployed. A second module for the "new version" case: same sources,
  `--no-default-features --features js,rhai`, sha256 `709491a6…163f` (5,593,299 B). The shared
  target's `icp_code_mode.wasm` was rebuilt with default features afterwards (raw sha256
  `9b3d5f5f…9e23`, as L2-r3 recorded).
- **Install:** `InitArgs { token_sha256 = [sha256 of a harness-only test token], allowlist =
  [(fixture, ["hang", "effect"])], limits = null }`; `set_candid_override` for the fixture.
  Upgrades are `install_code` mode `upgrade` with arg `(null)`, sent by the controller
  **without** `stop_canister` (pocket-ic's `upgrade_canister`, chunked: the module is over 2 MiB).
- **Fixture (temp dir only, not in the repo):** `hang(n)` logs `hang-enter n`, then keeps its call
  context open with one self-call per round until `release(true)`, then logs `hang-done n` and
  replies `n`. `effect(n)` logs `effect n` and replies at once. `events()` returns the log, so
  every target-side effect is visible.
- **Scripts** (one per language, submitted together as ingress `http_request_update`):
  `effect(base+1)`, then `hang(base+2)`, then `effect(base+3)` (JS 101-103, Python 111-113,
  Rhai 121-123).

## Experiments

| Id | Setup | Observed |
|---|---|---|
| U1 | Three runs (JS, Python, Rhai) inside `hang`; upgrade to the **same** module | Before: `active_runs = 3`, `pending_run_ids = [0,1,2]`, entries #0-#5 (`Started` effect, `Call` hang, per run). Upgrade returned `Ok` with the canister `Running`. After: `active_runs = 0`, `pending_run_ids = []`, no new entry, ingress calls still open. A new JS run worked (#6 `Started`, #7 `Ok`). `stop_canister` then stayed **`Stopping`** for 40 rounds (`get_policy` refused: "not running"). On `release`: target logged `hang-done 102/112/122` (target completed all three calls); `effect 103/113/123` never happened. The three replies were **delivered to the new module**: each callback trapped and its cleanup trapped too. JS: `CanisterTrapped: heap out of bounds … call_on_cleanup also failed: heap out of bounds`; Python and Rhai: `unreachable … call_on_cleanup also failed: unreachable`. Six `[TRAP]` records in the canister log. Stop then completed. Audit log after restart: #0-#7 only: runs 0, 1, 2 have no closing entry and no `Trapped` |
| U2 | As U1, upgrade to a **different** module (`709491a6…`, js+rhai build) | Same entries, same reset, same stuck `Stopping`. On release every reply invoked the old module's callback table slot in the new module: `CanisterFunctionNotFound: Canister requested to invoke a non-existent Wasm function 5656 from table 0 … call_on_cleanup also failed: … function 5657`, for all three runs. No closing entry, no `Trapped` |
| U3 | As U1 with `call_timeout_seconds = 30` (the README's remedy) set before the runs; time advanced 60 s after the upgrade, target still holding `hang` | The timeout rejects were delivered to the new module. JS and Rhai: `heap out of bounds`, cleanup also trapped. Python: the stale callback **ran ic-cdk's call machinery on reinterpreted new-heap memory**: `Panicked at 'internal error: entered unreachable code: CallFutureState for in-flight calls should only be Executing or Trapped (callback)'` (ic-cdk-0.20.3 `src/call.rs:984`), then the same panic in cleanup (`call.rs:1029`). The ingress reject returned to the MCP client carries that panic text, including the builder's absolute `$CARGO_HOME/registry/...` path. With the timeout the canister could stop afterwards (`Stopped` within 40 rounds). No closing entry, no `Trapped`; `pending_run_ids = []` throughout |
| T1 | Control: Python run, `effect`, then `3 ** 7000000` (trap in the continuation, no upgrade) | `CanisterInstructionLimitExceeded` to the client; cleanup succeeded: #1 `Trapped`, `pending_run_ids = []`, `active_runs = 0`. The documented path works when cleanup runs in the module that made the call |
| T2 | JS continuation allocating 400 x 1 MiB with `wasm_memory_limit` = current + 64 MiB | Did not trap: the run ended `Error` ("script stopped before finishing …") after 106 M instructions. Inconclusive as a memory-trap trigger; not pursued |
| T3 | Upgrade (no stop) after T1/T2, nothing in flight | `Ok`; following run `Started`/`Ok`. Upgrading an idle canister without a stop is harmless |

Cleanup trapping across the upgrade (E5d) therefore did not need an injected fault: every run
taken across a stop-less upgrade ends in a callback trap **and** a cleanup trap, in all three
languages, with either module and with or without a call timeout.

## Rows

| Row | Verdict | Why |
|---|---|---|
| K11 Upgrade while runs are suspended | **finding L13-r4-1** | The upgrade is accepted with calls outstanding (no `pre_upgrade` refuses it, `src/lib.rs:60-72`). Replies are not dropped: the IC invokes the old callback slots in the new module (U1-U3), which trap or, in U3, run ic-cdk code on reinterpreted memory. The canister cannot stop until the orphaned calls are answered (U1, U2) while nothing lists them |
| E2 `post_upgrade` | holds for its own work; finding L13-r4-1 for what it does not do | `apply(None)` returned early; token set, allowlist, limits and entries #0-#5 unchanged after every upgrade (U1-U3, T3). The upgrade hooks never look at in-flight state |
| E5c reply/reject callbacks | **finding L13-r4-1** | Pre-upgrade callbacks are executed after the upgrade in the new module: `heap out of bounds`, `unreachable`, `non-existent Wasm function 5656`, or an ic-cdk panic from misread state (U3 Python). All reproduced for JS, Python and Rhai |
| E5d cleanup callback | **finding L13-r4-1** (across an upgrade); holds otherwise | Without an upgrade cleanup writes `Trapped` (T1, also L2-r3). Across an upgrade every cleanup trapped too ("call_on_cleanup also failed"), so `IcBridge::drop` never ran: no `Trapped`, no slot or pending-id release (moot, the heap was already reset). The failed cleanup itself is L2-5's outcome reached without fault injection |
| V1 `ACTIVE_RUNS` | holds (documented reset) | 3 -> 0 at the upgrade (U1-U3), and new runs got slots. Note: 0 is shown while three call contexts are still open and blocking a stop |
| V3 `PENDING_RUNS` | holds as documented; misleading in practice (L13-r4-1) | `[0,1,2]` -> `[]` at the upgrade, as README.md:419-421 says. But these runs are still waiting on a reply (the README's own definition of pending, README.md:415-416) and still block a stop (U1, U2), so the empty list tells an operator the opposite of the truth |
| CL18 "A script run ends with exactly one of these" closing entries | **violated** (L13-r4-1) | Runs 0, 1, 2 end with none of `Ok`/`Error`/`PolicyViolation`/`Trapped` in U1-U3. README classifies them as "unknown" (419-421), so CL18's "exactly one" holds only for runs not cut by an upgrade |
| CL20 "pending or unknown, never proof of a trap. A trap is recorded as `Trapped`" | **violated** (L13-r4-1) | "A trap is recorded as `Trapped`": 6 traps in U1, 6 in U2, 6 in U3, none recorded. "Unknown … upgraded while the run was waiting" is accurate for the log, but the README does not say the canister will later trap on each reply, can stay `Stopping` with an empty pending list, or that the target still executes the call |
| README.md:430-433 call timeout remedy | holds for stopping only | With a timeout the post-upgrade stop completes (U3) but the traps and missing closings are unchanged |
| Register #44 / L18-9 interaction | note | The documented recovery for a stuck stop (icp-cli gives up and leaves the canister `Stopping`, L13-r2 B) is `icp canister start`; an operator who then upgrades without the stop (raw `install_code`, a proxy or governance canister, other tooling) lands in U1 |

## Candidate findings

### L13-r4-1 Medium: an upgrade without a stop is accepted while calls are outstanding; their replies then run in the new module, trap with their cleanup, and leave no closing entry and an unstoppable canister with an empty pending list

- **Where:** `src/lib.rs:60-72` (only `init`/`post_upgrade`; no `pre_upgrade` guard), state that
  already knows calls are outstanding: `src/bridge.rs:195-198, 229-235` (`PENDING_RUNS`),
  `src/lib.rs:200` (`ACTIVE_RUNS`); README.md:400-426 (CL18, CL20).
- **Invariants:** I3 (records match reality), I5 (clean failure), I10 (honest interface).
- **Evidence:** reproduced in PocketIC 15.0.0 with the release module from HEAD (U1-U3 above), in
  JavaScript, Python and Rhai, upgrading to the same module and to a different one.
- **Scenario:** a controller upgrades without stopping (anything but icp-cli: a raw
  `install_code`, a proxy or governance canister; icp-cli 1.7.0 always stops) while a run waits
  on a call. The upgrade succeeds. `get_policy` shows `active_runs = 0`, `pending_run_ids = []`.
  The target still executes the call. A later `stop_canister` hangs until the target answers.
  When it answers, the IC invokes the old callback's table slot and environment pointer in the
  new module, which traps (or, U3, runs ic-cdk code on memory the new module uses for something
  else), and the cleanup traps too. The MCP client gets a `CanisterError` reject; the audit log
  keeps `Started`/`Call` and nothing else.
- **Refutation tried:** replies might be dropped by the IC after an upgrade (they are not:
  U1-U3 show the callbacks executing); a call timeout might avoid it (it bounds the stop, not
  the traps: U3); the cleanup might still close the run (it traps in all 9 runs); with an idle
  canister nothing goes wrong (T3, true). Worse than a trap (a stale callback that matches a
  function of the new module's type and commits state) was not demonstrated; U3's panic shows
  the stale slot does run real code on reinterpreted state, so this is plausible, not shown.
- **Relation to known findings:** L18-4 (Low: upgrades leave no trace, runs cannot be told apart)
  is about the record. This is the cause: the canister accepts an upgrade it cannot survive.
  L18-4's proposed fix (persist pending ids and close them as `Error` in `post_upgrade`) would
  mark runs as ended whose calls the target still completes and whose replies then trap the new
  module. Verifier may merge the two under this root cause.
- **Fix:** add `#[ic_cdk::pre_upgrade] fn pre_upgrade()` that traps with "runs in flight (ids
  …): stop the canister before upgrading" when `PENDING_RUNS` is non-empty (every outbound
  message, from scripts and searches, inserts there before its callback can exist). A stopped
  canister always passes, so icp-cli is unaffected. Note `--skip-pre-upgrade` bypasses it by
  explicit choice, and an id leaked by a failed cleanup (L2-5) would block upgrades until a stop
  (correctly). Then restate README.md:419-421: an upgrade without a stop is refused, not
  "unknown". Regression test: PocketIC, run waiting on a never-replying fixture, `install_code`
  upgrade without a stop must be rejected and the run must close normally after release.

### L13-r4-2 Low (observation for L10/L15): trap text sent to MCP clients contains the builder's home directory

- **Where:** release profile (`Cargo.toml [profile.release]`, no `--remap-path-prefix`); ic-cdk
  panic locations; visible through any ic-cdk panic that reaches the client.
- **Evidence:** reproduced (U3): the ingress reject returned to the MCP caller reads
  `Panicked at '…' , <absolute $CARGO_HOME>/registry/src/index.crates.io-…/ic-cdk-0.20.3/src/call.rs:984:13`.
  769 such absolute paths are embedded in the module (L15-r1 noted the embedding as a
  reproducibility matter; the client-facing disclosure is new). Not specific to upgrades.
- **Fix:** build with `--remap-path-prefix` for `$CARGO_HOME` and the workspace (also helps I24).

## Not tested

- The real IC (register #57): PocketIC 15 runs the replica's execution environment, so the
  callback-in-new-module behaviour is expected to match, but it is not confirmed on mainnet or
  on the shared icp-cli network.
- A stale callback that does not trap and commits state (needs a new module whose table has a
  `(i32) -> ()` function at the old slot); not constructed.
- Searches (`run_search`, bounded-wait interface fetches) across a stop-less upgrade: same
  `IcBridge::sent` path, expected identical, not run.
- A memory-limit trap in a continuation (T2 did not trap) and a cleanup trap from a real
  trigger without an upgrade (L2-r3 used an injected trap).
- Whether dfx 0.29.2 `canister install --mode upgrade` stops first: its `--help` lists no stop
  step; not run against any network.
