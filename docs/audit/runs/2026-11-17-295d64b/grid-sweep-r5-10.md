# Variant sweep r5-10: a precision rule applied on one path and not its sibling

Run `2026-11-17-295d64b`, Phase 6, round 5. Pattern:

> A precision rule applied on one path and not its sibling.

Question asked of every inventory row: where a value is converted with an exactness rule
(the JS-safe integer rule, a single correctly rounded conversion, a time resolution, a
canonical form), is the same rule applied on every sibling path that carries the same
kind of value: argument vs result, text vs number, one runtime vs another, one numeric
width vs another, query vs update?

Known and not repeated: L7-3 / L5r2-4 (integral floats > 2^53 accepted as exact ints),
S4-7-1 (JS args text round trip, 1 ulp), L7-4 (NaN/Infinity -> null), L7-5 (float32
saturation/underflow, float text overflow), L7-9 (JSON-RPC ids), L7-12 (lenient numeric
text), L7-16 / L7-16b (raw string results; Python clock in whole seconds), L7-17 (Python /
Rhai ints 2^53..2^63 as JSON numbers), -0.0 sign dropped by JS, S10-2 (non-finite float
replies as strings), UTF-16 vs UTF-8 text counting, log cap units.

Evidence: **R** = reproduced, **C** = code path. Probe: private copy
`/private/tmp/claude-501/sweep-r5-10.HQbD` (`git archive 295d64b`, no `target/`, no
uncommitted files), `CARGO_TARGET_DIR=/private/tmp/claude-501/audit-target`, throwaway test
module `candid_bridge::sweep_r5_10` (deleted with the copy). Dependency source read for
Boa 0.22 `JsValue::from_json`/`to_json`, Monty 1.6.0 `modules/time.rs`.

## New candidate S5-10-1: float32 slots round twice; float64 slots round once (R)

`json_to_idl` turns every float input into an f64 first (`float()`, `candid_bridge.rs:170-176`:
`Number::as_f64` or `str::parse::<f64>`) and then narrows with `as f32`
(`candid_bridge.rs:216`). For a `float64` slot that is one correctly rounded conversion. For
a `float32` slot it is two roundings (decimal or integer -> f64 -> f32), so an input that
lies just off a float32 midpoint is first rounded onto the midpoint and then tie-broken to
the even neighbour: the sent float32 is not the nearest one. JS is not affected (its value is
already an f64); Python and Rhai integers and every decimal string are.

`cargo test --lib sweep_r5_10 -- --nocapture` (native, same pure-Rust code as the wasm):

| Input to `float32` | Sent | Correctly rounded |
|---|---|---|
| `"1.0000001788139343261718749"` (text) | `1.0000002` (bits `3f800002`) | `1.0000001` (bits `3f800001`) |
| `1152921573326323713` = 2^60+2^36+1 (Python or Rhai int, a JSON i64) | `1.1529215e18` (`5d800000`) = 2^60 | `1.1529216e18` (`5d800001`) |
| Same text to a `float64` slot | correctly rounded (equal) | |

Of 200,000 decimal texts within ~1e-40 relative of a float32 midpoint (`{:.40e}` of the
midpoint), 39,176 were sent as the other neighbour than `str::parse::<f32>` gives. The run
reports Ok; nothing tells the script. Impact: 1 float32 ulp, only for inputs near a
midpoint; float32 is rare in ICP interfaces. Severity Low. Fix: parse text with
`str::parse::<f32>` for float32, and convert integers (`as_i64`/`as_u64`, big-int strings)
straight to f32 instead of through f64; add the two rows above as a test. Merge candidate:
same line as L7-5 (`as f32`), different mechanism (double rounding, not overflow).

## Cells

