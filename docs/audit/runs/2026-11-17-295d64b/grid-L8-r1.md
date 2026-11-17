# Coverage grid: L8 Concurrency and shared state, round 1

Run `2026-11-17-295d64b`. Lens question: what if two of these interleave? Row ids from
`inventory.md`. Evidence: **R** = reproduced (native test in a private copy, or the local
network on port 8010), **C** = code path (unambiguous reading). Candidate findings are
numbered L8-1 to L8-5 (details in the lens report returned to the workflow).

Private copy: `/private/tmp/claude-501/audit-L8` (tests added there only, deleted at the end);
live requests: ordinary `execute_icp_script` calls with the test token, plus controller
`get_audit_logs` / `get_policy` / `canister status` queries. No settings were changed.

## Volatile shared state

| Row | Cell | Reason |
|---|---|---|
| V1 `ACTIVE_RUNS` | holds | Incremented in `RunSlot::take` and decremented in `Drop`, both in single executions; a trap before the first await rolls back the increment with the heap, a trap after an await runs `Drop` in ic-cdk's cleanup (`cancel_all_tasks_attached_to_method`, ic-cdk-executor 2.0.0 `machinery.rs:168`). Only leak path: the cleanup callback itself trapping (e.g. F4 stable memory full during the `Trapped` append), which would also leave the id in `PENDING_RUNS`; not reproducible without exhausting stable memory (not tested). `tests::run_slots_are_capped_and_returned` passes. Searches take no slot (LIM12 note): a multiplication question, left to L4. |
| V2 `DID_CACHE` | **finding L8-3, L8-5** | Insert after the fetch await is unconditional, so a `remove_allowlist` that evicts during an in-flight fetch is undone (L8-3). Concurrent misses for the same canister in one `join_all` round each reserve a cap slot and send their own fetch (L8-5). Cross-run reuse of a fetched interface is intended; a successful entry always wins over a stale failure because `cache_get` is consulted before `failed_get` in `candid()` and `service()` (C). |
| V3 `PENDING_RUNS` | holds | Inserted in the execution that sends the first message (`record_message`), removed in `IcBridge::drop` on every exit path, including cleanup after a trap (C). Searches add their ids too, which matches the doc text "runs that have sent messages". Same cleanup-trap caveat as V1. |
| V4 `DID_FAILED` | holds (by design) | One run's failed fetch is served to every run for 5 min (LIM18). Ordering with a concurrent success is safe (see V2). No way found for a token holder to force a failure of a healthy target (queue-full needs >500 in-flight messages; 8 runs x 25 = 200): not tested further. |
| V5 `OVERRIDES` | holds | Re-validated against the stored text on every lookup (`override_service`), no await inside; inserts from the query path are discarded with the query (C). Override set mid-fetch takes precedence over the later cache insert (C). |
| V6 `LIVE_MEMORY`/`BASELINE_MEMORY` | **finding L8-2** | Register #43's fix holds for other runs' allocations (test `memory_limit_survives_interleaved_runs` passes), but `rebased` also excludes this run's own reply, which is allocated inside the window and freed after conversion: every reply's size comes off the run's count. Reproduced natively: 200 MiB of `icp.candid`/`icp.call` replies held with the 64 MiB limit, while 200 MiB built in the VM gives `MemoryError`. |
| V7 getrandom `STATE` | holds | Shared stream, but nothing a script can observe reads it: Monty's `random` uses `RandomStart::Seed` (not getrandom), Monty str hashing uses a fixed-key `DefaultHasher`, Boa has no `Math.random`, Rhai maps are B-trees. Live: Python class repr/`id`/`hash` and set iteration order identical across runs interleaved with other users' traffic (R). Monty `create_uuid` draws from it for instances crossing the boundary, never shown in results. |
| V8 Boa GC heap | **finding L8-1** | One thread-local heap for every JS run; each collection (automatic and the `force_collect` at the end of every run) marks all live objects of all suspended JS runs and sweeps garbage left by cancelled ones. Reproduced live: `return 1` cost 701,875,371 instructions (audit id 727) when it interleaved with another run holding 600k objects, vs 19.0M normally. The GC threshold is also process-global and only ever raised. |
| V9 ic-cdk executor | holds | Each run's future is its own task; cleanup cancels only the trapped method's tasks (C). |
| V10 engine statics | not tested | Boa interner/shapes, Rhai and Monty internal statics not inspected beyond V7/V8. |
| V11 demo_store `STORE` | not applicable | Shared by design (demo), every update is one execution with no await. |

