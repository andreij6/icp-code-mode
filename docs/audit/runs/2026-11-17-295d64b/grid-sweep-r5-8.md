# Variant sweep r5-8: "Type information dropped at a boundary"

Pattern: a value or a fact about it (its type, kind, name, provenance or class) is known
on one side of a conversion and missing on the other, so the receiver cannot tell apart
cases that the sender kept apart. Searched every inventory row. Known findings from the
task's list are marked "known" and not raised again.

Evidence: the probe was run natively in a private `git archive` copy of 295d64b. The
crate version was bumped in the copy so the shared CARGO_TARGET_DIR could not reuse
another copy's build. The copy was deleted afterwards. Everything else is from reading
the code path.

## New candidates

| Id | Row(s) | Result |
|---|---|---|
| SW58-1 | B7, B16, E5b, CL3 | **finding (reproduced).** `Service::parse` -> `candid_parser::check_prog` turns `IDLArgType { name, typ }` into bare `Type`s (typing.rs:75-96), and `search::signature` / `type_text` re-print that `TypeEnv`. Argument names, result names and every `//` doc comment of the target's .did are dropped. Probe output: `transfer : (from : principal, to : principal, amount : nat) -> (ok : bool)` with a doc comment is shown as `"signature": "(principal, principal, nat) -> (bool)"`. For the project's own demo_store, the comments that explain `list_products` (the valid category values), `list_orders` and `Inventory."reserved"` never reach the model through search. `icp.candid` still returns the full text. |
| SW58-2 | O1, U8, U9, B5/B6 path, E16-E19, CL15 (README:397-398) | **finding (code path).** `HostBridge::call` returns `Result<Value, String>`. ic-cdk's typed `CallFailed` and its `is_clean_reject()` classification are turned into text, and the reply-decode step (`svc.decode_reply`, bridge.rs:532) adds another failure to the same channel. Only one marker is added back: " (outcome unknown…)", and only when `call_timeout` is set (bridge.rs:529). With the default unbounded wait, a `CanisterReject` or `CanisterError` (ic-cdk: not clean, so the callee may have committed state) carries no marker. A decode failure after the target ran ("candid decoding failed: …" or "reply nested deeper…") carries none either. Scripts get the same `Error` (JS), `RuntimeError` (Python) or runtime string (Rhai) as a policy refusal that sent nothing. README:397-398 says whether the target executed is in the error ("a reject, or 'outcome unknown'"). |

## Coverage cells

