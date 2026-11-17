# Variant sweep r4-7: lossy input accepted as exact; precision rule enforced on output only

Run `2026-11-17-295d64b`, Phase 6, round 4. Pattern under test:

> A lossy input accepted as exact; a precision rule enforced on output only.

Seed finding: L7-3 / L5r2-4 (integral JS floats above 2^53 accepted by `num_text` as exact
integers, while `idl_to_json` applies `MAX_SAFE_INT` to replies only). For each inventory row
the question is: does a value enter the app (from a script, a client, an operator or a
target) through a conversion that can silently lose precision or change the value, while
the app reports or treats it as the exact value?

Known and not repeated: L7-2 (JS args 128 deep -> null), L7-3 (integral floats > 2^53),
L7-4 (NaN/Infinity -> null), L7-5 (`f64 as f32` saturation/underflow, float text overflow),
L7-9 (JSON-RPC ids), L7-12 (lenient numeric/hex text), L7-16/L7-16b (raw strings, Python
time in seconds), L7-17 (Python/Rhai ints 2^53..2^63 as numbers), Python repr fallbacks.

## How it was tested

Private copy `/private/tmp/claude-501/sweep-r4-7.5flQ/repo` (no `target/`, `design/`,
`.env`, the uncommitted plan doc), `CARGO_TARGET_DIR=/private/tmp/claude-501/audit-target`.
Throwaway code (deleted with the copy):

- `examples/rt.rs`, `examples/rt2.rs`: serde_json 1.0.154 (the locked version; features
  `default`+`std` only per `cargo tree -e features -i serde_json`, so no `float_roundtrip`)
  `Value::from(f).to_string()` then `from_str`, bit comparison.
- `src/runtimes/mod.rs` test `sweep_r4_7::float_args_round_trip`: the same literal in JS,
  Python and Rhai, passed to `icp.call` through `LanguageRouter::execute` with
  `MockBridge`, then the recorded argument encoded with `candid_bridge::Service::encode_args`
  against `service : { f : (float64) -> (); n : (nat) -> () }`.

## Results

### New candidate S4-7-1: JS call arguments lose up to 1 ulp in a text round trip (reproduced)

`js.rs:262` stores the snapshot of the args as text (`a.to_string()`), and `js.rs:395`
parses it back with `serde_json::from_str`. serde_json is built without
`float_roundtrip`, so its default float parser is best-effort and differs from the
written value by 1 ulp for a large share of floats: 5,927,545 of 19,990,233 random finite
bit patterns; 10-30% of values per decade from 1e-30 to 1e40; 210,722 of 2,000,000 values
in [0,1). The result path (`to_json_safe` -> `Value`) has no text step and stays exact,
as do Python (`to_json` builds `Value` directly) and Rhai (`to_json_value`).

Same literal, three runtimes (native, `cargo test --lib sweep_r4_7 -- --nocapture`):

| literal | JS sent | Python / Rhai sent | Candid sent by JS |
|---|---|---|---|
| `0.36040155707569876` | `0.3604015570756987` | exact | `(0.3604015570756987 : float64)` |
| `1841021037.9102077` | `1841021037.9102075` | exact | `(1841021037.9102075 : float64)` |
| `1.1022301835603701e-10` | `1.10223018356037e-10` | exact | float64 1 ulp lower |
| `1.1296295699452381e18` | `1.129629569945238e18` | exact | `nat 1_129_629_569_945_238_016` (Py/Rhai `..._144`) |

The JS script's own `return {v: x}` shows the unaltered value, so the result and the
message disagree; the run reports Ok. Same site as L7-2 (both are the args text round
trip); distinct symptom: L7-2's depth fix would not fix this. Evidence: reproduced natively;
the wasm build runs the same pure-Rust parser (code path; not run on the local network).

### Cells

