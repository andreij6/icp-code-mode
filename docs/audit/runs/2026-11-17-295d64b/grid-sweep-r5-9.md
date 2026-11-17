# Variant sweep r5-9: same value, different precision in sibling runtimes

Run `2026-11-17-295d64b`, Phase 6, round 5. Pattern (seed: L7-16b / L11-3, known and not
re-reported: Python sees block time truncated to whole seconds, JS milliseconds, Rhai no
clock):

> The same value handled at different precision by sibling runtimes.

Question asked of every inventory row: is one value (a time, a number, a count, a length,
an identifier, a text) converted, measured or truncated by two or more sibling
implementations (the JS/Python/Rhai runtimes, query/update routes, metadata/tmp_hack fetch
paths, script/search runs, native/wasm32 builds, server/demo page), and does one of them
keep less of it than the others?

Known and not repeated (from the task list): L7-16b / L11-3 (clock seconds vs ms, Rhai no
clock), S4-7-1 (JS args 1 ulp via serde_json text), L7-3 (integral floats > 2^53),
L7-4 (NaN/Inf), L7-5 (float32 saturation), L7-17 (Python/Rhai ints 2^53..2^63 as numbers),
JS -0.0, UTF-16 vs UTF-8 text counting, JS key bytes, lone surrogates (three behaviours),
depth off by one, MAX_DEPTH Candid vs JSON levels, log cap raw vs escaped, Python print
cap, blob element counting, register R1 refutation (numbers vs decimal strings by
magnitude is deliberate).

## Method

- Read every conversion site in `src/` for the three runtimes (`js.rs` snapshot,
  `to_json_safe`, `display`, `from_json`; `python.rs` `to_json`/`from_json`, OsPolicy;
  `rhai_rt.rs` `to_json_value`, `to_dynamic`), `candid_bridge.rs` (`num_text`, `float`,
  `big`, `f64_json`, `key_matches`, variant tag match), `bridge.rs` (`now_ms`,
  `normalize_numbers`, `instruction_deadline`, `truncate`, TTL arithmetic), `lib.rs`
  (`run_search`/`run_script` audit values, `inspect_message`), `mcp.rs` (`authenticate`,
  `truncate_escaped`), `policy.rs` (`audit_page`, `verify_token`), `search.rs`,
  `demo_store/src/lib.rs`, `src/demo.html`.
- Dependency source: Boa 0.22 `JSON.parse` (serde_json only validates; values come from
  Boa's own parser), Monty 1.6 `time` module and `OsPolicy` defaults (`ProcessTime::Zero`,
  zone UTC), Monty `json.loads` via jiter 0.17 (`lexical-parse-float`, correctly rounded),
  Rhai 1.27 serializer (`serialize_u64` -> INT when it fits, else FLOAT; unreachable here).
- One native probe (R): private copy without `target/`, `.git`, `design/`, `.env`, `.icp`,
  `video/`, the uncommitted plan doc; `CARGO_TARGET_DIR=/private/tmp/claude-501/audit-target`;
  `cargo test --lib sweep_r5_9 -- --nocapture`; copy deleted afterwards. The test ran the same
  number literal in JS, Python and Rhai through `LanguageRouter::execute` with `MockBridge`
  (which echoes the `owner` argument back as the reply), and recorded the result, the sent
  argument and the printed line. Nothing deployed, nothing called on the local network.

Probe output (native):

| literal | JS result / sent / log | Python | Rhai |
|---|---|---|---|
| `5.0` | `5` / `5` / `5` | `5.0` everywhere | `5.0` everywhere |
| `0.1+0.2` | `0.30000000000000004` in all three, all channels | same | same |
| `9007199254740991` | exact everywhere | exact | exact |
| `2**53` | `9007199254740992.0` (result, sent, log) | `9007199254740992` | `9007199254740992` |
| `1e16`, `1e21`, `1e300` | `1e+16` etc. | `1e+16` etc. | result `1e+16`, log `1e16` |
| `0.000001` | `1e-6`, log `1e-6` | `1e-6`, log `1e-06` | `1e-6`, log `0.000001` |
| `0.10000000149011612` | exact everywhere | exact | exact |
| `123456.789`, `-5` | exact everywhere | exact | exact |

