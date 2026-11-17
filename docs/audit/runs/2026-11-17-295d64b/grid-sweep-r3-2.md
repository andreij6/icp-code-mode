# Variant sweep r3-2: limit checked after the work; limit on one dimension only

Run `2026-11-17-295d64b`, Phase 6, round 3. Pattern (seed, known, not re-reported: search
keyword count and length are unbounded while request bytes are capped by the platform):

> A limit checked after the work it is meant to prevent, and a limit on one dimension only:
> request bytes are capped by the platform, but the count derived from them is not.

Generalised for the sweep: (a) a check whose outcome does not depend on the work, placed
after that work; (b) an app limit on one dimension (bytes, elements, text units) that feeds a
platform or dependency limit on another dimension (encoded bytes, bindings, messages), with
nothing comparing the two before the work or the send.

Method: code reading at `295d64b`, plus live probes on the local network (port 8010) using
only `execute_icp_script` against allowlisted demo_store **queries** (`get_product`,
`search_products`) and scripts that call nothing; audit entries read with the controller
query `get_audit_logs`. Nothing was deployed, upgraded or reconfigured; no setting changed.
The bearer token was read from `.env` inside a helper script and never printed. Nothing was
compiled. Probe bodies lived in a private temp dir, deleted afterwards. R = reproduced live,
C = code path.

## Results

| Row | Check / limit | Verdict |
|---|---|---|
| O1 / LIM16 / B5 / U7: call arguments vs the platform payload limit | Value limit is 4 MiB of text (JS counts UTF-16 units), encoded Candid size is never compared with the inter-canister payload limit (10 MiB same subnet, 2 MiB cross subnet) before `with_raw_args(&enc.bytes)` (`src/bridge.rs:514-528`) | **finding SW3-2-1 (R)** |
| LIM23 / LIM27 / W11 Rhai compile vs `set_max_variables(1_000)` | `engine.compile(code)` (`src/runtimes/rhai_rt.rs:196`) runs before any budget check; Rhai's variable limit is enforced only when a `let` executes; compile is quadratic in distinct names | **finding SW3-2-2 (R)** |
| O1 / LIM1 composite_query refusal | refusal depends only on the method's type, checked after `reserve`, after `service()` (may fetch: second slot, up to 2 messages) and after full argument encoding (`src/bridge.rs:513-518`) | **finding SW3-2-3 (C)**, low |
| LIM23 / W8 JS compile | known (Boa quadratic bindings, SW2-0-2) | known |
| LIM23 / W10 Python compile | R: 100 KiB of 8,032 globals 0.28 B, 6,140 locals 0.23 B, 4,176 defs 0.47 B, 7,946 flat statements 0.31 B: linear; eval/exec known | holds |
| LIM27 Rhai other compile shapes | R: 12,700 shadowed `let a=0;` 0.19 B (linear); 9,390 statements 0.21 B; functions refused at parse (257th) ; map literal (r2-0) | holds |
| LIM27 Rhai runtime builders (`pad`, `blob`, `to_chars`) | C: `array.pad`, `blob(n)`, string `pad` call `throw_on_size` before allocating (rhai 1.27 `array_basic.rs:269`, `blob_basic.rs:85`, `string_more.rs:1298`); `to_chars`/`split` allocate at most 16 x the 1 MiB string first | holds |
| W8 JS RegExp compile with nested counted repetition | R: `(?:(?:(?:a{1000}){1000}){1000}){1000}` compiles in 22 M instructions (no unrolling) | holds |
| LIM23 `MAX_CODE_BYTES` placement | C: checked first in `LanguageRouter::execute` (`src/runtimes/mod.rs:52`), before any engine work | holds |
| LIM19 override size | C: size checked before `Service::parse` (`src/lib.rs:326-329`); parse cost known | holds / known |
| LIM19 fetched interface size | C: checked after the reply (bounded by the 2 MiB reply limit); fallback decode without quota known | known |
| LIM17 method length | JS/Python/Rhai copy first: known | known |
| LIM25 JS `MAX_PENDING` | C: checked before snapshotting args (`js.rs:253-257`) | holds |
| LIM25 / LIM1 JS args of calls the cap will refuse | C: each queued call's args are snapshotted, stringified (`js.rs:261-262`) and re-parsed (`js.rs:394`) before `reserve` refuses it; bounded by `budget(st)` per request and the per-round count | holds (cost bounded by the instruction budget; memory aspect is the accepted no-JS-memory-cap, register #24; see Not tested) |
| LIM9 / LIM8 / W1 | count after parse, parse bounded by ingress (r2-4) | holds |
| LIM5 / LIM13 / LIM14 / LIM15 / LIM6 / LIM7 / LIM11 / LIM31 / LIM32 | covered by r2-4 / r2-5 (count-only caps), known findings for logs, results, audit page, echoes | known / holds |
| LIM2 instruction budget | checked before fetch/encode not before send; compile (JS known, Rhai SW3-2-2); native ops (Python known) | known + SW3-2-2 |
| LIM3 / LIM4 / LIM12 / LIM18 / LIM20-22 / LIM24 / LIM26 / LIM28-30 | known findings cover each (timeouts, query path, slots, TTL failure cache, decoding quota, depth, safe int, Boa limits, Python budget, stack, memory, platform limits) | known |
| E1-E2 / E6-E11 / U11 / AD1-AD9 | admin inputs unbounded (known); override size before parse (holds) | known |
| E3 / U2 / U12 / W2 | header scan linear; decode quota known | holds / known |
| E4 / E4a / E4b / E5 / E5a-E5d | E5a: SW3-2-1, SW3-2-2, SW3-2-3; E5b search keywords seed; others known | see findings |
| E12-E15 | no derived count from caller bytes | n/a |
| E16-E19 host functions | SW3-2-1 (JS reaches 12 MB; Python/Rhai text limited to 4 MiB UTF-8, see SW3-2-1 scope) | finding |
| E20-E25 / U13 / W15 demo_store | `search_products` text per fixed product list: linear; orders known | holds / known |
| U1, U3-U10, U14-U16 | covered above or n/a (platform/browser/dev) | |
| O2-O10 | O2 args fixed-size; O3 entry size known; O6 reply cap holds; others n/a | holds / n/a |
| D1-D10, V1-V11 | no limit-after-work pattern beyond known (audit growth, caches, Symbol.for) | known / n/a |
| K1-K12 | SW3-2-1 adds a trap at the send in K1/K8 position | finding |
| F1-F16 | SW3-2-1 is a new trap site (system contract violation, not a Rust panic) | finding |
| B1-B17 | B5 SW3-2-1; others known or n/a | |
| VF1-VF8 | SW3-2-1/2 cost paid by the canister; no new flow | |
| S1-S10, C1-C15, A1-A12, CL1-CL37, T1-T12 | CL10 (4 MiB "in all three languages") is the documented limit SW3-2-1 exceeds the platform with; CL13 does not mention max_variables; T: no test sends an argument near the payload limit | see findings |

