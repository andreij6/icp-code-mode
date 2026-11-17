# Variant sweep r4-2: one unrepresentable value, three rules across sibling converters

Run `2026-11-17-295d64b`, Phase 6, round 4. Pattern under test (from confirmed L6-9):

> An unrepresentable value handled by three different rules (drop, stringify, error) across
> sibling converters.

Method: code reading at `295d64b` plus native probes. The probes ran in a private copy of the
tree (no `target/`) with `CARGO_TARGET_DIR=/private/tmp/claude-501/audit-target`, as a
throw-away test module `runtimes::sweep_r4_2` that drives `LanguageRouter::execute` with
`MockBridge` and prints result, logs and the JSON each call sent (`cargo test --lib
sweep_r4_2 -- --nocapture`). The copy was deleted afterwards. Nothing was deployed or
changed on the live network.

For each row the question was: is there a set of converters that should treat a value the
same way (the three runtimes, two directions, or two output paths of one runtime), and does
a value one of them cannot represent get dropped by one, turned into text by another and
refused by a third?

Known and not re-reported: L6-9 (results/args: function dropped in JS, repr in Python,
refused in Rhai), L7-2, L7-4 (NaN/inf to null), L7-6 (Python repr fallback), L7-7 (Python
non-str keys), L7-10 and SW11-3 (JS lone surrogates as literal `\uD800`), L7-11, L7-16,
L7-17, the README:369 depth-placeholder finding for printing, the Rhai reply-limit finding.

## Grid

