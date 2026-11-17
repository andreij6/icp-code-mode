# Variant sweep r4-8: a value with no JSON form becomes a substitute

Run `2026-11-17-295d64b`, Phase 6, round 4. Pattern (shared root with L7-6 and L7-10):

> A value with no JSON form is mapped to a substitute instead of an error or the string form
> used elsewhere (runtime -> JSON conversion substitutes unrepresentable values).

Known and not re-reported: L7-2 (128-deep JS args -> `null`), L7-4 (NaN/Infinity -> `null` in
results and args, all three runtimes), L7-6 (Python repr fallback), L7-7 (non-str dict keys),
L7-10 (JS lone surrogates -> literal `\uD800`), L6-9 (JS drops functions/undefined in results),
SW1-3 (boxed primitives), SW3-9-1 (Monty `<deeply nested>` / `[...]` markers), `<unprintable
value>` (README:369 finding), Rhai `from_dynamic` refusals (holds).

Question asked of every row: where does a value with no JSON (or no UTF-8) form cross a
boundary, and does it come out as an error, as the string form the rest of the system uses
for it, or as a silent substitute?

Evidence: **R** = reproduced, **C** = code path. Probes ran natively as a `#[cfg(test)]`
module (`src/sweep_probe.rs`) in a private copy made with `git archive 295d64b` (no
`target/`, no uncommitted files), built with the shared `CARGO_TARGET_DIR`. The copy's
package version was changed to `0.1.0-sweepr48`, because with the shared target directory
another copy's test binary was otherwise reused without compiling this one. The copy was
deleted afterwards. No live canister was touched.

## Candidates

### SW4-8-1 (Medium). JS console output and uncaught thrown values are formatted through the JSON snapshot, so Errors print as `{}`, NaN/Infinity/functions as `null`, Symbols as an empty string and BigInts as `<unserializable value>`

`display` (`src/runtimes/js.rs:217-225`) formats every non-string argument of
`console.log/info/warn/error/debug` and `print` with `to_json_safe`, the JSON snapshot
used for results. `error_string` (`js.rs:335-339`) does the same for uncaught non-Error
thrown values and rejection reasons. JSON has no form for an Error's message, NaN, Infinity,
functions or undefined, so these print as JSON substitutes. JavaScript's own string form
(`String(e)` prints `Error: policy: ...`) is not used. For Symbols, the special case calls
`to_string`, which throws for a Symbol, and `unwrap_or_default()` turns that into `""`.

R (native, MockBridge):

| Script | JS output | Python / Rhai for the same thing |
|---|---|---|
| `try { await icp.call('aaaaa-aa','raw_rand',null) } catch (e) { console.log('failed:', e) }` | log `failed: {}` | Python `print('failed:', e)`: `failed: policy: canister aaaaa-aa is not allowlisted`; Rhai the same |
| same, `console.log({err: e})` | `{"err":{}}` | Python `{'err': RuntimeError('policy: ...')}`; Rhai `#{"err": "policy: ..."}` |
| `try { null.x } catch (e) { console.log(e); console.error(e) }` | `{}`, `[error] {}` | |
| `console.log(NaN); console.log(Infinity, -Infinity)` | `null`, `null null` | Python `nan inf`; Rhai `NaN`, `inf` |
| `console.log('x', NaN, 'y')` | `x null y` (but `String(NaN)` prints `NaN`) | |
| `console.log(() => 1)` / `console.log(Symbol('s'))` / `console.log(10n)` | `null` / `""` (empty line) / `10` | Python `print(len)`: `<built-in function len>` |
| `console.log([NaN, undefined, () => 1])` | `[null,null,null]` | Python `[nan]`; Rhai `[NaN, ()]` |
| `console.log(new Date(NaN))` | `null` | |
| `throw undefined` / `throw NaN` / `throw Infinity` / `throw Symbol('s')` / `throw () => 1` / `await Promise.reject(undefined)` | run error `Uncaught null` | Rhai `throw 0.0/0.0` -> `Runtime error: NaN`; `throw ()` -> `Runtime error` |
| `throw 10n` / `await Promise.reject(10n)` | `Uncaught <unserializable value>` (while `console.log(10n)` prints `10`) | Rhai `throw 10` -> `Runtime error: 10` |
| `throw [NaN, undefined]` / `throw new Map([[1,2]])` | `Uncaught [null,null]` / `Uncaught {}` | |

