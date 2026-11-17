# Coverage grid, lens L6 (Consistency across variants), round 1

Run `2026-11-17-295d64b`. Row ids from `inventory.md`. Cells: **finding** (id below), **holds**
(why), **n/a** (why). Evidence: *live* = request to the project's local network on port 8010
(read-only or ordinary token-holder requests; no deploy, no settings changed); *native* =
differential test in a private copy (`cargo test --lib`, shared target dir); *code* = code path.

## Candidate findings

| Id | Sev | Title | Evidence |
|---|---|---|---|
| L6-1 | High | JS enforces the instruction budget only between host round-trips; ordinary synchronous JS (loops inside called functions) runs to the IC's 40B trap. Python and Rhai stop in-engine | live + native |
| L6-2 | High | Python's VM budget is not checked inside a single native operation (bigint `**`, `str()` of a large list); one expression runs past the budget to the 40B trap | live + native |
| L6-3 | Medium | JS counts value text in UTF-16 code units, Python/Rhai in UTF-8 bytes: JS lets 3x the documented 4 MiB of text leave a script | native |
| L6-4 | Medium | Python `print` bypasses the shared log and value paths: it raises `MemoryError` at the 64 KiB cap (JS/Rhai truncate and succeed), and printed values are formatted without the value limits | live + native |
| L6-5 | Medium | Rhai rejects replies JS and Python accept (arrays/maps over 10,000 entries, strings over 1 MiB) after the call has executed | native |
| L6-6 | Medium | Per-language builds still advertise all three languages and default to `javascript` in `tools/list`; a Rhai-only worker fails every call that omits `language` | native |
| L6-7 | Low | Query-path search ignores the failed-fetch cache the update path honours, so every search touching a failing interface is upgraded to a paid update call | code |
| L6-8 | Low | `MAX_SEARCHES` (5 per HTTP request) is enforced on the update path only; the query path answers 8 of 8, the update path 5 of 8 for the same batch | native |
| L6-9 | Low | A value the host cannot represent is dropped (JS), stringified as `Repr('<function f at 0xc>')` (Python) or refused (Rhai) | native |
| L6-10 | Low | Depth limit off by one between runtimes: 129 nested empty lists pass in Python and Rhai, fail in JS | native |

## Grid

### Components, actors

| Row | Cell |
|---|---|
| C1 | finding L6-1, L6-2, L6-3, L6-4, L6-5, L6-9, L6-10 (runtimes are the variants) |
| C1a | finding L6-6. Router side holds: `LanguageRouter::execute` returns a clean "not compiled" error (`src/runtimes/mod.rs:63-64`), native rhai-only run confirmed |
| C2 | n/a: one implementation, no sibling |
| C3 | holds for L6: the shim makes Monty's clock the same call-context counter JS/Rhai use for the budget (`shims/web-time/src/lib.rs:16-18`, `bridge.rs:128-133`); the gap is what Monty measures (L6-2), not the clock |
| C4, C6-C15 | n/a: no equivalent sibling implementation |
| C5 | n/a: single page; it only calls `POST /mcp` (both paths reviewed under E4b/E5b) |
| A1-A12 | n/a: actors are not variants; per-path checks reviewed under E4/E5 |

### Entry points