Every value round-tripped (script -> args -> reply -> script -> result) to the same f64 or
integer in all three runtimes; only the text spelling of the same value differs (JS prints
integral floats as integers below 2^53 and as `N.0` above; print formats differ per
language). No precision is lost outside the known S4-7-1 text round trip, which these
literals happen not to hit. Off-pattern observation from the same probe: Python's captured
output is split with `str::lines`, so `print('a\r\nb\rc')` is logged as `["a", "b\rc"]`
(the `\r` of the CRLF is dropped), where JS keeps `"a\r\nb\rc"`; text fidelity of a log line,
not precision, Low, not raised as a candidate.

## Candidates

None new. Every hit of the pattern found in this sweep is one of the known findings listed
above.

## Cells

R = reproduced (probe above), C = code path.

### Components, actors, entry points

| Row | Verdict | Reason |
|---|---|---|
| C1 | holds beyond known | the three runtimes are the siblings; see B8-B10, B14 |
| C1a | holds | feature subsets compile the same conversion code; no subset-specific precision (language default/validator issues are known) |
| C2 demo_store | holds | prices, totals and revenue are `u64` internally and `Nat` on the wire (`Nat::from(u64)`, exact); overflow/saturation are known (wrap, `restock` saturation) |
| C3 web-time shim | holds | `Instant` = `performance_counter(1)` in "ns" = instructions, the same counter JS/Rhai/policy use (`bridge.rs:128-130`); no rounding in `Duration` arithmetic (saturating/checked) |
| C4 vendored chrono | holds | Monty datetime precision comes from `DateTimeSource::Fixed{microsecond: 0}` (known), not chrono |
| C5 demo page | holds beyond known | `pretty()` re-parses result text with `JSON.parse`, so script ints > 2^53 (Python/Rhai, L7-17) display rounded: symptom of L7-17, not separate. `tok = chars/4` uses UTF-16 `length` for both columns (consistent within the page) |
| C6 demo-setup.sh | n/a | no value conversion |
| C7 icp.yaml | n/a | build steps |
| C8 .cargo/config | n/a | flags |
| C9 toolchain | n/a | |
| C10 release profile | holds beyond known | overflow-check difference native vs wasm is known (demo_store wrap) |
| C11 .did files | holds | `timestamp_ns : nat64` consistently ns; Limits units documented per field |
| C12 video/ | n/a | dev tooling, no shared values with the runtimes |
| C13 audit tooling | n/a | |
| C14 idea doc | n/a | |
| C15 exports | holds | getrandom stream is one u64 splitmix64 source for every engine (C) |
| A1-A12 | n/a | actors carry no values of their own; their inputs are U rows |
| E1/E2 init/upgrade | holds | `Limits` decoded once, u32/u64 as declared |
| E3 inspect_message | holds | calls the same `mcp::authenticate` as the method (`lib.rs:276-277`), so the token is parsed identically (decode-quota difference is known) |
| E4/E4a/E4b query | holds beyond known | query and update render search with the same `search::render`; known route differences (MAX_SEARCHES, DID_FAILED, budget) are not precision |
| E5/E5a/E5b update | holds | `run_script` records the instruction count it checked; `run_search` re-reads `instructions_used()` for the entry after the check (`lib.rs:151` vs `:180`), a few million instructions later at most (error join/truncate): cosmetic, not raised |
| E5c/E5d callbacks | holds beyond known | Trapped `instructions` omission is known |
| E6-E11 admin | holds | allowlist/override keys use `Principal::to_text()` everywhere; methods compared exactly in policy and search |
| E12 get_policy | holds | values returned as stored |
| E13/E14 audit paging | holds | `audit_page` works in `u64` with checked arithmetic, no `usize` cast, so wasm32 and native agree for any offset (C) |
| E15 | n/a | |
| E16 JS SDK | holds beyond known | args text round trip (S4-7-1) and -0 known; replies via `JsValue::from_json` exact (R) |
| E17 JS console | holds | numbers printed via `normalize_numbers` + serde_json: same f64 as the value (R), spelling differs from JS `String()` above 2^53 (`9007199254740992.0`), no precision lost |
| E18 Python SDK | holds beyond known | `to_json`/`from_json` exact for i64/f64 (R); bigints as repr strings known |
| E19 Rhai SDK | holds | `to_json_value`/`to_dynamic` exact for INT/FLOAT (R) |
| E20-E23 demo_store | holds beyond known | see C2 |
| E24 tmp_hack | holds | both fetch paths decode interface text strictly as UTF-8 (`String::from_utf8`, `candid::<String>`), no lossy variant (`bridge.rs:428-436`) |
| E25 | n/a | |
| E26 demo buttons | holds beyond known | see C5 |
| E27/E28 | n/a | |

