# Grid: L13 Upgrade and compatibility, round 5 (canister snapshots: take and load)

Run `2026-11-17-295d64b`. Earlier rounds covered reinstall (L16-13), upgrades with and without a
stop (L13-r2, L13-r4-1) and mentioned snapshots only as a recovery path (L18-r1, L18-2). Nobody had
taken or loaded a snapshot of this canister. This round does, and grades the result under L13, L16
and L18.

## Method

- **Module:** `git show HEAD` sources (295d64b). Raw wasm from the shared target, sha256
  `9b3d5f5f…9e23`, then icp.yaml's own `ic-wasm` metadata + shrink steps: sha256 `224cfa7a…c769`
  (11,402,905 B). This is the artifact L13-r4 recorded for HEAD. The working tree has
  uncommitted edits in `src/` (not by this audit). They were not built or audited.
- **PocketIC:** server 15.0.0 (`~/.cache/hive/pocket-ic`), `pocket-ic` crate 15.0.0, one fresh
  instance per scenario. The harness and fixture were in a private temp dir with
  `CARGO_TARGET_DIR` set to the shared directory, and were deleted afterwards. Fixture (not in the
  repo): `effect(n)` logs and replies, `hang(n)` holds its call context open with self-calls until
  `release(true)`, `events()` returns the log. icp_code_mode was installed with a harness-only
  token hash and the allowlist `fixture: [hang, effect]`, plus a `set_candid_override` for the
  fixture. Snapshots were taken and loaded through the management canister by the controller
  (`take_canister_snapshot`, `load_canister_snapshot`).
- **icp-cli 1.7.0 on the 8010 network:** two throwaway fixture canisters were created
  `--detached`, used and then deleted. Their cycles were recovered. `icp_code_mode` and
  `demo_store` on 8010 were not touched, and no setting was changed.

## Experiments

