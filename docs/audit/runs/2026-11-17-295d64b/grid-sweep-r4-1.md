# Variant sweep r4-1: a limit enforced in one route and missing in its sibling route

Run `2026-11-17-295d64b`, Phase 6, round 4. Pattern under test:

> A limit enforced in one route and missing in its sibling route.

Routes compared: query vs update HTTP handlers, `inspect_message` vs the method, admin
endpoints vs `InitArgs`, script vs search tools, `icp.call` vs `icp.candid`, the two
interface-fetch messages, the three runtimes, and inside each runtime the **exit routes for
script-produced data**: the result, call arguments, printed values and the **thrown value /
error**. Round 2 (`grid-sweep-r2-9.md`) swept the same pattern for per-request limits and
SDK arguments; this round covers what it did not: the error exit route, demo_store's
sibling updates, and a re-check of every r2-9 row against the known-finding list.

Method: read `src/mcp.rs`, `src/lib.rs`, `src/bridge.rs`, `src/policy.rs`,
`src/runtimes/{mod,js,python,rhai_rt}.rs`, `demo_store/src/lib.rs` at `295d64b`
(`git archive 295d64b`, because the working tree's `src/` has someone else's uncommitted
edits), plus `boa_engine-0.22.0/src/error/mod.rs:557-650, 929-944` and
`rhai-1.27.0/src/types/error.rs:180-188`, `src/serde/serialize.rs:55-86`. Native
reproductions in a private copy (`/private/tmp/claude-501/sweep-r4-1`, deleted afterwards)
with `CARGO_TARGET_DIR=/private/tmp/claude-501/audit-target`; one test module appended to the
copy's `src/runtimes/mod.rs` (text below). The test binary was copied out of the shared target
dir right after linking, because another agent's copy of the same package overwrote it twice.

Evidence key: R = reproduced natively (MockBridge, default features, debug build);
CP = code path; known = covered by a finding in the task's known list (not re-reported).

## Grid