## Commit points

| Row | Cell | Reason |
|---|---|---|
| K1 first-pending hook | holds | Hook runs in the execution that hands over the message; `record_message` borrows are temporaries, no shared state read before and used after (C). |
| K2 JS `join_all` round | **finding L8-3, L8-5** | Per-call `reserve` (allowlist, method, cap, budget) runs before `service().await`; when the interface is not cached the message is sent in a later execution without a re-check (L8-3). Duplicate fetches per round (L8-5). No `State`/`GcRefCell` borrow is held across the await (C). |
| K3 Python awaits | **finding L8-2, L8-3** | `rebased` window (L8-2); same bridge TOCTOU (L8-3). |
| K4 Rhai replay await | finding L8-3 (shared bridge path) | Mutex not held across the await (C); `answers` per run. |
| K5 `run_search` awaits | **finding L8-3** | `list_allowed()` captured before the loop: methods rendered are the pre-change list; a mid-search removal makes `check_interface_fetch` refuse, and the `halted` heuristic then skips every remaining canister. |
| K6 closing entry | finding L8-4 | Final budget check reads `policy::limits()` again while the engines enforced the deadline captured at bridge creation. |
| K7 cleanup | holds (untested) | Writes `Trapped`, removes `PENDING_RUNS`, returns the slot; heap borrow flags are rolled back to the last commit, where no borrows are held across awaits (C). |
| K8, K9, K10, K12 | not applicable | Single executions, nothing suspended. |
| K11 upgrade in flight | not applicable to L8 | Heap reset is documented; stopping with unbounded waits is L9 (#44). |

## Specific items from the task

| Item | Cell | Reason |
|---|---|---|
| `IcBridge::base_instructions` vs call-context counter across interleaved runs | holds | `call_context_instruction_counter` is per call context; live, run B's `Call` entries advance ~2.3M per call while run A (727) executed between them, and A's 702M is work A's own execution performed (the GC trace, L8-1), not B's. Five searches in one request are sequential and each bridge takes its own base (C). |
| `policy::limits()` at bridge creation vs `set_limits` mid-run (E8, LIM2, LIM3) | **finding L8-4** | `max_instructions` and `call_timeout_seconds` captured in `IcBridge::new` (engine deadlines, Python `vm_budget`, Boa `BoundedJobs.deadline`); `reserve` and the final checks in `run_script`/`run_search` read the live value. Not reproducible here: changing limits is reserved to the Live phase. |
| Allowlist change mid-run (E6, E7) | **finding L8-3** | See K2/K5. |
| Token revoked mid-run (E10) | finding L8-3 (symptom) | `authenticate` runs once per HTTP request; a run already started keeps sending (up to the cap) across later executions. |
| Override set mid-run (E11) | holds | Read fresh on each `service()`/`candid()`. |
| F8 RefCell re-entrancy | holds | JS: every `st.borrow*()` is released before user code runs (`to_json_safe`, `display`, resolvers); the `for line in take(&mut state.borrow_mut().logs)` borrow spans only `bridge.log`. Rhai: `request` holds the mutex only around `to_dynamic`; callbacks never nest. Bridge/policy thread-locals: borrows are statement temporaries; `override_service` drops the borrow before `Service::parse`. |
| I6 Isolation | **violated (L8-1)** | VM state isolation holds (`interleaved_runs_are_isolated` passes); cost and audit isolation do not (L8-1). |
| Register #43 | still fixed for the original cause; new related hole L8-2 | `memory_limit_survives_interleaved_runs` passes. |
| E4 query reading V2/V5 | holds | Queries see committed heap; writes discarded. |
| CL31 `active_runs` / `pending_run_ids` | holds | Text says "scripts" and "runs that have sent messages", matching the code. |
| D1-D5 stable state | holds | Every write is one execution; no read-modify-write spans an await (C). |
