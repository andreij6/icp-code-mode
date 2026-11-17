# Grid: L13 Upgrade and compatibility, round 2 (live fault injection)

Run `2026-11-17-295d64b`. Question: does a half-finished operation, and populated state, survive
an upgrade made with the project's own tooling? Round 1 (`grid-L13-r1.md`) decided every row by
reading and native tests and deferred the in-flight cases to a live test; this round runs them.

## Method

- **Network:** the project's running local icp-cli network (gateway port 8010, canisters
  `icp_code_mode` = `4caro-hl777-77775-aaaba-cai`, `demo_store` = `4fbx2-kt777-77775-aaabq-cai`).
  No second network was started.
- **Build:** `git archive HEAD` into a private temp copy, `target` -> the shared
  `CARGO_TARGET_DIR`. Sources were `touch`ed so cargo rebuilt from the committed tree (the shared
  build directory held an `icp_code_mode.wasm` built from another agent's copy, and `git
  archive` mtimes are older than it, so cargo would otherwise have reused it). Artifact
  `224cfa7af8c6f182f25625cd69bf7be565a2e4629c28f4eaaabd739c39eec769` (11,402,905 B; differs from
  the previously deployed `ad3ee2…` as predicted by `ahash/compile-time-rng`, baseline note).
  After the upgrades the deployed module hash equals this artifact. `demo_store` rebuilt to the
  same hash as before (`5ed9b4…`).
- **icp-cli from the copy:** the copy's `icp.yaml` differs from the committed one only in the
  `networks` block (`mode: connected` to `http://localhost:8010` with the running network's root
  key), and the canister ids were recorded with `icp canister link`. Build steps and
  `init_args: "(null)"` are unchanged, so `icp deploy` behaves as in the repo.
- **Never-replying target:** a throw-away fixture canister (`hang_target`, own icp project in the
  temp dir, not part of the repo) whose `hang` method does not reply until `release` is called
  (one self-call per round). Allowlisted for `hang` during the test, then removed from the
  allowlist and deleted. Closes the register #55 gap for this lens only.
- **State comparison before/after every upgrade:** `get_audit_log_len`, `get_policy` (token
  hashes redacted), and a full walk of `get_audit_logs` (page 100, falling back to 10 and 1 on a
  reply-size trap), compared entry by entry by id after normalising the Candid pretty-printer's
  whitespace. Scripts: `snap.py`, `cmp.py` in the session scratchpad (not in the repo).
- **Shared network caveat:** other audit agents were running live tests at the same time
  (8 runs pending on another fixture `4qggx-…` at 06:12-06:16, admin changes to the ledger and
  `aaaaa-aa` allowlist entries, `call_timeout_seconds` set to 600 by someone else). Every
  conclusion below is drawn only from my own runs (identified by `run_id`/script hash) or from
  differences that a `(null)` upgrade cannot produce.

## Experiments (times local, 2026-11-17)

| Id | Setup | What happened |
|---|---|---|
| A2 | Run 1151 (JS): 5 x `demo_store.get_product`, `hang_target.hang`, 10 x `get_product`. `icp deploy icp_code_mode` started while `hang` was held | Canister `Stopping` from 06:17:35. icp-cli waited ("Installing canisters"). During Stopping: `set_limits` refused (`Canister … is stopping`), MCP request -> HTTP 503, `get_policy` query refused. Released at 06:19:22: the run sent its 10 remaining calls **while the canister was Stopping** (entries 1194-1203), wrote `Ok` (1204), the client got HTTP 200 with the full result after 140.8 s, then the stop completed and the upgrade ran (exit 0, module hash 224cfa…). After: `pending_run_ids = []`, `active_runs = 0` |
| A1 | Client loop sending a 24-sequential-call JS script back to back (165 runs over 75 s); plain `icp deploy` (README Quickstart command, both canisters) at t=0 | +3.8 s: one run got 4 rejects `Canister 4fbx2-… is stopped` (icp-cli stops and upgrades `demo_store` too, same hash, state reseeded; the script caught them). +8.3 to +8.7 s: 4 requests HTTP 503 (icp_code_mode stopping/installing). +8.7 s: next request waited 3.5 s and succeeded. All 161 runs in the window have a closing entry; no run left without one |
| B | Run 5444: `get_product`, then `hang` (default unbounded wait); `icp deploy icp_code_mode` | Stop began about +80 s. **icp deploy gave up after about 390 s** (`Failed to stop canister 'icp_code_mode' before upgrade`, exit 1) **and left the canister in `Stopping`**; it stayed Stopping (checked for another 60 s) refusing every request. `icp canister start icp_code_mode` restored service at once; run 5444 stayed pending with its slot (`active_runs = 1`) |
| B4 | With run 5444 still waiting, `set_limits` with `call_timeout_seconds = opt 30`, then `icp deploy` again | Stopping for at least 54 s after the stop began (more than the 30 s timeout): a timeout set later does not release a call already waiting (L18-9, confirmed live). Deploy killed, canister started again. (Run 5444 ended later when the fixture itself rejected `hang` after 637 s; it closed `Ok` with entries 5503-5504) |
| C/D | Run 5529 started while `call_timeout_seconds` was 600 (set by another agent; the IC caps at 300 s), then `icp canister install --mode upgrade --wasm <artifact>` | `icp canister install` **also stops first** (Stopping from about +44 s). The stop lasted until the call deadline expired: client got `call rejected: 6 - Call deadline has expired. (outcome unknown…)` after 310.6 s, the run then sent one more call **while Stopping** (5534) and closed `Ok` (5535); the stop completed and the install succeeded (exit 0). After: `pending_run_ids = []`, `active_runs = 0` |