| Row | Sibling routes compared | Verdict | Reason |
|---|---|---|---|
| U7 / B8 (JS) | result (`Fulfilled` -> `to_json_safe`) vs thrown non-Error (`error_string` -> `to_json_safe`) vs **thrown `Error` (`error_string` -> `JsError::try_native` -> `Display`)** | **candidate 1, candidate 2 (R)** | Results and thrown plain values go through the bounded snapshot (`js.rs:378, 335-338`); a thrown `Error` takes `try_native` (`js.rs:326-333`), which copies `message` in full and, for `AggregateError`, reserves and walks `errors.length` entries (`boa error/mod.rs:604-631`) with no value limit and no budget check |
| U7 / B9 (Python) | result (`to_json` with `ValueBudget`) vs exception (`MontyException::to_string`, `python.rs:211, 275`) | **candidate 1 (R)** | `raise ValueError(s * 16)` with a 1 MiB `s` returns 16,777,330 bytes of error text; `[s] * 64` as the result is refused at 4 MiB. Bounded only by Monty's 64 MiB memory limit. Exceptions take at most one argument and repeated traceback frames are collapsed (190 frames of a 90 KB line gave 270 KB), so no N-fold amplification |
| U7 / B10 (Rhai) | result (`to_json_value`) vs thrown value (`EvalAltResult` Display, `rhai_rt.rs:208`) | holds (R) | Rhai's own string/array/map size checks (summed per value) refuse the thrown value before it exists: `throw [s, s]` with a 1 MB `s` fails with "Length of string too large", same as returning it. FnPtr curry contents are not printed by `Display` (`fn_ptr.rs:98-101`) |
| U7 Rhai `print` vs `debug` | curried `FnPtr` | holds (R) | `debug(f)` for `f` curried 400 times with a 1 MB string logs 33 bytes; `print(f)` 5 bytes. Result/call-arg route: `from_dynamic` refuses `FnPtr` (known r2-5) |
| U7 JS `console.*` vs result | strings and multi-arg formatting | known | "JS console.log/print formats every argument in full ..." |
| U7 JS `icp.call` args vs canister/method strings | value limits | known | "JS icp.call / icp.candid copy canister-id and method strings of any size" (r2-9) |
| U7 Python `print` vs result | formatting before the cap | known | "Python print fails ... printed values are formatted in full before the cap" |
| C1 / E4 vs E5, LIM9 `MAX_SEARCHES` | `http_query` vs `http_update` | known | seed finding |
| LIM8 `MAX_BATCH` | query vs update | holds (CP) | both call `parse_body` (`mcp.rs:293-304`) |
| LIM10 one script per request | query vs update | holds (CP) | query upgrades any `Execute` (`mcp.rs:366`); update refuses the second (`mcp.rs:392`) |
| LIM7 reply cap | query (`finish` only) vs update (per-result replacement) | known | "Oversized-reply fallback answers a batch with one error object and id null"; "When a batch reply is too big, http_update replaces every tool result" |
| LIM11 300 B rpc echo | `rpc_error` sites | holds (CP) | every JSON-RPC error goes through `rpc_error` (`mcp.rs:145-149`), parse errors included |
| LIM6 result cap | script vs search vs error text | holds (CP) | all three go through `tool_result` (`mcp.rs:156-170`), errors included; this is what keeps candidate 1 off the wire |
| LIM2 instruction budget | script vs search, query vs update, JS rounds vs JS host conversions | known, plus candidate 2 | query search budget, Python native ops, JS synchronous code are known; the host-side `error_string` walk is candidate 2 |
| LIM4 `certified_reads` | query vs update; demo page | known / holds | wrong-path/unauthenticated requests known; demo page is served by the query only and returns 404 in certified mode, while the update route never serves it (`authenticate` 404s first) |
| LIM12 run slots | script vs search | known | "Searches take no run slot" |
| LIM13 audit error cap | `run_script` vs `run_search` vs `Trapped` | holds (CP) | `lib.rs:169, 243`; `Trapped` text fixed (`bridge.rs:262`) |
| LIM14 violation caps | script vs search vs `Trapped` | known | `…and N more` dropped in `Trapped` |
| LIM15 log cap | JS vs Python vs Rhai | known | |
| LIM16 value limits | runtimes, result vs args | known (depth off by one, JS UTF-16 count, Python repr) + candidate 1 for the error route | |
| LIM17 `MAX_METHOD_LEN` | `call` in each runtime | holds (CP) | all reach `IcBridge::call` (`bridge.rs:511`) |
| LIM18 failed-fetch TTL | query search vs update search vs `icp.call`/`icp.candid` | known | query path ignores `DID_FAILED` |
| LIM19 `MAX_DID_BYTES` | override (set time) vs fetched (after reply) | holds (CP) | same constant and parser; parse-limit asymmetry (40B vs 5B query) is known |
| LIM20 decode quota | `decode_reply` vs fetch fallback vs `inspect_message` | known | |
| LIM21 depth | encode vs decode vs runtimes | known | |
| LIM23 `MAX_CODE_BYTES` | three runtimes | holds (CP) | one check in `LanguageRouter::execute` (`runtimes/mod.rs:52`) |
| LIM25-27 request caps | JS vs Python vs Rhai | known | Python has no per-batch cap; JS getter overrun |
| LIM31 principal echo | `parse_principal` vs candid_bridge vs search note | known | |
| LIM32 `MAX_METHODS` | query vs update search | holds (CP) | same `search::render` |
| LIM1 call cap | `icp.call` vs `icp.candid` vs search fetch | holds / known | `call` takes a slot, then a cold cache takes another for the fetch (`bridge.rs:515, 486`), as CL7 says; two messages per fetch slot known |
| LIM3 call timeout | script calls vs fetches | known | |
| LIM5 `MAX_AUDIT_PAGE` | `get_audit_logs` vs `get_policy` | known | |
| E3 vs E5 | auth; decode quota | holds / known | same `authenticate`; quota known. Admin methods: controller check in both `inspect_message` and the guard; `http_request` sent as an update falls to the controller branch (stricter) |
| E1/E2 vs E6/E8/E9 | `InitArgs` vs admin endpoints | holds (CP) | `apply` calls the same `add_token_hash`, `set_allowed`, `set_limits` (`lib.rs:38-49`); `InitArgs` carries no overrides, so `set_candid_override`'s size/parse check has no unchecked sibling |
| E9 vs E10 | token add vs revoke normalisation | holds (CP) | both `trim().to_ascii_lowercase()` (`policy.rs:209, 218`) |
| E7 vs E11 | `remove_allowlist` vs override | known | override outlives `remove_allowlist` |
| E21 vs E22 vs E23 (demo_store) | stock checks: `create_order` (availability), `cancel_order` (`saturating_sub`), `restock` (`saturating_add`) | holds / known | `create_order` wrap of `reserved`, duplicate-line oversell, zero-quantity lines and `sales_summary` wrap are known; `create_order`'s `total` (u64) cannot wrap within the 2 MiB argument limit (needs about 333,000 lines of `u32::MAX` units at 12,900 cents; one line is 12 bytes, so at most about 174,000 lines fit) |
| E20 demo_store reads | `search_products` lowercases the name but not the category | not applicable | seed categories are lowercase; no limit involved |
| E26 demo page `rpc()` vs `post()` | | holds (r2-9) | neither applies a limit |
| C2-C4, C6-C15 | build, tooling, docs | not applicable | no request routes |
| A1-A12 | actors | not applicable | A3 and A8 reach the same `http_update` |
| U1-U6, U8-U16 | inputs | covered above / known | |
| D1-D10, V1-V11 | state | not applicable | no per-route limit stored; V2/V4 asymmetry known |
| K1-K12 | commit points | candidate 2 touches K6/K7 | the trap in candidate 2 lands in the final execution: no entry if nothing was sent (#11), `Trapped` otherwise |
| W1-W16 | unbounded work | candidate 2 adds a row: W8 `error_string` host walk | |
| F1-F16 | failure sites | candidate 2 adds an F site | `Vec::reserve` capacity overflow in Boa's `try_native`, reached from app code |
| B1-B17 | conversions | B8/B9 above | |
| VF1-VF8 | value flows | not applicable | |
| AD1-AD9 | admin | covered above | |
| S1-S10 | supply chain | not applicable | |
| CL10 | "Any value leaving a script (result, call arguments, printed values)" | literally holds; spirit broken by candidate 1 | the sentence does not name thrown values, while the inventory (U7) and `bridge.rs:43-46` treat every value crossing to the host as bounded |
| T1-T12 | tests | gap | no test throws a large `Error`/exception or an `AggregateError`; T8 `values_errors` uses small errors only |

## Candidates

### 1. Thrown error text leaves the JS and Python sandboxes without the value limits; results and thrown plain values are held to 4 MiB

- **Where:** `src/runtimes/js.rs:325-333` (`error_string`, native branch); `src/runtimes/python.rs:211, 232-235, 275` (`MontyException::to_string`).
- **Sibling routes:** result (`js.rs:378` `to_json_safe`; `python.rs:241` `to_json` with
  `ValueBudget`) and thrown plain values (`js.rs:335-338`), which are bounded; the thrown
  `Error` / raised exception, which is not.
- **Reproduced (native, MockBridge):**
  - JS `let t='x'; for (let i=0;i<24;i++) t=t+t; return t;` -> `RangeError: value holds more than 4194304 bytes of text`.
  - Same `throw t;` -> `Uncaught <unserializable value>` (bounded).
  - Same `throw new Error(t);` -> error text of **16,777,223 bytes** handed to the host.
  - Python `s='x'*(1<<20)` then `[s]*64` -> `value holds more than 4194304 bytes of text`;
    `raise ValueError(s*16)` -> error text of **16,777,330 bytes**.
  - Rhai: the thrown value is refused by Rhai's own size checks (holds).
- **Effect (CP):** the host builds the message by `to_std_string` (Boa `try_native`), then
  `Display`, then `msg[..i].to_string()`: about three host copies of a string of any size
  (JS has no memory cap, register #24). It is cut to 1 MB in `tool_result` and 4 KiB in the
  audit, so nothing oversized reaches the wire. No N-fold amplification was found (`Display`
  ignores `cause`; Monty exceptions take one argument and collapse repeated traceback frames).
- **Severity:** Low. The limit is inconsistent between exit routes, but the bypass is a
  constant-factor copy of a string the script already holds, and CL10 does not name thrown
  values.
- **Fix:** bound error text at the source: truncate the native message to
  `MAX_VALUE_BYTES` (or to `MAX_RESULT_BYTES`) before copying it out of the engine, and do
  the same for `MontyException` messages; add a test that throws a 16 MiB `Error` and raises
  a 16 MiB `ValueError`.

### 2. A thrown `AggregateError` with a huge sparse `errors` array makes `error_string` reserve and walk `errors.length` entries outside every limit; at `2**32-1` this traps on wasm32

- **Where:** `src/runtimes/js.rs:326` (`err.try_native(ctx)` on a script-thrown value) ->
  `boa_engine-0.22.0/src/error/mod.rs:604-631`: `length_of_array_like` (script-controlled),
  `error_list.reserve(length as usize)`, then `for i in 0..length { errors.get(i) }`.
- **Sibling route:** the result route snapshots the same object under the 200,000-node limit
  and the instruction budget checks; the thrown-`Error` route calls into Boa's conversion,
  which has neither. The run's budget is not checked again before `error_string` runs.
- **Script:** `const e = new AggregateError([]); e.errors.length = N; throw e;` (one line,
  no allocation in the engine: the array is sparse).
- **Reproduced (native, debug build):** N = 2^20: 0.73 s; 2^22: 2.89 s; 2^24: 11.46 s, then
  `AggregateError` (the conversion is linear in N; the same setup ending in `return 1` takes
  4 ms). N = 2^32-1: still running after 60 s (the 64-bit host reserved 171 GB lazily);
  killed.
- **On wasm32 (CP, unambiguous):** `usize` is 32 bits, so `reserve(4294967295)` needs more
  than `isize::MAX` bytes for any element size and Rust's `Vec` panics with "capacity
  overflow"; `panic = "abort"` makes that a trap. Smaller N spend instructions in host code
  with no budget check (not measured on-chain: no deploy in this phase).
- **Outcome:** the request traps instead of returning a clean error. If the script sent no
  message first, the whole run leaves no audit entry (register #11); otherwise a `Trapped`
  entry replaces the closing one.
- **Severity:** Medium (clean-failure invariant broken from a one-line script; same outcome
  class as the known Boa capacity-overflow findings, but the call into the unbounded
  conversion is app code, so it is fixed here, not in Boa). May merge with "Further Boa
  capacity-overflow sites" if the verifier treats it as the same root cause.
- **Fix:** do not call `try_native` on script-thrown values. Pass every opaque error through
  the bounded snapshot (or read `name` and `message` with a length cap), and check the
  budget before converting the final error.

## Test used (appended to the private copy's `src/runtimes/mod.rs`; abridged)

```rust
// thrown_vs_returned_sizes: JS "let t='x'; for (let i=0;i<24;i++) t=t+t;" + "return t;" /
//   "throw new Error(t);" / "throw t;"; Python "s='x'*(1<<20)" + "[s]*64" /
//   "raise ValueError(s*16)"; Rhai "let s=\"\"; s.pad(1000000,'x'); throw [s, s];"
// js_aggregate_sparse_errors: for n in [1<<20, 1<<22, 1<<24]:
//   "const e = new AggregateError([]); e.errors.length = {n}; throw e;"
// js_aggregate_max_length (ignored): same with 4294967295
// rhai_debug_curry: debug(f) / print(f) for f curried 10/100/400 times with a 1 MB string
```

## Not tested

- The on-chain instruction cost of candidate 2 for N below 2^32-1, and the wasm32 trap
  itself: both need a deployment (Live phase).
- Rhai thrown values nested about 10,000 deep (the `Display` recursion is the same one the
  known "Rhai print deeply nested values unchecked" finding covers).
