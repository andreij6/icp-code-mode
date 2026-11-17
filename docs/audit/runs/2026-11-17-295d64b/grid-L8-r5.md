# Coverage grid: L8 Concurrency and shared state, round 5 (DID_FAILED poisoning, live)

Run `2026-11-17-295d64b`. Question: can an ordinary token holder make a healthy target's
interface fetch fail, so that `DID_FAILED` refuses that target to every caller for 5 minutes?
Round 3 (L8-7) described the cache semantics from the code only. This round ran it live.
Row ids are from `inventory.md`. Evidence: **R** = reproduced on the local network (port 8010),
**C** = the code path produces it unambiguously.

## Setup

- The canister found running was **not** the audited code. Module `0x89b4c117…` had been
  installed at 16:07Z by someone else, and its interface has fields that are not in the tree
  (`Limits.tool_calls_per_minute`, `PolicyView.candid_overrides`). I built HEAD `295d64b` in a
  private copy (`/private/tmp/claude-501/audit-L8r5`, shared `CARGO_TARGET_DIR`, `ic-wasm`
  metadata + shrink as in `icp.yaml`). That gave module `0x224cfa7a…ec769`, the same hash
  L18-r3 saw deployed. I installed it with `icp canister install --mode upgrade --args '(null)' --yes`.
  `--yes` was needed because of the Candid-breaking `get_policy` change. demo_store
  (`0xae53d21b…`) was left as deployed. It has public `candid:service` metadata and
  exports `tmp_hack`.
- Client: MCP JSON-RPC over the gateway (`Host: icp_code_mode.local.localhost:8010`). The
  token came from the `MCP_TOKEN` line of `.env` and was never printed. Scripts are in
  `scratchpad/l8r5/`.
- Eviction: `remove_allowlist` + `set_allowlist` on demo_store with its original 8 methods.
  This clears `DID_CACHE`, `DID_FAILED` and `OVERRIDES` for it.
- Liquid shortfall: the L18-r3 margin method. `freezing_threshold` was set to
  `(balance - margin) / idle_burn_per_second`, and a shell `trap` restored `2_592_000`
  within seconds. The idle-burn figure moved during the round, because another agent grew
  memory to 1.5 GB. So the margins below are targets, not measured liquid. The available
  figures in the rejection texts are measured.
- Final state, checked: threshold `2_592_000`; limits 25 / 20_000_000_000 / null / null (never
  changed); demo_store allowlist entry restored with the same 8 methods; demo_store Running;
  `pending_run_ids = vec {}`; `active_runs = 0`; demo_store interface fetchable (a search
  returned OK at 16:38:52Z). Another agent added `73qqu-…` to the allowlist during the round.
  I did not touch it. The network was left running, and `icp_code_mode` now runs `0x224cfa7a…`.

## Experiments

| # | Input | Observed |
|---|---|---|
| A1 | Evict, then 200 concurrent `search_icp_canisters {canister_id: demo_store}` | All 200 returned the interface. 25 fetch runs (audit `Started`/`Ok` pairs), started in 2 blocks (13 + 12). At most 25 fetches were open at once. No error, no `CallPerformFailed`, no `SysTransient`. 8.5 s total (R) |
| A2 | Evict, then 600 concurrent searches | Same: 600 OK, 25 fetches, 2 blocks, at most 25 open, no error (R). The first `canister_metadata` reply lands one block later and fills `DID_CACHE`, and every later search is served from it. On this replica, ingress induction (about 13 per block) caps fetches in flight far below the 500-message queue |
| B1 | Evict, `icp canister stop demo_store`, search, script call, `start` | The search fetched the interface **successfully** while the target was stopped: `canister_metadata` is answered by the management canister and never reaches the target. The script call was refused with `5 - Canister … is stopped`. That is a call error, not a fetch error, so it was not cached. After `start`, calls worked at once (R). Stopping a target that has public metadata does not poison the cache |
| C1 | Evict; margin 80B; one search; restore at once; then a search and a script every ~30 s | Search during the shortfall: `could not fetch candid for 4fbx2-… (insufficient liquid cycles balance, available: 39988878635, required: 42102449000); a controller can register it with set_candid_override`. After the restore (balance 13.56T), every search **and** every script (`icp.call(demo,'get_product',1)` -> `Error: could not fetch candid … insufficient liquid cycles …`) was refused with the same stale text, from 16:25:10Z through 16:29:59Z. First success at 16:31:39Z. The failing search wrote **no audit entry**: both sends were refused, so `calls = 0` and there were no violations (R). Margin 60B: the update could not start (HTTP 503), and nothing was cached (R) |
| C2 | Evict; margin ~900B target; **one ordinary token-holder script**: `Promise.allSettled([24 x icp.call(ledger,'icrc1_symbol',[]), icp.candid(demo)])`; restore; victim search every 45 s | The script returned HTTP 200 in 19.4 s. 6 ledger calls were sent, then 18 were refused with `insufficient liquid cycles balance, available: 30…`, and the `icp.candid(demo)` fetch was refused the same way and cached. Victim searches after the restore (balance 13.42T) were refused with `could not fetch candid for 4fbx2-… (insufficient liquid cycles balance, available: 30246110773, required: 42102449000)` from 16:33:35Z to 16:38:07Z. OK at 16:38:52Z. The attacker's audit entries are 10507 `Started` … 10513 `Ok`, `calls = 6`: nothing in the log shows that 18 sends and the fetch were refused or that a failure was cached (R) |
| C3 | Margin ~6T target; 8 scripts x 25 ledger calls, plus 6 searches 50 ms later | Every call and every search OK. The scripts finished over 0.8-5.8 s, so their calls did not overlap enough. Inconclusive (R) |
| C4 | Evict; margin ~3T target; 8 concurrent scripts, each `24 x ledger call + icp.candid(demo)` | Liquid ran out across scripts. One script got **0** calls through, others got 1 and 16. Three fetches returned the cached `available: 8024119516` failure. The other 5 scripts' fetches succeeded. A fetch already in flight that finished later cleared `DID_FAILED` and filled `DID_CACHE` (`src/bridge.rs:497-500`), so the victim search after the restore was OK. Aggregation across concurrent runs is real, but a lasting poisoning needs no successful fetch to finish after the failure (R) |

