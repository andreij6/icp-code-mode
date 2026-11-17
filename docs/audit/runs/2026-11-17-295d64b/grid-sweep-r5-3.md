# Variant sweep r5-3: supplied data silently discarded by a permissive type

Run `2026-11-17-295d64b`, Phase 6, round 5. Pattern (seed: lenient coercion in `json_to_idl`,
with L7-5 / L7-12 / L7-13, all known and not re-reported):

> Supplied data is silently discarded because the receiving type or signature is
> permissive: an extra argument, key, field or entry is accepted and dropped, and the
> operation reports success.

Method: code reading at `295d64b` of every place an input from a script, a client, an
operator or a target is mapped onto a fixed shape (function arity, record/func object, map
entry, Candid record): `src/candid_bridge.rs` (B5/B6), `src/runtimes/{js,python,rhai_rt}.rs`
host functions and converters (E16-E19, B8-B10), `src/mcp.rs` (U3-U5, B1, B11, B13),
`src/search.rs`, `src/lib.rs` `apply` and admin methods (E1/E2, E6-E14), `src/policy.rs`
setters, `demo_store/src/lib.rs`. The working tree has uncommitted test-only additions
(`git diff -U0 HEAD` hunks are all inside `mod tests`/`mod mock`); production code equals
`295d64b`. Probes ran in a private `git archive HEAD` copy
(`/private/tmp/claude-501/sweep-r5-3.xLJ4`, deleted afterwards) with
`CARGO_TARGET_DIR=/private/tmp/claude-501/audit-target`,
`cargo test --lib sweep_r5_3 -- --nocapture`. R = reproduced, C = code path.

## Probes

- P1 (`runtimes::sweep_r5_3::extra_call_args`, `MockBridge`, R):
  - JS `icp.call(L, 'icrc1_balance_of', {owner:'aaaaa-aa'}, {subaccount:[1,2]}, 'memo')`:
    one call recorded with args `{"owner":"aaaaa-aa"}`, run `Ok`.
  - JS `icp.call(L, 'icrc1_balance_of', undefined, {owner:'aaaaa-aa'})`: call recorded with
    args `null`, run `Ok`.
  - Python, same four arguments: `TypeError: icp.call(canister_id: str, method: str, args=None)`,
    no call.
  - Rhai, same four arguments: `Function not found: icp::call_canister (.., map, map)`, no call.
  - JS `icp.candid(L, 'extra')`: accepted (harmless: no data in the extra argument).
- P2 (`candid_bridge::sweep_r5_3::func_object_extra_keys`, R): against `func (nat) -> (text)`,
  `{"principal":"aaaaa-aa","method":"m","args":[1,2,3],"n":5}` and
  `{"principal":"aaaaa-aa","method":"m","cycles":1000000}` both encode as
  `func "aaaaa-aa".m`. The same extra key on a record (`{"cb":..,"x":1}`) is refused with
  `unknown record field `x``; a func object nested in that record accepts `"n":7` and drops it.
- P3 (`sweep_r5_3::init_args_duplicate_allowlist`, R): `apply(InitArgs { allowlist:
  [(ledger, ["icrc1_balance_of"]), (ledger, ["icrc1_symbol"])] })` leaves the allowlist as
  `[(ledger, ["icrc1_symbol"])]`; no error, install would succeed.

## Cells

