# Variant sweep r1-9: release bundled with a fallible write

Run `2026-11-17-295d64b`, Phase 6. Pattern (from the confirmed finding "a trap inside the
cleanup callback leaks a run slot, a pending run id and the run's task until upgrade"):

> Cleanup that releases shared resources in the same step as a write that can fail, so a
> failed write also cancels the release.

Generalised for the sweep: any release of a shared resource (counter slot, pending-set
entry, executor task, lock, UI busy state, temp directory, child process, debug port) that
is committed in one atomic step (one IC message execution, one function without
`finally`) together with a write or other work that can fail, so that the failure undoes
or skips the release and nothing retries it.

Method: code reading of every release site found by `grep` for `impl Drop`, `fn drop`,
`is_recovering_from_trap`, `saturating_sub`, `remove(`, `finally`, `kill`, `close`,
`rmSync`, `disabled`, `trap`, `mktemp` across `src/`, `demo_store/src/`, `shims/`,
`scripts/`, `video/`; ic-cdk-executor 2.0.0 `machinery.rs` (`cancel_all_tasks_attached_to_method`,
`in_trap_recovery_context_for`) read for what the cleanup callback releases. Evidence is
code path only; nothing was built or run (the L2 reviewer already reproduced the cleanup
path live, `grid-L2-r1.md` K7). Result: **no new candidate**; every hit reduces to the known
root cause.

Release sites that exist in the code (complete list):

| Release | Where | Committed with |
|---|---|---|
| `ACTIVE_RUNS` slot | `RunSlot::drop` `src/lib.rs:212-216` | the execution that drops the run's future: normal end (K6) or cleanup (K7) |
| `PENDING_RUNS` id | `IcBridge::drop` `src/bridge.rs:246` | same execution as the `Trapped` append in K7, or as the closing append in K6 |
| executor task + method context | ic-cdk-executor `cancel_all_tasks_attached_to_method` | the cleanup callback execution, together with the task destructors (`IcBridge::drop` append) |
| Boa GC garbage | `boa_gc::force_collect` `src/runtimes/js.rs:361` | end of `js::run` |
| Python memory baseline | `rebased` `src/runtimes/python.rs:151-157` | each resumption |
| heap interface caches | `evict_interface` `src/bridge.rs:270-275` | `remove_allowlist` execution, with the stable remove |
| demo_store `reserved` | `cancel_order` `demo_store/src/lib.rs:314-328` | the same update, with the status write |
| demo page busy state | `rpc()` / `runComparison()` `finally` `src/demo.html:235, 513` | `finally` |
| Chrome process / DevTools port / profile dir | `cdp.mjs close()`; callers `render.mjs:27`, `stills.mjs:17`, `capture-demo.mjs:43` | end of script, no `finally` |
| frame directory | `render.mjs:35` `rmSync` | end of script |

## Grid

| Row | Verdict | Reason |
|---|---|---|
| C1 canister | known (L2-5) | Only bundled release/write pairs are K6/K7 below |
| C1a feature subsets | holds | Release sites are not feature-gated; JS/Python/Rhai only add VM drops to the same cleanup |
| C2 demo_store | holds | `cancel_order` releases `reserved` and writes status in one update with no await; a failure rolls both back, which is correct (the cancel did not happen). No other release |
| C3 web-time shim | not applicable | Stateless clock; no release |
| C4 vendored chrono | not applicable | No release of shared state |
| C5 demo page | holds | Busy state released in `finally` (`demo.html:235`, `:513`); disable happens immediately before `try`, `store.set` is itself try-wrapped; `runComparison` uses its own `fetch` (`:367`), not `rpc()`, so no nested early release |
| C6 demo-setup.sh | not applicable | No temp resource; `set -euo pipefail` |
| C7 icp.yaml build steps | not applicable | No release step |
| C8-C10 build flags | not applicable | |
| C11 .did files | not applicable | |
| C12 video tooling | known (capture-demo finding) | `cdp.mjs launch` creates a profile dir that `close()` never removes and `close()` is called with no `finally` in `render.mjs`, `stills.mjs`, `capture-demo.mjs`; a failed `writeFileSync`/eval skips the kill. Same root cause as the known "capture-demo leaves Chrome profiles ... DevTools port open after a failure" (root cause in `cdp.mjs`); render/stills hold no token, so no new impact |
| C13 audit tooling | not applicable | Agent instructions, no release code |
| C14 doc | not applicable | |
| C15 non-method exports | not applicable | |
| A1-A12 actors | not applicable | Pattern is about state, not actors; A3 is the only actor that reaches K6/K7 |
| E1/E2 init/post_upgrade | holds | Single execution; `apply` trap rolls the whole upgrade back, old heap (slots, pending ids) kept consistent |
| E3 inspect_message | not applicable | No state change |
| E4/E4a/E4b query | not applicable | Query state discarded |
| E5 http_request_update | known (L2-5) | Tools in one HTTP request run sequentially (`mcp.rs:386-403`); a run's normal release (bridge + slot drop) shares an execution with the next tool. If that next tool traps before its first await, the release rolls back, but the trap is in a reply callback, so the cleanup callback drops the restored future and releases again. Only a trap in that cleanup leaks: the known finding |
| E5a run_script | known (L2-5) | Drop order in the suspended future: VM future, then `bridge` (append `Trapped`), then `_slot`; any trap in the first two skips/undoes the slot return |
| E5b run_search | known (L2-5) | No slot; cleanup trap leaks the pending id and task (same `IcBridge::drop`) |
| E5c reply callbacks | holds | Each callback is its own execution; callbacks after cancellation return early (L2 grid, `call.rs:961-1040`) |
| E5d cleanup callback | known (L2-5) | `cancel_all_tasks_attached_to_method` takes the task list and drops the tasks in the same execution as `IcBridge::drop`'s `append_audit`; if the append (or a VM destructor) traps, the task-table removal, `PENDING_RUNS` removal and slot return all roll back |
| E6 set_allowlist | not applicable | No release |
| E7 remove_allowlist | holds | `evict_interface` then `remove_allowed` in one execution, no await: a trap rolls both back and leaves the allowlist entry, cache and policy consistent |
| E8-E11 admin writes | not applicable | No release |
| E12-E14 admin queries | not applicable | |
| E15 exports | not applicable | |
| E16-E19 script host functions | holds | Per-run state only; nothing shared is released there |
| E20-E25 demo_store | holds | See C2 |
| E26-E28 other surfaces | see C5, C6, C12 | |
| U1-U16 untrusted inputs | not applicable | The pattern is triggered by any trap in K6/K7; inputs that cause traps are covered by the known trap findings |
| O1/O2 outbound calls | holds | The audit entry is written with the send (K1); an insert, not a release |
| O3 audit appends | known (L2-5) | The fallible write in the pattern (F4) |
| O4-O10 | not applicable | No release bundled |
| D1-D3, D5-D7 | not applicable | Admin-only single-execution writes |
| D4 audit log | known (L2-5) | See O3 |
| D8/V11 demo_store state | holds | See C2 |
| D9 page token | not applicable | |
| D10 .env | not applicable | |
| V1 ACTIVE_RUNS | known (L2-5) | |
| V2 DID_CACHE / V4 DID_FAILED / V5 OVERRIDES | holds | Inserts, not releases. Their rollback with a later trap is the separate known "trap is never negative-cached" pattern |
| V3 PENDING_RUNS | known (L2-5) | |
| V6 Monty memory atomics | holds | `rebased` restores the baseline after each await; on cancellation the VM's frees are counted in the cleanup execution and roll back with it together with the memory itself, so the counter stays consistent with the heap. Baseline is re-set at the start of every Python run (`python.rs:229`) |
| V7 getrandom state | not applicable | No release |
| V8 Boa GC heap | holds | `force_collect` is skipped on a cancelled run and on `new_context` failure, but the next run's `force_collect` reclaims it; nothing is held permanently. Cross-run GC charging is the separate known Boa GC finding |
| V9 executor state | known (L2-5) | Task and method context leak with the cleanup trap |
| V10 engine statics | not applicable | Not inspected (inventory) |
| K1-K5 | holds | Commit with inserts only |
| K6 | known (L2-5) | Closing append, `finish`, bridge drop and slot drop share one execution; a trap there is retried by K7 |
| K7 | known (L2-5) | The confirmed finding itself |
| K8, K9 | holds | First-execution trap rolls back the slot take as well |
| K10, K12 | holds | Single execution |
| K11 | holds | Upgrade resets the heap counters (documented) |
| LIM1-LIM32 | not applicable | Limits are not releases; LIM12 is the slot covered at V1 |
| W1-W16 | not applicable | Unbounded work only matters here as a trap trigger inside K6/K7 (known) |
| F1-F3, F5-F16 | not applicable | F2 `Encode!().expect` and F4 are the only failures inside a release step; F2 needs an unencodable entry, F4 is the known trigger |
| F4 | known (L2-5) | |
| B1-B17 | not applicable | Conversions hold no shared resources |
| VF1-VF8 | not applicable | No balance or reservation released in a fallible step (VF6 see C2) |
| AD1-AD9 | not applicable | See E1/E2, E6-E11 |
| S1-S10 | not applicable | |
| CL1-CL37 | not applicable | Claims about cleanup (CL20, CL31) are already contradicted by the known finding |
| T1 | holds (gap) | `run_slots_are_capped_and_returned` tests the normal drop only; no test of a trapping cleanup (known, register testability) |
| T2-T12 | not applicable | |
| History (register) | holds | No past finding of this class other than the source finding |

Out-of-pattern observation (not a candidate here, recorded for the next round): when a
completed script and a following cached search share one HTTP request and the search traps
before any await (for example the known unbounded search render), the script's committed-looking
`Ok` closing entry rolls back and the cleanup writes `Trapped` for a script that finished;
its result was indeed discarded with the reply, so the record is defensible. Code path only.
