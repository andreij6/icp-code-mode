# Coverage grid: L3 Resource bounds, round 5 — Rhai replay cache, replay cost, ahash

Run `2026-11-17-295d64b`. The Rhai-specific pass that L3-r4 and sweep-r2-5 left open. Rows from
`inventory.md`: W11, V10, LIM27, F13, B10 (plus the rows these measurements bound: LIM12,
LIM20, LIM29, V7). Cells: **finding**, **holds** (why), **n/a** (why). Evidence: R = reproduced
(native test, wasm32 harness or the port-8010 dev canister), C = code path.

Result: **no new finding**. W11 is bounded, and the bound is the instruction budget, not
Rhai's size limits. That bound was measured on chain. The earlier W11 "holds" figures (L4-r1:
about 85 MiB per run and 0.7 GiB for 8 runs) underestimated the per-run reply retention. They
did not change the verdict.

## Method

- Private copy, shared build dir. In the copy only, a `PEAK` counter was added to the existing
  test allocator (`lib.rs` `counting_alloc`), and probe tests were appended to
  `rhai_rt.rs` (`w11_probe`). Replies were real Candid bytes decoded by
  `Service::decode_reply` with the real `DECODING_QUOTA`.
- wasm32 sizes: a probe cdylib (`icp_code_mode` with feature `rhai`, plus a counting global
  allocator) was built for `wasm32-unknown-unknown` and run in Node with `ic0` stubbed.
  `rhai_rt::run` was driven by 1 or 8 interleaved bridges.
- On chain (dev canister, port 8010, test token from `.env`): five `execute_icp_script` Rhai
  calls (K = 0, 1, 4, 8, 16, described below), each with one real `get_product` call to
  `demo_store`. Instructions were read from the newest audit entry. No setting was changed and
  nothing was deployed or upgraded. Side effect: the canister's `memory_size` high-water rose
  from 107,212,395 to 219,278,955 B. Wasm memory does not shrink, so this stays until the next
  upgrade.

## Measurements

### Largest reply per shape under the effective quota (native 64-bit, `decode_reply`)

| Reply type | Max accepted | Wire | Held as `Value` | Peak during decode |
|---|---|---|---|---|
| `text` | 399,998 chars | 400,008 B | 0.38 MiB | 0.76 MiB |
| `vec text` (200 B each) | 1,960 | 395,931 B | 0.44 MiB | 0.92 MiB |
| `vec null` | 99,999 | 12 B | 4.0 MiB | 13.0 MiB |
| `vec record {}` | 49,999 | 14 B | 2.0 MiB | 6.5 MiB |
| `vec record { a : null }` | 26,666 | 16 B | **17.1 MiB** (672 B/element: a BTreeMap leaf per 1-entry map) | 27.0 MiB |
| `vec record { "<L bytes>" : null }` | refused once L x n is above about 400,000 (L=1,000: n=100 ok, 1,000 refused) | 18 B | — | — |

The quota charges label bytes for every record (x50 untyped), so long field names from a
hostile `.did` do **not** amplify the held `Value`. My hypothesis that per-record key copies
could reach GBs is refuted (R).

Largest reply that Rhai also accepts (the array has 10,000 or fewer elements and the maps have
10,000 or fewer entries): `record { r : vec record { a : null }; t : text }` with 9,998 records
and a 250,007 B text. It is 250,031 B on the wire, holds **6.77 MiB native / 4.5 MiB wasm32**,
and peaks at 10.95 MiB during decode. A larger reply (26,000 records) ends the run uncatchably
on the next replay. `try` does not catch "Size of array/BLOB too large", and only 1 call was
made (R). So it is never retained next to later answers. This confirms L3-r1.

### Retention in `answers` (25 call replies of the shape above plus 39 `icp::candid` texts of 512 KiB)

| Runs | `max_calls` | Script keeps answers | Native 64-bit peak | wasm32 peak above base / held at each await / `memory_size` |
|---|---|---|---|---|
| 1 | 25 (default) | no | 194.6 MiB | 136.4 / **105.0** / 161 MiB |
| 1 | 25 | yes | 330.9 MiB | 240.2 / 105.0 / 268 MiB |
| 8 | 25 | no | 1,480.7 MiB | 1,034.8 / 870.4 / 1,134 MiB |
| 8 | 25 | yes | 1,617.0 MiB | **1,138.7** / 870.4 / 1,240 MiB |
| 1 | 64 | no | 431.7 MiB | 287.2 / 275.3 / 321 MiB |
| 8 | 64 (operator-set) | no | 3,335.0 MiB | **2,214.4** / 2,202.5 / 2,380 MiB |