| Row | Verdict | Reason |
|---|---|---|
| B1 HTTP body -> JSON | holds (beyond L7-9) | Only numeric field used is `id` (L7-9). Tool arguments `code`, `query`, `canister_id`, `language` are strings |
| B2 Candid ingress -> HttpUpdateRequest | holds | Candid decode is exact or fails |
| B3 token -> SHA-256 | n/a | not numeric; trim/case-fold is documented |
| B4 principal text | holds | `Principal::from_text` checks the CRC; no lossy form accepted (whitespace in search is a known finding) |
| B5 `json_to_idl` ints | holds beyond L7-3/L7-12 | Non-integral floats refused; `|f| >= 1e19` falls to `n.to_string()` (`1e19`) and fails loudly; strings exact; Python ints > i64 arrive as exact decimal `py_repr` |
| B5 float64 | holds in `json_to_idl` | `n.as_f64()` exact; string via Rust `str::parse` (correctly rounded) |
| B5 float32 | known L7-5 | `as f32` rounding/saturation/underflow; no new symptom |
| B5 int -> float64 (Python int 2^53..2^63 to a float64 slot) | holds | rounding to the declared float type is the type's semantics, same as Python's own `float()` |
| B6 `idl_to_json` | holds | the output-side rule (`big`, `f64_json`) is the seed; no further loss |
| B8 JS -> JSON (result, console) | holds | direct `Value`; integral <= 2^53 -> i64 exact (`bridge.rs:89`) |
| B8 JS -> JSON (call args) | **finding S4-7-1** | text round trip, 1 ulp |
| B8 JSON -> JS (`JsValue::from_json`) | holds | replies hold numbers only <= 2^53 or f64 |
| B9 Python -> JSON | holds beyond L7-6/L7-17 | ints via `as_int`/repr exact; floats via `Number::from_f64` exact |
| B9 JSON -> Python | holds | i64 -> int, u64 -> bigint, else f64 |
| B10 Rhai <-> JSON | holds | `from_dynamic`/`to_dynamic` direct; FLOAT is f64 (no `f32_float`), INT i64 overflow is a parse/eval error |
| B11 result -> MCP text | holds beyond L7-1/L7-16 | serde_json `to_string` (ryu shortest round-trip), no re-parse (`mcp.rs` parses only the body, `:294`) |
| B12 stable Candid | holds | exact encode/decode |
| B13 language name | n/a | not numeric |
| B14 time / instructions | known L7-16b; holds otherwise | ns -> ms is the JS Date resolution; instruction count -> `Duration` 1:1 |
| B15 certificate header | n/a | constant bytes |
| B16 search JSON | n/a | text rendering |
| B17 demo page | holds | `JSON.parse` of replies whose unsafe ints are already strings; BigInt used for the supply sample |
| U5/U6/U7 script values | finding S4-7-1 (JS args) | other runtimes exact |
| U8 replies | holds | decoded exactly, output rule applied |
| U11 / AD4 `call_timeout_seconds > 300` | holds | IC caps at 300; documented in `.did:70`, `policy.rs:32`, README:432 |
| U11 / AD4 `max_instructions` | holds | u64 kept exactly; Python budget is a 1:1 Duration |
| U13 demo_store args | holds | Candid typed; overflow on arithmetic is a known finding, not input precision |
| E13 `get_audit_logs` limit | holds | cap at 100 documented (`.did`, CL32) |
| LIM16 / LIM22 value limits | holds | MAX_SAFE_INT is output-only by design; the input side is L7-3 |
| D1-D5 durable state | holds | Candid/String, exact |
| V2/V5 interface caches | n/a | text |
| VF4 amounts | known (L5r2-4) + S4-7-1 | an amount carried as a non-integral float has no Candid type; an integral float > 2^53 is L7-3 and is further changed by S4-7-1 in JS |
| CL10 / CL29 "identical semantics in every language" | finding via S4-7-1 | JS sends a different float than Python/Rhai for the same literal |
| T4/T8 tests | gap | `candid_bridge` round-trip tests start from `Value`, never from a JS float through `icp.call`; no test compares sent args across runtimes |

Refute attempts: checked `cargo tree` for any crate enabling `float_roundtrip` or
`arbitrary_precision` through feature unification (none); checked that integral values
<= 2^53 are unaffected (normalised to i64 before the text step); checked that the result
path is exact (it is, so the defect is confined to args).