| Row(s) | Result | Reason |
|---|---|---|
| C1, C1a | see B8-B10 rows | The runtimes are the sibling converters |
| C2-C15 | n/a | No value conversion between sibling converters (build config, page, scripts, tooling) |
| A1-A12 | n/a | Actors; covered through their inputs below |
| E1/E2 `apply` | holds | One converter (token hash text); bad hash refuses (F1) |
| E3 / B2 | known | inspect_message vs method decode quota is known |
| E4/E5 / B1 body parse | holds / known | One `parse_body` for both routes; a JSON `\uD800` escape refuses the whole body with -32700 on both routes. Id echo is L7-9 |
| E4b/E5 / U5 `parse_tool_call` | holds / known | Wrong-typed `code`, `language`, `query`, `canister_id` all refuse (`opt_string`, `mcp.rs:196-202`); null-as-absent and non-object `arguments` are the known inputSchema finding |
| E5a / U7 results (B8/B9/B10) | known (L6-9) + **SW4-2-2** | A caught host error is the value scripts most often hold that JSON cannot represent: JS `{}` (reason lost), Python `"RuntimeError('policy: ...')"`, Rhai the message text. Reproduced |
| E5b search / B16 render | holds | One converter (`search::render`); `as_func(..).ok()` and `find_type(..).ok()` can only drop items that `check_prog` already rejected |
| E5c/E5d | n/a | No value conversion |
| E6-E11 admin | holds | Override text validated by `Service::parse` at set time (`lib.rs:324-330`, since the first commit `9afe403`), so the three override consumers (icp.candid returns text, search parses, icp.call parses) never meet an unparseable override at this commit. Edge noted: a candid_parser change on upgrade would make them disagree (text / "interface not fetched" / error); same as the r3-0 edge row, not reported |
| E12-E14 | n/a | Candid replies of typed records |
| E15 | n/a | |
| E16/E17 JS console.log, print | **SW4-2-1** | `display` (`js.rs:217-225`) sends non-strings through the JSON snapshot: Error -> `{}`, NaN/Infinity/function -> `null`, Symbol -> empty string, top-level BigInt -> digits, nested BigInt -> `<unprintable value>` for the whole argument. Python and Rhai print the value's own text. Reproduced |
| E16 JS uncaught error text (`error_string`, `js.rs:325-340`) | **SW4-2-1** (same root) | `throw undefined` / `throw () => 1` / `throw Symbol()` / `throw NaN` -> `Uncaught null`; `throw 1n` -> `Uncaught <unserializable value>`; `throw new Map(..)` -> `Uncaught {}`. Rhai `throw Fn("f")` -> `Runtime error: Fn(f)`; Python refuses non-exceptions with TypeError. Reproduced |
| E18 Python print | holds | Prints `str()` of every value (`nan`, `<built-in function len>`, exception message) |
| E19 Rhai print/debug | holds | Prints `to_string()` (`NaN`, `inf`, `Fn(f)`, message) |
| E20-E28 | n/a | demo_store converts only typed Candid; page and tooling are clients |
| U6 script source text | **SW4-2-3** | Lone surrogate escape in a string literal: JS keeps it and exports `\uD800` text (known L7-10); Python's parser silently substitutes U+FFFD (`'\udc00x' == '�x'` is `True`, and `icp.call(.., {'owner': '\ud800'})` sends `"�"`), while `chr(0xd800)` raises "chr() arg not in range(0x110000)"; Rhai refuses at parse ("Invalid escape sequence"). Reproduced |
| U7 call args | known (L6-9, L7-2, L7-4, L7-6) | Same converters as results |
| U8 reply bytes / B6 -> B8/B9/B10 reply direction | holds | Every value `idl_to_json` emits is representable in all three runtimes (ints > 2^53 arrive as strings, non-finite floats as `"NaN"`/`"inf"` strings); register Testability grid p9 found 0 mismatches over 600 replies. Rhai's size limits are the known reply finding |
| U9 rejects/errors | holds | Strings end to end |
| U10 interface text (`fetch_did`) | holds | Only `canister_metadata` can carry non-UTF-8; it refuses ("candid:service is not UTF-8", `bridge.rs:431`). tmp_hack and override are Candid `text`, always valid UTF-8. That the non-UTF-8 refusal skips the tmp_hack fallback while a decode failure falls back is a fallback-policy question, not this pattern |
| U11 admin inputs | holds | Typed Candid; token hash case-folded and trimmed on add and revoke alike |
| U12-U16 | n/a / known | |
| O1/O2 | see U7, U10 | |
| O3/B12 audit storables | known | Unknown `AuditStatus` on rollback traps (known); one converter |
| O4-O10 | n/a | |
| D1-D10 | n/a | Typed stable records or client-side |
| V1-V11 | n/a | Counters and caches, no value conversion; V5 see E6-E11 row |
| K1-K12 | n/a | Commit points, no conversion |
| LIM1-LIM32 | n/a for this pattern | LIM16 value limits apply identically to the converted value; the per-runtime differences are known (L7-8, Rhai reply limits, JS UTF-16 counting) |
| W1-W16 | n/a | Cost rows; conversion cost is known |
| F1-F16 | n/a | F11 float32 saturation is L7-5 (one converter) |
| B3 token -> hash | holds | One converter |
| B4 principal text | known | Search trims, bridge does not (known); both refuse non-principals |
| B5 JSON -> Candid | holds / known | Non-finite float text `"NaN"`, `"inf"` parse back; `null` for a float refuses; unsupported types refuse (`candid_bridge.rs:298-299`). Opt-wrapped NaN-as-null becoming `None` is L7-4 |
| B6 Candid -> JSON | holds | Every `IDLValue` variant has a rule (`candid_bridge.rs:329-364`); non-finite floats become strings that B5 accepts back, so the pair round-trips |
| B7 .did parse | holds | One converter |
| B8 JS | **SW4-2-1**, **SW4-2-2** + known | Results/args follow JSON.stringify (known); display and uncaught-error paths reuse it where JS's own console does not |
| B9 Python | **SW4-2-2**, **SW4-2-3** + known | Probed: `set`/`frozenset` -> arrays (holds); `bytearray` not defined in Monty (n/a); exception objects -> repr text |
| B10 Rhai | holds | Probed: closure-captured (shared) arrays, maps, strings and blobs convert intact, at top level, nested and as call args, so the `unwrap_or_default()` calls in `to_json_value` (`rhai_rt.rs:94-112`) do not silently empty shared values; char -> string, unit -> null |
| B11 tool result text | known (L7-16) | |
| B13 language name | known | |
| B14 time | known (L7-16b) | |
| B15 certificate header | n/a | Constant tree |
| B17 demo page | known | `JSON.parse` of raw string results is L7-16's symptom |
| VF1-VF8 | n/a | |
| AD1-AD9 | n/a / see E6-E11 | |
| S1-S10 | n/a | Monty's parser is the source of SW4-2-3's U+FFFD substitution (S2, `monty =1.6.0`) |
| CL29 "identical semantics in every language" (shown to AI) | **SW4-2-1**, **SW4-2-2**, **SW4-2-3** | Each candidate is a same-input, different-output case |
| CL10 README:369 printed values | known | Depth placeholder finding |
| Other CL rows | n/a | |
| T1-T12 | gap | No test prints or returns an Error object, a thrown non-Error value, or a lone-surrogate literal in any runtime; `values_and_errors` asserts only `throw {code: 7}` |

## Candidates

### SW4-2-1 JS console.log/print and uncaught-error text pass values through the JSON snapshot: errors print `{}`, NaN/functions `null`, Symbols nothing (Low, reproduced)