The native numbers are within 2x of the 3 GiB limit, so the limit question went on chain (next
table). Between replays only `answers` survives an await. Script-built Dynamics are dropped
when each `eval_ast` ends, which confirms L2-r2.

### Retention without large replies: requests refused before any message

`IcBridge::call` refuses a method longer than 256 B (`bridge.rs:511`), and `candid` refuses a
bad canister id. Both refusals come before `reserve`, so they take no cap slot, send no message
and have no await. The `Request` is still kept in `answers`: canister text up to 1 MiB, method
text up to 1 MiB, and args up to the `ValueBudget` (4 MiB of text; Rhai does not count map
keys, see SW2-5b). One request kept 5.7 MiB (R native: k=63 gave 360.7 MiB per run, and 8 runs
x k=30 gave 1,375 MiB). Every later replay rebuilds, converts and compares each kept request.

On chain, the script was K refused requests of 5.5 MiB each (seven 512 KiB map keys, plus 1 MiB
canister text and 1 MiB method text) and then one real call:

| K | Result | Instructions (audit) |
|---|---|---|
| 0 | Ok | 242,586,541 |
| 1 | Ok | 730,782,383 |
| 4 | Ok | 3,143,285,012 |
| 8 | Ok | 8,662,753,177 |
| 16 | `policy: instruction budget exceeded (20053393702 > 20000000000)`, `PolicyViolation`, before the real call | 20,053,393,702 |

Fit: about 0.29B instructions per replay plus about 0.167B per kept request per replay, which
is about 29 instructions per retained byte per replay. The most one run can retain is
sqrt(2 x budget x S / c), about **90 MB** at the default 20B (K of about 13). The 40e9
per-message limit caps it at about 126 MB, because all these replays run in the first message.
For 8 runs that is about 0.7 to 1.0 GB, which is under 3 GiB by a factor of 3 or more. The
overshoot past the budget was 53M instructions, so the budget is polled at fine granularity.

### Replay cost (item 2)

Native: one op that converts one cached 9,998-record answer (`to_dynamic` plus Rhai's size
check) takes 2.5 ms. Comparing a cached 4 MiB-args `Request` takes 0.33 ms (clone included).
A whole run with 25 large replies and 39 interface texts (65 replays) takes 4.3 s. All of this
happens inside native calls that run one Rhai operation at a time. `on_progress` checks the
deadline before the next operation, so the unpolled work per operation is one answer conversion
or one request comparison, which `DECODING_QUOTA` and `ValueBudget` bound. On chain, the K=16
run stopped cleanly at the budget (R). The quadratic replay cost is the register-accepted
ponytail (`rhai_rt.rs:195-196`). It is spent inside the budget, not between operations.

### Rhai and ahash (item 3)

- Rhai uses ahash only through `func::hashing::get_hasher()` (`rhai-1.27.0/src/func/hashing.rs:57-63`).
  With no `RHAI_HASHING_SEED` that is `AHasher::default()`, the fixed keys from
  `get_fixed_seeds()[0]`. The build enables `ahash/runtime-rng` (rhai default) and
  `compile-time-rng`, and outside ahash's own tests the `runtime-rng` branch wins: 64 bytes
  drawn once from `getrandom`, which on wasm is the public splitmix stream (`lib.rs:433-449`,
  known L3r4-3). The keys are public up to the stream offset at first use.
- The hashes are u64 keys into `StraightHashMap` (std hashbrown with identity hashing): the
  function-resolution cache (`eval/cache.rs:27`, per evaluation, one lookup per call operation,
  polled), the strings interner (`types/interner.rs`, at most 256 entries of at most 32 B), the
  module and AST function index (at most 256 script functions), switch-case tables
  (`ast/stmt.rs:198`, filled at compile time), and closure and variable hashes.
- Forced collisions (R, native, keys read in-process): 9,000 switch-case string literals whose
  hashes share their low 15 bits (found in 48 s, 2.99e8 tries) compile in 744 ms. 9,000 random
  literals compile in 564 ms, with 37% more source text in the colliding case. Evaluation took
  12 µs. So there is no measurable quadratic blow-up: the probing is O(n/group) with u64 key
  comparisons, n is bounded by `MAX_CODE_BYTES`, and compile cost is already the known SW3-2-2.
- A full 64-bit collision (a 2^32 birthday search offline) would let two literals or two script
  functions share one hash, so a script would take the wrong switch arm or call the wrong
  function **in its own script**. `eval` is dispatched by name (`func/call.rs:614`) and disabled
  by symbol. Host functions are separate qualified hashes that the script can call anyway.
  Nothing crosses a run or the host boundary.
