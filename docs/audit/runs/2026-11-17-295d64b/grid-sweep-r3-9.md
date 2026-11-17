# Variant sweep r3-9: a sibling runtime delegates a shared concern to its engine

Run `2026-11-17-295d64b`, Phase 6, round 3. Pattern (seed, known and not re-reported: Python
`print` caps through Monty's `CollectString` and raises MemoryError; Rhai refuses replies
through its engine size limits):

> A sibling runtime routes a shared concern (log cap, value limits) through its dependency's
> own mechanism instead of the shared host path, so the semantics differ.

Question asked of every row: is there a concern the host implements once (`bridge.rs`:
`ValueBudget`, `MAX_DEPTH`, `MAX_LOG_BYTES`/`LogBuf`, `budget_error`, `now_ms`,
`normalize_numbers`; `candid_bridge`; `policy`), and does one sibling (runtime, route, fetch
path, check site) hand that concern, or part of it, to a dependency's own mechanism whose
rules differ?

Evidence: **R** = reproduced, **C** = code path. One native probe was built in a private copy
(no `target/`, `.git`, `design/`, `video/`, `.env`) with the shared `CARGO_TARGET_DIR`, as a
`#[cfg(test)]` module appended to `src/runtimes/python.rs` in the copy only; the copy was
deleted afterwards. No live canister was touched.

## Candidate

**SW3-9-1 (Medium).** Python call arguments are exported through Monty's object bridge, whose
depth guard shares the interpreter's recursion counter with the Python call stack. The
arguments of an `icp.call` made from deep recursion are cut at `200 - call depth` levels, and
every cut value, leaf strings included, becomes the text `Repr('<deeply nested>')`. Cyclic
values become `"[...]"`. The host's 128-level check only sees the cut graph, so the call is
sent with replaced data and the run is Ok. JS and Rhai send the same argument intact.

- `src/runtimes/python.rs:59-61` relies on the dependency: "Monty itself cuts exported values
  at the recursion limit and substitutes a `<deeply nested>` marker; the depth check here is
  lower, so deep data is an error instead of silently truncated." Not true when the script is
  more than about 72 frames deep.
- Monty `object_bridge.rs:182-186`: `push` takes `vm.recursion_guard()` before it looks at the
  value and on failure pushes `MontyNode::Repr("<deeply nested>")`. `bytecode/vm/recursion.rs:1-8`:
  the same counter charges "non-frozen call frames". `push_ref` turns a reference that is
  still being exported into `MontyNode::Cycle("[...]")` (`object_bridge.rs:226-236`).
- `type_name()` of these nodes is `"repr"` / `"cycle"` (`monty-types graph.rs:580`), so
  `to_json` takes the `_ => Value::String(o.py_repr())` arm (`python.rs:98`). One node and its
  repr length are charged, then the string is sent.
- R (native probe, Mock bridge records the args as sent):
  `def f(n, x): return icp.call(L, 'icrc1_balance_of', x) if n == 0 else f(n-1, x)` with
  `x = {'owner': 'aaaaa-aa', 'name': 'alice', 'deep': [[['leaf']]]}`:
  - n = 0 to 190: sent intact.
  - n = 195: `deep: [[["Repr('<deeply nested>')"]]]`. n = 197: `deep: ["Repr('<deeply nested>')"]`.
    All Ok.
  - n = 198: sent `{"Repr('<deeply nested>')": "Repr('<deeply nested>')"}`. All three keys were
    cut, collided, and only one survived. Ok.
  - `candid_bridge::Service::parse("service : { greet : (record { name : text; deep : vec vec vec text }) -> (text) }")`
    then `encode_args` of the marker strings: `Ok` (62 bytes). Text slots accept the
    replacement, so the message goes out. Slots of other types fail at encode with a
    type-mismatch error that names the marker.
  - Cycle: `a = ['x']; a.append(a); icp.call(L, m, {'owner': ..., 'path': a})` sent
    `"path": ["x", "[...]"]`, Ok. Returning `a` as the result gives `["x", "[...]"]`, Ok.
  - Same arguments, same recursion: JS `f(240, x)` and Rhai `f(30, x)` sent them intact. A JS
    cycle fails at the snapshot's 128-level check, Rhai has no cycles, and a Rhai cut never
    happens because `to_json_value` is a Rust walk.