| Row | Verdict | Reason |
|---|---|---|
| C1 canister | finding S5-10-1 | in B5 |
| C1a feature subsets | holds | converters are shared code; no feature-gated numeric path |
| C2 demo_store | holds (beyond known) | prices/totals are u64 cents, `Nat` on the wire; `create_order` saturating `available` vs page's raw `on_hand - reserved` only differs after an oversell (known oversell finding) |
| C3 web-time shim | holds | instruction count -> `Duration::from_nanos` 1:1 in every method; `checked_add` and `checked_sub` both use `u64::try_from(as_nanos)` (panic in `Add` is F9, not precision) |
| C4 vendored chrono | n/a | used by Monty for dates; the precision choice is the `DateTimeSource::Fixed` seconds (known L7-16b) |
| C5 demo page | holds | `JSON.parse` of replies whose unsafe ints are already strings; `BigInt` for the supply sample; ratio `toFixed(1)` prints `1.0x` for 0.96 (cosmetic, part of the known benchmark finding) |
| C6 demo-setup.sh, C7 icp.yaml, C8-C10 build flags | n/a | no value conversion |
| C11 .did files | holds | integer types kept as typed Candid |
| C12 video, C13 audit tooling, C14 doc, C15 exports | n/a | no numeric conversion on a script/target path |
| A1-A12 actors | n/a | the pattern is about conversions; actors reach them only through U rows |
| E1/E2 init / post_upgrade | holds | token hash normalised (`trim`, lowercase) by `add_token_hash`, the same function `apply` uses |
| E3 inspect_message | holds | `verify_token` hashes the raw token, same as `authenticate` |
| E4/E4b query path vs E5 update path | holds | both serialise with `serde_json::to_vec`; neither re-parses a reply; search output carries only small counts |
| E5a execute | finding S5-10-1 (through `icp.call` args) | |
| E5b search | holds | renders types, not values |
| E6/E7 allowlist | holds | keyed by `Principal`, canonical in every reader (`to_text`) |
| E8 set_limits | holds | u32/u64 stored exactly |
| E9/E10 tokens | holds | add and revoke apply the same `trim().to_ascii_lowercase()` (`policy.rs:209, 218`) |
| E11 overrides | holds | stored under `canister.to_text()`; every reader canonicalises first (`bridge.rs:208, 440, 474`) |
| E12-E14 admin queries | holds | Candid, exact; `audit_page` uses checked arithmetic |
| E16 JS `icp.call` | known S4-7-1 | float64 args 1 ulp; float32 goes through f64 anyway |
| E17 JS console | holds | same `to_json_safe` + `normalize_numbers` as results; serde/ryu shortest round-trip formatting |
| E18 Python `icp.call` | finding S5-10-1 | ints to float32 double-rounded; to float64 `i64 as f64` single rounding; ints > i64 go as `py_repr` text, `str::parse::<f64>` correctly rounded |
| E19 Rhai `icp::call_canister` | finding S5-10-1 | INT to float32 double-rounded |
| E20-E24 demo_store | holds | typed Candid |
| U3/U4 request body | known L7-9 | ids only numeric field |
| U5 tool arguments | holds | all strings |
| U7 script values | finding S5-10-1; known L7-3, L7-17, S4-7-1 | |
| U8 replies | holds | `idl_to_json` `big()` applies the JS-safe rule symmetrically to negatives (`trim_start_matches('-')`) for nat, int, nat64, int64; float32 widened exactly |
| U9 rejects, U10 interface text | n/a | text |
| U11 admin inputs | holds | see E6-E11 |
| U12 inspect_message bytes | holds | Candid decode |
| U13 demo_store args | holds | typed |
| U15 platform values | known L7-16b | JS ms (`now_ms`), Python seconds; Monty's `time`, `monotonic`, `perf_counter` and `_ns` forms all read the same Fixed clock (`monty modules/time.rs:3-7`), so no second Python resolution |
| O1 call | finding S5-10-1 | float32 payload |
| O2 interface fetch | n/a | no numeric args |
| O3/O4 stable writes | holds | Candid exact |
| O5-O10 | n/a | no value conversion |
| D1-D5 | holds | Candid / String, exact round trip (`policy.rs:100-116`) |
| D6-D10 | n/a | |
| V1-V11 | n/a | counters and caches hold no converted values (DID_CACHE / OVERRIDES keyed by canonical text: holds) |
| K1-K12 | n/a | commit points carry no conversions |
| LIM1-LIM2 | holds | `>=` call cap and `>` budget used the same way in `authorize`, `budget_error`, the post-run checks (`lib.rs:151, 228`) |
| LIM3 | holds | u32 seconds passed straight to `change_timeout` (fetch path is the known 300 s finding) |
| LIM6/LIM7, LIM13-LIM15 | known | different units per path already reported (escaped vs raw bytes, markers) |
| LIM16 | known | UTF-16 vs UTF-8 counting |
| LIM21 MAX_DEPTH | known | off-by-one / Candid vs JSON levels |
| LIM22 MAX_SAFE_INT | holds on replies, known on inputs and Python/Rhai results | `normalize_numbers` uses the same 2^53-1 bound (`bridge.rs:89`); `num_text`'s `< 1e19` is L7-3 |
| Other LIM rows | n/a | sizes/counts, not precision |
| W1-W16 | n/a | cost rows |
| F11 casts | finding S5-10-1 (double rounding) + known L7-5 (saturation) | `bridge.rs:89` `f as i64` only for integral `|f| <= 2^53-1`: exact |
| F1-F10, F12-F16 | n/a | aborts, not precision |
| B1 body -> JSON | known L7-9 | |
| B2 Candid ingress | holds | |
| B3 token hash | holds | same normalisation on every path |
| B4 principal text | holds (search whitespace is known) | |
| B5 JSON -> Candid ints | known L7-3, L7-12 | strings exact, Python ints > i64 via exact `py_repr` |
| B5 JSON -> Candid float64 | holds | one correctly rounded step from text or integer, exact from f64 |
| B5 JSON -> Candid float32 | **finding S5-10-1** | two rounding steps |
| B5 blob elements | holds | same `int::<u8>` rule as nat8 |
| B6 Candid -> JSON | holds | rule is symmetric over sign and width; Float32 widened exactly |
| B7 .did parse | n/a | |
| B8 JS -> JSON | known S4-7-1 (args), -0.0 sign | results/console exact |
| B8 JSON -> JS | holds | Boa `from_json`: i32 range as int, else `as_f64`; inputs are <= 2^53 or f64, so exact (`boa_engine value/conversions/serde_json.rs:50-54`) |
| B9 Python -> JSON | known L7-6, L7-17; S5-10-1 downstream | floats exact (`Number::from_f64`) |
| B9 JSON -> Python | holds | i64 int, u64 bigint, else f64 |
| B10 Rhai <-> JSON | holds; S5-10-1 downstream | INT/FLOAT are i64/f64; no `f32_float`, no `decimal` |
| B11 result -> MCP text | known L7-1, L7-16 | serde/ryu, no re-parse |
| B12 stable Candid | holds | |
| B13 language name | holds | case folding identical in query and update (same `parse_tool_call`) |
| B14 time | known L7-16b | no third resolution found (Rhai `no_time`; Boa has no `performance`/Temporal: `default-features = false`, no `temporal_rs` in Cargo.lock) |
| B15 certificate | n/a | constant |
| B16 search JSON | n/a | type text |
| B17 demo DOM | holds | |
| VF1-VF3, VF5 | n/a | cycles, no value conversion in app code |
| VF4 assets | known L5r2-4; S5-10-1 only for float32 (no ICRC amount is float32) | |
| VF6 demo stock | holds | u32 quantities |
| VF7-VF8 | n/a | |
| AD1-AD9 | holds | see E rows |
| S1-S10 | holds | no crate enables serde_json `float_roundtrip`/`arbitrary_precision` or rhai `f32_float`/`decimal` (`Cargo.toml:36`, `cargo tree` per r4-7/r4-9) |
| CL10 / CL29 "identical semantics in every language" | finding S5-10-1 | JS float32 args are single-rounded from its f64, Python/Rhai ints double-rounded |
| CL other | n/a or known | README:359 "±2⁵³" vs code 2^53-1: the code is the JS-safe bound, wording only |
| T4 candid_bridge tests | gap | float32 tested only for -0/NaN/inf; no test compares float32 text/int with `parse::<f32>` |
| T5-T12 | n/a | |

Refute attempts: checked that the float64 path is correctly rounded for the same text
(equal); checked JS cannot reach a different value (its number is already the f64
midpoint); checked that the mechanism is not the overflow/saturation of L7-5 (inputs are in
range and finite).
