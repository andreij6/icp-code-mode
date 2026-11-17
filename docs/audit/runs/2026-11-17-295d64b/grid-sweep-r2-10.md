# Variant sweep r2-10: a concurrency cap that covers one kind of suspended request but not a sibling

Run `2026-11-17-295d64b`, Phase 6, round 2. Source finding: searches take no run slot, so
concurrent searches suspended on interface fetches are not capped by `MAX_CONCURRENT_RUNS`
and not counted in `active_runs`.

Pattern, stated generally: **a cap on things that are suspended at once (runs, requests per
round, in-flight calls) counts one kind of suspended work, while a sibling kind that waits
on the same awaits is not counted.**

For each row the question is: does this row cap or count concurrently suspended work, and
is there other work that suspends on the same awaits and escapes the count? Known findings
are marked **known** and not reported again.

Evidence key: **R** = reproduced here; **CP** = code path unambiguous; **R-prior** = cited grid.

## Result

One new candidate:

- **SW10-1 (CP).** When a run traps in a reply callback, ic-cdk's cleanup drops the run's
  task, which returns its `RunSlot` and removes it from `PENDING_RUNS`. The other calls of
  the same `join_all` batch are still outstanding at the system level (ic-cdk only marks
  their state `Trapped`; `ic-cdk-0.20.3/src/call.rs:943-950, 977-981, 1003-1038`). Those
  orphaned calls keep waiting, but neither the run cap nor `pending_run_ids` counts them.

One observation that is not a finding: the demo page's comparison fans out 12 concurrent
`execute_icp_script` requests. That did not trip the 8-run cap on the local network
(see C5).

## Cells

### Components (C)

| Row | Cell |
|---|---|
| C1 icp_code_mode | **SW10-1** (new) and **known** (searches have no run slot). The caps on concurrent work are `MAX_CONCURRENT_RUNS` (`src/lib.rs:198`), JS `MAX_PENDING` (`src/runtimes/js.rs:28`) and Rhai sequential replay. Each is checked below |
| C1a feature subsets | holds: every language goes through `run_script` → `RunSlot::take` before `LanguageRouter::execute` (`src/lib.rs:218-225`), so no language escapes the run cap |
| C2 demo_store | n/a: it has no awaits, so nothing is ever suspended |
| C3, C4 shims/vendored | n/a: no suspension points |
| C5 demo page | holds for the page's own cap: `rpc()` and `runComparison()` disable every button while they run (`src/demo.html:215, 439`). The comparison's classic side does `Promise.all` over products (12 seeded) with one `execute_icp_script` HTTP request each (`src/demo.html:311-312, 386-389, 446-449`), which exceeds the server's 8-run cap. **R**: 12 concurrent single-call scripts against the local gateway gave 0 "busy" (each finished before the next was executed). 30 concurrent gave 1 "busy". On a subnet where all 12 land in one block, 4 would be refused. Recorded as unverified: not tested on mainnet |
| C6 demo-setup.sh | n/a: one sequential admin call per method |
| C7-C11 build/interface | n/a |
| C12 video | n/a: one Chrome instance driven sequentially |
| C13, C14 | n/a |
| C15 exports | n/a |

### Actors (A)

| Row | Cell |
|---|---|
| A1 anonymous web | n/a: query path, no suspension |
| A2 direct ingress | holds: direct ingress reaches the same `http_update` → `run_tool`, so the same cap applies |
| A3 token holder | **known** (searches uncapped). **SW10-1** gives a token holder a second way to keep calls in flight without holding a slot. It needs an allowlisted target whose reply makes the canister trap |
| A4 AI model | n/a |
| A5 controller | n/a: admin methods do not suspend |
| A6 allowlisted target | **SW10-1**: a target that traps the canister's reply handling (known trap triggers: nested reply bytes, nested interface text) turns the batch's other calls into uncounted, orphaned calls |
| A7 management canister | holds: interface fetches to `aaaaa-aa` run inside the run's bridge and so hold its slot (searches **known**) |
| A8 other canisters | holds: same `run_script` path |
| A9-A12 | n/a |

### Entry points (E)

