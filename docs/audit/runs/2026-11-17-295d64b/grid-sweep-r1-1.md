# Variant sweep r1-1

Pattern: "Check-then-await-then-act: policy evaluated before a suspension point and relied on
after it (same family as the register's 'process-wide state read before a suspension and
trusted after it')."

Question asked of every inventory row: does this row evaluate a policy (allowlist, method
list, call cap, instruction budget, token, run-slot cap, override precedence, failure cache,
size/count limits, limits snapshot) before a suspension point, and then act on that decision
after the suspension, when the decision may no longer hold?

Suspension points found (`grep await src/ demo_store/ src/demo.html video/`):
`bridge.rs:388` (`sent`), `:429`, `:435` (`fetch_did`), `:447`, `:489` (`candid`), `:516`,
`:528` (`call`); `lib.rs:137` (`run_search`); `mcp.rs:401` (one `exec` per batch message);
`js.rs:400` (`join_all` per round); `python.rs:252`, `:262` (`rebased(host)`,
`rebased(join_all)`); `rhai_rt.rs:215-216`; client-side awaits in `src/demo.html` and
`video/*.mjs`. demo_store has no `async` code.

Evidence key: **R** = reproduced on the local network (port 8010, canister
`4caro-hl777-77775-aaaba-cai`, target demo_store `4fbx2-kt777-77775-aaabq-cai`, default limits,
ordinary `execute_icp_script` requests with the test token from `.env`, audit read with the
controller query `get_audit_logs`); **C** = code path. No build was needed; no setting was
changed; nothing was deployed.

## New candidate

| Id | Severity | Title | Evidence |
|---|---|---|---|
| SW1-1 | Medium | Python's VM budget is the policy budget left at the start, and Monty's clock pauses while the host works, so host work done during suspensions is never charged: Python runs past `max_instructions` (1.5x measured) and up to the IC's 40B trap | R |

**SW1-1.** `python.rs:219` computes `vm_budget(bridge)` once, before the first suspension, as
`instruction_deadline() - instruction_counter()` (`python.rs:37-43`), and hands it to Monty as
`max_feed_duration` (`python.rs:223`). Monty's feed clock "is paused while suspended on the
host" and counts only VM execution (`monty-types-1.6.0/src/resource.rs:176-179, 289-293,
722-745`). Everything the host does between suspensions is outside that clock: `to_json` of
the arguments (`python.rs:244`), the `a.clone()` and `IcBridge::call` (reserve, encode,
decode, `from_json`), interface parsing after a fetch. After those suspensions the VM still
has its whole start-of-run allowance, so the decision "this run may spend N more instructions"
taken before the suspensions is relied on after host work has already spent part of N. The
bridge's live check (`reserve`) refuses further calls once over budget, but the refusal is a
catchable Python exception raised after the argument conversion, so neither the conversions
nor the VM stop. JS (`State.deadline`, absolute, checked in `request_fn` before conversion and
after every round) and Rhai (`on_progress` against the absolute deadline) do not have this gap.

Reproduction (audit ids from `get_audit_logs`):

| Script | Result | Audit |
|---|---|---|
| Python: one `list_products` call, then `while True: n += 1` | `instruction budget exceeded (20003140147 > 20000000000)` | id 891, 20.0B |
| Python: one call, `big = ["abcdefghijklmnopqrst"] * 199000`, 15 x `try: icp.call(S, "get_product", big)` (each fails to encode, no message), then the same spin | `instruction budget exceeded (29522812596 > 20000000000)` | id 885, 29.5B (1.48x the budget) |
| Same with 30 calls | HTTP 503, "Canister exceeded the limit of 40000000000 instructions for single message execution"; result discarded | id 887 `Trapped`, `instructions` 1,958,100 (known undercount) |
| Rhai: one call, 9,999-element string array, 15 x the failing call, then `loop {}` | `instruction budget exceeded (20020889327 > 20000000000)` | id 889, 20.02B |

Without the first real call the 40B trap happens before any message is sent, so the run leaves
no audit entry at all (register #11's accepted case, now reachable by budget overrun rather than
by a single native operation). Fix: give Monty a per-turn allowance recomputed from the live
deadline at every resume (`max_turn_duration = deadline - now` on each `resume`), or check
`budget_error(deadline)` in the driver before converting arguments and fail the run
uncatchably when it is spent. Regression test: needs register #58 / T-1's instruction-counter
seam; on-chain, a Python run with host work then a spin must end within one host call of the
budget.

## Volatile state, commit points and suspension sites

| Row | Cell | Reason |
|---|---|---|
| K1 `sent` / `record_message` | holds | Entry written in the execution that hands the message over; no policy decision carried across |
| K2 JS `join_all` round (`js.rs:391-400`) | known (L8-3) for the bridge re-check; holds for the engine | `request_fn` checks `budget(st)` (absolute deadline) before conversion; `check_budget(bridge)` after every `run_jobs`; `MAX_PENDING` getter case is a known finding |
| K3 Python awaits (`python.rs:252, 262`) | **SW1-1**; L8-2 known (`rebased`) | VM budget fixed before the suspensions, host time excluded |
| K4 Rhai replay await (`rhai_rt.rs:211-218`) | holds | `answers.len() >= MAX_REQUESTS` checked and pushed by the same sequential run; budget is the absolute deadline in `on_progress`; cached answers (including refusals) only replay, never send |
| K5 `run_search` (`lib.rs:129-149`) | known (L8-3 mid-search snapshot, `halted` heuristic) | `list_allowed()` taken before the loop |
| K5 `fetch_did` fallback (`bridge.rs:429-435`) | known (L1-2) | Fallback message sent after the `canister_metadata` await on the slot reserved before it |
| K6 closing entry | known (L8-4) | Live `max_instructions` vs deadline captured at bridge creation |
| K7 cleanup | n/a | No policy decision |
| K8-K10, K12 | n/a | Single executions, no suspension |
| K11 upgrade in flight | n/a | Heap reset; callbacks dropped, nothing acted on afterwards |
| V1 `ACTIVE_RUNS` | holds | `RunSlot::take` decides and increments in one step; the slot is held, not re-decided |
| V2 `DID_CACHE` | known (L8-3 / L1-1 variant) | Unconditional insert after the fetch await undoes a `remove_allowlist` eviction |
| V3 `PENDING_RUNS` | holds | Inserted in the send execution, removed on drop |
| V4 `DID_FAILED` | known (same root as L8-3) | A failure recorded after the await re-inserts an entry `remove_allowlist` just evicted; if the canister is re-added within 5 min every request gets the stale "could not fetch" error. Symptom of the unconditional post-await insert, not a separate root cause |
| V5 `OVERRIDES` / D5 override precedence | hit, no wrong outcome: holds | `service()` checks the override before the fetch and, after `self.candid(&key).await`, takes `cache_get` without re-checking it (`bridge.rs:439-448`); `candid()` likewise returns the fetched text. An override set during that window is not applied to the one request already waiting; the encoding then follows the target's own published interface, which goes only to that target. Every later lookup sees the override first. No harm demonstrated, so not a candidate |
| V6 `LIVE_MEMORY` / `BASELINE_MEMORY` | known (L8-2; register #43 fixed) | Re-based after every await |
| V7 getrandom `STATE` | known (stream carries over) | Not a policy |
| V8 Boa GC heap | known (L8-1) | Not a policy decision |
| V9 executor, V10 engine statics | n/a / not tested | No policy read |
| V11 demo_store `STORE` | n/a | No await |

## Entry points and host functions

| Row | Cell | Reason |
|---|---|---|
| E1, E2 init/post_upgrade | n/a | Synchronous |
| E3 `inspect_message` -> E5 | holds | `http_request_update` authenticates again (`mcp.rs:375`); nothing from the inspection is trusted |
| E4/E4a/E4b query -> gateway upgrade -> E5 | holds | The query's decision (`upgrade: true`, `certified_reads`) is not relied on; the update path re-authenticates and re-decides everything |
| E5 batch loop (`mcp.rs:386-402`) | known ("revoked tokens") | Token checked once per HTTP request, then up to 5 searches + 1 script awaited in sequence. `ran_script` / `searches` counters are per-request locals: hold |
| E5a `run_script` | **SW1-1** (Python); known L8-3, L8-4 | Slot taken once and held (holds); bridge decisions as above |
| E5b `run_search` | known (L8-3, L4-4) | |
| E5c reply callbacks | **SW1-1** | Host work in callbacks not charged to Python's VM clock |
| E5d cleanup | n/a | |
| E6-E11 admin updates | holds as the *changing* side | Each is one execution; their effect on in-flight runs is covered by L1-2/L8-3/L8-4 (and V5 above) |
| E12-E14 admin queries | n/a | Synchronous |
| E15 | n/a | |
| E16 JS host functions | holds | Budget checked before conversion and before queuing; `principal` captured but constant |
| E17 JS console | holds | `budget(st)` on every call |
| E18 Python host functions | **SW1-1** | Arguments converted (`python.rs:244`) before any budget check, VM clock paused meanwhile |
| E19 Rhai host functions | holds | `to_json_value` runs inside one operation; `on_progress` checks the absolute deadline on the next one |
| E20-E25 demo_store | n/a | No `async`; `create_order` checks stock and reserves in one execution |
| E26 demo page (`src/demo.html`) | n/a | Client awaits read the token from the input on every request (`rpc`, `post`); the search-to-script splice is a known finding; no policy decision carried |
| E27 `scripts/demo-setup.sh` | n/a | Sequential CLI calls, no shared decision |
| E28 video scripts | known | `cdp.mjs:9-23` polls `127.0.0.1:9333` then connects and later sends the token to whatever answered: the known "sends the MCP token to whatever process answers on 127.0.0.1:9333" |

## Outbound effects

| Row | Cell | Reason |
|---|---|---|
| O1 script calls | known (L8-3: allowlist, method, budget checked in `reserve` before `service().await`, parse and encode, then sent without a re-check); **SW1-1** for Python's own budget | The budget part of `reserve` before encode is listed in L8-3's K2 cell, so not re-reported |
| O2 interface fetch | known (L1-2 fallback; L4-1, L4-2) | |
| O3 audit writes | holds | Written in the execution of the event they record |
| O4 admin writes | n/a | |
| O5-O9 | n/a | No suspension between decision and effect |
| O10 video tooling | known | See E28 |

## Limits and admin surface

| Row | Cell | Reason |
|---|---|---|
| LIM1 call cap | holds | Slot taken before the await, so concurrent requests are counted; lowering the cap mid-run is read live (L8-4 covers the inconsistency) |
| LIM2 `max_instructions` | **SW1-1** (Python); known L8-4 | JS/Rhai deadlines are absolute instruction-counter values, so host work is charged; Python's is a duration fixed at start |
| LIM3 `call_timeout_seconds` | known (L8-4) | Captured at bridge creation |
| LIM4 `certified_reads` | holds | Read per request, never carried across an await |
| LIM5-LIM8, LIM11, LIM13-LIM17, LIM19-LIM23, LIM31, LIM32 | n/a | Checked and acted on in one execution |
| LIM9 `MAX_SEARCHES`, LIM10 | holds for this pattern | Per-request counters; query/update mismatch is known |
| LIM12 `MAX_CONCURRENT_RUNS` | holds | Decision is the held slot itself |
| LIM18 TTLs | known (V2/V4) | Insert after the await ignores an eviction in between |
| LIM24, LIM25 JS limits | holds | Per-call and per-round checks; getter case known |
| LIM26 Python limits | **SW1-1** | `max_feed_duration`/`max_turn_duration` from the start-of-run remainder; suspensions count is sequential (holds) |
| LIM27 Rhai limits | holds | |
| LIM28-LIM30 | n/a / **SW1-1** reaches LIM30 (40B per message) | |
| AD1-AD9 | holds as the changing side; consequences are L1-2, L8-3, L8-4, V5 | No admin method itself awaits |

## Other inventories

| Rows | Cell | Reason |
|---|---|---|
| C1, C1a | covered by the rows above | C1a Python-only builds have SW1-1; Rhai-only and JS-only do not |
| C2 | n/a | No `async` |
| C3 web-time shim | holds | `Instant` is the call-context counter, monotonic across awaits; the gap in SW1-1 is Monty pausing its clock, not the shim |
| C4-C11, C13-C15 | n/a | No suspension or no policy |
| C12 video | known (E28) | |
| A1-A12 | covered through E rows | A3 token holder is the actor of SW1-1 |
| U1-U16 | n/a except U7 | U7 script values: Python converts them before any budget check (SW1-1) |
| D1-D10 | holds | Every stable read-modify-write is one execution; D5 precedence see V5 |
| W10 Python conversions | **SW1-1** | Unbounded in count only by 256 suspensions, each up to 200k nodes / 4 MiB, none charged to the VM clock |
| W1-W9, W11-W16 | n/a or known | No decision across a suspension except those already listed |
| F14 IC 40B limit | **SW1-1** | New route to it from ordinary bounded operations |
| F1-F13, F15, F16 | n/a | |
| B1-B17 | n/a except B9 | B9 Python conversions sit in the uncharged host window (SW1-1) |
| VF1, VF5 | **SW1-1** | A Python run costs up to the 40B message limit per execution, about 2x the configured budget |
| VF2-VF4, VF6-VF8 | n/a | |
| S1-S10 | n/a | |
| CL8 instruction budget claim | **SW1-1** | "Checked before each host request": for Python the budget left at the start is what is enforced; the host work is not counted until the closing entry |
| CL1-CL7, CL9-CL37 | n/a or known | |
| T1-T12 | gap | No test can see SW1-1: instruction counter is 0 off-chain (register #58, T-1) |
| History (register) | family re-checked | Register pattern "process-wide state read before a suspension and trusted after it": #43 still fixed; L8-2 known |

## Not tested

- Whether an override set during an in-flight fetch can be turned into harm (V5): no
  scenario found where the target's own interface, used once for a request to that target,
  hurts anyone.
- SW1-1 with `call_async`/`gather` batches (`python.rs:262`): same code path (host work inside
  `rebased(join_all)` outside the VM clock), not run separately.
- The no-audit-entry variant of SW1-1 (trap before the first send) was not run; it follows
  from the 30-call run and register #11.
