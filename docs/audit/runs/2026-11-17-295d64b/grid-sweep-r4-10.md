# Variant sweep r4-10: fallback that reinterprets an unsupported value as text

Run `2026-11-17-295d64b`, Phase 6, round 4. Source pattern (shared root of L7-4 / L7-10,
not re-reported): **a conversion meets a value it cannot represent and, instead of failing,
substitutes text (or null / an empty default) that then travels on as if it were the data.**

For every inventory row the question is: does a conversion here have a fallback branch
(`_ =>`, `unwrap_or*`, `py_repr`, `to_string`, a sentinel string) that is reachable with a
value the target format cannot hold, and does the run then continue as if the value were
valid?

Evidence key: **R** = reproduced in a private copy (native test `sweep_r4_10`, appended to
`src/runtimes/mod.rs` in a temp copy, shared `CARGO_TARGET_DIR`, copy deleted afterwards);
**CP** = code path unambiguous; **known** = already reported, not re-reported.

## Result

Two new candidates (S10-1, S10-2). Everything else holds or is a known finding.

| Id | Candidate | Evidence | Severity |
|---|---|---|---|
| S10-1 | Python `to_json` dispatches on `type_name()`, which for a class instance is the script-chosen class name; the builtin branches then fall back with `unwrap_or_default` / `map_or(Null)`. An instance of a class named `str`, `bool`, `list`, `dict`, `float`, `NoneType` leaves the sandbox as `""`, `false`, `[]`, `{}`, `null`, `null` (data and attributes discarded), also inside `icp.call` args, and the call is sent | R | Low |
| S10-2 | Non-finite Candid `float32`/`float64` reply values become the strings `"NaN"`, `"inf"`, `"-inf"` (`candid_bridge::f64_json`). The tool description and README document strings only for nat/int, so scripts treat them as numbers: JS `x > 1e308` is `false` and `x + 1` is `"inf1"`; Python `y * 2` is `"NaNNaN"`; Rhai `r[0] + 1` is `"inf1"`; all three runs report Ok | R | Medium |

### S10-1 reproduction (Python, MockBridge)

| Script | Result |
|---|---|
| `class str:\n    pass\nstr()` | `Ok("")` |
| `class bool:\n    pass\nbool()` | `Ok(false)` |
| `class list: ...` / `class dict: ...` | `Ok([])` / `Ok({})` |
| `class float: ...` / `class NoneType: ...` | `Ok(null)` / `Ok(null)` |
| `class bool:` with `self.v = True`; `{'approved': bool(), 'amount': 5}` | `Ok({"amount":5,"approved":false})` |
| `class str:` holding `self.s = s`; `icp.call(L, 'icrc1_balance_of', {'owner': str('bob')})` | call **sent** with `{"owner": ""}`; run Ok |
| `class str: pass`; `icp.call(L, str())` | call sent with method `""` (the `host()` TypeError signature check is bypassed) |
| Control: `class Thing: pass; Thing()` | `Ok("Thing()")` (the known L7-6 repr path) |

Root: `src/runtimes/python.rs:67-99` matches on `o.type_name()`; monty-types
`graph.rs:546-580` returns `class.name` for `MontyNode::ClassInstance`. The `"str"`
(`as_str().unwrap_or_default()`), `"bool"` (`as_bool().unwrap_or_default()`), `"float"`
(`map_or(Value::Null, ..)`), list/tuple/set/frozenset (`items().unwrap_or_default()`) and
`"dict"` (`pairs().unwrap_or_default()`) branches assume the node kind matches the name.
`"bytes"`/`"bytearray"` fall to `py_repr` (no MontyNode for bytearray exists, so the
`bytearray` arm is reachable only through a class of that name). Fix: dispatch on
`monty_types::unstable::node(o)` (the node kind), and make every non-matching case an error.

### S10-2 reproduction

`Service::parse("service : { f : () -> (float64, float32) query }")`, reply bytes
`(inf, NaN)` encoded with `IDLArgs::to_bytes_with_types`, decoded by `decode_reply` to
`["inf","NaN"]`, then fed to each runtime by a bridge that returns it:

| Runtime | Script | Result |
|---|---|---|
| JS | `const [x, y] = await icp.call(...); return {gt: x > 1e308, sum: x + 1, typ: typeof x, nan: Number.isNaN(y), y2: y * 2}` | `Ok({"gt":false,"sum":"inf1","typ":"string","nan":false,"y2":null})` |
| Python | `x, y = icp.call(...)`; `{'typ': type(x).__name__, 'sum': x + '1', 'rep': y * 2}` | `Ok({"typ":"str","sum":"inf1","rep":"NaNNaN"})`; `x > 0` raises TypeError (loud) |
| Rhai | `#{typ: type_of(r[0]), sum: r[0] + 1}` | `Ok({"typ":"string","sum":"inf1"})` |

The strings were chosen deliberately (register #39: so the value can be sent back, and
`float()` parses them), but nothing tells the script or the AI model: TOOL_DESCRIPTION
(`src/mcp.rs:81`) and README.md:358-359 list only nat/int as possibly-string numbers. A
typed float field changes JSON type depending on its value. Fix: document it in both
places (or return a tagged value), and add the case to the tool description's value list.

## Grid (every inventory row against the pattern)