- Related, loud: JS's snapshot is JavaScript code, so its depth also shares Boa's 256-frame
  recursion limit. At call depth 240 a 10-deep argument fails with `RuntimeLimitError:
  reached the maximum number of recursive calls`, whose stack trace shows the snapshot's own
  `put`/`walk` frames. `console.log(x)` there prints `<unprintable value>`, which is known and
  part of the README:369 finding. At depth 200 a 30-deep value passes. The message is wrong,
  but no data is replaced.
- Refuted alternatives: the result path is exported at frame depth 0 (`RunProgress::Complete`),
  so only cycles reach results, not depth cuts. Canister id and method are exported one level
  below the call's frame, so cutting them gives a loud `invalid canister id` error.
- Overlap: the last step goes through the same `_ =>` repr fallback as L7-6 (unsupported types
  become repr strings). This trigger is a supported type (list, dict, str) cut by the engine,
  and the depth at which it happens depends on the call stack. A fix that only converts known
  unsupported types (`range`, functions) would not fix it. The verifier may merge it into L7-6
  if it prefers.
- Fix: in `to_json`, treat `"repr"` nodes whose text is the marker, and every `"cycle"` node,
  as errors. Better, refuse every `repr`/`cycle` node, which is the L7-6 fix. Also export
  call arguments with a guard that does not share the call-stack counter, or check
  `depth + current frame depth`. Correct the `python.rs:59-61` comment. Regression test:
  `icp.call` from recursion depth 195 with a 4-deep argument must be an error, not a sent
  call.

## Grid

