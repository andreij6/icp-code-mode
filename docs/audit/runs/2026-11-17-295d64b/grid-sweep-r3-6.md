# Variant sweep r3-6: in-engine limits that are per frame, per turn or per item

Run `2026-11-17-295d64b`, Phase 6, round 3. Pattern (seed, known, not re-reported: JS checks
the instruction budget only between host round-trips, and Boa's 1M loop limit is per call
frame):

> A limit enforced inside the engine in sibling runtimes (Rhai `on_progress`, Monty clock)
> but only at host boundaries in this one. A per-frame or per-item counter that multiplies
> with call depth or iteration count.

Question asked of every inventory row: does a counter or cap here reset per frame, per turn,
per replay, per round, per value or per item, so that the real total is that cap times a
count the caller controls; and is any budget enforced only at a boundary in one variant while
its siblings enforce it inside the engine?

Evidence: **R** = reproduced, **C** = code path. One native probe was built in a private copy
(no `target/`, `.icp/`, `design/`, `video/`, `.env`, `docs/per-customer-canisters-plan.md`)
with the shared `CARGO_TARGET_DIR`; the copy was deleted afterwards. No live canister was
touched.

## Candidate

**SW3-6-1 (Medium).** Python's `MAX_TURN_INSTRUCTIONS` cap is per turn, not per message, so
with `max_instructions` above about 40e9 Python reaches the IC's 40B trap as JS and Rhai do.
L14-2 says Python is unaffected.

- `src/runtimes/python.rs:29-31`: "Longest single stretch between suspensions ... below the
  IC's 40B limit per message, whatever the policy budget is". `python.rs:223-224`:
  `max_feed_duration(budget)`, `max_turn_duration(budget.min(30e9))`, where
  `budget = deadline - now`. With the default 20e9 the turn cap equals the feed cap and has no
  effect. It only matters when `max_instructions` > 30e9, the case it was written for.
- Monty resets the turn clock at every host resume (`monty-types-1.6.0/src/resource.rs:110-113,
  757-763`; `monty-1.6.0/src/repl.rs:1368-1375`). Turns are not message executions. The driver
  answers `icp.principal()` (`python.rs:133`), `import icp` (OsCall, `:266-270`), name lookups
  (`:265`), a policy-refused `icp.call` (`reserve` errors before any await) and a cached
  `icp.candid` without awaiting. So `rebased(host(..)).await` completes on its first poll and
  the next turn runs in the same IC message execution. Up to 256 turns chain this way, and
  each may use the full 30e9.
- R (native probe, Monty 1.6.0 driven like `python.rs`, wall clock): one 2M-iteration loop
  took 354 ms. With turn cap = 532 ms (1.5x) and a large feed cap, three such loops with no
  host call failed with `TimeoutError: turn time limit exceeded`. Six loops separated by
  `icp.principal()`, each answered synchronously in the same poll, finished `Ok` in 2.14 s:
  4.0x the turn cap, 7 turns, all inside one synchronous drive.
- C (on wasm): with `max_instructions = 60e9` (accepted unvalidated, L14-2), the script
  `import icp` / 29e9-instruction loop / `icp.principal()` / 29e9-instruction loop exceeds
  40e9 in one execution. That traps the canister. No message was sent, so there is no audit
  entry (K8).
