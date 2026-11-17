# Coverage grid: Testability (register class), round 2

Run `2026-11-17-295d64b`, second discovery round for this lens. Lens question: can the
behaviour of this row be checked by the project's tooling or was it observed live, and if
not, which harness is missing? Rows: CL9, CL19, CL22, CL35, I13, I14, I17, I21, I27, the
limits behind them (LIM12-LIM14, LIM18, LIM19, LIM25-LIM27), T1-T12, and register #16, #29,
#32, #45, #46, #54, #55, #58.

Method:

- Native probes in a private copy (`git archive HEAD` into
  `/private/tmp/claude-501/audit-testability-r2`, package version suffixed `-auditr2t`,
  shared `CARGO_TARGET_DIR=/private/tmp/claude-501/audit-target`, `cargo test --lib audit_r2`,
  `RUST_MIN_STACK=8388608`; copy deleted afterwards). Probes: `q1` busy refusal through the
  real `run_script`, `q2` Rhai replay divergence by corrupting the replay cache between two
  evaluations of the real engine, `q3` JS `MAX_PENDING` boundary (64/65), `q4` Python
  `gather` width (64/65/200).
- Read-only observation of the shared local network (port 8010): `get_audit_logs`,
  `get_audit_log_len`, `get_policy`, `icp canister status`, plus ordinary token-holder
  scripts (one `sales_summary` call each, about every 30-60 s from 11:09 to 11:19 UTC) to
  watch the interface-cache TTL. No setting was changed, nothing was deployed. The Live
  phase was running at the same time (it added canisters `4qggx-…` and `4xhad-…` to the
  allowlist, ran 8 hanging runs, and stopped/upgraded the canister at least twice); entries
  it wrote are cited only as observations.

Evidence key: **R** reproduced natively in this round (probe named), **L** observed live in
this round (audit id or time), **C** code path, **X** reproduced by another lens or round
(id given).

## Grid

