# Coverage grid: L2 Atomicity and commit points, round 2 (cleanup-callback limits)

Run `2026-11-17-295d64b`. Question for this round: can an ordinary token holder make the
cleanup callback trap, so that a run gets no closing entry and leaks its run slot and pending
id? Round 1 found one trigger only (stable-memory growth failure, L2-5, Low).

Evidence key: **R** reproduced, **C** code path, **L** read-only on the shared network (port
8010).

Where it ran:
- **Private network.** icp-cli 1.7.0, a separate project in the scratchpad, gateway port 8047,
  its own state. It ran the exact deployed artifacts from `.icp/cache/artifacts`
  (`icp_code_mode` sha256 `ad3ee221…2411`, `demo_store` `5ed9b423…a029`) with a freshly
  generated token. It was stopped and deleted afterwards.
- **Shared network (8010).** Read-only use: `get_audit_logs`, `get_policy`, `icp canister logs`,
  and management `canister_info` through the network's proxy canister.

Nothing was compiled.

## Platform facts used

- **Cleanup instruction budget.** The replica (`rs/execution_environment/src/execution/response.rs`)
  holds back 5% of the message limit for cleanup. The response callback runs under a limit
  reduced by that reserve. Cleanup gets whatever the callback left unused, plus the reserve. So
  after a callback that hit the instruction limit, cleanup has about 2B instructions (5% of 40B).
- **Wasm memory limit.** Per the DFINITY PSA on the Wasm memory limit, `wasm_memory_limit` is
  enforced only in update, init and post_upgrade. It is not enforced in response callbacks, so
  in a callback or in cleanup the only memory ceiling is the 4 GiB wasm32 address space.
- **Drop order** (ic-cdk 0.20.3, `call.rs:1006-1038`; executor `machinery.rs:146-189`). Cleanup
  first makes two small allocations: the `"cleanup"` reject string and
  `Vec::with_capacity(tasks)`. It then drops the whole task. Inside `run_script` the awaited
  `LanguageRouter::execute` future is the innermost live value, so it is dropped first. That
  future holds the Boa Context and State, the Monty snapshot and heap, the Rhai Engine, AST and
  `answers`, and the in-flight `join_all`. Next `IcBridge::drop` runs (`PENDING_RUNS` removal,
  then the `Trapped` append), and `RunSlot` drops last. The R evidence below confirms this order:
  the Monty heap teardown shows up in the `Trapped` entry's instruction count.

## Cells