| Row | Cell |
|---|---|
| C1, C1a | Covered by the B and E rows below. C1a's language advertisement is known. |
| C2 (demo_store) | holds for this pattern: the narrowing casts (`as u32`/`u64`) are known wraps (`reserved`, `sales_summary`, `restock`). |
| C3 web-time shim | known: instructions are relabelled as nanoseconds (Python budget findings). |
| C4, C9, C10, C13, C14, C15 | n/a: no data crosses a typed boundary at run time. |
| C5, C12, E26, E28 | known: string results are passed raw (L7-16). n/a for video. |
| C6 scripts/demo-setup.sh | known: parses human-readable icp-cli output. |
| C7, C8 | n/a. |
| C11 .did files | holds: the tests assert that they match `export_candid!`. |
| A1-A12 | n/a: actors. The fact "which token holder" is reduced to a 16-hex prefix (B3). That is a design choice already in the register (#49). |
| E1/E2 InitArgs, E8, E11 | known: opt fields that do not fit their type decode as null (set_candid_override text, Limits). |
| E3 inspect_message | known (quota). |
| E4/E4b query path | holds: the same render as the update path. Query vs update divergences are known. |
| E5a run_script | Closing status is derived from `violations`, not from error text. holds. |
| E5b run_search | `e.starts_with("policy:")` classifies by text prefix. holds: every non-policy error from `candid()` is wrapped as "could not fetch…" or "invalid canister id…", and none starts with "policy:". When an override cannot be parsed, `cached_service` gives `Ok(None)` and search says "interface not fetched", not a parse error. That merges into the known override findings (pre-5d856e3 overrides, query-limit re-parse). |
| E5c/E5d | known (Trapped entry fields). |
| E6 set_allowlist | in-band sentinel: `"*"` is both the wildcard and a legal Candid method name (`"*" : () -> ()`), so a method literally named `*` cannot be allowed on its own. `check_interface_fetch` uses `"<candid interface>"` as a pseudo-method in the same way. Not raised: no real interface has these names, so there is no demonstrated failure (Low hardening at most). |
| E7, E9, E10, E12-E14 | holds. get_policy hides overrides (known). Audit labels and statuses are known. |
| E16 JS host fns | Args pass through snapshot -> JSON text -> `serde_json::from_str(..).unwrap_or(Null)`. The depth->null and float 1-ulp cases are known (L7-2, float re-parse). Map/Set become `{}` and typed arrays become index-keyed objects, as `JSON.stringify` does; the README says values are "JSON-like". For blob/record arguments the encoder then fails loudly. Not raised. Errors: SW58-2. |
| E17 console | known (snapshot printing). |
| E18 Python host fns | Tuple and list both become JSON arrays, so Python cannot mark positional args precisely and falls back to the `[x]` heuristic. Merges into the known single-argument heuristic finding. bytes -> array is documented. Errors: SW58-2. |
| E19 Rhai | Through `from_dynamic`, FnPtr, Range, other `Variant`s and `Shared` give a type_error (loud). Char -> string (no JSON char type). holds. Errors: SW58-2. |
| E20-E25 demo_store | n/a (no conversions beyond Candid derive). |
| U1-U4 | known: JSON-RPC id forms, null `language`, Origin. |
| U5 query/canister_id | known: whitespace padding. An invalid principal text is reported as "not allowlisted"; the `scope` doc comment says so on purpose. holds. |
| U6, U7 | U7 is covered by B8-B10 below. |
| U8 reply bytes | known: opt->null for stale interfaces, quota, depth, gamma clones. The executed-but-undecodable outcome is SW58-2. |
| U9 reject codes/messages | SW58-2: the reject code and the clean/unclean fact are flattened into text. |
| U10 interface text | SW58-1 (names and comments dropped on re-print). Provenance: `icp.candid` returns override text with no signal that it is not the target's own interface. Merges into the known override-precedence and get_policy findings. |
| U11 admin inputs | known. |
| U12-U16 | n/a or known. |
| O1 | SW58-2. |
| O2 | known: the audit names only aaaaa-aa, and the fallback discards the metadata error. |
| O3/O4 audit | known: `methods` labels without escaping, "…and N more" stored in-band in `violations`, language "search". |
| O5-O10 | n/a. |
| D1 Limits | known: an omitted opt reverts to null. |
| D2 AllowedMethods | see E6. |
| D3 tokens | holds: case is folded before storing. |
| D4 AuditEntry | known (AuditStatus additions, labels). |
| D5 overrides | Keyed by canonical principal text. holds. |
| D6-D10 | n/a. |
| V1-V11 | n/a: these are caches of already-typed values. For V2/V5, the parsed `Service` keeps no names or comments; see SW58-1. |
| K1-K12 | n/a (commit points). SW58-2 is the error side of K2-K4. |
| LIM1-LIM32 | n/a: these are numeric limits. LIM21 depth counting and LIM16 key bytes are known. |
| W1-W16 | n/a for this pattern. W16's demo regex is known. |
| F1-F16 | F11 `f64 as f32` saturation is known (L7-5). The float32 reply widened to f64 (0.1f32 -> 0.10000000149011612) is exact and round-trips, so it holds. |
| B1 | known (id forms, result/error keys). |
| B2 | known. |
| B3 | Token -> 16-hex caller id. holds (by design; see A1-A12). |
| B4 | holds (canonicalised everywhere). Search padding is known. |
| B5 json_to_idl | All silent cases are known: opt opt, `[x]` heuristic, quoted-number labels, duplicate aliased keys, null/reserved slots, lenient numbers, float32 saturation, 2^53 floats. A `func` object ignores extra keys, but every required part is checked loudly. holds. |
| B6 idl_to_json | Blob -> plain array drops the blob type. Its symptoms (per-byte value budget; Rhai refusing over 10k elements) are known. Zero returns, a `null` return and an `opt` None all give `null`. A tuple record and a multi-value return give the same array. These are inherent to the JSON mapping, the signature shows them, and they are documented. Non-finite floats as strings are known. |
| B7 .did -> TypeEnv | SW58-1. |
| B8 JS | -0, NaN, BigInt, lone surrogates, boxed primitives and Symbols are known. Integral float64 replies leave JS as `3` while Python prints `3.0`. That is JS's single number type and is not raised. |
| B9 Python | known: class-name dispatch, repr fallback, dict keys, surrogates, clock. |
| B10 Rhai | known: size limits on replies, INT range. |
| B11 tool_result | known: raw strings, truncation, isError only. |
| B12 Storable | known: schema evolution, opt defaults. |
| B13 language | known: null accepted, per-language builds. |
| B14 time | known: ms vs s. |
| B15 certificate | known: certified_reads claim. |
| B16 search JSON | SW58-1. Known: labels rendered in a form the converter does not accept, and methods listed that icp.call refuses. |
| B17 demo page | known. |
| VF1-VF8 | n/a. SW58-2 matters for VF4: a script that retries a "failed" transfer whose reply was undecodable sends it twice. Inferred, not demonstrated. |
| AD1-AD9 | AD2 see E6. The rest are known. |
| S1-S10 | n/a. |
| CL3, CL15, CL29 | CL15 (README:397-398) is contradicted by SW58-2. CL3/CL29 are known. |
| T1-T12 | No test renders a named-argument or commented interface through search (SW58-1). No test checks the error text of a post-execution decode failure (SW58-2). |
