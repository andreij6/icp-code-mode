# Coverage grid: L2 Atomicity and commit points, round 3 (target-side reconciliation)

Run `2026-11-17-295d64b`. Question for this round: does the audit log match what the
targets actually received, run by run? The claim under test (README.md:394-396, CL17) is
"the log never shows a message that was not sent, and never misses one that was". Until
now it had been checked one finding at a time and by code path. No automated test drives the
production bridge (register #58). This round builds the missing harness by hand and runs it.

Evidence key: **R** reproduced on the private network below, **C** code path.

## Method

**Network.** I used a private, disposable icp-cli 1.7.0 network: its own project in the session
scratchpad, gateway port 8061, its own state. It ran the deployed `icp_code_mode` artifact
copied from `.icp/cache/artifacts` (sha256 `ad3ee221…2411`) with a freshly generated test
token. When the test finished, the network was stopped and its state, the token and the
wasm files were deleted. I sent nothing to the shared network on port 8010, which is still
running. That network was not used because this round needs upgrades and `set_limits`
changes. A different port avoids any collision.

**Recording target.** `rec_target` is a throw-away Rust canister (ic-cdk 0.20.3) and is not
part of the repo. Every message it receives is appended to a heap log with: sequence number,
`msg_caller`, method, sha256 of the raw Candid argument, the decoded tag, and `time()`. It
has these methods:
- `ping(text)`: replies.
- `nope(text)`: records, then sends an explicit reject. The record is kept.
- `hold(text)`: records, then loops on self-calls until `set_hold(false)`. Used for slow
  targets, timeouts and upgrades.
- `secret(text)`: never allowlisted. It is a canary for sends that bypass policy.
- `__get_candid_interface_tmp_hack`: exported as an update, so interface fetches that reach
  the target are logged too.

The wasm has no `candid:service` metadata. Every fetch therefore sends two messages:
`aaaaa-aa.canister_metadata`, which fails, and then the fallback to the target. Only
`ping`, `nope` and `hold` were allowlisted.

**Scripts.** A generator writes random scripts in all three languages. Every call site carries
a unique literal tag `<run>.<n>`, so each target-side message maps to exactly one run. The
step kinds are:
- sequential calls;
- `Promise.all` / `asyncio.gather` batches mixing allowed, rejected and policy-refused calls;
- un-awaited calls;
- caught and uncaught rejects;
- caught and uncaught policy refusals;
- `icp.candid`;
- bursts of 20-30 calls past the 25-call cap;
- traps at random positions.

The traps are a catastrophic regex in JS (register #23), `3 ** 7000000` in Python, and
`index_of` over a 1 MiB string in Rhai. The Rhai trap needs `max_instructions = 50e9`
(L14-2). That value was set only for the Rhai trap phases and then put back.

**Reconciler** (scratchpad `recon.py`, `extra.py`). For every `run_id` it checks:
- the first message entry is `Started`, and the message entries carry `calls` = 1..n;
- there is exactly one closing entry;
- the closing `methods` equals the per-message entries in order;
- the closing `calls` equals their number;
- the closing `canisters` equals the canisters named in `methods`;
- the closing status matches what the client saw: `Ok`/`PolicyViolation` for a success,
  `Error`/`PolicyViolation` for an error, `Trapped` for a gateway reject;
- the sequence of messages to the target in the audit equals the sequence the target logged
  with that run's tags. Order counts.

Globally it checks:
- fetch messages counted in the audit equal fetch messages received by the target;
- no tagged target message lacks an audit run;
- no `secret` arrived;
- every argument hash equals the Candid encoding of its tag;
- the only caller is `icp_code_mode`;
- each message entry's timestamp is ≤ the target's receive time, which is ≤ the closing
  entry's timestamp.

## What ran (all reconciled together at the end)

| Phase | Content |
|---|---|
| A (seeds 1, 4) | 88 random scripts, 8 at a time, interface cached |
| B (seeds 2, 3) | 14 rounds of 6-10 concurrent scripts and searches. The interface cache was evicted before each round (`remove_allowlist` + `set_allowlist`), so concurrent runs and searches raced on fetches |
| C | 10 concurrent scripts (JS/Python/Rhai) held on `hold` (8 slots), plus 3 searches while held, then release |
| D | `call_timeout_seconds` 5 and 0 with `hold` in all three languages, timed-out calls caught and followed by more calls |
| E1 | `icp deploy icm` (upgrade) while 2 Python, 2 Rhai and 1 JS run waited on `hold`. Released during `Stopping` |
| E5 | Same, but never released, with `call_timeout_seconds = 240`: the deadline expired during `Stopping` |
| E4 | Same as E5, but the canister ran short of liquid cycles mid-run (unplanned on-the-spot refusals) |
| F, R (seeds 7, 11) | Trap phases with `max_instructions = 50e9`: 36 random scripts, then 20 scripts with the trap forced after 1-6 sends (12 Rhai) |
| G | One HTTP batch: an uncached search followed by a Python script that traps before any send |
| I | Fault-injection build (see the K7 row): a trap inside the cleanup callback, one run per language |

The final reconciliation covered 271 audit runs. Phase R ran after the client-request
counts and the argument checks, so its numbers below are reported separately:
- Client requests, all phases except R: 282 (86 Python, 80 Rhai, 78 JS, 38 searches).
- Phase R: 20 more scripts.
- Messages to the target: 868 tagged script messages and 212 interface fetches.
- `Trapped` closings: 41 (17 JS, 13 Rhai, 10 Python, 1 search).

**Result.** Zero differences between the target log and the audit fields `methods` and
`calls`, and zero differences in closing status, in every phase. Argument hashes, callers and
timestamp order were checked over the first 1,035 receipts (before phase R): every hash
matched, the only caller was `icp_code_mode`, and all 823 order checks passed. The only reconciler flags are the three runs of
phase I: no closing entry, which is the documented "unknown" case (already reported as
L2-5). Two known findings were reproduced, not only shown by code path: SW7-1 (phase G) and
L2-5 (phase I).

## Cells

| Row | Verdict | Reason / evidence |
|---|---|---|
| CL17 "never shows a message that was not sent, and never misses one that was" | holds | **R.** Every tagged message the target received has exactly one `Started`/`Call` entry in the right run and position. Every such entry addressed to the target has a matching receipt. The same holds under concurrency (8 slots full), un-awaited calls, `Promise.all`/`gather` batches, cap bursts, caught and uncaught rejects, call timeouts, upgrades, and traps in all three languages. Policy-refused calls (`secret`, calls over the cap) never reached the target and have no entry. On-the-spot refusals for insufficient liquid cycles (E4) also have no entry. Messages sent in an execution that later trapped were neither delivered nor logged. Rhai replays never re-sent an answered request (no duplicate tags) |
| CL15 "one entry per outbound message … plus one closing entry per run" | holds (except the known cases below) | **R.** Per-message entries: `Started` first, then `Call`, with `calls` equal to the position, in 271 runs. One closing entry per run in every phase except I (cleanup trap, known L2-5). Interface fetches: 212 audit entries for the target fallback against 212 receipts. `canister_metadata` goes to `aaaaa-aa` and cannot be seen at the target; its label is the known L2-1 |
| CL18 "exactly one of these" closing entries | holds for single tool calls; known exceptions reproduced | **R.** Status matched the client outcome in all 271 runs: success → `Ok`/`PolicyViolation`, error → `Error`/`PolicyViolation`, gateway reject → `Trapped`. **SW7-1 reproduced (G):** in the batch [search, then a script that traps before sending] the search, which completed, got `Trapped` (entries 1012-1014, `methods = [aaaaa-aa.canister_metadata, rec.__get_candid_interface_tmp_hack]`, consistent with the target's one receipt), and the script that actually trapped left nothing. Messages still reconcile; only the attribution of the trap is wrong (known) |
| I3(a) entry iff message accepted | holds | **R**, as CL17 |
| I3(b) exactly one closing entry, or pending, or "unknown" only in documented cases | holds, apart from known L2-5 / SW7-1 | **R.** After every phase `pending_run_ids = []` and `active_runs = 0`, except phase I, where the three runs stay pending forever and their 3 slots leak until the next upgrade (L2-5). Reverting to the original wasm cleared both |
| I3(c) `calls` = messages sent; `methods` in order | holds | **R.** Closing `methods` equals the per-message entries in all runs. The audit order equals the target's receive order for each run, so batch sends keep their poll order end to end |
| I3(d) refusal always recorded | violated by design (register #11, accepted) | **R.** For example run A1x27 (JS): a caught `secret` refusal, then a trap before the first send. No entries at all, and nothing at the target. This matches README "Not in the log at all" |
| K1 first-pending commit | holds | **R.** Every receipt has an entry, and every entry to the target has a receipt. Refusals for insufficient liquid cycles (E4: "available 4.59e9, required 4.21e10") produced no entry and no receipt |
| K2 JS `join_all` rounds | holds | **R.** Batches of 2-30 calls (including over-cap bursts and mixed rejects): the target's order equals the audit order. Un-awaited calls are sent in the next round when one exists, and dropped with no entry otherwise (known L2-2; consistent with the log) |
| K3 Python awaits | holds | **R.** Un-awaited `call_async` was sent with the next `gather` (in queue order) and logged in that order. `gather` with a rejecting member: all members were sent and logged |
| K4 Rhai replay awaits | holds | **R.** About 90 Rhai runs, including 13 trapped after 1-6 sends: no duplicate receipts and no missing entries. Answers come from the replay cache, so each request is sent once |
| K5 `run_search` / `fetch_did` awaits | holds | **R.** 38 searches, plus concurrent fetch races between scripts and searches (phase B, cache evicted each round). Every fallback fetch is logged once per message, including duplicate concurrent fetches (known L4-1 class) |
| K6 closing entry + reply | holds (single tool call); SW7-1 for batches | **R** for single tool calls. The batch case is SW7-1, reproduced in G (known) |
| K7 cleanup callback | holds when cleanup succeeds; L2-5 reproduced | **R.** Traps in a response callback produced `Trapped` with the right `methods`/`calls` in JS (17), Rhai (13), Python (10) and search (1). **Fault injection:** a private copy (repo sources without `target/`, built into the shared `CARGO_TARGET_DIR`) added one `ic_cdk::trap` after the `Trapped` append in `Drop for IcBridge`, firing only when the run had sent `nope`. The shared build directory's `icp_code_mode.wasm` was restored byte for byte afterwards (sha256 `9b3d5f5f…9e23`, original mtime). Deployed to the private network only, as module `749920a5…3471`. Result: the three runs (one per language) kept their `Started`/`Call` entries, which matched the target exactly. They had no closing entry, stayed in `pending_run_ids`, and `active_runs` stayed at 3. This is L2-5, now reproduced. The original wasm was reinstalled afterwards |
| K11 upgrade while runs wait (Python and Rhai) | holds with icp-cli (stop first) | **R.** E1: `icp deploy icm` with 2 Python, 2 Rhai and 1 JS run held. The canister went `Stopping`. After release, every run sent its remaining 3-4 calls **while Stopping** (entries 559-574, all received by the target), wrote `Ok`, and only then did the upgrade run. E5: the 240 s deadline expired during `Stopping`. All five runs caught "outcome unknown", made further calls while Stopping, and closed `Ok`; the upgrade followed. A new request during Stopping got HTTP 503 and has no entry and no receipt. Sending during Stopping is the known L13-r2-1. An upgrade without a stop was not tested (below) |
| O1 script calls | holds | **R**, as CL17. Timeouts (D, E5): the target received `hold` and the audit logged it. The client error says "outcome unknown" (documented SYS_UNKNOWN case). With timeout 0, calls still completed and were received (D0) |
| O2 interface fetch | holds (count); label known | **R.** 212 against 212. One cap slot sends up to two messages (known); the `aaaaa-aa` label is known L2-1 |
| CL19 "two kinds of request write no entries" | holds for the two named kinds; other kinds known (L2-6, #11) | **R.** "busy" refusals (C: 2 of 10) left no entry and no receipt. Searches answered from cache (C) left no entry and sent nothing |
| CL20 "pending or unknown … A trap is recorded as `Trapped`" | holds except known SW7-1 / L2-5 | **R**, as K6/K7 |
| V1 `ACTIVE_RUNS`, V3 `PENDING_RUNS` | holds; L2-5 reproduced | **R.** 0 / empty after every phase except I |
| D4 / O3 audit appends | holds | Every append observed was in the same execution as its effect, or in the closing/cleanup step. No half entries |
| Register #58 (no test drives the bridge) | still open | The harness here (recording target, tagged random scripts, reconciler) is a manual tool in the session scratchpad, not a test in the repository. It does show what #54/#55/#58 would need: a fixture target that records receipts and holds on demand, plus a diff of receipts against `get_audit_logs` |

## Not tested

- **Upgrade without a stop.** Both icp-cli upgrade paths stop first (L13-r2). A raw chunked
  `install_code` on a running canister would be needed to drop in-flight callbacks. The
  documented outcome ("unknown": `Started`/`Call` with no closing entry) is unverified.
- **A message accepted but never delivered** (a bounded-wait call that times out before
  delivery). A local network delivers within a round, so every timed-out call here had already
  been received. The README covers this as "outcome unknown".
- **A target that traps.** The recorder cannot log a message whose execution it rolls back.
  The caller-side behaviour is the same as for `nope` (a reject after an accepted send).
- **The original direction of SW7-1** (the script completes and a later search in the same
  execution traps). It would need the script's last execution calibrated to within the
  search's cost of 40B. The reverse direction (G) shows the same root cause.
- **Cleanup trap from a real trigger** (stable-memory growth failure, F4). It was injected
  with a modified build instead. See L2-r1/L2-r2 for why the real trigger was not reachable.
- **Mainnet timing** (cross-subnet delays, queue-full refusals). This was local PocketIC
  only.
