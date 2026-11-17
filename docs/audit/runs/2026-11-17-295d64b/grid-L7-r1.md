# Grid: L7 Data fidelity, round 1

Run `2026-11-17-295d64b`. Reviewer: L7 lens agent. Method: read every conversion in
`src/candid_bridge.rs`, `src/bridge.rs`, `src/runtimes/{js,python,rhai_rt}.rs`, `src/mcp.rs`,
`src/search.rs`, `src/lib.rs`, plus `src/demo.html` rendering; then probe tests in a private
copy (`cargo test --lib audit_l7`, natively, shared target dir). Probe ids P1-P15 refer to
`src/audit_l7.rs` / `mcp::audit_l7_mcp` in that copy (not committed). A property test
(P13b) ran 1.2M accepted values through json -> idl -> json -> idl for 10 primitive types:
no mismatch. Candidate findings are numbered L7-1 ... L7-17 in the lens output.

Evidence: **R** = reproduced natively, **C** = code path unambiguous.

| Row | Cell | Reason / evidence |
|---|---|---|
| B1 HTTP body -> JSON-RPC | **finding** L7-9 (Low) | Ids that do not fit i64/u64, or use exponent form, are parsed as f64 and echoed altered: `18446744073709551616` -> `1.8446744073709552e+19`, `1e2` -> `100.0` (R, P14). Duplicate object keys: last wins silently (R, `{"code":"A","code":"B"}` -> `B`); serde_json behaviour, listed under L7-9 as a note, not a separate finding |
| B2 Candid ingress -> `HttpUpdateRequest` | holds | ic-cdk derived Candid types (text headers, blob body); no coercion in app code |
| B3 token -> SHA-256 hex, caller prefix | holds | Exact hashing; `trim()` only strips surrounding whitespace that cannot be part of a header token; 64-bit caller prefix is an id, not data |
| B4 principal text <-> Principal | holds | `Principal::from_text` then `to_text` everywhere (bridge, search scope, candid args); invalid text is an error |
| B5 JSON -> Candid (`json_to_idl`) | **findings** L7-3, L7-5, L7-11, L7-12, L7-13, L7-15 | L7-3: integral floats above 2^53 (JS numbers) accepted as exact integers (R, P1: `12345678901234567` sent as `12345678901234568`; `9007199254740993` as `...992`). L7-5: float32 overflow -> `inf`, underflow -> 0, float64 text `"1e400"` -> `inf` (R, P4). L7-11: `null`/`reserved`-typed fields and null-payload variant tags accept and discard any value (`{"A": 5}` -> `A`) (R, P12). L7-12: underscores anywhere in numeric text (`"_5_"`, `"1_"`, `"100_200"`), repeated `0x` prefixes (`"0x0x0a"`) accepted (R, P9). L7-13: duplicate aliased record keys (`"5"`, `"_5_"`, `"05"`) for one id field: first wins, rest dropped silently (R, P15). L7-15: single `opt vec`/`opt blob` argument given `[]` becomes `opt vec {}`, while README says an array is the positional list (`[]` = zero args = `null`) (R, P15). Holds: unknown record fields rejected (#35), nat8 range, non-integral floats for ints rejected, `-0` (#40), hex blobs documented, round trip identity (P13b) |
| B6 Candid -> JSON (`idl_to_json`) | holds, with **finding** L7-14 (exotic) and #42 note | Ints > 2^53 as decimal strings, non-finite as `"NaN"/"inf"` strings, depth error >128 (#36). L7-14: a record with a quoted label `"1"` and an id label `1` maps both to JSON key `"1"`; one value is lost silently (R, P11). #42: `opt null` and `opt reserved` also collapse to `null` (R, P15), wider than the README's "`opt opt T`" wording (claims note, no new finding) |
| B7 `.did` -> TypeEnv | holds | candid_parser `check_prog`; class unwrapped; `as_service`/`as_func` trace vars; invalid text is an error. Stale or wrong overrides decode under Candid's opt-subtyping rule (mismatched `opt` field -> `null`): Candid spec behaviour, not app code |
| B8 JS <-> JSON | **findings** L7-2, L7-3, L7-4, L7-10 | L7-2: call args are serialised to text and re-parsed with `serde_json::from_str(..).unwrap_or(Value::Null)` (`js.rs:395`); serde_json's 128-level limit is one below the snapshot's, so args exactly 128 levels deep become `null` and the call is still sent (R, P2/P2b: JS sends `(null, null)`, Python sends `(vec.., "x")` for the same value). L7-4: `NaN`/`Infinity` -> `null` (R, P3). L7-10: lone surrogates become the literal six characters `\uD800` (R, P10). Holds: BigInt refused, depth/size limits, getters read once, `Map`->`{}` and typed arrays -> objects follow `JSON.stringify` semantics (typed array for a blob then fails loudly) |
| B9 Python <-> JSON | **findings** L7-4, L7-6, L7-7, L7-17 | L7-4: non-finite floats -> `null` (R, P3). L7-6: values of unsupported types become their repr text instead of an error: `range(3)` -> `"Repr('range(0, 3)')"`, `len` -> `"<built-in function len>"`, also inside call args (R, P6). L7-7: dict keys that are not `str` are stringified with repr and collide silently: `{1:'a','1':'b'}` -> `{"1":"b"}` (R, P6). L7-17: ints between 2^53 and 2^63 leave as JSON numbers (R, P15 `2**60`). Holds: big ints beyond i64 -> exact decimal string, bytes -> array (#38), depth error (#33) |
| B10 Rhai <-> JSON | holds, plus L7-4 and L7-17 | `from_dynamic` fallback errors loudly for ranges and `Fn` pointers (R, P7); char -> string, unit -> null. L7-4: `0.0/0.0` -> `null` (R, P3). L7-17: INT above 2^53 leaves as JSON number (R, P15). Replies above 10,000 elements fail Rhai's own size limit: loud |
| B11 result -> `CallToolResult` | **finding** L7-1 (High), L7-16 (Low) | L7-1: a value that passes the value limits (4 MiB text) but escapes to more than 1 MB is cut to 1 MB, invalid JSON, `isError: false`, audit `Ok` (R, P5/P5b: JS and Python 2 MB string, JS 30k-object array -> 829,644 chars, `parses_as_json=false`). Same path for search results (`lib.rs:189`). L7-16: string results are passed raw, so `"42"` and `42` produce the same text (R, P14) |
| B12 audit/limits Candid in stable memory | holds | Candid round trip of derived types; truncation of `error` (4 KiB) and violations (16 x 512 B) is documented (CL15) and marked |
| B13 language name | holds | Case-insensitive match on a closed set; anything else is an error |
| B14 time / instructions | **finding** L7-16b (Low) | Python's clock is block time truncated to whole seconds (`now_ms()/1000`, microsecond 0, `python.rs:214`); JS gets milliseconds. Same moment, different values per language (C). Instruction -> Duration is a budget, not data |
| B15 `IC-Certificate` header | not applicable | Fixed skip-certification tree, carries no caller data (L10 owns it) |
| B16 interface -> search JSON | holds | Signatures and types pretty-printed by candid at infinite width; `as_func` traces vars; cut methods counted (#41). Output passes through B11, so an oversized search result inherits L7-1 |
| B17 demo page JSON -> DOM | holds for L7, note | Results shown through `JSON.parse`/`JSON.stringify` (`pretty`, `demo.html:241`) round integers above 2^53 that L7-17 lets through; `esc(..).slice(0, 40)` can cut an entity in a display summary. Display only; injection is L17's |
| F11 casts | **finding** L7-5 | `f64 as f32` saturates (R, P4). `bridge.rs:89` f64->i64 only below 2^53: holds. `bridge.rs:333` len as u32 bounded by call cap: holds |
| I8 Fidelity invariant | **violated** | By L7-1, L7-2, L7-3 (the three that send or return wrong data as success), and the Low items. Known/accepted: #42 |
| LIM6 MAX_RESULT_BYTES | **finding** L7-1 | Boundary: 1,000,000 escaped bytes; a 2,000,000-char result is truncated, not refused (R) |
| LIM15 MAX_LOG_BYTES | **finding** L7-8 (Medium) | 100 x 1,000-char prints: JS and Rhai finish `Ok("done")` with truncated logs; Python fails with `MemoryError: memory limit exceeded: 66065 bytes > 65536 bytes` (R, P8) |
| LIM16 value limits | holds | Same depth/nodes/bytes errors in all three runtimes for results; for call args JS has the extra 128-level hole (L7-2) |
| LIM21 MAX_DEPTH | holds | Both directions error past 128; L7-2 is the text round trip, not this limit |
| LIM22 MAX_SAFE_INT | holds on output for Candid values; see L7-3 (input) and L7-17 (script results) |
| CL10 / CL29 "identical semantics in every language" | **finding** via L7-2, L7-6, L7-7, L7-8, L7-16b | Same value handled differently by runtime (R) |
| Register #34 | still fixed | `js::big_js_integers_pass_as_nat` passes; P1 shows the adjacent unsafe-range gap (L7-3) |
| Register #35 | still fixed | `errors_are_descriptive` passes; residual: duplicate aliased keys (L7-13) |
| Register #36 | still fixed | `deep_replies_error_instead_of_truncating` passes |
| Register #37 | still fixed, residual | `[x]` consistent; `[]` for a single array-like `opt` argument is the value, not "zero args" (L7-15) |
| Register #38 | still fixed | Python bytes and Rhai blobs -> arrays (guards pass) |
| Register #39 | still fixed (decode side) | `js_style_floats_and_non_finite` passes; the encode side maps script NaN/inf to `null` (L7-4) |
| Register #40 | still fixed | `-0.0` accepted as nat8 |
| Register #41 | still fixed | `search::unfetched_and_failed_interfaces_and_cap` passes |
| Register #42 | accepted, still accurate | Also covers `opt null` / `opt reserved` (P15) |

Not tested here: on-chain behaviour (wasm, real `IcBridge`, real targets) of any of the
above; the converters are the same code natively, but the live reply path, the gateway and
real MCP clients were not exercised (Live phase). JS lone-surrogate behaviour on a real
target (Candid `text` would carry the escaped form). No fuzzing of `idl_to_json` with
hostile wire bytes (only typed IDLValues); candid's own decoder was taken as given.