| Row | Cell |
|---|---|
| E1/E2 | holds: both call the same `apply` and `certified_data_set` (`src/lib.rs:62-72`) |
| E3 | holds: uses the same `mcp::authenticate` as E4/E5 (`src/lib.rs:277`) |
| E4 / E4a | holds: demo page routing identical in both paths (only the query serves it) |
| E4b vs E5b | finding L6-7 (failed-fetch cache ignored on query path), L6-8 (search count cap update-only). Holds otherwise: both render with the same `search::render`, same `search::scope` canonicalisation, same override-then-cache lookup (`bridge.rs:207-213` vs `439-449`); live direct `http_request` query answered cached searches with `upgrade = null` |
| E5 | holds: shares `authenticate`, `parse_body`, `classify` with E4 |
| E5a vs E5b (run_script vs run_search) | budget: holds (both `IcBridge::reserve` per message and the same final `instructions > max` check, `src/lib.rs:151-154`, `226-231`). Audit: holds as documented (CL19: search writes entries only if it sent or was refused; script always closes). Violations/truncation: holds (same `MAX_AUDIT_ERROR_BYTES`, same `violations_dropped` handling). Slots: holds by design: searches take no `RunSlot` (`src/lib.rs:121` vs `219`); the slot exists to bound suspended VM heaps, and a search holds none (its parsed interfaces are shared `Rc`s in `DID_CACHE`). Not a finding; CL31 text says "Scripts running", which is accurate |
| E5c/E5d | holds: one `IcBridge` drop path for both scripts and searches |
| E6-E14 | n/a: no siblings (single guard `controller` on all; L1 owns it) |
| E15 | n/a |
| E16 vs E18 vs E19 (`icp.call` / `call_async` / `call_canister`) | finding L6-1, L6-2, L6-3, L6-5. Argument arity and types: holds (two-argument form sends `null` in all three; wrong types are a TypeError/function-not-found error in all three). Request counts: holds as documented: JS 64 rounds x 64 pending, Python 256 suspensions, Rhai 64 requests (native: refusing target gives "64 host round-trips" / "256 host suspensions" / "64 icp requests"); messages are bounded by the shared call cap in all three |
| E16 vs E18 vs E19 (`icp.candid`, `icp.principal`) | holds: all go through `HostBridge::candid` / `principal` |
| E17 vs E18 `print` vs E19 `print`/`debug` | finding L6-4 |
| E20-E25 | n/a: demo_store, no sibling |
| E26-E28 | n/a |

### Untrusted inputs, outbound effects, state

