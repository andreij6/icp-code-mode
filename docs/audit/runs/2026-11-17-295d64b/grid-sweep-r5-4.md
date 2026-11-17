# Variant sweep r5-4: lenient parser accepting malformed input as a valid value

Run `2026-11-17-295d64b`, Phase 6, round 5. Pattern:

> A lenient parser accepts malformed input as a valid value.

Known and not repeated: L7-12 (stray `_`, repeated `0x`, plus the `+` spellings merged in
r4-9), L7-13 (aliased record id keys), L7-11 (null/reserved slots take any value), L7-5
(float text `inf`/`infinity`/underflow), L7-9 (JSON-RPC id forms), L7-15 (`[]` for one opt vec),
the inputSchema finding (unknown keys, `language: null`, aliases), whitespace-padded search
`canister_id`, opt-typed admin args decoded as null, duplicate JSON keys (last wins, noted under
L7-9), the `_n_` / quoted-number variant tag finding, Python repr fallbacks, JS dropping
unrepresentable values.

Method: read every parser or decoder on an inventory row: `mcp.rs` (`authenticate`,
`parse_body`, `classify`, `parse_tool_call`, `opt_string`, `demo_page`), `runtimes/mod.rs`
(`Language::from_str`), `policy.rs` (`add_token_hash`, `revoke_token_hash`, `authorize`),
`lib.rs` (`apply`, `inspect_message`, admin endpoints, `run_search`), `search.rs` (`scope`,
`keywords`), `bridge.rs` (`parse_principal`, `fetch_did`, `candid`, `call`), `candid_bridge.rs`
(`Service::parse`, `encode_args`, `json_to_idl`, `decode_reply`), the three runtimes' host
functions, `demo_store` endpoints, `src/demo.html`, `scripts/demo-setup.sh`, `video/*.mjs`.
Dependency parsers read at source: `ic_principal-0.1.5` `Principal::from_text`,
`candid-0.10.40` `Nat/Int::parse`, `IDLArgs::from_bytes_with_types_with_config` + `done()`,
`candid_parser-0.4.1` `check_prog` / `check_meths` / `check_defs`.

Probes: native, private copy (no `target/`, `design/`, `.env`, uncommitted plan doc),
`CARGO_TARGET_DIR=/private/tmp/claude-501/audit-target`, throwaway test modules in
`src/candid_bridge.rs`, `cargo test --lib sweep_r5_4`; copy deleted afterwards.
R = reproduced, C = code path.

