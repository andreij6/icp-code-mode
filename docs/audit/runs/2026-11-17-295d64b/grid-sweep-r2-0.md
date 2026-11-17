# Variant sweep r2-0: byte limits standing in for super-linear cost; cache writes in a trapping execution

Run `2026-11-17-295d64b`, Phase 6, round 2. Pattern:

> A size limit (bytes) standing in for a cost limit that is super-linear or recursive in the
> input's structure; a failure-cache write in the same execution as the work that can trap.

Not re-reported (known): candid_parser parse cost/stack on interfaces under 512 KiB, the
uncached fetch trap, json_to_idl value-driven cost, decoder depth and quota, Boa parser
stack overflow, Boa built-ins, synchronous JS never checking the budget, Python eval/exec,
search keywords x methods, demo page regex, Rhai O(requests^2) replay (documented).

Result: **three new candidates** (SW2-0-1 encoder type table, SW2-0-2 Boa compile of bindings,
SW2-0-3 override validated under the update limit, parsed under the query limit).

## How it was tested

- Private copy of the tree (no `target/`, `design/`, `docs/per-customer-canisters-plan.md`,
  `video/`, `.env`) at `/private/tmp/claude-501/sweep-r2-0.*`, shared
  `CARGO_TARGET_DIR=/private/tmp/claude-501/audit-target`. The copy's package was renamed
  (`icp_code_mode_sweepr20`, lib name unchanged) because copies of the same workspace in the
  shared target dir get the same metadata hash and overwrite each other's test binary.
- One `#[cfg(test)]` probe (`src/sweep_probe.rs` in the copy only), `cargo test --release
  --lib`, each case on a thread with a 2 GiB stack, one process per case. Cost = macOS
  `/usr/bin/time -l` "instructions retired" for the whole process (native aarch64; a proxy for
  wasm instructions, not an on-chain count; process overhead about 0.03 B). Calls go through
  the app's own `Service::parse`, `Service::encode_args`, `Service::decode_reply`,
  `search::render`, and `runtimes::{js,python,rhai_rt}::run` with `MockBridge`.
- No live canister was touched; nothing was deployed and no setting changed.

## Measurements

| Case | Input | Bytes | Native instructions |
|---|---|---|---|
| `Service::parse` | `f : (opt^16000 nat) -> ()` | 64,030 | 0.54 B |
| `encode_args("f", [null])` | same interface, `opt^N nat`, N = 1k / 2k / 4k / 8k / 12k / 16k | 4 KB - 64 KB | 0.25 / 1.00 / 4.41 / 18.4 / **43.7** / **80.5** B (quadratic) |
| `search::render` | same interface, 16k | 64 KB | 0.64 B |
| `decode_reply` | rets `opt^N nat`, hostile tables (N-entry chain, 1-entry `μT. opt T`), N up to 16k | | <= 0.55 B (clean error or `null`) |
| JS `(function(_0,_1,...){return 1}), 1` never called | 10,000 / 20,000 params | 48,692 / 98,692 | 15.8 / **65.2** B (quadratic) |
| JS `(function(){var a0=0;...}), 1` never called | 8,000 vars | 94,907 | 11.1 B |
| JS object literal / `switch` cases | 8,000 | 63-79 KB | 0.30 / 0.21 B |
| Python module assigns / function locals / dict literal | 8,000 | 63-79 KB | 0.15-0.17 B |
| Rhai map literal | 4,500 / 10,000 keys (10,001+ refused at parse) | 35 / 79 KB | 0.71 / 3.52 B |
| Rhai 9,000 statements, 256 functions | | 98 KB | 0.67 B / 0.04 B |
| `Service::parse` alias chain `type t_i = t_{i+1}` | 1,000 / 2,000 / 4,000 | 18 / 38 / 78 KB | 1.20 / 4.65 / **20.97** B |

## Cells