### Inputs, effects, state

| Row | Verdict | Reason |
|---|---|---|
| U1/U2 | holds | header name and scheme compared case-insensitively in one function used by both routes |
| U3/U4 | holds beyond known | ids (L7-9) known; no other float in request JSON reaches the app |
| U5 | holds | `language` canonicalised by `FromStr` before audit (`lib.rs:250`); search key from raw text known |
| U6 | holds | `MAX_CODE_BYTES` measured on the same `&str` bytes for all three (`runtimes/mod.rs:52`) |
| U7 | holds beyond known | counting units known (UTF-16 vs UTF-8, keys, blobs); numeric precision equal except S4-7-1 (R) |
| U8 | holds | `idl_to_json` is shared: nat/int <= 2^53 numbers, else decimal strings, float32 widened exactly, in all runtimes (C, R for the widened value) |
| U9 | holds | error text passed as a string to each engine unchanged |
| U10 | holds | see E24 |
| U11 | holds | `call_timeout_seconds` u32 seconds passed to `change_timeout(secs)` unchanged; `max_instructions` u64 used with saturating add (`bridge.rs:546`) |
| U12 | holds | see E3 |
| U13 | holds beyond known | see C2 |
| U14 | holds beyond known | see C5 |
| U15 platform values | holds beyond known | time: ns in audit/TTLs, ms (JS), s (Python), none (Rhai): known. Instruction counter: one call-context counter for policy, JS, Rhai and Monty (C) |
| U16 | n/a | |
| O1 | holds beyond known | argument bytes come from shared `encode_args`; only JS's arg text step differs (known) |
| O2 | holds | one fetch routine for all callers |
| O3 | holds | `timestamp_ns` from `time()` at write in every entry kind |
| O4-O10 | n/a / holds | no value converted at two precisions |
| D1 | holds | `Limits` fields have one unit each, read the same way by every consumer (snapshot vs live is known, not precision) |
| D2/D3/D5 | holds | keys canonical (`Principal`, lowercase hex, principal text) |
| D4 | holds beyond known | caller id is the first 16 hex chars of the token hash in both `run_script` and `run_search` (`lib.rs:122, 222`): same truncation everywhere |
| D6-D10 | n/a | |
| V1-V5 | holds | DID TTLs compare `time()` ns with ns constants on both routes (`bridge.rs:278-287`) |
| V6 | holds | memory counted in bytes by one allocator |
| V7 | holds | one stream |
| V8-V11 | n/a | |
| K1-K12 | n/a | commit points carry no converted value |

### Limits, work, failures

