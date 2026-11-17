# Coverage grid: L8 Concurrency and shared state, round 2 (cross-run memory exhaustion)

Run `2026-11-17-295d64b`. Focus: one JS run (no memory cap, register #24) commits a large
heap across awaits while other token holders' runs have to allocate. Row ids from
`inventory.md`. Evidence: **R** = reproduced on the local network (port 8010), **C** = code
path. Candidate findings L8-6 (and the trap attribution) are detailed in the lens report.

No private build was needed (no code compiled). Live work used the shared test token and
demo_store only; one reset upgrade used the already-deployed module copied from the local
replica state (no rebuild). Limits were never changed (an identical `set_limits` was sent
while the canister was refusing updates, and was rejected). Other agents used the same
network at the same time: 8 run slots were held by their `hang` runs 11:11-11:17Z, and they
upgraded the canister at about 11:19Z (module `224cfa7a…`) and again before 11:21:55Z.
**Outage caused by this lens:** 11:08:36Z-11:09:25Z every update call on `icp_code_mode` was
refused (any 503 another lens saw in that window comes from this test).

## Live measurements (all R)

| Step | Input | Result |
|---|---|---|
| Cost | JS `keep.push(s + i)` with `s` 2^20 chars, 1200 times, no await | Ok, 2,072,316,190 instr; memory_size 1,246,514,630 -> 1,372,409,286 (1 byte/char, ~1.65 instr/byte) |
| Under limit, no await | same, 3000 times | Ok, 5,146,000,407 instr; memory_size 3,259,715,014 |
| Over limit, no await (update context) | same, 3150 times | 503 `wasm-memory-limit-exceeded`; no audit entry; **no canister-log line** |
| Over limit after one await (response callback) | `await icp.call(demo,'get_product',1)` then 3150 times | **Ok** (audit 972 `Started`, 973 `Ok`, 5,403,041,437 instr); memory_size 3,416,804,806 > `wasm_memory_limit` 3,221,225,472 |
| Afterwards, any update | JS `return 1`, Python `1+1`, Rhai `1+1`, search, controller `set_limits` (identical values) | all refused: "Canister exceeded its current Wasm memory limit of 3221225472 bytes. The peak Wasm memory usage was 3338207232 bytes"; no audit entries (len stayed 974), no log line; still refused 42 s later |
| Afterwards, queries | `tools/list`, `get_policy`, `get_audit_logs` | answered (`active_runs` 0, `pending_run_ids` empty) |
| Idle cost in that state | `canister status` | 88,156,668,817 cycles/day (vs 3,163,349,992 at 90 MB after the reset) |
| Reset | `icp canister install --mode upgrade` with the same module `ad3ee221…` | memory_size 90,001,030; `return 1` Ok |
| Hold under the limit | attacker: 1 await, 3015 strings, 22 more awaits; victim loops (JS 50 MiB, JS `return 1`, Python 400k list, Rhai 5,000 strings) | attacker Ok (23 calls, 5,326,836,719 instr, run 1341 -> 1368). First JS-50 MiB victim, run during the hold: 503 `wasm-memory-limit-exceeded`, no audit entry (18 requests, 17 `Ok` entries). Small victims and all victims after the hold: Ok |
| Hold, more headroom | same with 2990 strings | every victim Ok (headroom about 30-50 MB was enough) |
| High-water after the run | `canister status` after the hold | memory_size 3,281,866,566, idle 84,709,261,147 cycles/day, until the next upgrade |
| Trap attribution | JS `return ((((…1…))))` 3000 deep | 503 "Canister trapped: heap out of bounds" and log line `[TRAP]: heap out of bounds`; 400 deep: Ok |

## Cells

| Row | Cell | Reason |
|---|---|---|
| I6 Isolation | **finding L8-6** | One run's heap decides whether every other caller's update can run at all (R above). VM-state isolation still holds (round 1). |
| A3 Bearer-token holder | **finding L8-6** | Needs only a token and one allowlisted method (demo_store `get_product`): one `await`, then ~3.1 GB of strings. Costs the caller nothing, the canister ~5.4B instructions. |
| V8 Boa GC heap | **finding L8-6** (memory); L8-1 (cost, round 1) | The thread-local heap keeps every suspended JS run's objects; nothing bounds its size. After the run, `force_collect` frees the objects and later runs reuse the space (R: victims Ok after the hold), but Wasm memory never shrinks. |
| V9 ic-cdk executor | holds (mechanism) | The suspended future keeps the VM heap committed across awaits by design; it is the carrier of L8-6, not a defect of its own. Each run's continuation executes in a response callback, which is what removes the platform limit (LIM29). |
| V6 Python accounting | holds for this lens; L8-2 (round 1) unchanged | Monty's 64 MiB counter is global and rebased around awaits; it is not a heap guard for other languages. JS and Rhai allocations are counted in `LIVE_MEMORY` (python feature) but nothing reads it outside Monty. |
| V1 `ACTIVE_RUNS` | holds | Refused updates never take a slot (rolled back); the attacker's slot is returned normally. During the over-limit state `active_runs` read 0. |
| LIM29 Wasm memory limit 3 GiB | **finding L8-6** | The only memory bound for JS is a platform setting the code never mentions. Not enforced in response callbacks (R: 3.34 GB reached by a run after its first await), and once crossed it refuses every update, admin ones included, until an upgrade or a settings change. No `wasm_memory_threshold`/low-memory hook (icp.yaml sets none). |
| LIM26 Python 64 MiB | holds (bounded victim) | Python victims were never the trigger; reply bytes escape the count (L8-2). |
| LIM27 Rhai size limits | not tested | 1,000 variables x 1 MiB strings (about 1 GiB) per run looks possible by reading the limits; not measured. |
| LIM30 platform limits | **finding L8-6** (mechanism) | The 4 GiB Wasm32 ceiling is the next limit in callbacks; not tested (the platform warns the canister "may become unrecoverable" there). |
| F13 Out of memory | **finding L8-6** | In update context a JS hoarder gets a clean platform refusal (no entry, no log line). After an await it succeeds and the whole canister is refused instead. |
| E5 `http_request_update` | **finding L8-6** | Refused for every caller in the over-limit state; during an under-limit hold, refused only for callers whose execution must grow the heap. Refusals leave no audit entry and no log line. |
| E4 `http_request` (query) | holds | Answered throughout (tools/list, cached searches). |
| E6-E14 admin updates | **finding L8-6** (symptom) | `set_limits` refused in the over-limit state (R). Recovery needs an upgrade or `update_settings` from outside. |
| K2 JS awaits / K7 cleanup | holds | Attacker and victims that were already suspended continue in callbacks (exempt). No cleanup trap was observed. |
| D7 canister settings | **finding L8-6** (symptom), L18-8 (cost) | Idle cost stays at the high-water mark: 84.7B-88.2B cycles/day after one run vs 3.16B at 90 MB (about 27x) until an upgrade. Cost aspect already reported as L18-8; L8-6 shows a token holder sets it on purpose with one run. |
| CL README.md:451-453 ("ends in a trap (a 503 for the client). The cap of 8 concurrent scripts limits how far this can spread") | **finding L8-6** (symptom) | False after an await: the hoarding run returns Ok and every other caller gets the 503, until an upgrade. The 8-slot cap does not limit it: one run is enough. Register #24 was accepted on this text. |
| Register #24 | still open in effect; consequence understated | See L8-6. |
| grid-L18 8 `[TRAP]: heap out of bounds` lines (05:01:56Z-05:05:34Z) | attributed: native recursion, not memory | R: 3000-deep JS parentheses produce exactly that line; the 8 MiB stack sits at the bottom of linear memory, so overflowing it addresses memory below 0 (C, `-zstack-size`, `.cargo/config.toml`). Exceeding the memory limit writes **no** log line (R, three times), and memory_size was 1.25 GB then. The 05:00-05:06Z audit block is the round-1 resource tests (deep loops, big strings, budget overruns); runs that trap before a send leave no entry, so each line cannot be tied to one script. Same class as L3-1/L3-2 (F12). |