| Row | Cell | Reason |
|---|---|---|
| LIM19 `MAX_DID_BYTES` / W5 / B5 encoder | **finding SW2-0-1** | 512 KiB bounds text; candid's `TypeSerialize::build_type` keys a `BTreeMap<Type, i32>` by structurally compared types, so encoding any value of a deeply nested inline arg type is O(depth^2 log depth). 48-64 KB interface: 44-80 B native per call, parse 0.5 B (R) |
| LIM19 / W7 parse | known | candid_parser cost/stack (L3-2, L9-2) |
| LIM19 / W6 / B6 decode against deep rets | holds (R) | Header limits and charged table cost: <= 0.55 B for 16k-deep chains and recursive wire types |
| LIM19 / W3 / B16 render, `collect_vars` | holds (R) | 0.64 B at depth 16k; keywords x methods is known |
| LIM20 `DECODING_QUOTA` | known | 50x untyped penalty, depth guard no-op on wasm |
| LIM23 `MAX_CODE_BYTES` (JS) / W8 | **finding SW2-0-2** | Boa parse/compile (`ctx.eval` in `drive`, `src/runtimes/js.rs:368`, before any budget check) is quadratic in bindings per scope: a 98.7 KB script whose only content is one never-called function with 20,000 parameters costs 65 B native (R) |
| LIM23 (Python) / W10 | holds (R) | Monty compile linear for the shapes tried (assign, locals, dict literal), <= 0.17 B at 63-79 KB; eval/exec known |
| LIM23 / LIM27 (Rhai) / W11 | holds (R) | Map literal is quadratic but Rhai refuses more than 10,000 entries at parse (3.5 B max); statements, functions linear; replay O(requests^2) documented and stopped by the per-operation budget check |
| AD7 / E11 / V5 / E4b override parse in query | **finding SW2-0-3** | `set_candid_override` validates with `Service::parse` in an update (40 B limit, `src/lib.rs:329`); the query path parses it again (`search_cached` -> `cached_service` -> `override_service`, `src/bridge.rs:216-226`, `src/lib.rs:114`) under the 5 B query limit, and its `OVERRIDES` insert never commits. A 3,000-alias override (about 57 KB, about 11 B native) passes validation and cannot be parsed in a query (CP + native measurement) |
| V2 / V4 cache writes in the reply-callback execution (`src/bridge.rs:495-506`) | **finding SW2-0-1 (part)** / holds otherwise | The parse and `DID_CACHE`/`DID_FAILED` insert share the execution with whatever the script does next (encode, synchronous JS). A trap there rolls the cache write back, so the fetch repeats on the next run. Demonstrated cost driver: SW2-0-1. With a script that traps by itself, the lost cache write is incidental to a known finding |
| E5b `run_search` cache writes then `render` + audit append | holds | Render cost measured above; audit append trap is known (F4) |
| V5 `OVERRIDES` in update path | holds | Parsed and committed in the update; text compared on every hit |
| W1 / U3 `parse_body`, LIM8 | holds (CP) | serde_json linear, depth 128, ingress-bounded |
| W2 / U12 `inspect_message` | known | decode without quota recorded |
| W12 / LIM6 / LIM7 / B11 | holds (CP) | `truncate_escaped` single char scan; re-serialization linear in reply |
| LIM15 / LogBuf | holds (CP) | Constant work per line after the cap; Python format-before-cap known |
| LIM16 value limits / B8 / B9 / B10 | holds / known | Walks are linear in nodes and bytes; per-node encode cost known (json_to_idl) |
| LIM11 / LIM31 / LIM17 | holds (CP) | Fixed truncations, linear |
| W4 / LIM5 / W13 admin-sized lists | known | admin inputs unbounded, page by count |
| E20-E23 / W15 demo_store | holds (CP) / known | `search_products` and `create_order` linear in input x seeded products; `sales_summary` orders x items with small category list; unbounded orders known |
| W16 / B17 demo page | known | regex quadratic |
| B2 / B3 / B4 / B13 / B14 / B15 | holds (CP) | Constant or linear per input |
| D1-D5 / B12 | n/a | Written by the app or a controller, flat records |
| C3 / C4 / C6 / C7-C10 / C12 / C13 / S* | n/a | No untrusted structure reaches them at run time |
| A* / CL* | n/a for this pattern | Claims about size limits: CL22 "Interfaces over 512 KiB are refused" holds literally; README does not claim 512 KiB bounds cost |

## Candidates

### SW2-0-1 Encoding a call against a deeply nested inline argument type costs O(depth^2) inside candid's encoder; a 48 KB interface traps every call to that method

- Where: `src/candid_bridge.rs:88-90` (`IDLArgs::to_bytes_with_types`), reached from
  `IcBridge::call` (`src/bridge.rs:517`); cost in `candid-0.10.40/src/ser.rs:264-290`
  (`TypeSerialize::build_type`, `type_map: BTreeMap<Type, i32>`, derived structural `Ord`).
- Repro (native): `Service::parse("service : { f : (opt^12000 nat) -> (); }")` 0.44 B;
  `encode_args("f", [null])` 43.7 B; at 16,000: 80.5 B. Interface 48-64 KB, value trivial.
- On chain (inferred, not run): the encode runs after `reserve` and before the send, in one
  execution, with no budget check, so it reaches the 40 B per-message limit and traps. If the
  interface is already cached (any earlier search) nothing was sent: the request rolls back
  with no audit entry, and every later call does the same for the hour the cache entry lives.
  If not cached, the fetch messages are logged, then parse, `DID_CACHE` insert and encode run in
  one callback execution: the trap rolls the cache insert back, a `Trapped` entry is written,
  and every later call re-sends both fetch messages and traps again.

### SW2-0-2 Boa's parse/compile of a script is quadratic in bindings per scope; a 98.7 KB script costs 65 B instructions before any limit is checked

- Where: `src/runtimes/js.rs:366-368` (`ctx.eval` of the wrapped source), guarded only by
  `MAX_CODE_BYTES` (`src/runtimes/mod.rs:15, 52`).
- Repro (native): `(function(_0,_1,...,_<20,000 base-36 names>){return 1}), 1` (98,692 B,
  function never called) 65.2 B; 10,000 params 15.8 B; `(function(){var a0=0;... x8000}), 1`
  (94,907 B) 11.1 B.
- Distinct from the known findings: not stack (no nesting), not runtime (nothing runs), and
  a budget hook inside Boa's VM loop would not reach the compile step.

### SW2-0-3 `set_candid_override` validates under the update limit, but the query path re-parses the override under the 5 B query limit and never keeps the result

- Where: `src/lib.rs:324-333` (validation in an update), `src/lib.rs:111-117` and
  `src/bridge.rs:207-227` (parse in `http_request`, insert discarded with the query).
- Measured: alias-chain overrides 2,000 / 4,000 types (38 / 78 KB) parse in 4.65 / 20.97 B
  native; about 3,000 aliases (~57 KB) lands between the two limits.
- Effect (code path): with such an override set, every query-path search whose scope
  includes that canister (every unscoped search) fails instead of answering or returning
  `upgrade`, so it never reaches the update path that would cache the parse. Only a script
  call to that canister (which needs an allowed method) or `certified_reads` gets past it, and
  the heap cache is empty again after each upgrade. Not reproduced live: setting an override
  is a configuration change outside the Live phase.