| Row | Verdict | Reason |
|---|---|---|
| LIM1 | holds beyond known | slots vs messages known |
| LIM2 | holds beyond known | same counter and same deadline (`base.saturating_add(max)`) in JS/Rhai/Python (`vm_budget` = deadline - counter); check granularity differences are known |
| LIM3 | holds beyond known | seconds everywhere; fetches using the 300 s default is known |
| LIM4-LIM11 | holds | single implementation each |
| LIM12 | holds | |
| LIM13/LIM14/LIM31 | holds | all use `bridge::truncate` (bytes on a char boundary) |
| LIM15 | holds beyond known | bytes in all three (Monty `check_print_collect_limit` counts bytes); newline counting and raise vs truncate known |
| LIM16 | holds beyond known | known counting differences |
| LIM17 | holds | bytes, one check in `IcBridge::call` |
| LIM18 | holds | ns |
| LIM19 | holds | `did.len()` bytes for fetched and override text |
| LIM20/LIM21 | holds beyond known | |
| LIM22 | holds | `normalize_numbers` (2^53-1) and `big()` (2^53-1) agree; `num_text`'s 1e19 bound is an input-acceptance bound (L7-3 known) |
| LIM23 | holds | see U6 |
| LIM24-LIM27 | holds beyond known | per-engine limits are documented per language; their units are known findings |
| LIM28-LIM30 | n/a | platform |
| LIM32 | holds | single implementation |
| W1-W16 | n/a | the pattern is about value fidelity, not work size; W12 `truncate_escaped` counts escaped bytes as documented |
| F1-F10 | n/a | aborting constructs, no shared value |
| F11 casts | holds beyond known | `len as u32` (counts << 2^32); `f64 as f32` (L7-5 known); `f as i64` guarded to <= 2^53 |
| F12-F15 | n/a | |
| F16 | holds beyond known | demo_store wrap known |

### Conversions, flows, admin, supply, claims, tests

| Row | Verdict | Reason |
|---|---|---|
| B1 | holds beyond known | serde_json parse of the body; only ids carry numbers (L7-9) |
| B2 | holds | see E3 |
| B3 | holds | one hash, one 16-char caller prefix |
| B4 | holds beyond known | whitespace difference search vs call known |
| B5 | holds beyond known | float32 from text goes str -> f64 -> f32 (double rounding possible in rare halfway cases) for every runtime alike: not a sibling difference; `key_matches` accepts `07`/`+7` for record ids but variant tags only `7`/`_7_`: aliasing known (L7-12/L7-13) |
| B6 | holds | shared by all runtimes (R) |
| B7 | n/a | |
| B8 | holds beyond known | (R) integral floats <= 2^53 become integers, above stay floats; -0 and S4-7-1 known |
| B9 | holds beyond known | (R) exact for i64/f64; repr fallbacks known |
| B10 | holds | (R) exact; Rhai serializer branch for u64 > i64::MAX unreachable (replies above 2^53 are strings) |
| B11 | holds beyond known | raw strings known |
| B12 | holds | Candid round trip of stored records |
| B13 | holds | ASCII case fold |
| B14 | known | L7-16b / L11-3 (the seed) |
| B15 | n/a | |
| B16 | holds beyond known | label rendering known |
| B17 | holds beyond known | see C5 |
| VF1-VF8 | n/a | no value is accounted at two precisions (cycles are not tracked in-app) |
| AD1-AD9 | holds | admin values have one unit each (see D1, U11) |
| S1-S10 | holds | serde_json without `float_roundtrip` matters only at `js.rs:395` (S4-7-1); Boa `JSON.parse` uses its own parser for values, Monty `json.loads` uses jiter/lexical (correctly rounded) |
| CL10 | holds beyond known | "in all three languages": precision of numbers equal (R) except S4-7-1 |
| CL11/CL29 | known | clock claims (L11-3) |
| CL1-CL9, CL12-CL28, CL30-CL37 | n/a for this pattern | no precision claim, or covered by known findings |
| T1-T12 | gap (known) | no test compares the same literal across the three runtimes' result, argument and print paths; `sweep_r5_9` above is such a test (throwaway). Recorded as a testability note, already covered by known "no cross-runtime differential tests" items |

## Not tested

- The wasm32 build was not run; the conversions involved are pure Rust and identical
  across targets (C).
- Formatting of printed values was compared only for the literals in the table.