| Row | Cell | Reason / evidence |
|---|---|---|
| CL9 / LIM12 / I14 8-run cap, "busy" refusal (register #29 "never triggered live") | holds; natively testable today, untested (variant of the known testability root, not re-reported) | R q1: with 8 `RunSlot`s held, `run_script` returns `{"isError":true, text:"busy: 8 scripts are already running; retry shortly"}` and `policy::audit_len()` is unchanged (0 -> 0): no entry, as CL19 says. The branch runs before any platform read, so it needs no new harness: T1 tests `RunSlot` only, never `run_script`. Live: `get_policy` showed `active_runs = 8` at 11:15 while the Live phase held 8 runs on a never-replying `hang` method (closing entries 1137-1144 at 11:16:56); one monitor probe at 11:12:56 and one at 11:15:04 failed during that window, but the reply text was not captured, so the live "busy" text is **not** claimed here (Live phase / L9-3 owns it) |
| CL9 JS 64 concurrent per round (LIM25 `MAX_PENDING`) | holds (R q3), untested in the suite | 64 calls in one `Promise.all`: Ok, 64 sent. 65: `RangeError: more than 64 concurrent icp requests`, 0 sent (the throw rejects the script before the round is dispatched) |
| CL9 JS 64 rounds / Python 256 suspensions / Rhai 64 requests | known (X: SW5-1) | Off-by-one at the round limit already reported; boundary values re-seen in the live log (ids 920 and 939, Live phase) |
| CL9 vs Python `gather` width (LIM26) | **finding T2-1** (Low) | R q4: one `asyncio.gather` of 64, 65 and 200 `icp.call_async` all succeed and dispatch 64/65/200 concurrent requests in one batch; JS refuses the 65th. Python's only width bound is `MAX_SUSPENSIONS` (each queued `call_async` costs one suspension, so about 255) and the call cap. README:368 states the JS and Rhai bounds and none for Python |
| CL19 "a script refused as busy writes no entry" | holds (R q1) | See first row |
| CL22 / LIM18 / I21 TTL 1 h (register #45, guard none) | gap stands: **not observed live; harness missing** (T-1) | `cache_get` reads `ic_cdk::api::time()` (panics off-chain, X: T-1 p4). Live attempt: probes at 11:11:40 and 11:17:08 sent only `sales_summary` (closing entries 1134, 1150: `calls = 1`, interface served from cache). Expiry could not be watched: the Live phase upgraded/stopped the canister at about 11:09-11:10 (Wasm memory over the 3 GiB limit at 11:09:16, every update refused, memory size 3.42 GB -> 3.15 GB at 11:10:17) and again from about 11:17 ("Canister ... is not running", updates still refused at 11:18:42 when the monitor was stopped), and each upgrade empties `DID_CACHE`. The earlier history (ids 38-973, 05:03-11:08) has no window of 1 h without an upgrade in which a re-fetch of one canister can be attributed to expiry. Missing harness: a clock behind `HostBridge` (or a thread-local test clock replacing `ic_cdk::api::time()` in `cache_get`, `failed_get`, `candid`) plus a fake transport for `fetch_did`; or, live, one hour without an upgrade |
| CL22 / LIM18 TTL 5 min for failures (register #46, guard none) | gap stands: **not observed live; harness missing** | Needs an allowlisted target whose fetch fails without trapping. Only the Live phase's `4qggx-…` did so (id 910, 11:06:19, `Error`, "could not fetch candid ... candid decode failed"); id 925 at 11:06:44 fetched it again 25 s later, which the TTL forbids unless the entry was evicted (allowlist change, `evict_interface`) or the heap was reset. The Live phase was changing exactly those things, so the observation is not attributable and is **not** reported as a TTL defect. Later attempts trapped (925, 943, 948, 951, 964, 971: `Trapped`), and a trap never sets `DID_FAILED` (known). Missing harness: test clock (as above) and a target that fails cleanly (register #55) |
| CL22 / LIM19 / I21 "interfaces over 512 KiB are refused" | override: holds (X: T-1 round p4, boundary 524,288 / 524,289). Fetched: gap stands | Fetched-size check `bridge.rs:490-493` runs after the fetch; natively unreachable (send panics, X: #58). No audit entry in ids 38-1193 carries "over the 524288-byte limit", so it was never observed live. Missing: a target serving a > 512 KiB `candid:service` or `__get_candid_interface_tmp_hack` (register #55) or a fake transport |
| CL35 / I27 Rhai replay divergence | holds (R q2); natively testable with the existing harness, untested | Fault injection: first evaluation suspends on `icp::call_canister("a","m")`; the cache is then given a different request (`"a"."something_else"`) for sequence 0; the second evaluation of the same `Engine`/`AST` sets `diverged = "non-deterministic script: request #0 changed between replays"` and ends with the uncatchable terminate marker. Same result inside `try { } catch` and through a closure `f.call(1)` inside `try/catch` (the wrapped, catchable error is re-terminated by `on_progress` before the catch body runs: final logs empty). No production path diverges: answers are replayed from the cache, maps are `BTreeMap`, `principal` is captured once, there is no clock or randomness (C). So the branch is defensive code reachable only by fault injection; the harness it needs exists (module-private test), no engine hook is required |
| I13 / CL8 instruction budget in native tests (register #32, guard manual) | gap stands (X: T-1, round 1 p5); observed live by others | Live: closing entries 874, 878, 880, 881, 885, 889, 891, 893, 895 (10:33-10:49, L6 and SW1-1) show clean `policy: instruction budget exceeded` at 20.0B-29.5B for Python and Rhai; JS overrun is L6-1. Natively: `bridge::instruction_counter()` is 0 off-chain, so `budget_error(0)` is `Ok` for any deadline and every JS/Rhai budget branch is dead; Python takes a wall-clock branch (`vm_budget`, `python.rs:37-43`). Missing harness: counter behind `HostBridge` (e.g. `fn instructions(&self) -> u64`) advanced by `MockBridge` per host call or per engine step, and `vm_budget` derived from the bridge instead of `cfg!(target_arch)` |
| I17 / LIM13 audit error cap (register #16, guard none) | finding (X: T-4) + live data point | Live: id 75 (05:24:34, an earlier deployed build: `run_id = null`) stores an `error` of 4,110 bytes ending in `…[truncated]` = 4,096 + 14-byte marker, the overshoot T-4 reproduced natively. No newer entry reaches the cap |
| I17 / LIM14 violations cap | finding (X: T-4) + live data point of a known finding | Live: id 847 (10:29:12, `Trapped`, run 846) holds exactly 16 violations (`early_bad0`..`early_bad15`) and no `…and n more` line: the `Drop` path does not append the overflow count (already reported, `bridge.rs:261` vs `lib.rs:156-158, 232-235`). The violation strings there are 78 B, so the 512 B cap itself was not exercised live |
| T1 `run_slots_are_capped_and_returned` | holds for `RunSlot`; gap for the busy branch | Asserts the counter, not `run_script`'s refusal or "no entry" (R q1 shows both are testable now) |
| T2 `.did` match tests | holds (X: round 1) | No CI (X: T-2) |
| T3 `on_first_pending_fires_once_and_only_if_pending` | holds for the mechanism | `IcBridge::sent` never runs natively (#58) |
| T4 `candid_bridge::tests::*` | holds | Fuzz/property coverage X: round 1 p7-p8 |
| T5 `mcp::tests::*` | holds | Cached-search, batch and per-request caps asserted; `http_query` search-cap gap is a known finding |
| T6 `policy::tests::*` | holds | Pure-function checks; no `IcBridge` |
| T7 `cross_language_tests::*` | holds for logic; no differential test of limits (X: L6, T7 cell) | R q3/q4 is such a differential and found T2-1 |
| T8 `js::tests::*` | holds; `MAX_PENDING`, `MAX_ROUNDS` untested (R q3; SW5-1) | |
| T9 `python::tests::*` | holds; `MAX_SUSPENSIONS`, print cap and gather width untested | |
| T10 `rhai_rt::tests::*` | holds; divergence branch untested (R q2) | `request_count_bounds_replays` checks the 65th request only |
| T11 `search::tests::*` | holds | Rendering only; fetch path X: #58 |
| T12 `demo_store::tests::seed_is_consistent` | holds | Writes untested (X: round 1 C2) |
| Register #29 | still fixed; live trigger seen only as `active_runs = 8` (Live phase), refusal text not captured here | |
| Register #32 | still fixed (X: L6 live); guard manual stands | |
| Register #45 / #46 | still fixed by code; guards none stand; expiry not observed (above) | |
| Register #16 | fixed with overshoot (X: T-4, live id 75) | |
| Register #54 / #55 / #58 | still open | The Live phase built ad-hoc misbehaving targets (`4qggx-…`, `4xhad-…` with `hang`) that are not in the repository, so #55 stays open |

## Candidate finding of this round

**T2-1 (Low, consistency/testability). Python has no per-batch request cap; JS refuses the
65th concurrent request.** `src/runtimes/js.rs:254-257` refuses a request once 64 are pending
in a round (`MAX_PENDING`). `src/runtimes/python.rs:249` queues every
`icp.call_async` without a count, and `:262` dispatches the whole queue with one
`join_all`. R q4 (MockBridge): `asyncio.gather` of 64, 65 and 200 calls returns 64/65/200
results with 64/65/200 requests dispatched in one batch; JS with 65 returns
`RangeError: more than 64 concurrent icp requests` and dispatches none. Under default limits
the call cap (25) bounds both; with `max_calls_per_script` raised above 64 (any `u32` is
accepted) a Python batch sends up to about 255 concurrent messages in one execution where
JS sends 64. README:368 lists the JS and Rhai bounds and none for Python. No failure
demonstrated, hence Low. Fix: apply `MAX_PENDING` to `queued` in Python (refuse the 65th
`call_async` with a catchable error), or document the Python bound.

## Not tested, and why

- Live 1 h and 5 min TTL expiry: the canister was upgraded or stopped by the concurrent Live
  phase at least twice within the observation window, which empties the cache; an
  attributable observation needs an hour without an upgrade, or the test clock above.
- Live "busy" reply text: the 8-run window (`active_runs = 8`, about 11:12-11:17) belonged to
  the Live phase; the monitor did not record reply bodies until after it had closed.
- Fetched-interface size cap and clean fetch failures: need a misbehaving target (#55).
- JS and Rhai instruction-budget branches natively: no counter seam (T-1).