- Where: `src/runtimes/js.rs:217-225` (`display`), `:325-340` (`error_string`).
- Repro (native, MockBridge): `try { await icp.call('nope-bad','m',null) } catch (e) { console.log('failed', e) }`
  logs `failed {}`. Python `print('failed', e)` and Rhai `print("failed " + e)` log
  `failed policy: canister nope-bad is not allowlisted`.
  `console.log(NaN, Infinity, -0)` -> `null null 0`; `console.log(() => 1)` -> `null`;
  `console.log(Symbol('s'))` -> empty line; `console.log(1n)` -> `1` but `console.log([1n])`
  -> `<unprintable value>`; `console.log(new Error('boom'))` -> `{}`. Python prints
  `nan inf -0.0`, `<built-in function len>`, `boom`; Rhai prints `NaN`, `inf`, `Fn(f)`.
  Thrown values: `throw undefined|() => 1|Symbol('s')|NaN` -> `Uncaught null`;
  `throw 1n` -> `Uncaught <unserializable value>`; `throw new Map([[1,2]])` -> `Uncaught {}`.
- Three rules inside one JS display path: drop (Error, Map -> `{}`, function/NaN -> `null`,
  Symbol -> empty because `to_string` throws and `unwrap_or_default` hides it), stringify
  (top-level BigInt, undefined), error placeholder (nested BigInt, deep values). The two
  sibling runtimes stringify every case.
- Impact: the catch-and-log idiom, the usual way an AI-written script reports a policy
  refusal, hands the model `{}` instead of the reason, against "identical semantics in
  every language" in the tool description (`mcp.rs:79`). No effect on calls or the audit.
- Refutation tried: is this just JSON.stringify semantics like the result path (where the
  L7 grid accepted Map -> `{}`)? For results, yes; but a console has no JSON contract, JS's
  own console prints errors and NaN as text, and the sibling runtimes print text. Does
  `error_string` catch native errors first? Yes (`try_native`), so `throw new Error(..)` is
  fine; only non-Error throws are affected.
- Fix: in `display`, print strings, numbers (`Number.prototype.toString`, so NaN stays
  `NaN`), bigints, symbols (`description`) and Error objects (`name: message`) directly and
  use the snapshot only for plain objects and arrays; in `error_string`, fall back to
  `JsValue::display()` for primitives before the snapshot.

### SW4-2-2 A caught host error returned in a result is `{}` in JS, a repr string in Python and the message in Rhai (Low, reproduced; merge candidate into L6-9)

- Where: `src/runtimes/js.rs:45-94, 207-215` (snapshot: Error has no enumerable own keys),
  `src/runtimes/python.rs:98` (`_ => py_repr`), `src/runtimes/rhai_rt.rs:111-114` (catch
  binds the message string).
- Repro: JS `const r = await Promise.allSettled([icp.call('nope-bad','m',null)]); return r`
  -> `[{"reason":{},"status":"rejected"}]`, run Ok. JS `catch (e) { return {err: e} }` ->
  `{"err":{}}`. Python `except Exception as e: {'err': e}` ->
  `{"err":"RuntimeError('policy: canister nope-bad is not allowlisted')"}`. Rhai
  `catch (e) { #{err: e} }` -> `{"err":"policy: canister nope-bad is not allowlisted"}`.
- Same converters and root as L6-9 (no shared rule for non-JSON values); the new part is the
  value class: every rejected `icp.call` produces one, and `Promise.allSettled`, the batch
  idiom for partial failure, returns all reasons as `{}` while the run reports Ok.
- Fix: as L6-9 (one shared rule). For JS, convert Error objects to `{name, message}` (or
  their message) in the snapshot before the type rules.

### SW4-2-3 A lone-surrogate escape is exported as literal text in JS, silently replaced by U+FFFD in Python, and refused in Rhai (Low, reproduced; cross-runtime extension of L7-10)

- Where: script source parsing in the engines (Monty's string-literal decoding, `monty
  =1.6.0`; Rhai's lexer; Boa strings + `to_std_string_escaped`, `js.rs:197`).
- Repro: Python `x = '\ud800'; [x, len(x)]` -> `["�", 1]`; `'\udc00x' == '�x'`
  -> `True`; `icp.call(L, 'icrc1_balance_of', {'owner': '\ud800'})` sends
  `{"owner":"�"}`, run Ok. Python `chr(0xd800)` -> `ValueError: chr() arg not in
  range(0x110000)` (the argument is in range; the message is wrong). Rhai `"\ud800"` ->
  `syntax error: Invalid escape sequence`. JS `'\ud800'` -> sent as the six characters
  `\uD800` (known L7-10).
- Three rules for one value: stringify (JS), drop/replace (Python literal), error (Python
  `chr`, Rhai). Python's substitution makes two distinct literals equal inside the script.
- Reachability: Python literals with surrogate escapes are rare (Python slicing never
  produces them, unlike JS); the practical impact is small. Graded Low.
- Fix: one documented rule. Refuse lone surrogates when a string leaves the sandbox in JS
  (L7-10's fix), and report them as an error in Python too (a compile-time check or an
  upstream Monty issue), so all three refuse.