| Row(s) | Verdict | Reason |
|---|---|---|
| C1, C1a, C2-C15 | n/a except as below | build/config/tooling rows hold no value conversion; C5 is B17; C12 video scripts pass values through as files |
| A1-A12 | n/a | actors; their inputs are the U rows |
| E1/E2 `apply` | holds | bad token hash traps (F1), no text fallback |
| E3 `inspect_message` | holds | decode failure rejects |
| E4/E4b `classify` | holds | non-string `method` or bad `id` type is -32600 with `id: null` (JSON-RPC rule); `params.name` non-string becomes `""` and is refused as "unknown tool" (loud, CP `mcp.rs:211`) |
| E5/E5a `parse_tool_call` | holds | `code` must be a string; `language`/`query`/`canister_id` via `opt_string` reject non-strings (`mcp.rs:202-208`); non-object `arguments` is known |
| E5b search | holds | `canister_id` that is not a principal yields no entries and a note (documented in `search.rs:36-37`); `as_service(..).unwrap_or_default()` (`search.rs:107`) turns a non-service actor into "no methods" rather than text: not this pattern |
| E5c/E5d callbacks | n/a | no value conversion |
| E6-E14 admin | holds | Candid-typed arguments; principals canonicalised; override key `to_text()` |
| E16 JS `icp.call/candid` | holds | `arg_string` refuses non-strings (`js.rs:195-200`); args via snapshot: undefined/function/symbol dropped is known ("A value the host cannot represent is dropped (JS)"); 128-deep -> null is known L7-2 |
| E17 JS console | holds (by design) | printing is text by definition; `<unprintable value>` marker is in the log only |
| E18 Python host functions | known (L7-6) for non-class values: `icp.call(L, Ellipsis)` sends method `"Ellipsis"`, `X()` sends `"X()"` (R) because args pass `to_json` before `host()` checks `Value::String`; class named `str` is S10-1 |
| E19 Rhai | holds | typed `ImmutableString` parameters; non-string canister/method is "Function not found" (R, `('a','f')` chars refused) |
| E20-E25 demo_store | holds | Candid-typed; no text fallback (only `format!` of a seeded email) |
| E26-E28 | holds | demo page `pretty()` shows non-JSON text as-is (presentation, not data); `String(e)` for errors |
| U1-U4 | holds | see E4 |
| U5 | holds | see E5 |
| U6/U7 | S10-1 (Python class names); known L7-4/L7-6/L7-10/L7-2/L7-7 | JS and Rhai dispatch on the value kind, not a name the script controls |
| U8 reply bytes | S10-2 | `f64_json` sentinel strings; other `idl_to_json` arms are lossless or documented (big ints as decimal strings, principals as text, reserved/null as null) |
| U9 reject text | holds | error channel; isError true |
| U10 interface text | holds | non-UTF-8 `candid:service` is an error (`bridge.rs:431`) |
| U11-U16 | holds | typed or n/a |
| O1-O10 | holds | O1 args are B5; others carry no converted value |
| D1-D10 | holds | `Decode!` failure traps (F3), never defaults |
| V1-V11 | n/a | no value conversion |
| K1-K12 | n/a | commit points |
| LIM1-LIM32 | n/a | limits; LIM22 drives B6 |
| W1-W16 | n/a | cost rows |
| F1-F16 | holds | F11 `f64 as f32` saturation is known L7-5 |
| B1 | holds | see E4 |
| B2 | holds | decode error rejects |
| B3 | holds | `get(..16).unwrap_or(whole)` keeps the hash |
| B4 | holds | `Principal::from_text` errors; search's padded id known |
| B5 `json_to_idl` | holds except known | text only from strings; bool only from bool; float from string uses Rust `parse` (accepts `NaN`/`inf`, the intended round trip of S10-2; overflow to inf known L7-5); numeric text leniency known L7-12; null/reserved accept anything known L7-11; func extra keys ignored (not this pattern) |
| B6 `idl_to_json` | **S10-2** | `f64_json` |
| B7 | holds | parse errors are errors (echo size known) |
| B8 JS<->JSON | known | L7-2, L7-4, L7-10, dropped values, boxed primitives; `error_string` renders a thrown `undefined`/function/symbol as `Uncaught null` (CP `js.rs:337`): the run still fails, error text only, not reported |
| B9 Python<->JSON | **S10-1**; known L7-4, L7-6, L7-7, L7-17, cycle/depth markers | `from_json` total over JSON |
| B10 Rhai<->JSON | holds | kind-based (`is_blob/is_array/is_map/is_string`), `from_dynamic` errors loudly for FnPtr/range; char -> string, unit -> null are faithful; NaN -> null known L7-4 |
| B11 tool result | known L7-16 | string results raw |
| B12 stable Candid | holds | `expect` traps (F2/F3) |
| B13 language | holds | unknown name is an error |
| B14 time | known | seconds truncation |
| B15 certificate | holds | fixed tree |
| B16 search JSON | holds | pretty-printed types are text by design |
| B17 demo page | holds | presentation |
| VF1-VF8 | n/a | value flows carry no conversion |
| AD1-AD9 | holds | typed, validated or known |
| S1-S10 | n/a | supply chain |
| CL1-CL37 | S10-2 touches CL29-adjacent value-mapping text (`mcp.rs:81`) and README.md:358-359 | no claim says floats are always numbers, but the value lists omit them |
| T1-T12 | gap | `candid_bridge::js_style_floats_and_non_finite` asserts `f64_json(NaN) == "NaN"` (locks S10-2 in, no script-side test); no test for Python class instances crossing `to_json` |
| History | known | #39 is the origin of S10-2's design |

## Not tested

- S10-1 and S10-2 on the deployed canister: both conversions are pure functions exercised
  natively with the shipped code; demo_store has no float method, so S10-2 needs a target
  with a float return type to show live.