## SW3-2-1 Call arguments are never checked against the inter-canister payload limit: a JS argument within the value limits traps the canister

- Lens L3/L9 (also L5 clean failure). Invariant 5 (clean failure), 3 (records).
- Where: `src/bridge.rs:514-528` (`encode_args` then `Call::...with_raw_args(&enc.bytes)`;
  no size check). Value limit `MAX_VALUE_BYTES` 4 MiB (`src/bridge.rs:48`), counted in
  UTF-16 units by JS.
- R1 (warm-up `get_product`, then): `let s='一'.repeat(1000); while (s.length*2 <= 4100000)
  s = s + s; await icp.call(demo_store,'search_products',[s])` (4,096,000 units, 12,288,000
  UTF-8 bytes). HTTP 503, gateway details: `Canister violated contract: Request to
  4fbx2-...:search_products has a payload size of 12288026, which exceeds the allowed limit of
  10485760`. Audit: `Started` 9855 for `get_product`, then `Trapped` 9856 "instruction or
  memory limit, or a bug".
- R2 same string as the first message, inside `try { ... } catch`: same 503; the catch does
  not run; `get_audit_log_len` unchanged (9,857 before and after): the run is not audited.
- R3 1,024,000 units (3,072,000 bytes) is sent and answered (`found: 0`): the local subnet
  takes up to 10 MiB. Cross-subnet (mainnet, 2 MiB) the same 2-4 MiB argument would fail in
  `call_perform` with an opaque error after the slot and the encode (not reproducible on the
  single-subnet local network; unverified).
- Scope: JS reaches it directly with text (3 bytes per UTF-16 unit). Python and Rhai text is
  held to 4 MiB of UTF-8, so they reach 10 MiB only through type expansion (each missing
  `opt` field encodes a byte), which needs a wide allowlisted interface (code path, not
  reproduced).

## SW3-2-2 Rhai compiles before any budget check, and its 1,000-variable limit applies only at run time: a 100 KiB script spends 35.8 B instructions in compile

- Lens L3. Invariant 4 (bounded cost), 10 (budget documented as checked on every Rhai operation).
- Where: `src/runtimes/rhai_rt.rs:196` (`engine.compile(code)` before the replay loop;
  `on_progress` is the only budget check), `set_max_variables(1_000)` at `:130`.
- R: `if false{ let q; let z; ... }0` with N distinct names inside a block that never runs:
  N=250 25 M, 500 72 M, 1,000 249 M, 2,000 0.90 B, 4,000 3.55 B, 5,000 5.58 B (quadratic);
  10,233 `let qab=0;` 22.8 B; 12,792 `let qab;` 35.4 B; 12,944 densest names 35.8 B (102,393
  bytes, under `MAX_CODE_BYTES`). Each run reported `policy: instruction budget exceeded
  (35786363838 > 20000000000)` after the fact (audit id 9883). Executing the same `let`s
  stops at "Too many variables defined (line 1001)" only after compile (id 9860, 7.5 B).
  Shadowing one name is linear (12,700 lets 0.19 B), so the cost is the count of distinct
  bindings, which neither the 100 KiB byte cap nor `max_variables` bounds before compile.
- 35.8 B is 89% of the 40 B message limit; no trap was reached at default limits.
- Sibling of the known Boa compile finding (SW2-0-2); r2-0 measured Rhai statements and
  functions as linear but did not try distinct variable names.

## SW3-2-3 composite_query refusal comes after the call-cap slot, a possible interface fetch and full argument encoding

- Lens L3/L4. C only (demo_store has no composite_query).
- `call()` runs `reserve` (slot), `service()` (on a miss: second slot, up to two messages),
  `encode_args` (cost known to reach the budget for wide types), then refuses on
  `FuncMode::CompositeQuery`, a fact known from the interface alone. Refusal is an `Error`,
  not a recorded violation.

## Not tested

- Cross-subnet behaviour of 2-10 MiB arguments (single-subnet local network).
- Heap high-water from JS calls holding 4 MiB+ args (`args_json`, the parsed `Value` and
  `enc.bytes`) across awaits, x 25 calls x 8 runs: not probed live because Wasm memory never
  shrinks and the known wasm_memory_limit finding shows that growth blocks later updates.
- Whether a denser Rhai shape (other binding kinds) passes 40 B in compile.
