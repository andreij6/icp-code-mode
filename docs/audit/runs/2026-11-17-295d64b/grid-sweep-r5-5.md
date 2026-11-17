# Variant sweep r5-5: several input spellings mapped to one slot without a collision check

Run `2026-11-17-295d64b`, Phase 6, round 5. Pattern:

> Several input spellings mapped to one slot without a collision check (residual of
> register #35).

Generalised for the sweep: wherever two or more distinct inputs (aliases, case or
whitespace variants, escape forms, stringified keys, sentinel values) are normalised into
one key, label, map entry or meaning, check whether the code detects two of them meeting
(or a sentinel meeting a literal) and refuses, or silently keeps one.

Method: read at `HEAD` (`git show HEAD:`; the working tree carries uncommitted test
additions in `src/` and `demo_store/` that are not part of the audited commit) every
normalisation site: `key_matches`/`field_value`/variant tag lookup/`label_name`
(`src/candid_bridge.rs`), `num_text`, blob hex, `Principal::from_text` uses
(`src/bridge.rs`, `src/search.rs`, `src/policy.rs`), `authorize` (`src/policy.rs:170-190`),
`add_token_hash`/`revoke_token_hash`/`verify_token`, `InitArgs` `apply`, `authenticate`
(header, scheme, path), `parse_body`/`classify`/`parse_tool_call` (`src/mcp.rs`),
`Language::from_str`, `run_search` audit key, Python `to_json`/`from_json`/`host`, Rhai
`to_json_value`/`request` replay comparison, JS `arg_string`/`SNAPSHOT_JS`/Boa
`JsValue::from_json` (boa_engine 0.22 `value/conversions/serde_json.rs:68-79`,
`property/mod.rs:695-699`), `search::render` maps, demo_store lookups, demo page builders.
Native probes in a private `git archive HEAD` copy (`/private/tmp/claude-501/sweep-r5-5.*`,
shared `CARGO_TARGET_DIR=/private/tmp/claude-501/audit-target`, `cargo test --lib` with
probe tests added to the copy only; 57 HEAD tests + probe, so the HEAD tree was built).
Copy deleted. Nothing deployed; the local network was not touched.

Evidence: R = reproduced (native test), C = code path. "known" = in the do-not-re-report
list given to this sweep.

## New candidates

| Id | Severity | Candidate | Evidence |
|---|---|---|---|
| SW5-5-1 | Low | The allowlist wildcard `"*"` and a Candid method literally named `*` share one spelling. `authorize` grants when `m == "*" \|\| m == method` (`src/policy.rs:178`) and `search::render` shows every method when `allowed` contains `"*"` (`src/search.rs:103`); `set_allowlist`/`InitArgs.allowlist` take the strings as given (`src/policy.rs` `set_allowed`, `src/lib.rs:41-44`) with no escape or collision check. A controller who allowlists only the target's method named `*` (a legal quoted Candid name) allows every method on that canister, and `get_policy` shows the same `["*"]` either way | R: `Service::parse(r#"service : { "*" : () -> (); transfer : (nat) -> () }"#)` accepts both methods; with `AllowedMethods(vec!["*"])`, `authorize(.., "*", ..)` -> `Ok`, `authorize(.., "transfer", ..)` -> `Ok`; `transfer` encodes (8 bytes); `search::render` lists `*` and `transfer`. Not tested: a deployed canister exporting `canister_update *` (Wasm export names are arbitrary UTF-8; not demonstrated on the local network) |

## Grid

### Components, actors, entry points

| Row | Verdict | Reason |
|---|---|---|
| C1, C1a | see rows below | |
| C2 demo_store | known / holds | duplicate `product_id` lines in one order = known oversell; products, inventory, customers, orders looked up by numeric id or `Principal` (binary, canonical); `search_products` lowercases the query, not a key |
| C3 web-time shim, C4 vendored chrono | n/a | no key or label mapping of untrusted input |
| C5 demo page | holds | request ids from a counter; tool defs pushed into an array, not keyed; one localStorage key |
| C6-C11 | n/a | build/deploy files; `.did` labels checked by tests |
| C12 video tooling | n/a | env/.env/~/.env precedence for one dev key; dev only |
| C13, C14, C15 | n/a | |
| A1-A12 | n/a | covered via the rows they reach |
| E1/E2 `apply` | holds (pattern) | `token_sha256` folded by `add_token_hash` (same token, same slot); duplicate allowlist principals in one `InitArgs` replace earlier ones (one `Principal` meaning, controller input); label reset to "init" on upgrade is part of the known post_upgrade finding |
| E3 inspect_message | holds | same `authenticate` as the method, so header/path normalisation cannot differ between filter and method |
| E4/E4a/E4b, E5 | holds | path: `/mcp`, `/mcp/`, `/mcp//`, `/mcp?x` all mean the one endpoint (no state keyed by path); `Authorization` header matched case-insensitively, first one only, in both routes |
| E5a parse_tool_call | holds | `language` spellings (`js`/`JavaScript`/`PY`...) fold to one `Language`; the audit stores the canonical `as_str()`, so no two spellings diverge; kwargs-style aliases do not exist |
| E5b run_search | holds | audit key `search:{query}:{canister_id}`: two different (query, canister_id) pairs can produce the same text only when the canister part contains `:` (not a principal, so no fetch and no entry) or is `None` vs `""` (the same unscoped search). Raw-spelling hash = known |
| E5c/E5d | n/a | |
| E6 set_allowlist / AD2 | **SW5-5-1** | wildcard vs literal `*` |
| E7 remove_allowlist | holds | keyed by `Principal` |
| E8 set_limits | n/a | no keys |
| E9/E10 tokens / AD5/AD6 | holds | `trim` + lowercase fold: case/whitespace variants are the same digest, so merging them is correct; re-adding overwrites the label silently (cosmetic). Caller id = 64-bit hash prefix; collision between registered tokens needs ~2^32 controller-chosen tokens |
| E11 set_candid_override | holds | key is `Principal::to_text()` (canonical); register #1 fixed |
| E12-E14 | holds | `list_tokens` shows 16-hex prefixes (display only; revoke takes the full hash) |
| E16 JS host fns | known | canister/method via `to_std_string_escaped`: a lone surrogate and its literal `\uXXXX` spelling become one string (known L7-10 and the JS key-collapse finding); policy checks the same text that is sent, so no policy bypass |
| E17 console | n/a | no keys |
| E18 Python host fns | holds | kwargs refused (`src/runtimes/python.rs:240-242`), so positional and keyword spellings of one parameter cannot meet; `icp.call(c, m)` and `icp.call(c, m, None)` both mean no args |
| E19 Rhai host fns | holds | 2-arg and 3-arg `call_canister` with `()` both mean no args; replay compares `Request` by the JSON actually sent, so two Dynamics that serialise alike are the same request |
| E20-E25 demo_store | known / holds | see C2 |
| E26-E28 | holds / n/a | see C5, C6, C12 |

### Untrusted inputs, outbound effects, state

| Row | Verdict | Reason |
|---|---|---|
| U1 method/url | holds | see E4 |
| U2 headers | holds | see E3/E4; bearer value trimmed (`" tok"` and `"tok"` are one token; a token registered with surrounding whitespace can never authenticate, operator-side only) |
| U3/U4 body, JSON-RPC keys | holds (noted) | serde_json keeps the last of duplicate or escape-spelled keys silently (R: `{"code":"A","code":"B","id":1,"id":2}` -> `{"code":"B","id":2}`). Every in-repo consumer (`inspect_message`, query, update) uses the same parser, so nothing in the repo sees a different value; the hashed and audited code is the code that ran. Ids beyond u64 that collapse to one echoed id are part of known L7-9 |
| U5 tool args | holds | see E5a/E5b |
| U6 source | n/a | |
| U7 script values | known | record id-field aliases (L7-13), one key filling two fields, `_n_`/quoted-number variant tags, Python non-str dict keys (L7-7), JS lone-surrogate keys: all known |
| U8 reply bytes | known | quoted-number label vs id label printing alike (L7-14) and variant arms printing alike: known; Boa `from_json` canonicalises numeric keys to `Index` (`property/mod.rs:695-699`), so a reply key `"1"` is reachable as `o[1]` and `o["1"]` |
| U9 rejects | n/a | text only |
| U10 interface text | holds | candid_parser `check_prog` refuses duplicate method names and field-hash collisions; the only label collision it admits (quoted `"1"` vs id `1`) is known |
| U11 admin inputs | **SW5-5-1** / holds | see E6, E9-E11 |
| U12 | holds | see E3 |
| U13 demo_store args | known | duplicate lines |
| U14 demo page | holds | see C5 |
| U15, U16 | n/a | |
| O1 calls | **SW5-5-1** | wildcard governs which methods are sent |
| O2 interface fetch | holds | `check_interface_fetch` uses its own `["*"]` and sentinel `<candid interface>`; a script call to a method named `<candid interface>` is checked as an ordinary method name and logged under the real label (audit-label ambiguity is known) |
| O3 audit writes | known / holds | `canisters` from canonical `to_text()`; `canister.method` label ambiguity known; `language` disambiguates a search key text equal to a script's source |
| O4-O10 | n/a / holds | no aliasing |
| D1 LIMITS | n/a | |
| D2 ALLOWLIST | **SW5-5-1** | values: method strings with `"*"` sentinel; keys `Principal` |
| D3 TOKENS | holds | see E9 |
| D4 AUDIT | holds | ids from `len()`; run_id unique per Started entry (snapshot re-issue known) |
| D5 CANDID_OVERRIDES | holds | canonical principal text |
| D6-D10 | n/a | single values |
| V1, V3 | n/a | counters / run ids |
| V2 DID_CACHE, V4 DID_FAILED, V5 OVERRIDES | holds | keyed by canonical principal text everywhere (`cached_service`, `IcBridge::candid`, `service`, `evict_interface`) |
| V6-V11 | n/a | no keyed state from untrusted spellings |

### Commit points, limits, work, failures

| Row | Verdict | Reason |
|---|---|---|
| K1-K12 | n/a | no key or label normalisation at commit points |
| LIM1-LIM32 | n/a | numeric bounds; none keyed by a normalised spelling |
| W1-W16 | n/a / known | W5 `field_value` linear alias scan is the known L7-13 site |
| F1-F16 | n/a | |

### Boundary conversions

| Row | Verdict | Reason |
|---|---|---|
| B1 | holds (noted) | see U3/U4 |
| B2 | holds | Candid decode of a fixed type |
| B3 | holds | see E9 |
| B4 | holds / known | canonicalised before allowlist, cache and override lookup; search-only whitespace trim known |
| B5 json_to_idl | known / holds | record aliases (`5`, `05`, `+5`, `_5_` all match `Id(5)` via `parse::<u32>`) = known L7-13 family; quoted-number vs id fields and `_n_` variant tags known; numeric underscores and hex prefixes known (L7-12); func given as `[p, m]` or `{principal, method}` are two shapes of one value with no overlap; variant objects must have exactly one key |
| B6 idl_to_json | known | L7-14 / variant arms; `opt null`, `null`, `reserved` all print `null` = register #42 / L7-11 |
| B7 | holds | see U10 |
| B8 JS | known / holds | lone-surrogate key collapse known; snapshot defines own properties (`__proto__` stays data); integral floats fold to integers by design |
| B9 Python | known | non-str dict keys via `py_repr` (L7-7); class-name dispatch known (SW4-5-1) |
| B10 Rhai | holds | map keys are unique strings both ways |
| B11 | known | string results passed raw (L7-16) |
| B12 | n/a | |
| B13 Language | holds | see E5a |
| B14, B15 | n/a | |
| B16 search render | **SW5-5-1** / holds | `"*"` shows every method; `types` map keyed by unique type-env names |
| B17 demo page | holds | see C5 |

### Value flows, admin, supply chain, claims, tests

| Row | Verdict | Reason |
|---|---|---|
| VF1-VF3, VF5, VF7, VF8 | n/a | |
| VF4 | **SW5-5-1** | a target whose `*` method is allowlisted also exposes its transfer/approve methods |
| VF6 | known | duplicate order lines |
| AD1 | holds | see E1/E2 |
| AD2 | **SW5-5-1** | |
| AD3-AD9 | holds / n/a | see E7-E11 |
| S1-S10 | n/a | |
| CL6 "deny by default" / `.did` and policy.rs comment "`[\"*\"]` allows every method" | **SW5-5-1** | the documented wildcard has no escape for a method of that name |
| CL23 canonicalisation claim | holds | verified for allowlist, caches and overrides |
| other CL rows | n/a | no aliasing claims |
| T1-T12 | n/a | no test covers a `*`-named method or the wildcard collision (SW5-5-1) |
| History (#1, #35, #37, #42) | re-checked | #1 holds for all heap and stable keys; #35 residuals in B5/B6 are the known findings listed above |
