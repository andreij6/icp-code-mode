# Variant sweep r2-11: global slot cap held across an external wait with no deadline

Run `2026-11-17-295d64b`, Phase 6. Pattern under test:

> A global per-process slot cap held across an external wait with no deadline, letting one
> caller's waits starve all callers.

Already known and not re-reported here: a run slot is held across unbounded-wait calls, so
8 runs waiting on a silent target make every later script "busy"; in-flight response
prepayment held by unbounded waits; searches take no run slot; no single-flight on interface
fetches; `call_timeout_seconds` not applied to interface fetches (300 s each); a call
timeout bounds each call, not the stop; Python and JS heaps held while suspended
(`wasm_memory_limit`, Boa GC heap, Python memory accounting); DID_FAILED blocking all callers
for 5 minutes.

Method: listed every per-process cap, counter, pool and lock (grep for `thread_local!`,
`static`, `Cell<`, atomics, `Mutex`, `bounded_wait`, `unbounded_wait`, `change_timeout`,
`deadline`, `timeout` in `src/`, `shims/`, `demo_store/src/`), then for each one asked whether
it is held across an `await` on another party and what ends that wait. Evidence: CP = the
code path settles it. Nothing was built or run: the only candidate needs a target that does
not reply, which only the Live phase may deploy, and `IcBridge` cannot run natively
(register #58).

## Cells

| Row | Item | Cell | Reason |
|---|---|---|---|
| V1 / LIM12 / E5a | `ACTIVE_RUNS` run slots (8, process-wide) with `call_timeout_seconds` **set** | **finding (CP), candidate 1** | `RunSlot` is held until `run_script` returns (`src/lib.rs:218-262`). The only run-level deadline is the instruction deadline (`src/bridge.rs:25-38`), and instructions do not advance while a call waits. `call_timeout_seconds` is applied per call (`src/bridge.rs:524-527`). So with the timeout configured, a run still holds its slot for up to (number of sequential host round-trips) x timeout: at defaults (25-call cap, IC cap 300 s) 24 sequential calls x 300 s plus the fetch, about 2 h per run; with a raised `max_calls_per_script` it grows to JS 64 rounds (`js.rs:26, 374`), Rhai 64 requests (`rhai_rt.rs:23, 211`) or Python 256 suspensions (`python.rs`), i.e. up to about 21 h. Slots are not shared per caller (the caller hash is computed but never used for slot accounting, `lib.rs:221`), so one token holder who keeps 8 such runs in flight and resubmits as each ends keeps every other caller "busy" indefinitely, with the documented remedy enabled |
| V1 / LIM3 (timeout unset) | Same, default unbounded wait | not re-reported | Known finding |
| V1 / E5b | Searches and the run slot | not re-reported | Known (searches take no slot) |
| V2 / V4 / K5 / O2 | DID cache / DID_FAILED around the interface fetch | holds for this pattern (CP) | Not a slot count: nothing is reserved while the fetch waits; a waiter blocks nobody else (no single-flight, known). Both fetch messages are `bounded_wait` (`bridge.rs:426, 434`), so the wait has the default 300 s deadline. DID_FAILED's 5-minute block is known |
| V3 | `PENDING_RUNS` | n/a | Uncapped set used for reporting; it refuses nothing, so it cannot starve callers |
| V5 | Parsed-override cache | n/a | No wait, no cap |
| V6 | Python `LIVE_MEMORY` / `BASELINE_MEMORY` | holds for this pattern (CP) | Process-wide counter, but `rebased` (`python.rs:151-157`) re-bases around every await, so one run's suspended heap is not charged to another run's 64 MiB limit; total heap held by suspended runs is the known memory finding |
| V7 | getrandom `STATE` | n/a | Not a cap, no wait |
| V8 / V9 / V10 | Boa GC heap, ic-cdk executor, engine statics | not re-reported | Heap held by suspended runs is known (Boa GC heap; `wasm_memory_limit` after an await); no count-type slot exists in them |
| `python.rs:168` `LOCK: Mutex<()>` held across awaits | test-only serialisation lock | n/a | Under `#[cfg(test)]` (`python.rs:162-193`); not in the shipped Wasm |
| E5 batch loop (`mcp.rs:386-404`) | sequential tool calls within one HTTP request | holds (CP) | The slot is taken inside `run_script` and dropped when it returns, not held for the rest of the batch; searches after a script in the same batch do not hold it, and a script after searches takes it only when it starts |
| LIM9 / LIM10 / LIM8 | per-request caps (5 searches, 1 script, 50 messages) | n/a | Per request, not process-wide; they reserve nothing across a wait |
| LIM25 / LIM26 / LIM27 | JS `MAX_PENDING`/`MAX_ROUNDS`, Python suspensions, Rhai `MAX_REQUESTS` | n/a for starvation | Per run; they shape candidate 1's maximum hold time only |
| LIM1 / LIM2 / O1 | call cap and instruction budget | n/a | Per run; reserved per run in `IcBridge`, not shared across callers |
| LIM30 platform: per-destination reservation (queue capacity) and callback quotas | platform slot held by each outstanding call | unverified, not a candidate | At defaults script calls in flight are at most 8 x 25 = 200, below the platform per-destination capacity; reaching it needs a raised `max_calls_per_script` (config) or many concurrent searches whose fallback fetches go to one silent target (bounded at 300 s). Not reproducible without deploying a silent target (Live phase); recorded as unverified |
| VF1 / VF5 | cycles held by waits | not re-reported | Response prepayment held by unbounded waits is known |
| E1 / E2 / K11 | upgrade/stop while runs wait | not re-reported | Known (stop stuck; slots lost on upgrade with heap) |
| E3 / E4 / E6-E14 | ingress filter, query path, admin endpoints | n/a | No awaits; nothing held across a wait |
| K1-K10 | commit points | n/a beyond V1 | The only resource held across K2-K5 that other callers compete for is the run slot (candidate 1) and heap (known) |
| C2 / E20-E25 / D8 / V11 | demo_store | n/a | No awaits, no caps |
| D1-D7 | durable state | n/a | Written synchronously; no reservation held across a wait |
| C5 / E26 / B17 / W16 | demo page comparison runner | n/a | Runs in the user's own browser; any waiting is self-inflicted on that tab, no shared cap |
| C6 / E27 | `scripts/demo-setup.sh` | n/a | Operator script, sequential CLI calls |
| C12 / E28 | video tooling (Chrome DevTools port, Gemini calls) | n/a | Dev tooling on the developer's machine; no shared caller population |
| C3 / C4 / C7-C11 / C13-C15 / S1-S10 | build, shims, supply chain | n/a | No runtime slots; the web-time shim's `Instant` is an instruction counter, which does not advance during waits (this is why candidate 1 has no wall-clock bound) |
| A1-A12 / U1-U16 / W1-W16 / F1-F16 / B1-B16 / AD1-AD9 / CL1-CL37 / T1-T12 | remaining rows | n/a except as noted | None holds a process-wide count across an outbound await; claims touched by candidate 1: CL9 ("At most 8 scripts run at once"), README:416-418 and :430-433 (timeout presented as the remedy for waits that never end); T1 tests the cap but not hold time |

## Candidates

1. **With `call_timeout_seconds` set, a run still holds its slot for up to (round-trips x
   timeout), about 2 h at defaults, and slots have no per-caller share, so one token holder
   keeps every other script "busy" indefinitely with the documented remedy on.** L4/L9,
   Medium, CP. `src/lib.rs:198-221` (process-wide `ACTIVE_RUNS`, no per-caller accounting),
   `src/bridge.rs:25-38` (only an instruction deadline), `src/bridge.rs:524-527` (per-call
   timeout only). README:416-418 and :430-433 present the timeout as what stops waits from
   lasting indefinitely.