- `rhai::Map` is `BTreeMap<Identifier, Dynamic>` (`rhai-1.27.0/src/lib.rs:328`), so maps are
  immune to hash collisions. The JSON side is immune too: `serde_json::Map` is a BTreeMap,
  because `preserve_order` is not enabled (checked with `cargo tree -e features`).

## Grid

| Row | Cell | Reason |
|---|---|---|
| W11 Rhai `answers` / replays | **holds** (R) | Each kept answer is limited by `DECODING_QUOTA` and by Rhai's element limits (an oversized answer ends the run uncatchably). Kept requests are limited by `ValueBudget` plus 1 MiB per string. Together they are bounded by `MAX_REQUESTS` and above all by the instruction budget, because every kept byte is re-processed on every later replay (about 29 instructions per byte per replay, on chain). Worst case measured: replies 105 MiB per run held across awaits (wasm32, default) and 1.14 GiB peak for 8 runs. Refused requests: about 90 MB per run within the default budget. Both are 2.5x or more under 3 GiB, and 2,380 MiB `memory_size` even at `max_calls` 64. The earlier W11 figures (L4-r1: 52 MiB, about 85 MiB per run, 0.7 GiB for 8) were low but led to the same verdict. |
| W11 vs "heap past `wasm_memory_limit` after an await" (L8-6) | **holds** (R) | `answers.push` runs in a response callback, where the limit is not enforced. But W11 alone reaches at most about 1.1 GiB (default) or 2.4 GiB (`max_calls` 64, 8 runs). Neither crosses 3 GiB. Reaching L8-6 through Rhai needs the in-engine route (SW2-5b, known). |
| V10 engine-internal statics (Rhai) | **holds** (C) | Rhai statics are constant tables plus three write-once cells (`HASHING_SEED`, `INDEXER_HASHES`, `DUMMY_RESOLVER`). ahash's `SEEDS` `OnceBox` is filled once per instance. Nothing grows across runs. The engine, its `StandardPackage` and its caches are rebuilt for each run in `build_engine`. |
| V7 shared seed stream (Rhai's use) | known (L3r4-3); **holds** for Rhai's harm | Rhai's ahash keys are public, as above. The only effects are a measured, insignificant collision cost and same-script mis-dispatch. Maps are BTreeMaps. |
| LIM27 Rhai limits | **holds** (R) | `MAX_REQUESTS` 64 is reachable without spending cap slots (interface texts and refused requests), and each answer costs a full replay. 1M operations per replay are polled, and the budget stops the run cleanly (K=16 on chain). The array limit refuses oversized replies uncatchably (R). Map keys are not counted (known SW2-5b). The 1 MiB string limit does not apply to error values thrown by host requests: a 3 MiB mock error reached `catch (e)` with `e.len() == 3145728` (R, native). But no on-chain error source above 1 MiB was found (the largest is the L9-6 interface parse error, at most about 512 KiB), so this is not a finding. |
| LIM12 8 runs | **holds** (R) | 8 runs interleaved at awaits were measured. Totals as in W11. |
| LIM20 `DECODING_QUOTA` | holds as the per-answer bound (R). Its size (x50) is known L9-4 | The largest held `Value` from one reply is 17.1 MiB native (`vec record { a : null }`, 16 B on the wire). Label bytes are charged (R). |
| LIM29 3 GiB | **holds** for W11 (R) | See W11. |
| F13 out of memory (Rhai) | known (SW2-5b) for in-engine memory; **holds** for `answers` (R) | The replay cache alone does not reach the limit, as measured above. |
| B10 Rhai JSON <-> Dynamic | **holds** (R) | `to_dynamic` runs once per cached answer per replay, inside one operation (2.5 ms native for the largest answer Rhai accepts). The size check runs after conversion, but the input is quota-bounded. `to_json_value` is bounded by `ValueBudget`, and the comparison is bounded by the same. Map keys and `FnPtr` are known (SW2-5b). |

## Not tested

- On-chain retention of large **replies** (the 9,998-record shape) needs a target canister
  that returns it. Deploying one is outside this phase. The on-chain instruction cost was
  measured only for the refused-request route. The reply route's cost per replay was not
  measured on chain, so its own budget ceiling is unknown. The memory figures above assume
  the runs finish within budget.
- 8 concurrent runs on chain were not attempted. Driving `memory_size` toward 3 GiB is
  destructive. Only the native figures and the wasm32 harness were used.
- IC charges for `memory.copy` versus a byte loop were not separated. The cost of about 29
  instructions per byte is measured end to end.
- Whether the platform caps reject-message size, which would bound the error-string route
  under LIM27, was not checked.
