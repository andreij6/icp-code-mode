# Coverage grid: L8 Concurrency and shared state, round 3 (DID_FAILED poisoning)

Run `2026-11-17-295d64b`. Question: can a token holder make `DID_FAILED` hold a failure for a
healthy target, so every caller is refused for 5 minutes? Row ids from `inventory.md`.
Evidence: **C** = code path (unambiguous reading). Nothing was reproduced in this round. No
private build was made, and no request was sent to the local network. Flooding the shared
instance with more than 500 concurrent searches, or pinning calls until its liquid cycles run
out, is a destructive load test. Under the run rules that belongs to the Live phase, not to this
lens. Candidate finding: **L8-7**.

## How the code behaves (C)

- `IcBridge::candid` (`src/bridge.rs:471-507`) caches **every** `Err` from `fetch_did`, size
  check or parse in `DID_FAILED` under the target's canonical id. The entry lasts 5 minutes,
  is shared by all callers, and is not tied to the run or caller that caused it. Only policy
  refusals from `reserve` are kept out of the cache, because they return earlier through `?`.
- `fetch_did` (`src/bridge.rs:423-437`) falls back to `__get_candid_interface_tmp_hack` on any
  error from `canister_metadata`, including a send refusal. It returns the fallback's error.
- In ic-cdk 0.20.3 (`call.rs:612-660`), two kinds of error are clean and not the callee's
  fault. One is a synchronous refusal: `CallPerformFailed` (output queue full, or the
  canister cannot pay for the send or the response reservation) or
  `InsufficientLiquidCycleBalance`. The other is a `SysTransient` or `SysUnknown` reject, such
  as a bounded-wait timeout or a dropped best-effort response. ic-cdk marks the
  `SysTransient`/`SysUnknown` rejects as immediately retryable. `candid()` treats them all like
  a permanent "this target has no interface" and caches them.
- The two causes behave differently on the fallback:
  - A cycles shortfall refuses both sends: metadata to `aaaaa-aa`, then the fallback to the
    target.
  - A full queue to `aaaaa-aa` refuses only the first. The fallback then goes to the target's
    own queue. demo_store exports `__get_candid_interface_tmp_hack` (`demo_store/src/lib.rs:342`),
    so it would still succeed there.
  - On a healthy target without `tmp_hack` (many Rust canisters), the fallback's "method not
    found" is cached. In that case, failing only the metadata step is enough.
- The cache is not single-flight. Concurrent misses from separate ingress messages each send
  their own fetch, and the last one to finish decides what is stored. A fresh `DID_CACHE`
  entry still wins over a later failure, because `cache_get` is read before `failed_get`.
- Recovery: a controller's `set_candid_override` is read before both caches, so it bypasses
  the lockout. Callers have no way to clear the entry.

## Cells

| Row | Cell | Reason |
|---|---|---|
| V4 `DID_FAILED` | **finding L8-7** (cache semantics, C); poisoning by a token holder **unverified** | Transient and self-caused send failures are cached as if they were a property of the target, for every caller (see above). Whether a token holder can cause one on demand was not tested (see "Not tested"). This revises the round-1 V4 cell, whose bound of 8x25 did not count searches. |
| V2 `DID_CACHE` | holds for this lens; expiry symptom already reported | A fresh entry wins over a concurrent failure (C). After the 1 h TTL, a failed refresh makes the held copy unusable and the target is refused for 5 minutes. That is the reported finding at `src/bridge.rs:284-289, 439-449, 471-507`. L8-7 widens the set of failures that trigger it to transient ones. |
| E5b `run_search` | **finding L8-7** (carrier) | Searches take no run slot (LIM12) and fetch through the same `candid()`. Within one HTTP request, searches run one after another (`src/mcp.rs:400-402` awaits each `exec`), so one request has at most 1 fetch in flight. More than 500 sends in flight therefore need more than 500 concurrent ingress messages. That is plausible because searches are not slot-capped and fetches are not single-flight, but it was not measured. |
| O2 interface fetch | **finding L8-7** | 2-step fallback that keeps only the last error. With a cycles shortfall both steps fail synchronously, in the same execution, and the failure is cached (C). |
| LIM9 `MAX_SEARCHES` | holds | It bounds the searches per request, and they run sequentially. It does not bound concurrency across requests. That gap is LIM12's and is already reported (search not slot-capped). |
| LIM12 `MAX_CONCURRENT_RUNS` | not new | The search exclusion is already reported. Here it is only what makes the ingress-flood vector plausible. |
| LIM18 TTLs | **finding L8-7** (symptom) | The 5-minute TTL is applied to transient errors that are retryable right away. Any caller who arrives in that window is refused, even if the very next fetch would have succeeded. |
| A6 allowlisted target | holds (not caller-controlled) | The pinning vector needs an allowlisted target that never replies to an unbounded call. demo_store answers every method at once and exports `tmp_hack`. A token holder cannot add targets. In production, a slow or stopping target is in the hands of a third party, not the token holder. |

## Not tested

- **The ingress flood.** More than 500 concurrent searches against an uncached target would
  need a load test on the shared instance, which belongs to the Live phase. Whether the queue
  to `aaaaa-aa` actually fills was also not checked: management messages may be inducted
  every round, so it might never reach 500.
- **Cycle exhaustion by pinned unbounded calls.** The per-call reservation (the task's figure
  of 42.8B) was not measured. No silent allowlisted target exists on the demo deployment.
- **The production bridge off-chain.** It cannot run there (register #58), so no native test
  drives `candid()` with an injected `CallPerformFailed` or `SysTransient`. Harness needed: a
  trait seam over `Call` in `IcBridge`, so that fault injection can return each `CallFailed`
  variant, and then a check of what lands in `DID_FAILED`.