| Row | Cell |
|---|---|
| E1, E2 init/post_upgrade | n/a: they do not suspend. Heap counters reset on upgrade (K11 below) |
| E3 inspect_message | n/a |
| E4, E4a, E4b query | n/a: queries cannot await (no composite_query) |
| E5 http_request_update | holds within one request: the batch runs its tools one at a time (`src/mcp.rs:395-416`, `exec(...).await` inside the loop), so a request has at most one suspended tool. Across requests, the run cap counts scripts only (**known** for searches) |
| E5a execute_icp_script | **SW10-1**: the slot is returned in cleanup (`src/lib.rs:202-203, 212-216`) even when calls of the same batch are still outstanding |
| E5b search | **known** |
| E5c reply callbacks | **SW10-1**: the callback for an orphaned call later finds the state `Trapped` and traps ("Call already trapped", `call.rs:981`). Nothing is counted while it waits |
| E5d cleanup callback | **SW10-1** root: `IcBridge::drop` removes the run from `PENDING_RUNS` (`src/bridge.rs:243-246`), and `RunSlot::drop` decrements `ACTIVE_RUNS`. Neither checks whether the batch still has messages in flight |
| E6-E14 admin | n/a: no awaits |
| E15 exports | n/a |
| E16 JS icp.call / icp.candid | holds: `MAX_PENDING` counts both kinds of request (`src/runtimes/js.rs:250-273`; `request_fn` serves both `call` and `candid`). The getter bypass is **known** |
| E17 console | n/a: synchronous |
| E18 Python | **known** (no per-batch cap for `call_async`). Sync `icp.call`, `icp.candid` and eager awaits are dispatched one at a time (`src/runtimes/python.rs:251-254`) |
| E19 Rhai | holds: one request per replay, so at most one is ever in flight. `MAX_REQUESTS` counts calls and candid alike (`src/runtimes/rhai_rt.rs:211`) |
| E20-E25 demo_store | n/a: no awaits |
| E26-E28 | see C5, C6, C12 |

### Untrusted inputs (U)

| Row | Cell |
|---|---|
| U1-U5, U12 | n/a: parsed before any suspension |
| U6, U7 script code and values | holds: scripts choose how much work is in flight, and that work is capped as described under E16-E19 |
| U8 reply bytes | **SW10-1** trigger: a reply that traps the decoder (**known** trap) while sibling calls are in flight |
| U9 rejects | holds: a reject is an `Err` and does not trap |
| U10 interface text | **SW10-1** trigger: an interface that traps `Service::parse` (**known** trap) inside a batch |
| U11 admin inputs | n/a |
| U13-U16 | n/a |

### Outbound effects (O)

| Row | Cell |
|---|---|
| O1 script calls | **SW10-1**: unbounded-wait calls orphaned by a trap stay outstanding, with no slot and no pending id. This also means the bound in the **known** finding about prepayment held by in-flight calls (8 runs x 25 calls) is not a real bound |
| O2 interface fetches | **known** (2 messages per slot; duplicate fetches on concurrent cache misses). They are in flight inside the run, so they hold its slot. They can be orphaned the same way as O1 (**SW10-1**) but are bounded-wait (300 s) |
| O3-O10 | n/a: synchronous writes or non-suspending effects |

### Durable state (D)

| Row | Cell |
|---|---|
| D1-D10 | n/a: durable state does not count concurrent work. The `Trapped` entry for an orphaning run lists the messages sent (`methods`), so the audit itself still shows them |

### Volatile shared state (V)

| Row | Cell |
|---|---|
| V1 ACTIVE_RUNS | **known** (searches not counted) + **SW10-1** (orphaned calls of a trapped run are not counted) |
| V2 DID_CACHE | **known** (no single-flight) |
| V3 PENDING_RUNS | **SW10-1**: documented as runs "still waiting on a reply" (`src/lib.rs:34-35`, `icp_code_mode.did:28`, README.md:415-417). After a trap the run is removed while its orphaned calls are still waiting |
| V4 DID_FAILED, V5 OVERRIDES | n/a: caches, not caps |
| V6 Monty LIVE/BASELINE memory | holds for this pattern: `rebased` wraps both kinds of Python await, the sync `host()` and the `join_all` batch (`src/runtimes/python.rs:252, 262`). Host replies not counted is **known** |
| V7 getrandom stream | n/a |
| V8 Boa GC heap | **known** (process-wide GC charges other runs) |
| V9 ic-cdk executor | **SW10-1**: the executor drops the task, but the system keeps the call context open while orphaned calls are outstanding |
| V10, V11 | n/a |

### Commit points (K)