| Row | Site | Verdict | Reason |
|---|---|---|---|
| U1 / E4 / E5 path | `authenticate` `trim_end_matches('/')` | holds (covered by L1/L12 grids) | Accepts `/mcp/`, `/mcp//`; one function on all three entry points, no spelling bypasses the token check |
| U1 HTTP method | `eq_ignore_ascii_case` | holds | `post` == `POST`; same function everywhere, no effect |
| U2 `Authorization` | first header, scheme case-insensitive, `trim` | holds / known | Empty bearer is the known L14-1; otherwise one canonical form (SHA-256 of the trimmed token) |
| U3 / B1 body | `serde_json::from_slice` | holds / known | Strict JSON (no BOM, no NaN, lone surrogates refused); duplicate keys known |
| U4 JSON-RPC | `classify` | holds / known | `jsonrpc` must equal `"2.0"`, method must be a string; id forms known (L7-9); `initialize` without params covered by L12-r4 |
| U5 tool args | `parse_tool_call`, `opt_string` | known | Unknown keys, `language: null`, aliases: the inputSchema finding |
| B13 language | `Language::from_str` | known | Case-folded aliases `js`/`py` beyond the schema enum: inputSchema finding |
| U5 search `query` | `keywords` (`split_whitespace`, lowercase) | holds | No malformed form: any text is a keyword list; empty matches all, as documented |
| U5 search `canister_id` | `scope` `trim()` | known | |
| B4 principal text | `Principal::from_text` (bridge, `json_to_idl`, search) | holds (C) | Parses case-insensitively (as the spec says) then requires the canonical grouping and CRC (`ic_principal-0.1.5/src/lib.rs:189-226`); `""` is TextTooShort |
| B3 / AD5 / AD6 token hash | `add_token_hash`, `revoke_token_hash` | holds | Trim + lowercase, then exactly 64 hex; both functions normalise the same way |
| AD1 InitArgs | `apply` | holds / known | Bad hash traps (strict); opt sub-fields of the wrong type decode as null: known |
| AD2 allowlist methods | `authorize` exact match | holds | Misspelled names allow nothing (fail closed); `"*"` known |
| AD4 Limits | Candid decode | known | Unvalidated values and opt-as-null are known findings |
| **AD7 / U11 / B7 override text** | `set_candid_override` -> `Service::parse` -> `candid_parser::check_prog` | **finding SW5-4-1** (R) | `import` declarations are accepted and ignored. `import service "x.did"` drops every imported method without an error; `import "x.did"` passes when the imported types are unused, and fails as an unbound type otherwise |
| U10 / B7 fetched interface | `fetch_did`, `Service::parse` | holds (beyond SW5-4-1) | `String::from_utf8` strict; `candid_parser` rejects duplicate bindings, methods and fields (`typing.rs:108,122,250`); trailing tokens are refused (R: `service : {..}; type T = nat;` -> parse error); compiled `candid:service` sections carry no imports |
| U8 / W6 reply bytes | `decode_reply` | holds / known | `done()` refuses trailing bytes (`candid-0.10.40/src/de.rs:130-145`); extra or missing values and fields follow Candid subtyping (spec); the stale-interface opt rule is known |
| B5 ints | `num_text` + `FromStr` / `BigUint::parse_bytes` | known | L7-12 (`_`, `+`); fractional and >= 1e19 floats refused |
| B5 float | `float` | known | L7-5 |
| B5 blob | `hex::decode(trim_start_matches("0x"))` | known / holds | Repeated `0x` known (L7-12). `""` and `"0x"` -> empty blob, `"cafe"` -> bytes CA FE (R): the documented hex convention (README:358, SEARCH_DESCRIPTION), not a malformed value |
| B5 record | unknown keys refused; tuple from array; `{"0":..}` / `{"_0_":..,"01":..}` for tuples (R) | known | Id aliases are L7-13 |
| B5 variant | `"Tag"` shorthand | holds (R) | `"B"` for `B : opt nat` -> `B = null`, `"C"` for `C : record {}` -> `C = record {}`, `"D"` for `D : nat` -> error. Each accepted spelling is the arm's only empty value; nothing is guessed |
| B5 variant | `{"A": 5}` for a null arm | known | L7-11 |
| B5 func | object form ignores extra keys | holds (R) | `{"principal","method","bogus","metho"}` -> `func "aaaaa-aa".m`. Both required keys must be present (missing `method` is an error), so an extra or misspelled key cannot change the value sent |
| B5 / `encode_args` arg shapes | `null`, `{}` for zero args, `[x]` heuristic | known | L7-15 and the `[x]` heuristic finding |
| E16 JS host args | `arg_string` | holds | Only string primitives; boxed strings and numbers refused |
| E18 Python host args | `host` match, kwargs refused | holds | Exact arity; kwargs give TypeError |
| E19 Rhai host args | typed `ImmutableString` registrations | holds | Wrong types are "function not found" |
| U7 / B8-B10 value converters | snapshot / `to_json` / `to_json_value` | known | Substitutes for unrepresentable values: known (r4-8, r4-10) |
| E13 `get_audit_logs` | `audit_page` | holds | Typed u64; documented clamp |
| U12 / B2 `inspect_message` | `decode_one::<HttpUpdateRequest>` | holds / known | Candid typed decode; quota difference known |
| U13 / E20-E23 demo_store | Candid typed args | holds | `list_products` category case-insensitive by design; zero-quantity lines known |
| C5 / B17 demo page | `JSON.parse`, `.did` regex | known / holds | Raw string results known (L7-16); regex cost and method count known; token `trim()` is the same normalisation the server applies |
| C6 `scripts/demo-setup.sh` | `sed` on `icp` output | known | "parses human-readable icp-cli output" |
| C12 video | `.env` regexes | not applicable | Developer tooling reading the developer's own files; a wrong value fails loudly (401 / API error) |
| C3 / C4 shims, vendor | no parsers on app inputs | not applicable | chrono parsing is reachable only from a script's own `datetime` calls |
| Engine-internal parsers (Boa `JSON.parse`, Monty `json.loads`, Rhai `parse_json`) | script-chosen | not applicable | Called by untrusted scripts on data they already hold; no app boundary |

## New candidate

**SW5-4-1 (Low): `set_candid_override` accepts `.did` text with `import` declarations and
silently ignores them, so imported methods disappear.** `Service::parse`
(`src/candid_bridge.rs:34-46`) uses `candid_parser::check_prog`, which skips
`Dec::ImportType` / `Dec::ImportServ` (`candid_parser-0.4.1/src/typing.rs:144`). Imports are
resolved only by the file-based `check_file` (`typing.rs:302-303`). A standalone text cannot
resolve them, so this text is incomplete, yet the parse succeeds. Reproduced natively:
`Service::parse("import service \"ledger.did\";\nservice : { local_m : () -> () }")` returns Ok
with methods `["local_m"]`, and `method("icrc1_transfer")` gives
`` method `icrc1_transfer` is not declared in the target's candid interface ``.
`import "/etc/passwd";\nservice : { m : () -> () }` also parses. Only when an imported *type*
is referenced does it fail (`Unbound type identifier Account`). `set_candid_override`
(`src/lib.rs:316-325`) uses this parse as its validation and returns `Ok(())`. The stored text
still shows the `import` line through `icp.candid`, while search lists only the local methods
and calls to imported methods are refused with a message saying they are not declared.
Effect: an operator is misled and the AI model sees an interface that does not match what
calls accept. No wrong value is sent. The failure is a refusal, so the grade is Low.
Fixed-interface fetches are unaffected in practice, because compiled `candid:service`
sections contain no imports. Fix: in `Service::parse`, refuse any `Dec::ImportType` /
`Dec::ImportServ` in `prog.decs` ("imports are not supported; inline the imported
definitions").

Refute attempts: checked `check_prog` for an error on unresolved imports (none: the arm is
`=> ()`); checked that `check_meths` and `check_defs` reject the other malformed forms
(duplicates: refused in the parser; trailing declarations after the service: parse error, R).