| Row | Verdict | Reason / evidence |
|---|---|---|
| E5d / K7: cleanup instruction cost, Python | holds | **R.** Script: build 500,000 heap objects (at the 64 MiB limit), `icp.call` (commit), then `3 ** 7000000` in the callback. The callback trapped with "instructions for single message execution". The cleanup still wrote `Trapped` (id 33, `calls = 1`); `active_runs = 0` and `pending_run_ids = []` afterwards. `Trapped.instructions − Started.instructions` was 96.8M: everything committed after `Started` plus the Monty heap drop, about 190 instructions per object. The cap on the heap is 64 MiB counted, plus the L8-2 reply allowance. Each reply is held to `DECODING_QUOTA`: a 5,000-item `list_orders` reply decoded and 10,000 items failed (R). With 25 replies of at most a few MiB each, the heap stays near or below 1M objects, so teardown is about 0.2B. That is roughly 10x under the 2B reserve. Monty frees iteratively (`Heap::dec_ref` work stack, flat `StableHeap::drop`), so there is no recursion and no stack risk |
| E5d / K7: cleanup instruction cost, JS | holds | **R.** Script: hold 400k objects, `icp.call`, then a burn loop inside a function (L6-1) that trapped at the instruction limit. `Trapped` was written (id 39), with only 0.81M between `Started` and `Trapped`. Boa GC objects are only unrooted on drop (`boa_gc` `Gc::drop` finalizes the handle; `Context::drop` decrements a counter), so heap size does not affect cleanup cost. The objects are freed by the next JS run's `force_collect` |
| E5d / K7: cleanup instruction cost, Rhai | holds | **C.** Across an await Rhai keeps only the Engine, the AST and `answers` (JSON replies plus request args). It keeps no script-built Dynamics, because each replay finishes `eval_ast` before the await (`rhai_rt.rs:188-218`). AST nesting is capped by `set_max_expr_depths(64, 32)`. Request args are capped at depth 128 by `to_json_value`. Replies are capped by `DECODING_QUOTA` (5k-item ceiling, R) and `MAX_DEPTH`. At most 25 answers, so teardown is far below 2B and serde_json's recursive drop stays shallow. Not reproduced: Rhai has no in-callback 40B trap under default limits (`on_progress` stops cleanly; L14-2 needs `max_instructions ≥ 40e9`) |
| E5d / K7: cleanup memory | holds (code path, ceiling not reproduced) | **C.** Before any drop, cleanup allocates only two tiny blocks. In every run, the execution that commits before a suspension sends a message, and `record_message` there appends an audit entry and frees its Candid buffer and string clones before the commit. So free chunks larger than those two blocks exist at every commit point. Every engine also frees memory before the `Trapped` append runs: JS frees Boa's 1,024-slot VM stack (`Stack::new(1024)`, 16 KiB contiguous), Python the Monty heap, Rhai the Engine and AST. The append needs under about 1 KiB. Not reproduced at the 4 GiB ceiling (see Not tested) |
| E5d / K7: panic during cleanup drops | holds | **C.** No RefCell or Mutex guard is held across an await in `IcBridge`, `js.rs` (comment at `:382`), `python.rs` or `rhai_rt.rs`. Monty's `dec_ref` asserts `readers == 0`, which holds at a suspension. **R:** the Monty and Boa teardowns completed in cleanup |
| E5d / K7: drop order engine → `IcBridge` → `RunSlot` | holds | **C** (Rust drop order of an async fn's live locals; the awaited temporary goes first). **R:** the 96.8M Python delta against 0.81M for JS. All engine teardown happens before the closing record and before the slot and pending id are released. That is a variant of L2-5 (fallible work before the release), bounded today as measured above |
| E5d / K7: other trap triggers in callback | holds | **R.** Boa `String.prototype.repeat` with a count of 2e8 panics with "capacity overflow" (candidate L2r2-1). After a send, cleanup wrote `Trapped` (id 44), the slot came back, and the pending list was empty. **R:** the instruction-limit trap (Python bigint `**`, JS loop) also left `Trapped` |
| V1 `ACTIVE_RUNS` | holds | **R.** 0 after every trapped run on the private network (4 `Trapped` runs). **L:** 0 on 8010, which has had no icp_code_mode code change since 09:32:24 and 4 `Trapped` runs since then, so no cleanup failure has leaked a slot there |
| V3 `PENDING_RUNS` | holds | **R** and **L**: empty, under the same conditions as V1 |
| CL18 "exactly one closing entry" | holds (single tool call; SW7-1 known for batches) | **R.** Private log: 15 runs with `Started`, each with exactly one closing entry. **L.** 8010: entries 680-899 (current module) have no orphan |
| CL20 "pending or unknown … A trap is recorded as `Trapped`" | holds; the six live orphans explained | **L.** See "Six live orphan runs" below. Every orphan was written by a code version that had no `Trapped` cleanup (register #9 era), not by the audited module |
| K11 upgrade while runs are suspended | not applicable to the six orphans | **L.** No orphan was still waiting at the upgrade that followed it: `demo_store` (their only target) kept answering other runs inside each window. See below |
| F4 stable growth in cleanup | known (L2-5), not retested | Still the only demonstrated cleanup-trap trigger (and only by code path) |
| W11 Rhai `answers` retention | holds for cleanup | Bounded as above. Memory retention is L3's question |
| V6 Monty memory atomics in cleanup | holds | The frees during cleanup go through `sub()`, and this cleanup is kept, so the counter matches the freed heap. Baseline is re-set per run (`python.rs:229`) |
| V8 Boa GC heap after cleanup | holds (note) | **R.** Cleanup does not free the trapped run's GC objects; the next JS run's `force_collect` does. This is a delay, not a leak |
| Canister log as a trap witness | note (trap visibility already reported) | **R.** Instruction-limit traps in a response callback leave no canister log record on this replica. Rust panics do (`Panicked at …` plus `[TRAP]`, including in a callback whose cleanup succeeded). **L:** the 8010 log has 17 records (05:01-05:05 and 10:34-10:35), nothing for its 4 `Trapped` runs, and nothing between 05:05 and 10:34 |

## Six live orphan runs (465, 469, 470, 471, 488, 654)

Upgrade times come from management `canister_info` (read through the 8010 proxy canister).
Commit times come from `git log`.

| Run | Started (UTC) | Module (deployed at) | Next upgrade | Messages | Closing entry existed in that version? |
|---|---|---|---|---|---|
| 465 | 08:53:21 | `bc5a98…` (08:53:07, commit f5305be "run_id") | 09:04:30 | 1 to demo_store (old schema: `calls = 0`, no `methods`) | No. `Drop for IcBridge` did not exist |
| 469, 470 | 08:58:40, 08:58:53 | `bc5a98…` | 09:04:30 | demo_store; same script `91c57ccc` twice | No |
| 471 | 09:04:46 | `a7d5cf…` (09:04:30, d508ece) | 09:22:21 | 5 (list_orders, fetch, 3× create_order) | No |
| 488 | 09:22:47 | `21872f…` (09:22:21, 5d856e3) | 09:23:58 | 2 (sales_summary, fetch); script `e5fdbc9c` | No |
| 654 | 09:27:22 | `f32a00…` (09:27:17, 13abe24) | 09:32:24 | 1 (sales_summary); same script `e5fdbc9c` | No. `Drop` only removed the pending id; no `Trapped` |

`Trapped` closing entries arrived with cdfa3d9, deployed as `ad3ee221` at 09:32:24. Since then
there have been 220 entries (680-899), 4 `Trapped` runs and no orphan.

Cause, with the evidence:
1. **Not cut off by an upgrade.** Every orphan targeted only `demo_store`, a synchronous
   canister that replies within a round. Other runs to `demo_store` finished inside the same
   windows (467 eight seconds after 465; 491 eight seconds after 488; 679 after 654). The gaps
   to the next upgrade (71 s to 17.5 min) are far longer than any round trip. `demo_store` had
   no code change inside any window.
2. **Deterministic.** The same script was orphaned twice in a row (469/470 `91c57ccc`), and
   across two module versions (488/654 `e5fdbc9c`). A run cut short by an upgrade would not
   repeat like that; a script that traps after its first reply would.
3. **Instruction-limit trap, most likely.** The 8010 canister log has no record between 05:05
   and 10:34. The log is not full (17 records, indices 0-16, none evicted) and records persist
   across upgrades. Wasm traps and panics are logged, also in callbacks (R); instruction-limit
   traps in callbacks are not (R).

So each orphan was a response-callback trap, most likely at the instruction limit (register
#23 / L6-1 class), under a version without the `Trapped` cleanup (register #9, fixed in
cdfa3d9). None is evidence of a failed cleanup in the audited code.

## Not tested

- **Cleanup out of memory at the 4 GiB ceiling.** Making it fail would need the committed heap
  to have under about 64 bytes free with no reusable small chunk, so that ic-cdk's two
  pre-drop allocations fail before anything is freed. Reaching that takes about 4 GiB of JS
  allocations spread over several concurrent runs (about 24 instructions per byte, within the
  20B per-run budget), followed by byte-level calibration across many probes. Not attempted:
  each probe costs minutes and GBs of replica state. The code-path argument in the
  cleanup-memory row says the window does not open, because the committing execution frees
  its `record_message` temporaries. Unverified.
- **Exact cleanup budget on this PocketIC version.** Read from the replica source on master;
  the measured teardowns (≤ 0.1B) are far below any plausible reserve.
- **Rhai in-callback 40B trap.** Not reachable with default limits.