| Row | Cell |
|---|---|
| U5 | holds: `parse_tool_call` shared by both paths |
| U6 | holds: one `MAX_CODE_BYTES` check in the router before any engine (`src/runtimes/mod.rs:52`) |
| U7 | finding L6-3, L6-9, L6-10 |
| U8 | finding L6-5 (Rhai only); JS/Python accept the same decoded JSON (native, 10,001-element array, 10,001-key map, 1.5 MB string) |
| U9 | holds: errors from `HostBridge` reach every runtime as a catchable error (`cross_language_tests::policy_errors_surface_and_are_catchable`) |
| U1-U4, U10-U16 | n/a: one handler each, no sibling |
| O1/O2 | holds: one bridge for all runtimes and for search |
| O3 | holds: same `append_audit` for script, search, cleanup |
| O4-O10 | n/a |
| D1 (`max_instructions`) | finding L6-1, L6-2: the configured budget is honoured in-engine by Rhai (every operation) and by Python's bytecode loop, not by JS synchronous code nor Python native ops |
| D1 (`max_calls_per_script`) | holds: enforced once in `reserve` for every runtime and search |
| D2-D10 | n/a: single writer/reader path per store |
| V1 | holds: see E5a vs E5b slots |
| V2/V4/V5 | finding L6-7 (V4 consulted on the update path only) |
| V3 | holds: scripts and searches share `record_message`/`Drop` |
| V6 | n/a: Python-only by design (memory cap exists only there; JS gap is register #24, accepted) |
| V7-V11 | n/a |

### Commit points

| Row | Cell |
|---|---|
| K1 | holds: same `sent` hook for scripts and searches |
| K2 vs K3 vs K4 | holds for commit behaviour (each await ends an execution in all three). Instruction trap between awaits: finding L6-1 (JS) and L6-2 (Python); Rhai holds (budget checked on every operation, `rhai_rt.rs:143-153`) |
| K5-K12 | holds/n/a: single path each; K6/K7 shared by both tools |

### Limits

| Row | Cell |
|---|---|
| LIM1 | holds: shared `reserve` |
| LIM2 | finding L6-1, L6-2. Live: JS 12 x 999,999 iterations ran to 27.86B with budget 20B (clean error only after the fact); JS 20 x 999,999 trapped at 40B; same work in Python stopped at 20.0009B |
| LIM3, LIM4 | holds: read once per bridge / per query |
| LIM5 | n/a |
| LIM6, LIM7 | holds: one `tool_result` / `finish` for scripts and searches, both paths |
| LIM8 | holds: `parse_body` shared |
| LIM9 | finding L6-8 |
| LIM10 | holds: query upgrades any batch with a script; update enforces one |
| LIM11 | holds: shared `rpc_error` |
| LIM12 | holds (see E5a vs E5b) |
| LIM13, LIM14 | holds: shared constants for both tools |
| LIM15 | finding L6-4 |
| LIM16 | finding L6-3 (units), L6-4 (printed values in Python), L6-10 (depth) |
| LIM17-LIM21 | holds: in the shared bridge / candid_bridge |
| LIM22 | holds: one `idl_to_json` for all runtimes |
| LIM23 | holds |
| LIM24, LIM25 | finding L6-1 (the 1M loop limit is per call frame in Boa, `vm/call_frame/mod.rs:69,153`; native: 100 x 999,999 = 1e8 iterations completed) |
| LIM26 | finding L6-2 (time budget not checked inside native ops), L6-4 (print cap raises) |
| LIM27 | finding L6-5 (engine size limits also apply to replies) |
| LIM28-LIM32 | holds/n/a: shared or platform |

### Unbounded work, failure sites

| Row | Cell |
|---|---|
| W3 | holds: same `render` in both paths |
| W8 vs W10 vs W11 | finding L6-1, L6-2 |
| W9 | holds as documented (see E16-E19 request counts) |
| W1, W2, W4-W7, W12-W16 | n/a: single implementation |
| F14 | finding L6-1, L6-2 (both reach the 40B trap; README says only one Boa built-in can) |
| F1-F13, F15, F16 | n/a for L6: single site each |

### Boundary conversions

| Row | Cell |
|---|---|
| B8 vs B9 vs B10 (script -> JSON) | finding L6-3, L6-9, L6-10. Holds: non-finite floats become `null` in all three (native: JS `NaN`/`Infinity`, Python `float('nan')`/`inf`, Rhai `0.0/0.0`, `1.0/0.0`); big integers: JS refuses BigInt, Python sends exact decimal text, Rhai has no bigints (documented per language) |
| B8 vs B9 vs B10 (JSON -> script) | finding L6-5 (Rhai). Holds otherwise: same `serde_json::Value` input; integers <= 2^53 as numbers, beyond as strings from `idl_to_json` in all three |
| B11 | holds: shared `tool_result` |
| B13 | holds: one parser; but see L6-6 for what is advertised |
| B14 | holds for L6 (block time to all three clocks) |
| B1-B7, B12, B15-B17 | n/a: single implementation |

### Value flows, admin, supply chain

| Row | Cell |
|---|---|
| VF1, VF5 | finding L6-1, L6-2 (a token holder can make the canister burn 40B instructions per request in JS or Python; Rhai cannot) |
| VF2-VF4, VF6-VF8 | n/a |
| AD4 | finding L6-1, L6-2 (setting `max_instructions` lower does not bound JS synchronous code or Python native ops) |
| AD9 | finding L6-6 |
| AD1-AD3, AD5-AD8 | n/a |
| S1-S10 | n/a for L6 (L15 owns); C1a subsets compile and their tests pass (baseline) |

### Claims

| Row | Cell |
|---|---|
| CL8 | finding L6-1, L6-2 ("Checked before each host request, between JS rounds, and on every Rhai operation" is accurate for JS, but the design goal "a clean error before the 40B trap" holds for Rhai only) |
| CL9 | holds (documented per-language request counts reproduced) |
| CL10 | finding L6-3, L6-4 ("in all three languages") |
| CL12 | finding L6-2 ("The VM stops when the policy's instruction budget is spent") |
| CL14 | finding L6-4 (logs are capped in all three, but Python fails the run) |
| CL19 | holds |
| CL24 | finding L6-1, L6-2 ("Every other path stops with a clean error") |
| CL29 | finding L6-5, L6-9, L6-4 ("identical semantics in every language", shown to the AI model) |
| CL31 | holds (counts scripts, says scripts) |
| CL36 | finding L6-8 (the exemption applies only on the update path) |
| other CL | n/a for L6 |

### Tests, invariants, register

| Row | Cell |
|---|---|
| T7 | gap: `cross_language_tests` checks logic equality only; no differential test of limits (text units, depth boundary, print cap, reply sizes, budget) |
| T9 | gap: no Python print-cap test (I16) |
| T5 | gap: no test of a search batch over 5 on the query path |
| I13 | violated: L6-1 (JS), L6-2 (Python); holds for Rhai |
| I15 | violated: L6-3 (JS), L6-4 (Python printed values); L6-10 cosmetic |
| I16 | holds for the cap itself in all three; behaviour differs (L6-4) |
| Register #32 | still fixed: Python's budget is the configured one (`python.rs:37-43`); live: same loop work stopped at 20,000,868,065 instructions with `max_instructions` 20e9. Native ops escape it (L6-2, new) |
| Register #33 | still fixed: Python deep data is an error (native: 130 nested lists -> "nested deeper than 128 levels"; `python::values_are_bounded` passes) |