## Cells

| Row | Cell | Reason |
|---|---|---|
| V4 `DID_FAILED` | **finding L8-r5-1** (R) | A fetch refused because **this canister** is short of liquid cycles is stored as the target's failure. It is served to every caller, in searches and scripts, for 5 minutes after the balance has recovered (C1, C2). The text it serves is stale and misleading: "insufficient liquid cycles balance, available: 30246110773" while the balance was 13.4T. This revises round 1 (holds) and confirms round 3's L8-7, with a trigger an ordinary token holder controls (C2). The opposite race (a late success clears the failure) was also observed (C4) |
| O2 interface fetch | **finding L8-r5-1** (R) | With a liquid shortfall, both steps (`canister_metadata` to `aaaaa-aa`, then `tmp_hack` to the target) are refused synchronously in one execution, and the second error is cached (`src/bridge.rs:424-437, 502-505`). A refused fetch writes no `Started` entry (correct: nothing was sent). A stopped target does not fail the fetch when it has public metadata (B1). Without metadata, the fallback's `is stopped` reject would be cached: C only, not reproduced, because no target without metadata is deployed |
| E5b `run_search` | **finding L8-r5-1** (R, carrier); ingress-flood vector **refuted on this replica** (R) | 800 concurrent fetching searches never had more than 25 fetches open, and none failed (A1, A2). The first reply fills the cache within one block, so the >500-in-flight vector from round 3 does not reach the queue here. Searches are still a carrier: a search fetch during a shortfall poisons the target and leaves no audit entry (C1). That logging guard is already reported (`src/lib.rs:136-161`, not repeated) |
| LIM9 `MAX_SEARCHES` | holds (R) | 5 per request, run one after another. It plays no part in the poisoning: the flood was across requests, and it was self-limited by the cache fill (A1, A2) |
| LIM12 `MAX_CONCURRENT_RUNS` | holds for this lens (R) | The 8-script cap bounds how much liquid one token holder can pin at once: 8 x 25 x 42.1B, about 8.4T. That is the upper bound of the balance band in which the poisoning can be caused (C4 showed liquid run out across concurrent scripts at a ~3T target margin). It did not prevent the single-script case (C2). Searches not counted: already reported |
| LIM18 TTLs | **finding L8-r5-1** (R, symptom) | `DID_FAIL_TTL_NS` (5 min) is applied to a self-caused, immediately retryable condition. Measured lockouts: 16:25:10-16:29:59Z (C1) and 16:33:35-16:38:07Z (C2). Each ended between 5 and 6.5 minutes after the cause. The failure entry outranks nothing (`cache_get` is read first), so only an empty or expired `DID_CACHE` can be poisoned. That window is reopened every hour per target and at every upgrade. "Stale cache expires into a refusal" is already reported (`src/bridge.rs:284-289`) |
| A3 Bearer-token holder | **finding L8-r5-1** (R) | One script with a valid token and two allowlisted targets (24 calls to one, then `icp.candid` of the other) made the other target unavailable to every caller for 5 minutes (C2). This is deterministic within one execution: the calls reserve liquid before the fetch is polled in the same `join_all` round. Precondition: liquid above the freezing threshold no more than about 24 x 42.1B = 1.0T (one script), and up to about 8.4T with concurrent scripts landing in the same round (C4). Plus a target whose cache entry is absent or expired. The attacker can repeat right after expiry, because an unexpired failure costs nothing to hit |
| A6 Allowlisted target | holds: the target cannot be poisoned by its own downtime when it has public metadata (R, B1); **finding L8-r5-1** as victim | A healthy target (demo_store answered every call before, during and after the tests) was locked out by a condition of the calling canister. A target without public `candid:service` that is stopped or slow during a fetch would cache its reject or a `SysUnknown` timeout after 300 s (bounded wait): C, not reproduced |

## Not tested

- **Queue-full and `SysTransient` triggers on mainnet.** Ingress induction per block and
  cross-subnet reply latency differ there. With mainnet block rates, the in-flight fetch
  count could be higher than the 25 seen here. Needs a staging subnet.
- **A target without public metadata, stopped or never answering during a fetch** (fallback
  reject or 300 s best-effort timeout cached). No such target is deployed; building one is
  outside this round.
- **Lasting multi-script poisoning above 1T liquid.** C4 showed liquid exhausted across 8
  scripts, but a concurrent successful fetch cleared the entry. Making the poisoning stick
  needs the fetch to be the last one to finish, which was not attempted.
- **Native fault injection** into `IcBridge::candid` (`CallPerformFailed`,
  `InsufficientLiquidCycleBalance`, `SysTransient`) still needs the `Call` seam described in
  round 3 (register #58).
