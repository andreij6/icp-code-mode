# Coverage grid: L16 Claims, round 4

Run `2026-11-17-295d64b`. Scope: the five claims still without a test or reproduction after
three rounds: CL22, CL23, CL31, CL33, CL35 (and the invariants built on them: I14, I19, I21,
I27). Each cell: **finding**, **holds** (why), or **n/a**.

Evidence key: **R-native** reproduced in a private copy (`cargo test --lib l16_`, shared
`CARGO_TARGET_DIR`, copy deleted afterwards; probe source below); **R-live** observed on the
local network (port 8010) through ordinary MCP requests with the test token and read-only
`get_policy` / `get_audit_logs` queries; **C** code path only. Nothing was deployed, no
setting was changed, no write method of `demo_store` was called. This round's own live runs are
audit ids 9906-9909, the benchmark runs 9913-9974 (14:48:00-14:48:18, apart from entries other
agents wrote in between), and 9983-9992.

## Candidate findings

None. All five claims hold where they could be exercised; CL31's nested-run part was not
reproduced (see Not tested).

## Grid

| Row | Cell | Evidence and reason |
|---|---|---|
| CL22 "cached for one hour, then fetched again" (README:441-444, `DID_CACHE_TTL_NS`) **holds (R-live, boundary)** | `demo_store`'s interface was fetched at entry 9850 (`aaaaa-aa.canister_metadata` reply handled at `timestamp_ns` 1_794_924_675_543_657_000, 14:11:15.54 UTC; the cache time is taken in that same execution, `bridge.rs:495`). The ledger's was fetched at 9907 (14:45:41) by an L16 probe. The same JS script (`sales_summary` on `4fbx2-…`, then `icrc1_symbol` on `ryjl3-…`) was run three times. P1, run 9983 at 15:09:45 (demo_store age 3,510 s): no fetch. P2a, run 9986 at 1_794_928_261_084_370_000 (age **3,585.5 s**): no fetch, `methods` = the two calls. P2b, run 9989 at 1_794_928_295_955_937_000 (age **3,620.4 s**): `Started aaaaa-aa.canister_metadata`, then `4fbx2-….sales_summary`, then `ryjl3-….icrc1_symbol`: demo_store re-fetched, the ledger (age about 1,555 s) not re-fetched. That rules out an upgrade or an eviction as the cause, because either would have emptied both entries. The re-fetch happens between 3,585 s and 3,620 s, consistent with `now - at <= 3600 s` (`bridge.rs:287`). Earlier rounds could not observe this (Testability-r2: no hour without an upgrade). The native gap (`time()` read directly, register #45 guard "none") is unchanged |
| CL22 failures 5 min; > 512 KiB refused | holds as before (not re-tested this round) | 512 KiB override boundary R-native in Testability-r1; fetched-size and 5-min paths still need a target that fails cleanly or serves > 512 KiB (register #55) |
| CL23 canonical ids before allowlist, cache, override (README:446-447), bridge path (register #1 guard "none") | **holds (R-native)** | Probe `l16_cl23_bridge_spellings_are_equivalent` drives the production `IcBridge` with `ryjl3-tyaaa-aaaaa-aaaba-cai`, `RYJL3-TYAAA-AAAAA-AAABA-CAI` and `RyJl3-TyAaA-AaAaA-AaAbA-CaI` (allowlisted for `m`, `q`; override `.did` with `q` composite_query). All three spellings give identical results: `call(q)` -> composite_query refusal (so `reserve` passed and the override was found under the canonical key), `call(other)` -> `policy: method \`other\` on ryjl3-tyaaa-aaaaa-aaaba-cai is not allowlisted` (canonical id in the violation), `call(m)` -> reaches the send (`cost_call should only be called inside canisters`), `candid()` -> the override text. Not-allowlisted `4fbx2-…` in three spellings: identical refusal, canonical text; its `candid()` reaches `cache_get` (`time should only be called inside canisters`) identically. Malformed spellings (leading space, no dashes, misplaced dash, trailing dash) are refused as `invalid canister id` on both paths. Through a script: `l16_cl23_python_script_with_uppercase_id` (Python on `IcBridge`) gets the same refusal and the same override text for lower, upper and mixed case. Reason it holds: `Principal::from_text` (ic_principal 0.1.5 `lib.rs:189-223`) is case-insensitive and otherwise exact, and every downstream key is `Principal` or `p.to_text()` (`bridge.rs:473-477, 514-515`). The cache-key part of `candid()` and the fetch reserve are behind `time()` natively (register #58); C: same `key`. Only known divergence: `search::scope` trims, the bridge does not (already reported) |
| I21 | 1-h TTL holds (R-live above); 5-min and fetched-size parts unchanged | |
| I19 | holds; guard can become a suite test | The probe above runs natively with no new harness; register #1's bridge guard can be this test |
| CL31 `active_runs` "at most MAX_CONCURRENT_RUNS"; under nested self-runs (L1-3) | holds by code path; nested case **not reproduced** (C) | `RunSlot::take` (`lib.rs:208-212`) refuses at 8 whatever the caller, so a nested run started by a self-call to `http_request_update` takes a slot from the same counter: `active_runs` cannot exceed 8; the 9th nested level gets the `busy` reply (no entry, as CL19 says). Each nested run builds its own `IcBridge` and writes its own `Started`/closing entries (`lib.rs:218-262`), so every nested run is audited, but with its own `run_id` unlinked to the outer run, a fresh 25-call cap and a fresh budget: already reported (L1-3). Starvation of other callers by nested runs holding all slots: same root as the reported slot-held-across-execute finding |
| CL31 `pending_run_ids` "still waiting on a reply" | holds except the reported `Trapped` case (SW10-1) | Not re-tested |
| I14 | holds for scripts (unchanged) | |
| CL33 benchmark ratios 2.6x / 1.2x / 1.4x (README:194-203) | **holds (R-live)** | `bench.mjs` (a line-for-line port of `src/demo.html:288-516`, same formulas, same scenarios, same chars/4 estimate) run against the live gateway at 14:48 UTC. Top 3 customers: 4 classic calls, results ≈3,152 tok vs ≈484 tok, input 9,954 vs 3,838 tok, ratio 2.594 (README 2.6x, ≈3.2k / ≈0.5k). Restocking: 13 calls, 4,515 vs 3,863, ratio 1.169 (1.2x). Tea report: 2 calls, ≈3,123 vs ≈508, 5,609 vs 3,884, ratio 1.444 (1.4x, ≈3.1k / ≈0.5k). Both sides returned the same answer in all three (`same = true`). Tool definitions: 10 classic tools ≈1,203 tok, Code Mode ≈880 tok. All figures round to the README's. Caveats (not findings): tokens are estimated, not counted by a tokenizer (the README says so); the data is the seed (60 orders, `sales_summary.orders = 54` non-cancelled), so the figures are tied to the seed data. Sizes (12 MB / 3.6 MB / 6 MB) were settled in L15-r2 |
| CL35 "a divergence is reported as an error" (`rhai_rt.rs:3-8`), I27 | **holds (R-native, fault injection)**; branch unreachable from scripts | Probe `l16_cl35_divergence_fault_injection` (inside `rhai_rt`, uses `build_engine` directly; no engine hook needed): after the first evaluation suspends on `icp::call_canister("a","m")`, a different request is placed at sequence 0 and the same AST is evaluated again. In all four shapes (plain, inside `try/catch`, through a closure `f.call` inside `try/catch`, inside a `sort` comparator) `diverged = "non-deterministic script: request #0 changed between replays"` and the evaluation ends with the uncatchable `ErrorTerminated`, which `run` turns into `Err` (`rhai_rt.rs:200-202`). Unreachable from scripts (C): no clock, no randomness, ordered maps, `principal` captured once, answers (including errors) replayed by sequence, `request` refuses while one is pending, so each replay's request prefix is fixed by the cached answers; the `sort` swallowing (L11-r3-2) does not change the request sequence |

## Probe source (private copy only)

`src/l16_probe.rs` (`#[cfg(test)] mod l16_probe;` in `lib.rs`): builds `IcBridge::new("c",
"javascript", "s")`, `policy::set_allowed(ryjl3…, ["m","q"])`,
`policy::set_candid_override(ryjl3…, "service : { m : () -> (); q : () -> () composite_query; other : () -> () }")`,
then for each spelling runs `call(q)`, `call(other)`, `candid()`, `call(m)` under
`catch_unwind` and asserts the outcome tuples are equal. The Python probe runs
`icp.call(cid, 'q')` and `icp.candid(cid)` for three spellings of `rrkah-fqaaa-aaaaa-aaaaq-cai`.
`rhai_rt.rs` gained a test module calling `build_engine(&state, "p", u64::MAX)`, evaluating,
pushing `(Request::Call("a","other",Null), Ok(1))` into `answers`, resetting `seq`, and
evaluating again. Result: 3 passed.

## Not tested

- CL31 nested self-runs live: needs `set_allowlist(<icp_code_mode>, ["http_request_update"])`
  on the running instance (a settings change) and a script that embeds the bearer token in
  the call's headers. This round is Phases 3-4, not the Live phase, so no setting was changed.
  Procedure for the Live phase: allowlist self for `http_request_update`, run a JS script that
  calls itself recursively with a depth counter, read `get_policy().active_runs` during the
  run and the audit entries after it (expect `active_runs <= 8`, one `Started`/closing pair
  per level with unrelated `run_id`s, a `busy` reply at level 9), then `remove_allowlist(self)`.
- CL22 failure TTL (5 min) and the fetched-size refusal: no target that fails cleanly or
  serves a > 512 KiB interface (register #55); natively blocked at `time()` / `cost_call`
  (register #58).
- CL33 with real tokenizer counts: the page and the README use chars/4; no tokenizer was run.