| Row | Cell |
|---|---|
| K1 first-pending audit entry | holds: orphaned calls already have their `Started`/`Call` entries |
| K2 JS join_all | **SW10-1**: JS code never runs while a batch is in flight. A trap inside one batch member's Rust continuation (decode or parse) drops the whole batch |
| K3 Python awaits | same as K2 for the `gather` batch |
| K4 Rhai | holds: one request in flight, so a trap leaves nothing orphaned |
| K5 search awaits | holds: fetches are sequential, so there is no sibling in flight (the cap gap itself is **known**) |
| K6 end of update | holds: by K6 every call has completed |
| K7 cleanup | **SW10-1** |
| K8, K9 | n/a: nothing sent |
| K10, K12 | n/a |
| K11 upgrade while suspended | not tested: an upgrade resets `ACTIVE_RUNS` and `PENDING_RUNS` while pre-upgrade calls may still be outstanding. That needs a controller upgrade without stopping, which this phase must not do |

### Limits (LIM)

| Row | Cell |
|---|---|
| LIM1 call cap | **known** (fetch fallback; duplicate fetches). It is a per-run count, so orphaned calls from SW10-1 stay within their own run's cap |
| LIM2 instruction budget | n/a for concurrency |
| LIM3 call_timeout | **known** (not applied to fetches). With a timeout set, SW10-1's orphaned calls end within the timeout. The default is unbounded |
| LIM4-LIM8 | n/a |
| LIM9, LIM10 per-request caps | **known** (query path skips MAX_SEARCHES). Within a request, tools are sequential (E5) |
| LIM11 | n/a |
| LIM12 MAX_CONCURRENT_RUNS | **known** + **SW10-1** |
| LIM13-LIM24 | n/a: size, depth and per-run limits, not caps on concurrent work |
| LIM25 JS MAX_ROUNDS/MAX_PENDING | holds: counts call and candid (E16). Getter bypass **known** |
| LIM26 Python suspensions | **known** (no per-batch cap) |
| LIM27 Rhai MAX_REQUESTS | holds (E19) |
| LIM28-LIM32 | n/a |

### Unbounded work (W)

| Row | Cell |
|---|---|
| W1-W8, W10, W12, W13, W15, W16 | n/a: synchronous work |
| W9 JS rounds x pending | holds: the 64 per round count both kinds of request |
| W11 Rhai answers x 8 runs | holds: bounded by the run cap, and Rhai is never concurrent within a run |
| W14 PENDING_RUNS growth | **SW10-1**: the set shrinks on trap while outstanding calls do not |

### Failure sites (F)

| Row | Cell |
|---|---|
| F1-F3, F5-F11, F15, F16 | n/a |
| F4 audit append trap | **known** (cleanup trap leaks the slot, the opposite direction) |
| F12-F14 stack, OOM and instruction traps after an await | **SW10-1** triggers. Only traps inside the Rust continuation of one batch member (decode, parse, record_message) orphan siblings, because script code runs only after the batch completes |

### Boundary conversions (B)

| Row | Cell |
|---|---|
| B5, B6, B7 | SW10-1 trigger sites (decode_reply, Service::parse). Otherwise n/a |
| all other B rows | n/a: no suspension |

### Value flows (VF)

| Row | Cell |
|---|---|
| VF1 cycles | **SW10-1**: each orphaned unbounded-wait call keeps its response prepayment reserved until the target replies, with no cap on how many accumulate across runs |
| VF5 | **known** (no rate limit) |
| VF2-VF4, VF6-VF8 | n/a |

### Admin surface (AD)

| Row | Cell |
|---|---|
| AD4 set_limits | `call_timeout_seconds` bounds how long SW10-1's orphaned calls last. It is off by default (**known**: set_limits resets it when omitted) |
| AD2 allowlist | SW10-1 needs a target that traps the reply handling to be allowlisted (A6) |
| others | n/a |

### Supply chain (S)

| Row | Cell |
|---|---|
| S2 ic-cdk 0.20.3 / ic-cdk-executor 2.0.0 | **SW10-1** mechanism: `cleanup` → `cancel_all_tasks_attached_to_current_method` drops the task. `CallFuture::drop` while recovering only sets `Trapped`. There is no way to cancel an outstanding call |
| other S rows | n/a |

### Claims (CL)

| Row | Cell |
|---|---|
| CL9 "At most 8 scripts run at once" | holds literally (scripts), with **known** searches. SW10-1 is about calls, not scripts |
| CL20 "pending or unknown"; CL31 `pending_run_ids` "still waiting on a reply", `active_runs` | **SW10-1**: a `Trapped` run can still have calls waiting. It is not listed as pending |
| CL36 "Searches are cheap", exempt | **known** |
| other CL rows | n/a for this pattern |

### Tests (T)

| Row | Cell |
|---|---|
| T1 run_slots_are_capped_and_returned | asserts slot return on drop, the very behaviour that orphans siblings in SW10-1. There is no test for in-flight calls after a trap (no IC harness, register #58) |
| other T rows | n/a |