- Impact: the most common JS error idiom, `catch (e) { console.log(e) }`, hides the reason
  for every policy refusal, call rejection and runtime error. The captured output is what
  the AI model reads to debug its script, and it gets `{}`. The same script in Python or
  Rhai shows the message. The tool description promises "identical semantics in every
  language" (`src/mcp.rs:79`, shown to the AI). Thrown substitutes reach the tool result
  text and the closing audit entry's `error` field (`lib.rs` run_script, C): `throw NaN`
  and `throw undefined` are both recorded as `Uncaught null`, and `throw 10n` loses its
  value.
- Not covered by known findings: L7-4 / L6-9 are about values that leave as results or
  call arguments, where JSON is the contract. Here the target is human-readable text, and
  the JavaScript string form exists. `<unprintable value>` (README:369 finding) is the
  snapshot error path. This finding is the snapshot success path returning substitutes.
- Refuted alternatives: strings, `undefined` at top level and BigInt in `console.log` are
  special-cased and print correctly. Thrown `Error` objects go through `try_native` and
  keep their message. Only non-Error throws and console arguments are affected.
- Fix: format console arguments and thrown values with a bounded ToString-style formatter,
  not the JSON snapshot. For example, use `String(x)` for primitives, symbols, functions
  and Error objects (`name: message`), and the snapshot only for plain objects and arrays,
  with NaN/Infinity printed as `NaN`/`Infinity`. Remove the dead `is_symbol()` branch, or
  use `Symbol.prototype.toString`. Regression test: in JS, `catch (e) { console.log(e) }`
  after a policy refusal must log text that contains `not allowlisted`, as Python and Rhai
  do.

### SW4-8-2 (Low). Python string literals with a lone-surrogate escape are silently replaced by U+FFFD and sent

Monty's parser, `ruff_python_parser` 0.0.14 (`src/string.rs:179`, in `Cargo.lock`), maps
`\uD800`-`\uDFFF` escapes to `std::char::REPLACEMENT_CHARACTER`. A Monty `str` is strict
UTF-8, so the script never sees the surrogate it wrote, and nothing reports the change.
CPython keeps the surrogate in the `str` and raises `UnicodeEncodeError` when it is
encoded. JS has the same input and sends the six literal characters `\uD800` (L7-10).
Rhai refuses the literal at compile time (`syntax error: Invalid escape sequence`). The
same value thus has three outcomes, and only Rhai's is an error.

R (native, MockBridge):
- `icp.call(L, 'icrc1_balance_of', {'owner': 'aaaaa-aa', 's': 'x\ud800y'})`: the call is
  sent with `s: "x\u{FFFD}y"` and returns Ok. JS sends `"x\\uD800y"`, also Ok.
- `s = 'x\ud800y'; [len(s), s == 'x�y', ord(s[1])]` -> `[3, true, 65533]`.
- `'a\udc00b'.encode('utf-8')` -> bytes `97, 239, 191, 189, 98` (CPython raises).
- `print('x\ud800y')` logs `x\u{FFFD}y`.
- Loud siblings (hold): `chr(0xD800)` raises `ValueError: chr() arg not in range(0x110000)`,
  and Monty's codecs raise `NotImplementedError` for `surrogateescape`/`surrogatepass`
  (`monty-1.6.0 src/codecs.rs:230-262`). So Monty treats lone surrogates as errors
  everywhere except in the parser.
