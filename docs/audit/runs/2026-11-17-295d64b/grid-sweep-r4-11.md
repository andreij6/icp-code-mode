# Variant sweep r4-11

Pattern: "Map conversion where distinct keys collapse to one without error (same pattern as
L7-14, different code)." Stated both ways: (a) two distinct source keys map to one target key
and one value is lost; (b) one source key is taken by two distinct target keys, so the
script cannot address them separately.

Method: code reading at `295d64b` of every place a keyed structure crosses a boundary
(`grep` for `Map`, `BTreeMap`, `BTreeSet`, `insert`, `from_iter`, `field_value`,
`key_matches` over `src/` and `demo_store/src/`), plus scratch tests in a private copy
(`cargo test --lib sweep_r4_11`, shared `CARGO_TARGET_DIR=/private/tmp/claude-501/audit-target`,
copy deleted afterwards). R = reproduced, C = code path.

Result: three new candidates (SW11-1, SW11-2, SW11-3). Known hits: L7-14 (record decode),
L7-7 (Python non-str dict keys), L7-13 (aliases for one id field).

| Row | Keyed conversion | Can distinct keys collapse? | Verdict |
|---|---|---|---|
| B6 `idl_to_json` record (`candid_bridge.rs:344-349`) | `label_name` -> `Map::insert` | yes: `"1"` (Named) and `1` (Id) | **known** L7-14 |
| B6 `idl_to_json` variant (`candid_bridge.rs:352`) | `label_name(&f.id)` as the single key | yes: `variant { "1" : text; 1 : text }`, reply `"1" = "named"` -> `{"1":"named"}`, reply `1 = "id"` -> `{"1":"id"}` (R) | **SW11-2** |
| B5 `json_to_idl` variant tag lookup (`candid_bridge.rs:283-287`) | `label_name == tag` first match in field order | yes: tag `"1"` always selects the Id arm; the named reply above, passed back, is sent as `variant { 1 = "named" }` (R) | **SW11-2** (encode half) |
| B5 `json_to_idl` record object (`candid_bridge.rs:255-265`, `key_matches`/`field_value` `:136-149`) | each field scans the JSON keys; matching is not exclusive | yes, direction (b): `record { "1" : text; 1 : text }` given `{"1":"x"}` sends both fields `"x"`; `record { "01" : nat; 1 : nat }` given `{"01":7}` sends both `7`, and given `{"01":7,"1":9}` sends both `7` and drops `9` with no error; same for `"_1_"` (R). The unknown-key check passes because every key matches some field | **SW11-1** (distinct from L7-13: one key filling several fields, not several keys for one field; shares its root `key_matches`) |
| B5 `json_to_idl` tuple / `is_tuple` (`candid_bridge.rs:130-132, 247-254`) | positional | no: requires ids 0..n-1, no Named | holds |
| B5 `json_to_idl` func object (`candid_bridge.rs:221-230`) | `get("principal")`, `get("method")` | no collapse (extra keys are ignored, a different pattern) | holds for this pattern |
| B5 variant object with several keys | `o.len() == 1` guard | refused | holds |
| B8 JS -> JSON (Boa `JsValue::to_json`, `boa_engine-0.22.0 src/value/conversions/serde_json.rs` keys via `to_std_string_escaped`, `map.insert`) | JS property key -> Rust `String` | yes: keys `"\uD800"` (lone surrogate) and `"\\uD800"` (six characters) are two properties in JS (`Object.keys(o).length` = 4 for two such pairs) and one key after conversion; result `{"\\uD800":2,"a\\uDC00":4}`; the same object as an `icp.call` argument is sent as `{"\\uD800":"text"}`, run Ok (R) | **SW11-3** (symptom of L7-10's root cause; merge candidate) |
| B8 JSON -> JS (`JsValue::from_json`, `js.rs:403`) | JSON key -> JS property key | no: string keys map one-to-one | holds |
| B9 Python -> JSON dict (`python.rs:89-96`) | non-str keys `py_repr`'d | yes: `{True:1,'True':2}`, `{None:1,'None':2}`, `{b'a':1,"b'a'":2}`, `{(1,2):1,'(1, 2)':2}` each leave as one key (R) | **known** L7-7 (same root, wider key types than its `{1,'1'}` example) |
| B9 Python str keys with surrogates | `'\ud800'`, `'\ud801'` | they become one key, but inside Monty already: `len({'\ud800':1,'\ud801':2})` = 1 and `'\ud800' == '\ud801'` is True (R); the conversion itself is one-to-one | holds for this pattern (Monty lexer, not a boundary conversion) |
| B9 JSON -> Python (`python.rs:115`) | JSON key -> `str` | no | holds |
| B10 Rhai -> JSON map (`rhai_rt.rs:102-109`) | `rhai::Map` key (Rust string) -> `String` | no: keys are already UTF-8 strings, one-to-one (R: `#{"a":1,"b":2}` round trip) | holds |
| B10 JSON -> Rhai (`rhai::serde::to_dynamic`, `rhai_rt.rs:76`) | JSON key -> `rhai::Map` key | no | holds |
| B1 / U3 `parse_body` (serde_json `Value`) | JSON object with repeated identical keys: last wins | identical keys, not distinct ones; RFC 8259 leaves duplicates undefined; body is from a token holder only (C) | out of pattern, holds |
| U2 `authenticate` headers (`mcp.rs:274-279`) | first `Authorization` header (case-insensitive name) | list scan, not a map conversion (C) | holds |
| B16 search `types` map (`search.rs:135-138`) | type names from `TypeEnv` (unique) | no | holds |
| B16 search `methods` (`search.rs:103-110`) | service method names (candid_parser rejects duplicates) | no | holds |
| B4 principal keys: D2 `ALLOWLIST` (`Principal`), D5 overrides, V2/V4/V5 caches, `touched` (`to_text()` of a parsed principal) | canonical text | different spellings of one principal collapse by design (CL23) (C) | holds |
| B3 / D3 token hash keys (`policy.rs:211-232`) | trim + lowercase of hex | different spellings of one hash collapse by design; caller id = first 16 hex (64-bit prefix) collides only by chance among controller-registered tokens (C) | holds |
| B13 language names | case-insensitive, aliases | by design | holds |
| B12 stable Candid records | typed decode | no keyed collapse | holds |
| C2 demo_store | vectors only, no maps (grep) | duplicates in one order are the known oversell finding, not a key collapse | holds (known elsewhere) |
| W11 Rhai `answers` / JS `pending` / Python `queued` | sequence numbers | no | holds |

## Candidates

- **SW11-1** `json_to_idl` record matching is not exclusive: one JSON key fills every field
  whose label it matches (`"1"` matches both `"1"` and `1`; `"01"`, `"_1_"` likewise), and a
  second key meant for the id field is dropped. R (scratch test in a copy).
- **SW11-2** Variant arms with a quoted numeric label and an id label print the same tag:
  the reply's arm is lost, and the tag always encodes as the id arm. R.
- **SW11-3** JS object keys that differ only by a lone surrogate versus its escaped spelling
  collapse to one key when the value leaves the sandbox (result or call argument). R.
  Root cause is L7-10's.