| Row | Shared concern / sibling compared | Cell |
|---|---|---|
| C1 runtimes (E16/E17/E18/E19) | value depth on call args: host `MAX_DEPTH` vs Monty export guard (Python) vs JS snapshot inside Boa | **SW3-9-1** (Python silent); JS loud (note); Rhai holds (Rust walk) |
| C1a feature subsets | tools/list language advertising | known |
| C2 demo_store | no host path shared with C1 | n/a |
| C3 web-time shim | Monty clock for budget | known (Python budget findings); `time.monotonic`/`perf_counter` read the `Fixed` source and `process_time` is `Zero` by default (`monty-1.6.0 modules/time.rs:276-281`, `monty-types os_policy.rs:340-347`) | holds (C) |
| C4 vendored chrono | Monty datetime under `DateTimeSource::Fixed` | holds (C) |
| C5 demo page | `rpc()` vs `post()` | covered by r2-9; no dependency-delegated limit |
| C6-C15 | scripts, build config, interface files, video, audit tooling, exports | n/a: no runtime concern shared with a sibling |
| A1-A12 | actors | n/a for this pattern (the concern is per code path, not per actor) |
| E1/E2 vs E8 | limits through `apply` vs `set_limits` | known (no validation, additive re-apply) |
| E3 vs E5 | token check | holds (C): both call `mcp::authenticate` (`lib.rs:276-277`, `mcp.rs:276-288`); decode quota difference known |
| E4 vs E5 | search render, reply cap, budget | known (MAX_SEARCHES, budget, reply collapse) |
| E5a vs E5b | audit construction for scripts vs searches | known ("search" language, sha of raw spelling) |
| E5c/E5d | Trapped entry built in `Drop` | known (instructions, violation count) |
| E6-E14 | single implementation each | n/a |
| E15 | exports | n/a |
| E16 JS host fns | canister/method strings bypass `ValueBudget` | known |
| E17 JS console | own cap then `LogBuf` | known (marker, per-arg format) |
| E18 Python host fns | args exported by Monty | **SW3-9-1**; name lookups/OS calls eating suspensions already SW5-1 (r1-5) |
| E18 Python print | Monty `CollectString` | known (seed); stderr (`file=sys.stderr`) lands in the same buffer with no `[error]` label (`monty-types io.rs:132-137`): cosmetic, not reported |
| E19 Rhai host fns | `to_json_value` + `from_dynamic` fallback | holds (R in r1-11/L7-r1: FnPtr and ranges refused). Shared (closure-captured) values: `is_array`/`is_map`/`is_string`/`is_blob` read through `Union::Shared` (`rhai-1.27.0 types/dynamic.rs:2221-2305`), so they take the bounded walk (C) |
| E19 Rhai print/debug/throw | Rhai Display | known / holds (r1-11) |
| E20-E28 | demo_store, page, scripts | n/a |
| U1-U5 | HTTP inputs | holds: one parser (`parse_body`, serde_json 128) for both routes |
| U6 | script source | `MAX_CODE_BYTES` shared (`runtimes/mod.rs:51`); engine compile costs known |
| U7 | values leaving scripts | **SW3-9-1** (Python args); JS UTF-16 count known; Python repr fallback known (L7-6, SW2-5a) |
| U8 | replies into scripts | JS `JsValue::from_json` defines data properties (`boa_engine value/conversions/serde_json.rs:68-79`), so a `__proto__` field stays data: holds (C). Python `from_json`: holds. Rhai `to_dynamic` + engine limits: known |
| U9 | reject text | known (echo in full) |
| U10 | interface text: `canister_metadata` reply decoded by ic-cdk default `candid()` vs `decode_reply` quota | holds: the management canister builds the reply (flat `record { value : blob }`); the tmp_hack sibling is known |
| U11 | override text parse | known (query re-parse) |
| U12/U13 | ingress decode | known (inspect_message quota) |
| U14-U16 | page, platform, video | n/a |
| O1 vs O2 | timeout and decode quota per path | known (fetch uses ic-cdk defaults) |
| O3 | audit append | holds: single `policy::append_audit` |
| O4-O10 | admin writes, cert data, HTTP, accept, trap, demo_store, video | n/a |
| D1-D10 | durable state | n/a: single writer path per store |
| V1-V5 | slots, caches | known |
| V6 | Monty memory counter | known (host replies uncounted) |
| V7 | getrandom stream | known; Python `hash('abc')` and string-set order identical across three runs (R), so no extra hash-seed symptom |
| V8-V11 | Boa GC, executor, engine statics, demo store | known / n/a |
| K1-K12 | commit points | n/a: the pattern is about value and limit semantics, not commit placement |
| LIM1 | call cap | shared `reserve` | holds |
| LIM2 | instruction budget | Python via Monty clock: known (several) |
| LIM3 | call timeout | fetch path uses ic-cdk default: known |
| LIM6/LIM7 | result/reply size | known |
| LIM11/LIM31 | echo truncation | candid error Display in full: known |
| LIM12 | run slots | n/a (single site) |
| LIM13/LIM14 | audit caps | known |
| LIM15 | log cap | known (seed) |
| LIM16 | value nodes/bytes | known (JS UTF-16, Python repr counted as one node); SW3-9-1 replaced values are counted as one node each |
| LIM17 | method length | holds: checked in `IcBridge::call` for all runtimes |
| LIM18/LIM19 | TTLs, interface size | n/a: single site |
| LIM20 | decoding quota | known |
| LIM21 | depth 128 | **SW3-9-1** (Python args cut below 128 depending on call depth; JS refused below 128 depending on call depth); off-by-one known |
| LIM22 | safe integer | known (L7-3, L7-17) |
| LIM23 | code size | holds (shared) |
| LIM24-LIM27 | engine limits | engine-specific by design; per-frame/per-turn interactions are r3-6 and known items |
| LIM28-LIM30 | stack, heap, platform | n/a |
| LIM32 | search methods | n/a |
| W1-W16 | unbounded work | covered by other lenses; no new dependency-delegated limit |
| F1-F16 | failure sites | n/a |
| B1-B4 | parse/auth/hash/principal | holds / known (whitespace) |
| B5/B6 | json<->idl | shared by all runtimes: holds for this pattern |
| B7 | .did parse | known |
| B8 | JS <-> JSON | JS-side depth shares Boa's recursion limit (loud, note under SW3-9-1); L7-2 known |
| B9 | Python <-> JSON | **SW3-9-1** |
| B10 | Rhai <-> JSON | holds (above) |
| B11-B17 | result text, stable, language, time, cert, search render, page | known / n/a |
| VF1-VF8 | value flows | n/a |
| AD1-AD9 | admin surface | known |
| S1-S10 | supply chain | n/a |
| CL10 "128 levels ... in all three languages" | | **SW3-9-1** (Python cuts below 128 without an error) |
| CL29 "identical semantics in every language" | | **SW3-9-1** |
| CL other | | n/a for this pattern |
| T7/T9 | tests | no test runs a Python `icp.call` from deep recursion or with a cyclic value |

## Not tested

- The Wasm release build: Monty's `max_recursion_depth(200)` is set by the app and is the same
  in both profiles, and frame charging does not depend on the target. Only the native debug
  build was probed.
- The live canister: a live repro needs an allowlisted method with a `text` argument. Under
  this phase's rules that was left to the verifier.