| Row | Site | Verdict | Reason |
|---|---|---|---|
| E16 JS `icp.call` (`js.rs:250-273`, registered with length 3 at `js.rs:294`) | arguments after the third | **finding SW5-3-1** (R, P1) | `request_fn` reads `args.get(2)` and never looks at `args.len()`; Boa passes extra arguments silently. A 4th+ argument is dropped and the message is sent; Python and Rhai refuse the same call. L6-r1 checked the two-argument form and wrong types only |
| E16 JS `icp.candid`, `icp.principal` | extra arguments | holds | Extra arguments carry no data the host would use; nothing is sent differently |
| E17 JS `console.*`, `print` | arguments | holds | Every argument is formatted (`js.rs:234`) |
| E18 Python `icp.call/call_async/candid/principal` (`python.rs:124-139, 245-248`) | arity, kwargs | holds (R, P1) | Exact slice patterns; anything else is a `TypeError`; kwargs refused before dispatch |
| E19 Rhai `call_canister` (2 and 3 args), `candid`, `principal` (`rhai_rt.rs:155-180`) | arity | holds (R, P1) | Native registrations are exact-arity; a 4th argument is "Function not found" |
| B5 `json_to_idl` func object (`candid_bridge.rs:220-230`) | keys other than `principal`/`method` | **finding SW5-3-2** (R, P2) | `o.get("principal")`, `o.get("method")` only; extra keys dropped while the record branch (`:264-268`) refuses unknown keys with the comment "A typo ... must not silently become `null`". r4-11 noted it as "a different pattern" |
| B5 `json_to_idl` record object (`:264-273`) | unknown keys | holds | Refused (`unknown record field`); duplicate aliases **known** L7-13; one key filling two fields **known** (SW11-1) |
| B5 tuple from array (`:256-263`) | extra elements | holds | Length must equal field count |
| B5 variant (`:280-297`) | extra keys / payload | holds; null/reserved payload **known** L7-11 | `o.len() == 1` required; tag aliasing **known** (SW11-2, `_n_` alias finding) |
| B5 `encode_args` positional (`:63-87`) | extra positional args | holds | `positional.len() > func.args.len()` refused; `[x]` heuristic and opt-vec `[]` **known** (L7-15, single-arg heuristic) |
| B5 numbers / hex / float32 (`:151-176, 216, 247-249`) | lenient text | **known** L7-5, L7-12 | Seed findings |
| B5 opt (`:231-234`) | wrong-typed value under opt | holds | Inner conversion errors propagate; no Candid-style "opt absorbs mismatch" on the way out |
| B6 `idl_to_json` | reply fields/values not in the interface, reserved | **known** | Stale/overridden interface turns data into null and drops new fields/returns (known); record/variant key collapse known (L7-14, SW11-2) |
| B8 JS snapshot (`js.rs:45-94`) | undefined/function/symbol members, Map/Set, extra array properties | **known** / holds | Dropped values are known L6-9; Map -> `{}` and array non-index properties follow `JSON.stringify` (L7-r1 "holds") |
| B9 Python `to_json` (`python.rs:62-100`) | repr fallbacks, non-str keys, class-name dispatch | **known** (L7-6, L7-7, class-name finding) | |
| B10 Rhai `to_json_value` (`rhai_rt.rs:88-117`) | FnPtr, ranges, char, unit | holds | `from_dynamic::<Value>` refuses FnPtr/range (r2-5, r4-8); char -> string, unit -> null are faithful |
| U3/B1 JSON-RPC body (`mcp.rs:236-304`) | duplicate keys, extra members | holds (L1-r2, r4-11) | serde_json last-wins for identical keys, noted under L7-9; extra members (`_meta`) are allowed by MCP |
| U5 `parse_tool_call` (`mcp.rs:202-228`) | unknown `arguments` keys, non-object `arguments`, `language: null` | **known** | "Tool arguments are not checked against the advertised inputSchema" |
| U5 `query` / `canister_id` types | non-string | holds | `opt_string` refuses non-strings (`mcp.rs:202-208`); padded/invalid canister_id **known** |
| B13 `Language::from_str` (`runtimes/mod.rs:34-44`) | case, aliases | holds | Case-folding and `js`/`py` aliases keep the meaning; unknown names refused |
| U2 headers (`mcp.rs:276-281`) | repeated `Authorization` | holds | First header wins; a second valid token is ignored and the request fails closed (401) |
| U12/B2 `HttpUpdateRequest` / `HttpRequest` Candid decode | extra record fields | holds | Standard Candid subtyping on a gateway-defined type; the app reads only method/url/headers/body |
| E1/E2 `apply(InitArgs)` allowlist (`lib.rs:38-49`, `policy.rs:155-157`) | duplicate principals in `vec (principal, vec text)` | **finding SW5-3-3** (R, P3) | `set_allowed` is a `StableBTreeMap::insert`; later entries for the same principal replace earlier method lists with no error. Fails closed (fewer methods) |
| E1/E2 `token_sha256` duplicates | | holds | Same hash, same label `"init"`: nothing lost |
| E1/E2/E8 Candid records with misspelled opt field (`limit`, `call_timeout_secs`) | unknown field skipped, opt reverts to null | **known** | Same outcome as "set_limits replaces the whole record: omitted opt settings revert to null" and "arguments under opt that do not fit their type are decoded as null"; a typo is an omission plus a skipped field |
| E6 `set_allowlist` | replaces the method list | holds | "set" semantics; README shows a full list per call |
| E6 method names with whitespace / not in the interface | stored literally | out of pattern | Kept, not discarded; never matches, fails closed (admin validation is the known "Admin inputs are unbounded/unvalidated" class) |
| E9 `add_api_token_hash` existing hash | label replaced | holds | Label is metadata; hash case/whitespace normalisation keeps the value |
| E11 `set_candid_override(p, opt text)` | mistyped text under opt | **known** | Listed in the "under opt ... decoded as null" finding |
| E13 `get_audit_logs` limit > 100 | clamped | holds | Documented (CL32) |
| E20-E23 demo_store | zero-quantity lines, duplicate lines, restock saturation | **known** | Zero-quantity and oversell findings; restock saturation SW4-9-1 |
| E20 `search_products` / `list_products` | text filters | holds for this pattern | Text is used, not discarded |
| W3 search keywords (`search.rs:80-82`) | whitespace-only query | holds | Equivalent to no filter, documented as "None matches everything" |
| T1-T12 | tests | gap | No test passes a 4th argument to JS `icp.call`, extra keys to a func object, or duplicate InitArgs allowlist entries |

## New candidates

### SW5-3-1 (Medium): JS `icp.call` silently drops arguments after the third and sends the call

`request_fn` (`src/runtimes/js.rs:250-273`) takes `args.get(2)` as the call arguments and
ignores the rest. `icp.call(L, 'icrc1_balance_of', {owner}, {subaccount})` sends
`{owner}` alone (for an ICRC ledger: the default-subaccount balance, reported Ok);
`icp.call(L, m, undefined, x)` sends `null`. Python raises `TypeError` and Rhai
"Function not found" for the same four-argument call, so the tool description's
"identical semantics in every language" (CL29, shown to the AI) does not hold. Reproduced
natively with `MockBridge` (P1). Fix: refuse `args.len() > 3` (and `> 1` for
`icp.candid`) with a `TypeError`, matching Python.

### SW5-3-2 (Low): `json_to_idl` func objects accept and drop unknown keys

`candid_bridge.rs:223-226` reads only `principal` and `method` from an object; any other
key is discarded, while records refuse unknown keys. Reproduced (P2), including a func
nested inside a record. Fix: refuse an object whose keys are not exactly
`principal`/`method`.

### SW5-3-3 (Low): duplicate principals in `InitArgs.allowlist` replace each other silently

`apply` (`src/lib.rs:43-45`) calls `policy::set_allowed` per entry, an overwriting insert,
so only the last method list for a principal survives install or upgrade. Reproduced (P3).
Fails closed. Fix: merge or refuse duplicates in `apply` (trap, like a bad token hash).