State comparisons (all common ids byte-identical after normalisation, none missing):

| Pair | Upgrade in between | Log len | Common entries changed | Token set / limits | Allowlist |
|---|---|---|---|---|---|
| s1 -> s2 | A2 | 1149 -> 1206 | 0 of 1147 | identical | ledger entry lost `icrc1_transfer`, `transfer`: another agent's `set_allowlist` (a `(null)` upgrade runs `apply(None)`, which returns at `src/lib.rs:39`) |
| s3 -> s4 | A1 | 1418 -> 5444 | 0 of 1416 | identical | identical |
| s8 -> s9 | C | 5527 -> 5536 | 0 of 5525 | identical | `aaaaa-aa -> []` added: another agent |
| s0 -> s9 | all | 981 -> 5536 | 0 of 979 | identical (1 token, `local-test`) | see above |

The walk could not read two entries at any page size (even `limit = 1` traps on the 3 MiB query
reply limit), before and after every upgrade alike: already reported (page size bounded by count
only, per-entry size unbounded). A full page walk is therefore not a complete check of D4 with
the current interface.

## Rows

| Row | Verdict | Why |
|---|---|---|
| K11 Upgrade while runs are suspended | holds with icp-cli; **finding L13-r2-1**; L18-9 confirmed | Both `icp deploy` and `icp canister install --mode upgrade` stop first and wait (A2, B, C). In-flight runs keep executing during Stopping, close normally with their closing entry, and only then does the upgrade run. The stop is bounded per call, not per run (L13-r2-1). With a target that never replies, icp-cli gives up after about 390 s and leaves the canister Stopping (B; L18-9 / #44, live evidence below) |
| K11, upgrade without a stop | not tested | Not reachable with the project's tooling: both icp-cli upgrade commands stop first. Would need a raw chunked `install_code` from another tool. README.md:419-421 ("unknown") stays the documented outcome |
| E2 `post_upgrade` | holds (for `init_args: "(null)"`) | Three upgrades with icp.yaml's `(null)`: token set and limits identical, every allowlist difference traced to concurrent admin calls by other agents (an `apply(None)` cannot add an `aaaaa-aa` entry or drop ledger methods). Re-applied non-null args: L13-2, already reported |
| E5c reply/reject callbacks | holds (live) | With the stop-first upgrade, callbacks run to completion: A2 continued 10 calls after the held reply; C continued after the deadline reject |
| E5d cleanup callback | not applicable here | No trap occurred in these runs; the stop-first path never discards callbacks, so cleanup is not reached by an upgrade with icp-cli |
| V1 `ACTIVE_RUNS` | holds (live) | 0 after every upgrade; during B the stuck run held its slot until it ended (L4-5/L9-3, known) |
| V2 `DID_CACHE` | holds (code) | Heap, emptied by the upgrade. Live check inconclusive: another agent's all-canister search refilled the entry for the fixture before my probe |
| V3 `PENDING_RUNS` | holds (live), with a gap in L13-r2-2 | Empty after every upgrade, and only after the runs had closed, so nothing was lost. While Stopping it cannot be read at all (query refused) |
| V4 `DID_FAILED`, V5 `OVERRIDES` | holds (code) | Heap caches rebuilt on demand; no failed fetch or override involved in my runs |
| D1-D3 policy | holds (live) | See comparisons. L13-1/L13-2 (whole-record set, re-applied args) already reported |
| D4 audit log | holds (live) | 979 entries present since before the first upgrade, and 5,525 by the last, unchanged byte for byte across all upgrades; log length only grows. Two oversized entries unreadable via `get_audit_logs` (known) |
| CL5 "survive upgrades" | holds (live) | Policy, tokens and audit log survived three upgrades of a populated canister (about 5.5k entries) with runs in flight. Previously only a native decode test |
| CL20 "pending or unknown, never proof of a trap" | holds; gap L13-r2-2 | Upgrades with icp-cli produced no run without a closing entry. "`get_policy` lists these runs in `pending_run_ids`" is not available during a stop, the moment it is needed (L13-r2-2) |
| L18-9 (no recovery for a stuck stop) | confirmed live; extra evidence in L13-r2-2 | `set_limits` is refused while Stopping; after `icp canister start` a new timeout does not release the existing wait (B4); `icp canister start` is the working recovery (service back, run keeps its slot until the target answers) |
| README.md:430-433 "With a timeout … that can't happen" | **finding L13-r2-1** | See below |
| AD9 Candid check / `--yes` | holds | Same `.did`: icp-cli ran "Checking compatibility" without a prompt |
| C7 icp.yaml | holds | `init_args: "(null)"` on upgrade changes no state (above) |
| Register #44 | still open (opt-in fix) | B reproduces it with icp-cli 1.7.0: deploy fails after about 390 s and leaves the canister Stopping |

## Candidate findings

### L13-r2-1 Medium: a call timeout bounds each call, not the stop: runs keep sending calls while the canister is Stopping

- **Where:** `src/bridge.rs:510-528` (`call` sends a new message on every `icp.call`, with no check
  of the canister's status), `src/lib.rs:218-225` (the run continues until the script ends);
  README.md:430-433 ("With a timeout (the IC caps it at 300 s) that can't happen").
- **Evidence:** reproduced (components) + code path (bound). A2: the stop began at 06:17:35 and the
  run then sent 10 new messages (entries 1194-1203) before closing; C: after the 300 s deadline
  reject, the run sent another message (5534) while Stopping. Each new message can wait up to the
  timeout again, so one run can hold a stop for up to `max_calls_per_script` x timeout (25 x 300 s
  by default settings and the IC cap, about 2 h), and every run in flight adds its own. B: icp-cli
  gives up after about 390 s and leaves the canister Stopping, so in practice one run with two
  slow sequential calls at a 300 s timeout makes `icp deploy` fail and the service stay down.
- **Refutation tried:** the IC could refuse outbound calls from a stopping canister: it does not
  (entries above were accepted and answered). The fetch path ignoring the timeout is a separate,
  already-reported cause.
- **Fix:** refuse new outbound messages once the canister is stopping: in `IcBridge::reserve`,
  check `ic_cdk::api::canister_status()` (the `ic0.canister_status` system API, ic-cdk 0.20.3
  `src/api.rs:205`) and return a `policy:`-style error when it is `Stopping`, so a stop waits for
  at most one wait per run in flight; and restate the README: the timeout bounds each call, not
  the stop.

### L13-r2-2 Low: while a stop is stuck the canister refuses everything, including the diagnostics and the setting meant to prevent it, and icp deploy leaves it that way

- **Where:** README.md:416-418 (`get_policy` lists pending runs), 430-433; operational, with
  register #44 / L18-9 as root cause. Listed separately only for the observability part; the
  verifier may merge it into L18-9.
- **Evidence:** reproduced (B, A2). While Stopping: `get_policy` (query) is refused, so
  `pending_run_ids`, the documented way to see which run is waiting, cannot be read;
  `set_limits` is refused; every MCP request gets HTTP 503. After icp-cli gives up (about 390 s),
  the canister stays Stopping until an operator runs `icp canister start`; nothing says so.
- **Fix:** document the recovery (`icp canister start`, read `pending_run_ids`, wait for or abandon
  the target) and that `get_policy` is unavailable while stopping; read the audit log tail
  before upgrading to find open runs.

## Not tested

- Upgrade without a stop (callbacks discarded, cleanup not run, "unknown" runs): icp-cli 1.7.0
  always stops first; needs a raw `install_code` (chunked, 11.4 MB) from another tool.
- A trap during cleanup or a trapped run across an upgrade (E5d): no trapping fixture used.
- Python and Rhai runs across an upgrade: only JavaScript was used. The stop/upgrade behaviour is
  in the IC and `IcBridge`, shared by all three, so the result is expected to carry over.
- The exact point at which icp-cli gives up (measured once: about 390 s from start of deploy) and
  the 30-70 s icp-cli spends in "Installing canisters" before it issues the stop (seen every time,
  not explained).
- Mainnet (register #57).

## Side effects on the shared network

- `icp_code_mode` now runs module `224cfa…` built from HEAD (was `ad3ee2…`, same commit).
  `demo_store` was upgraded once with an identical module (state reseeded, by design).
- About 4,500 audit entries added (A1 loop, probes). The fixture `4xhad-gd777-77775-aaacq-cai` was
  removed from the allowlist and deleted (cycles recovered to the deploying identity).
- `call_timeout_seconds` was set to 30 during B4, overwritten to 600 and back to 30 by another
  agent, and restored to the defaults (`null`, 25, 20_000_000_000, `null`) at the end. Other
  agents' allowlist entries (`aaaaa-aa`, `4qggx-…`) were left as found.
- The network was left running; every canister ended `Running`.
