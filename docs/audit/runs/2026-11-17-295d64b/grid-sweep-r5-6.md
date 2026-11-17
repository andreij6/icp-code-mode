# Variant sweep r5-6

Pattern: "Map conversion where distinct keys collapse to one without error (same pattern as
L7-7)". Read as: a keyed structure crosses a boundary (or is normalised into a store), and
two keys that are distinct on the source side become one key on the target side, so one
value is lost or overwritten with no error. Identical keys (same canonical text) are out of
pattern.

Builds on grid-sweep-r4-11 (same pattern via L7-14), which already found SW11-1, SW11-2 and
SW11-3; those are in the known list and are not re-reported.

Method: code reading at `295d64b` of every keyed conversion and keyed store (`grep` for
`insert`, `BTreeMap`, `BTreeSet`, `Map::new`, `to_lowercase`, `eq_ignore`, `trim` over `src/`,
`src/runtimes/`, `demo_store/src/`, plus `src/demo.html` object maps), Boa 0.22.0
`value/conversions/serde_json.rs` and monty-types 1.6.0 `object.rs` accessors, and one
scratch test in a private copy (`cargo test --lib sweep_r5_6`, shared
`CARGO_TARGET_DIR=/private/tmp/claude-501/audit-target`, copy deleted afterwards).
R = reproduced, C = code path.

Result: no new candidate.

| Row | Keyed conversion / store | Can distinct keys collapse? | Verdict |
|---|---|---|---|
| B8 JS -> JSON, canonical-index keys (`SNAPSHOT_JS` `put` via `defineProperty`, Boa `to_json` index keys then shape keys) | `"1"`, `"01"`, `"1.0"`, `" 1"`, `"1e0"`, `"-0"`, `"4294967294"`, `"4294967295"`, `"__proto__"`, `"constructor"` on one object | no: `Object.keys` = 10 and the result and the `icp.call` argument both carry 10 keys (R) | holds |
| B8 JS arrays with string-index props | `a["1"]` is the same element as `a[1]` inside JS; `a["01"]` dropped | not a collapse (non-index array props are dropped, a different pattern) (R) | out of pattern |
| B8 JS Unicode-equivalent keys | `"é"` (NFC) vs `"é"` (NFD) | no normalisation: 2 keys out (R) | holds |
| B8 JS lone-surrogate vs escaped spelling | Boa `to_std_string_escaped` | yes | known (SW11-3) |
| B8 JSON -> JS (`JsValue::from_json`, `js.rs:403`) | JSON keys are unique UTF-8 strings -> UTF-16 | no (C) | holds |
| B9 Python -> JSON, `str` keys (`python.rs:89-96`, `as_str` = `MontyNode::String` only) | `'1'`, `'01'`, `' 1'` | no: 3 keys out (R) | holds |
| B9 Python -> JSON, non-`str` keys | `py_repr`; also Monty prints a 1-tuple key `(1,)` as `"(1)"`, so `{(1,):..,'(1)':..}` collides | yes (R) | known (L7-7, same root) |
| B9 class instances / modules (`pairs()` returns attrs) | attribute names unique per instance | no; class-name dispatch is S10-1 | known elsewhere |
| B9 JSON -> Python (`from_json`) | JSON keys -> `str` | no (C) | holds |
| B10 Rhai -> JSON (`rhai_rt.rs:102-109`) | `rhai::Map` keys are Rust strings | no: `"1"`, `"01"`, `" 1"`, NFC/NFD `é` -> 5 keys out (R) | holds |
| B10 JSON -> Rhai (`to_dynamic`) | one-to-one | no (C) | holds |
| B5 `json_to_idl` record / variant | `key_matches` / tag lookup | yes | known (L7-13, SW11-1, SW11-2) |
| B6 `idl_to_json` record / variant | `label_name` | yes | known (L7-14, SW11-2) |
| B6 decoded records with fields unknown to our interface | typed decode (`from_bytes_with_types_with_config`) skips them, so no wire `Id(hash)` label can meet a `Named` label | no (C) | holds |
| B1 / U4 JSON-RPC body, `params`, `arguments` (`parse_tool_call` `mcp.rs:209-226`) | serde_json Map; no key aliases (`code`, `language`, `query`, `canister_id` read by exact name) | only identical keys (last wins; RFC 8259 leaves it undefined); `"code"` and `"code"` are the same JSON string (C) | out of pattern |
| B1 batch replies (`http_update` / `http_query`) | replies kept in a `Vec`, not keyed by id; ids `1` and `"1"` each get their own reply (C) | no | holds |
| U2 `authenticate` headers (`mcp.rs:276-281`) | first `Authorization` header, name case-insensitive per HTTP | by spec (C) | holds |
| B13 language names (`runtimes/mod.rs:34-44`) | case-insensitive, `js`/`py` aliases | by design | holds |
| W3 / B16 search keywords (`search.rs:81`) | lowercased; collapse is the intended case-insensitive match | by design (C) | holds |
| B16 search `types` map (`search.rs:144-147`) | TypeEnv names are unique | no (C) | holds |
| D2 `ALLOWLIST` keys (`policy.rs:156`), E1/E2 `apply` (`lib.rs:37-48`) | `Principal` keys; two InitArgs entries for one principal: last wins | identical canonical keys only (CL23 by design) (C) | out of pattern |
| D2 method names, `authorize` (`policy.rs:177`) | exact string compare, no case-folding or trimming | no (C) | holds |
| D3 `TOKENS` (`policy.rs:209-213`) | hex trimmed + lowercased | different spellings of one hash, by design (C) | holds |
| B3 caller id / `list_tokens` prefix = first 16 hex | 64-bit prefix | a collision between two holders needs a 2^64 second preimage; a holder cannot choose another's prefix (C) | holds |
| D5 `CANDID_OVERRIDES`, V2/V4/V5 caches, `touched` (`bridge.rs:195-223, 398`) | canonical `Principal::to_text()` | by design (C) | holds |
| V3 `PENDING_RUNS` | `u64` run ids | no (C) | holds |
| C2 demo_store | vectors only; `category` filter case-insensitive by design | no map (C) | holds |
| C5 / B17 demo page (`demo.html:301-353, 404-413, 463`) | plain objects keyed by customer principal, product id, Candid type name; `canon` sorts keys | keys unique on demo_store's interface; a type named `__proto__` would be lost, but only from a benchmark character count on the demo target (C) | holds (no demonstrated failure) |

## Candidates

None new.