| Id | Setup | Observed |
|---|---|---|
| S1 | One run (`effect 1`, entries #0-#1). **Snapshot**. Two more runs (`effect 2`, `effect 3`, entries #2-#5). The operator then revokes the token and removes the fixture from the allowlist. **Load** the snapshot. Run once more. Done twice: canister running at take/load, and stopped around take and load as icp-cli does | Both variants gave the same result. After the load: `get_audit_log_len` = 2, entries #2-#5 are gone, and nothing records the load. The fixture still logs `effect 2` and `effect 3`: the target's state is not rolled back. `tokens` = 1 and `allowlist` = 1, so the **revoked token and the removed allowlist entry are back**. The next run used the revoked token and succeeded (`"4"`). It was written as **#2/#3 with `run_id` 2**: the same ids that had described the `effect 2` run, with new content and a later timestamp |
| S2 | 8 JS runs, each `effect`, `hang`, `effect`, all waiting in `hang` (`active_runs = 8`, `pending_run_ids = [0..7]`, entries #0-#15; a 9th script gets "busy"). **Snapshot taken while Running with 8 open call contexts: accepted.** Release. All 8 runs finished `Ok` (#16-#31, the target logged every `hang-done` and the second `effect`). **Load** while Running: accepted | After the load: `active_runs = 8`, `pending_run_ids = [0,1,2,3,4,5,6,7]`, audit length 16. Runs 0-7 have `Started`/`Call` entries only, although they completed. Every new script answers `busy: 8 scripts are already running` (3 tries, and more after 50 rounds). `stop_canister` now **succeeds** (there are no call contexts), but after a stop and a start the state is still `active_runs = 8` and "busy". Only an upgrade with the same module fixed it (`active_runs = 0`, the next script ran). That script took id **#16**, an id that had held a run-0 `Call` entry before the load |
| S3 | Snapshot of an idle canister. Two runs then wait in `hang` (entries #0-#3). **Load the idle snapshot while their calls are open: accepted.** Release | After the load the audit log is **empty** (len 0), with `active_runs = 0` and `pending_run_ids = []`. On release the target completed `hang` for both runs (`effect 301/311` and `hang-enter/done 302/312` logged). Both replies trapped in icp_code_mode with `heap out of bounds … call_on_cleanup also failed: heap out of bounds`, and both MCP calls were rejected with that text. Canister log: 4 `[TRAP]` lines. Effects on the target that **no audit entry ever records**. The next run reused entry **#0 / run 0** |
| C1 | icp-cli 1.7.0 `icp canister snapshot create` on a Running canister | Refused on the client: "is currently running. Please stop the canister first with `icp canister stop`" |
| C2 | `icp canister snapshot create` on a canister left **Stopping** by an open call to a hanging target | Refused: "is still stopping. Please wait for it to fully stop." |
| C3 | `icp canister snapshot restore` on a Running canister, then on a Stopped one | Running: refused with the same message. Stopped: "Restored … from snapshot …". A fixture `effect 3` done after the snapshot disappeared from its state |
| C4 | `--help` for `icp canister snapshot`, `create` and `restore` | Neither says a stop is required, nor that restore rewinds stable memory. The repo README (HEAD) contains no occurrence of "snapshot" |

## Rows

| Row | Verdict | Why |
|---|---|---|
| D4 `AUDIT` StableLog | **finding L13-r5-1** | A load rewinds the log to the snapshot's length (S1, S2, S3). `AuditEntry.id = l.len()` and `run_id` = the Started entry's id (`src/policy.rs:247-253`, `src/bridge.rs:395-416`), so the next entries **reuse ids** that had already been handed out, with different content (S1 #2/#3, S2 #16, S3 #0). The log keeps no epoch, hash chain or canister version. Nothing is written about the load, because no hook runs on a load |
| D1 `LIMITS` | finding L13-r5-1 (symptom) | Same stable memory as D3. A load restores the limits as they were at the snapshot (code path, `policy.rs:128-133` one MemoryManager; not run separately) |
| D2 `ALLOWLIST` | finding L13-r5-1 (symptom) | Reproduced (S1): after `remove_allowlist` and a load, the allowlist entry is back and the target can be called again |
| D3 `TOKENS` | finding L13-r5-1 (symptom) | Reproduced (S1): a token revoked after the snapshot is accepted after the load. A run with it succeeded and was audited under its hash prefix |
| D5 `CANDID_OVERRIDES` | finding L13-r5-1 (symptom) | Same stable memory. Restored to its state at the snapshot (code path; not run separately) |
| V1 `ACTIVE_RUNS` | **finding L13-r5-2** | The heap is part of the snapshot. A load restores the counter of runs that were in flight at the take (S2: 8). Their futures will never be woken, `RunSlot::drop` never runs, and a stop/start does not reset it. Every script answers "busy" until an upgrade. With icp-cli (C1-C3), both take and load need a Stopped canister, and a stop drains every slot, so this path needs platform-level take/load (raw management call, a proxy or governance canister, other tooling) or a slot leaked by a failed cleanup (L2-5) |
| V3 `PENDING_RUNS` | **finding L13-r5-2** | S2: `pending_run_ids = [0..7]` after the load, for runs that completed (and whose closing entries were erased by the same load). README.md:415-418 defines these as "still waiting on a reply". That is false, and stays false until an upgrade, after which they become "unknown" |
| K11 lifecycle operations while runs are suspended | **finding L13-r5-2** (snapshot variant of L13-r4-1) | S2 (take while calls are open) and S3 (load while calls are open) are accepted by the platform. S3 reproduces L13-r4-1's trap-and-cleanup-trap outcome **within the same module**, and also erases the runs' `Started`/`Call` entries. L13-r4-1's proposed `pre_upgrade` guard does not cover this: no canister hook runs on take or load |
| CL4 "append-only on-chain `StableLog`" / "immutable audit log" (README.md:32, 42, 90, 374) | **finding L13-r5-1** | L16-r1 marked "append-only" as holding for this code, and L16-13 (Low) covers "immutable" against reinstall or a rewriting upgrade. A load is different from both. The **shipped** code, with no reinstall and no code change, loses entries **silently and without a gap**. Ids are reused, so a consumer that mirrors entries by id cannot see the rewind once the log has grown past its old length (S1: len 6 -> 2 -> 4; a few more runs bring it past 6 with different #2-#5) |
| CL5 "survive upgrades" (README.md:42, 378) | holds | Upgrades preserve D1-D5 (L13-r2). Snapshots are not upgrades and the claim does not cover them. That nothing is said about them is part of L13-r5-1's documentation gap |
| CL20 missing closing entry is "pending or unknown … a trap is recorded as `Trapped`" (README.md:414-426) | **finding L13-r5-2** | S2: completed runs are shown as pending (listed in `pending_run_ids`) indefinitely. S3: 2 traps, no `Trapped`, and not even a `Started` entry for runs whose calls executed. The README lists only "cleanup failed" and "upgraded while waiting" as causes of "unknown" |
| CL31 `active_runs` "scripts running right now"; `pending_run_ids` "still waiting on a reply" (`src/lib.rs:32-35`, `.did`) | **finding L13-r5-2** | S2: `active_runs = 8` with no script running and no call context open; `pending_run_ids` lists 8 runs that are not waiting on anything |
| AD8 canister settings, upgrades and reinstall (snapshots are the same controller surface) | **findings L13-r5-1, L13-r5-2** (L18 grading) | The README has no section on snapshots. An operator who restores one (the recovery path L18-r1 suggests) silently loses audit entries, re-enables revoked tokens and removed allowlist entries, and, if the snapshot was taken by tooling that does not stop first, gets a canister that refuses every script until an upgrade, with a pending list that is false |
| E2 `post_upgrade` | holds | It resets V1/V3 by reinitialising the heap. This is the only recovery from L13-r5-2 (S2), and it is undocumented as such |

## Candidate findings

### L13-r5-1 Medium: loading a snapshot silently rewinds the audit log, policy and tokens, and the log then reuses entry and run ids

- **Where:** `src/policy.rs:247-253` (`entry.id = l.len()`, `run_id` defaults to the id),
  `src/bridge.rs:395-416` (`run_id` = the Started entry's id), `src/policy.rs:120-133` (all of D1-D5 in
  one stable memory), no lifecycle hook for loads (`src/lib.rs:60-72`). Claims: README.md:32, 42, 90,
  374 ("append-only", "immutable"). The README never mentions snapshots.
- **Invariants:** I3 (records match reality: target effects remain, their records are gone), I9/I10
  (honest interface: "append-only", "immutable"), I1 (a revoked token works again).
- **Evidence:** reproduced in PocketIC 15.0.0 with the HEAD release module (S1 with the canister
  running and with it stopped around take and load, S2, S3). icp-cli's restore flow (stop, restore,
  start) was reproduced on a throwaway canister on 8010 (C3).
- **Scenario:** an operator snapshots before an upgrade, as is common practice and as L18-r1
  proposes. Later, after N runs and a token revocation, they restore it, for example to undo a bad
  upgrade. The N runs' entries disappear with no marker. Their effects on target canisters
  remain. The revoked token works again. The next entries take the erased ids and run ids, so
  "entry #k" now names a different run. An external mirror that pages by id and polls after the log
  has grown past its pre-restore length sees a consistent, gap-free log.
- **Refutation tried:** the load might be refused while Running (it is not: S1, S2, S3). icp-cli
  might add a marker (it does not: the log is the canister's own memory). The id might come from a
  counter outside the log (it does not: `l.len()`). Reinstall (L16-13) might already cover this. It
  does not: reinstall empties the log and the policy, which is visible (len 0, no tokens), while a
  load is invisible and keeps a plausible prefix. The platform records loads in canister history
  (`canister_info`), but nothing in the app or the README points auditors there. That was not
  checked in this run.
- **Fix:** (1) Make a rewind detectable. Store a hash chain in each entry (`prev_sha256`) and expose
  the head hash in `get_audit_log_len`/`get_policy`, so a mirror that keeps the last head notices
  divergence. Or stamp entries with `ic_cdk::api::canister_version()` and document checking
  `canister_info` history for `load_snapshot` changes. (2) README: a "Snapshots" section that says
  a restore rewinds the audit log, tokens, allowlist, limits and overrides to the snapshot. Export
  the log first (`get_audit_logs`) and re-apply revocations after a restore. (3) Soften "immutable"
  to "append-only for this code; controllers can rewind it by restoring a snapshot or reinstalling".
  Regression test: PocketIC, snapshot, run, load, run. The id of the new entry must differ from any
  id issued before, or the head hash must change in a way a mirror can check.

### L13-r5-2 Medium: a snapshot taken while runs are in flight brings back phantom runs when loaded: `active_runs` stuck (all scripts "busy" until an upgrade) and `pending_run_ids` listing finished runs. A load while calls are open erases those runs' entries and traps on their replies

- **Where:** `src/lib.rs:200-216` (`ACTIVE_RUNS`, `RunSlot`), `src/bridge.rs:195-198, 229-235,
  243-246` (`PENDING_RUNS`), `src/lib.rs:32-35` and the `.did` (CL31), README.md:414-426 (CL20).
  Nothing in the app can react to a load, because no canister hook runs on take or load.
- **Invariants:** I3, I5 (clean failure: S3 traps with cleanup traps), I10 (CL20, CL31).
- **Evidence:** reproduced in PocketIC 15.0.0 (S2, S3). The platform accepted take and load on a
  Running canister with 8 or 2 open call contexts.
- **Reachability:** icp-cli 1.7.0 refuses `snapshot create` and `restore` unless the canister is
  Stopped (C1-C3), and a stopped canister has no open call contexts, so the icp-cli path cannot
  produce S2 or S3. They need a controller that calls the management canister directly: a proxy
  or orchestration canister, an SDK, or other tooling. This is the same reachability as L13-r4-1.
  A slot or pending id leaked by a failed cleanup (L2-5) is also captured and carried by any
  snapshot.
- **Refutation tried:** a stop/start might clear the counter (it does not: S2). New runs might
  eventually get a slot (they did not after 50 rounds; the counter only drops in `RunSlot::drop`).
  The load might replay the stale futures (no: their call contexts no longer exist after S2. In S3
  the replies arrive at a heap without those futures and trap). Upgrade recovers (yes: S2).
- **Relation:** a variant of L13-r4-1 (a lifecycle operation accepted while calls are outstanding).
  It is not fixed by L13-r4-1's `pre_upgrade` guard, because loads and takes run no hook. It shares
  L18-4's root, heap-only in-flight bookkeeping. A verifier may merge it under that root, but the
  fix differs.
- **Fix:** README: "take and restore snapshots only from a Stopped canister (icp-cli enforces
  this). If a restored canister answers busy with no runs, upgrade it with the same module". Code
  option: keep an epoch in the heap and compare it with `ic_cdk::api::canister_version()` (or
  another value that changes on load) when taking a slot. On a mismatch, reset `ACTIVE_RUNS` and
  `PENDING_RUNS`. Regression test: PocketIC S2. After the load a new script must run, and
  `pending_run_ids` must not list completed runs.

## Not tested

- The real IC (register #57). PocketIC 15 runs the replica's execution environment. Whether
  mainnet accepts `take_canister_snapshot`/`load_canister_snapshot` on a Running canister with
  open call contexts (S2, S3) was not checked there.
- Whether `ic_cdk::api::canister_version()` changes on a load and on nothing else between update
  calls. This decides whether the L13-r5-2 fix option works and was not measured.
- `canister_info` history entries for loads (would give auditors an out-of-band signal for L13-r5-1).
- Python and Rhai runs across a snapshot load. JS only here. The state involved (`ACTIVE_RUNS`,
  `PENDING_RUNS`, the log) is language-independent, so the outcome is expected to be the same.
- Searches (`run_search`) in flight across a load. Same `IcBridge::sent`/`PENDING_RUNS` path,
  not run.
- Snapshot upload/download (`icp canister snapshot upload`): a controller can upload an edited
  snapshot, which would rewrite the log arbitrarily. Not tried. It is a stronger form of L13-r5-1
  (a controller can falsify entries without changing the code).
- Limits (D1) and overrides (D5) rollback were not run separately (code path only).