- Fix: cap the turn at what is left of the current message, not per turn. Either have the
  driver call `budget_error` against `min(deadline, message_start + 30e9)` and force an
  `await` (yield) between turns, or validate `max_instructions` <= ~30e9 in `set_limits` /
  `apply` (L14-2's fix). Also fix the `MAX_TURN_INSTRUCTIONS` comment and L14-2's "Python
  unaffected".
- Regression test: needs the instruction-counter seam (register #58). Natively, a test can
  assert that a Python run with turn cap T and k synchronous host answers stops at T in
  total, not at k x T.

## Grid

### Components, actors

| Row | Cell |
|---|---|
| C1 | **SW3-6-1** (Python runtime); JS per-frame loop limit known; Rhai holds (budget per op, `rhai_rt.rs:143-153`) |
| C1a | holds: feature subsets compile the same runtime code; no per-build counter |
| C2 | known (per-line stock check, oversell); no other per-item counter |
| C3 | holds: `Instant` is the call-context counter, so Monty's clock is absolute; the per-turn reset is Monty's, not the shim's (SW3-6-1) |
| C4-C15 | n/a: no engine limit or counter |
| A1-A12 | n/a: actors; reach via E rows |

### Entry points

| Row | Cell |
|---|---|
| E1/E2 | n/a: no counter; limits stored unvalidated (L14-2 known) |
| E3 | holds: one decode and one hash per ingress |
| E4/E4a/E4b | known: query path applies no instruction budget; 50 cached searches per batch |
| E5 | holds for this pattern: 1 script + 5 searches per request. Each run's budget is relative to its own start on the call-context counter (`bridge.rs:128-133`). Several runs in one execution can only exceed 40B through search render cost, which is known. The 2 MiB ingress is shared by all searches' keywords, so 5 searches do not multiply it (L3-7 measured 2.4B per search at the ingress limit) |
| E5a | **SW3-6-1** (Python); JS known |
| E5b | known (own call cap per search, render cost checked after render) |
| E5c/E5d | holds: no per-callback counter |
| E6-E14 | n/a (admin; L14 known) |
| E15 | n/a |
| E16/E17 (JS host fns) | known: loop limit per frame, budget only at round boundaries / every 256 jobs. Holds: `MAX_PROMISE_JOBS` is per run (`BoundedJobs` is built once per `Context`, `js.rs:281`), not per round; recursion limit counts frames plus `host_call_depth` together (r1-11); stack-size limit is VM-global |
| E18 (Python host fns) | **SW3-6-1**: every synchronously answered `icp.*`, `import icp` or name lookup starts a new Monty turn in the same message |
| E19 (Rhai host fns) | holds: `max_operations` is per replay (README CL13 holds per L16), but the budget is an absolute deadline checked on every op, and per-replay log counters are cleared with the logs (`rhai_rt.rs:191-195`), so nothing multiplies past the budget |
| E20-E25 | known (demo_store per-line check) / n/a |
| E26-E28 | n/a |

### Untrusted inputs, outbound effects

| Row | Cell |
|---|---|
| U1-U4 | holds: per-request parse, batch 50 counted once |
| U5 | known (keyword count unbounded) |
| U6 | known (Boa compile quadratic; Python `exec`/`eval` compile). Holds for the Monty main-script compile (linear, r2-0) |
| U7 | known: JS console per-argument fresh budget; Python `py_repr` uncharged; refused-call snapshots in JS run in synchronous JS (seed) |
| U8 | known (decode cost, quota); each reply is decoded in its own callback execution |
| U9-U16 | n/a: no per-item counter |
| O1 | holds: call cap per run, checked in `reserve` with the absolute deadline |
| O2 | known (one slot, two messages) |
| O3-O10 | n/a |

### State, commit points

| Row | Cell |
|---|---|
| D1 (`max_instructions`) | **SW3-6-1** for values > 30e9 (variant of L14-2) |
| D1 (other fields), D2-D10 | n/a |
| V1 | holds: one counter per process |
| V2-V5 | known (no single-flight) / n/a |
| V6 | holds for this pattern: rebased around awaits; synchronous turns share one baseline. Host replies not counted is known |
| V7-V11 | n/a |
| K1 | holds |
| K2 (JS rounds) | known: refused rounds chain in one execution, sync code unchecked (seed) |
| K3 (Python awaits) | **SW3-6-1**: a host answer without an await is a Monty turn boundary but not a commit point |
| K4 (Rhai replays) | holds: refused requests chain replays in one execution, but `on_progress` checks the absolute deadline every op (above 40e9: L14-2 known) |
| K5-K12 | n/a |

### Limits

| Row | Cell |
|---|---|
| LIM1 | holds per run; per-search caps known |
| LIM2 | **SW3-6-1** (Python turn cap); JS known; Rhai holds |
| LIM3 | known (per call, multiplies with calls) |
| LIM4 | n/a |
| LIM5 | known (page by count) |
| LIM6, LIM7 | holds: per result, and the total reply is checked |
| LIM8-LIM11 | holds / known (LIM9 query path) |
| LIM12 | known (searches excluded; slot held across waits) |
| LIM13, LIM14 | holds per entry / per run; overshoot by markers known |
| LIM15 | holds per run in JS (formatting cost known). In Rhai it is per replay, but the logs are cleared each replay, so output is not multiplied. Python known |
| LIM16 | known (per value, fresh per call or argument) |
| LIM17-LIM22 | holds: one check per conversion; per-reply decode in its own execution |
| LIM23 | holds per run |
| LIM24 | known (seed: loop limit per frame) |
| LIM25 | holds: rounds, pending and promise jobs are per run; known getter bypass of `MAX_PENDING` |
| LIM26 | **SW3-6-1** (turn <= 30e9 is per turn); feed one-time allowance known (SW1-1); suspensions per run holds |
| LIM27 | holds (see E19) |
| LIM28-LIM32 | n/a / holds |

### Unbounded work, failure sites, conversions

| Row | Cell |
|---|---|
| W1, W2 | holds |
| W3, W4 | known |
| W5, W6, W7 | known |
| W8, W9 | known (seed) |
| W10 | **SW3-6-1**; other parts known |
| W11 | holds (per-op budget; retention measured by L4) |
| W12-W16 | holds / known / n/a |
| F14 | **SW3-6-1** adds a Python route to the 40B trap when `max_instructions` > 40e9 |
| F1-F13, F15, F16 | n/a for this pattern |
| B1-B17 | holds / known: every conversion budget is per value (known variants listed above) |

### Value flows, admin, supply chain, claims, tests

| Row | Cell |
|---|---|
| VF1, VF5 | **SW3-6-1** (under that configuration, a run burns 40B per request and leaves no record) |
| VF2-VF8 | n/a |
| AD4 | **SW3-6-1** (variant of L14-2) |
| AD1-AD3, AD5-AD9 | n/a |
| S1-S10 | n/a |
| CL8 | known (Python budget text) |
| CL11 | known (JS "1M loop iterations") |
| CL12 | holds as worded (no turn claim) |
| Code claim `python.rs:29-31` | **SW3-6-1** ("below the IC's 40B limit per message, whatever the policy budget is" is false) |
| other CL rows | n/a for this pattern |
| T9 | gap: no Python test with synchronous host answers between long turns; needs the counter seam (register #58) |
| T1-T8, T10-T12 | n/a |

## Not tested

- The wasm trap itself for SW3-6-1. It needs `max_instructions` > 40e9 on a live canister,
  which this phase may not set. The native probe shows the per-turn reset on a wall clock. The
  on-chain chain (synchronous host answer, no await, same execution) is shown by the code
  path.