- Overlap: the same class as L7-10 (lone surrogates do not survive and the run is still Ok),
  through a different mechanism, in a dependency's lexer. The verifier may merge it into
  L7-10 as the Python half of a cross-runtime finding.
- Fix: before `MontyRun::new`, refuse sources that contain `\uD8xx`-`\uDFxx` escapes in
  string literals. A conservative check: scan the source for `\\u[dD][89a-fA-F]` and
  `\\U0000[dD][89a-fA-F]`. Otherwise document the substitution. Better, fix L7-10 and this
  together, so that a lone surrogate is an error in every runtime. Regression test: the
  Python `icp.call` above must fail and send nothing.

## Grid

| Row | Where a no-JSON-form value crosses | Cell |
|---|---|---|
| C1 runtimes | JS console + throw formatting; Python lexer | **SW4-8-1**, **SW4-8-2** |
| C1a feature subsets | same code per runtime | n/a (per-runtime behaviour unchanged by subset) |
| C2 demo_store | Candid only | n/a: every Candid value has a Candid form |
| C3 web-time shim | instruction count -> `Duration` | n/a: integers, no JSON |
| C4 vendored chrono | Monty datetime | n/a: no JSON conversion |
| C5 demo page | `pretty(s)` falls back to the raw text on parse failure (`demo.html:241`); `JSON.parse` of results (`:458`) throws on non-JSON | holds: display fallback is the original text, not a substitute; string results raw is known (L7-16) |
| C6-C15 | scripts, build, interface files, video, audit tooling, exports | n/a |
| A1-A12 | actors | n/a: the pattern is about values, not actors |
| E1-E3 | Candid args | n/a |
| E4/E5 | JSON-RPC ids | known (L7-9); `HttpResponse::json` `to_vec(..).unwrap_or_default()` (`mcp.rs:62`) and `tool_result` `to_string(..).unwrap_or_default()` (`mcp.rs:159`) cannot fail for a `serde_json::Value` (string keys only): holds (C) |
| E5a | closing audit `error` from script error text | **SW4-8-1** (`Uncaught null` / `<unserializable value>` recorded) |
| E5b | search render: principal text, pretty-printed types (`search.rs:49-159`) | holds (C): all values are text |
| E5c/E5d | Trapped entry | n/a: fixed text |
| E6-E14 | admin, Candid | n/a |
| E15 | exports | n/a |
| E16 JS `icp.call` args | snapshot | known (L7-2, L7-4, L7-10, L6-9). `new Date(NaN)` in args -> `null`, sent (R). Spec `Date.prototype.toJSON` behaviour (non-finite time -> null), the same root as L7-4, not new |
| E16 JS `icp.call` canister/method | `arg_string` -> `to_std_string_escaped` | known (L7-10) |
| E17 JS console/print | `display` | **SW4-8-1** |
| E18 Python args/result | `to_json` | known (L7-4, L7-6, L7-7, SW3-9-1); `complex` -> repr string `"(1+2j)"` (R) is L7-6; lone-surrogate literal -> U+FFFD: **SW4-8-2** |
| E18 Python print | Monty str/repr | holds: prints `nan`, `inf`, `<built-in function len>` (R); surrogate literal prints U+FFFD (SW4-8-2) |
| E18 Python raise | Monty exceptions | holds: `raise 5` -> TypeError; `ValueError(None)` -> Monty internal-error text (loud, cosmetic, not reported) |
| E19 Rhai args/result | `to_json_value` + `from_dynamic` | holds / known: unit -> null, char -> string, NaN -> null (L7-4), FnPtr/range refused |
| E19 Rhai print/debug/throw | Rhai Display | holds (R): `NaN`, `inf`, `()`, `Runtime error: NaN`, `Runtime error: 10` |
| E19 Rhai string literals | Rhai lexer | holds (R): `"\uD800"` and `"\u{D800}"` are syntax errors |
| E20-E28 | demo_store, page, scripts | n/a / C5 above |
| U1-U5 | HTTP / JSON-RPC | known (L7-9) / holds |
| U6 | script source escapes | **SW4-8-2** (Python); JS known (L7-10); Rhai holds |
| U7 | values leaving scripts | **SW4-8-1** (printed and thrown values); rest known |
| U8 | Candid -> JSON (`idl_to_json`, `candid_bridge.rs:303-366`) | holds (C): non-finite floats use the `"NaN"`/`"inf"`/`"-inf"` strings that `float()` reads back; reserved/null -> `null` (no data); opt-opt known (#42); label collision known (L7-14) |
| U9 | reject text | n/a: Rust `String` |
| U10 | interface text | holds (C): non-UTF-8 `candid:service` is an error (`bridge.rs:431`); tmp_hack is decoded as Candid `text` (UTF-8 by spec) |
| U11-U16 | admin, inspect_message, demo_store, page, platform, video | n/a |
| O1/O2 | outbound args | known (above) / n/a |
| O3 | audit entries | **SW4-8-1** (error text) |
| O4-O10 | admin writes, cert data, HTTP, accept, trap, demo_store, video | n/a |
| D1-D10 | durable state | n/a: Candid `Storable`s, no JSON |
| V1-V11 | volatile state | n/a |
| K1-K12 | commit points | n/a |
| LIM16 | value limits | n/a for this pattern (counting known) |
| LIM22 | `MAX_SAFE_INT` | known (L7-3, L7-17) |
| other LIM | | n/a |
| W1-W16 | unbounded work | n/a |
| F11 | `f64 as f32` saturation | known (L7-5) |
| other F | | n/a |
| B1 | body -> JSON | known (L7-9) |
| B2-B4 | Candid ingress, hash, principal | n/a / holds |
| B5 | JSON -> Candid | known (L7-3, L7-5, L7-11, L7-12, L7-13, L7-15); `null` for a float slot is an error (C, `candid_bridge.rs:170-176`) |
| B6 | Candid -> JSON | holds (see U8) |
| B8 | JS <-> JSON | **SW4-8-1**; known L7-2/4/10, L6-9, SW1-3. `normalize_numbers` maps `-0` to `0`, which matches `JSON.stringify(-0)`: holds |
| B9 | Python <-> JSON | **SW4-8-2**; known L7-4/6/7/17. `from_json`'s `unwrap_or(f64::NAN)` (`python.rs:112`) is unreachable: serde_json without `arbitrary_precision` always gives an f64 (C) |
| B10 | Rhai <-> JSON | holds / known |
| B11 | result -> MCP text | known (L7-1, L7-16) |
| B12 | stable Candid | n/a |
| B13 | language name | n/a |
| B14 | time | known (L7-16b) |
| B15 | certificate header | n/a |
| B16 | search JSON | holds |
| B17 | page JSON -> DOM | holds (C5) |
| VF1-VF8 | value flows | n/a |
| AD1-AD9 | admin | n/a |
| S1-S10 | supply chain | `ruff_python_parser` 0.0.14 surrogate substitution: SW4-8-2 |
| CL10 "printed values ... in all three languages" | | **SW4-8-1** (JS prints substitutes; Python and Rhai print engine text) |
| CL29 "identical semantics in every language" | | **SW4-8-1**, **SW4-8-2** |
| CL other | | n/a |
| T5/T7/T8/T9 | tests | no test checks console output of an Error, NaN or Symbol, or a thrown non-Error value; no test uses a lone-surrogate Python literal |

## Not tested

- Wasm build and live canister: the code paths are target-independent (Boa formatting,
  ruff lexer). A live repro needs only an ordinary token holder and the existing allowlist,
  but under this phase's rules it was left to the verifier.
- Boa `console.log` of Proxy, revoked Proxy and cross-realm objects: these go through the
  same snapshot and hit the known `<unprintable value>` or `{}` paths.
