# Variant sweep r1-10: documented exhaustive lists the code does not match

Run `2026-11-17-295d64b`, Phase 6. Pattern (from the confirmed "two kinds of request write
no entries" finding): **a documented exhaustive list ("two kinds", "exactly one of these",
"every", "only", "nothing can", an enumerated set of shapes or cases) that the code's
condition does not match**. Every row of `inventory.md` was searched for a list or an
absolute in its docs, interface comments, tool descriptions or code comments, and the
list was compared with the condition the code actually tests.

Evidence: reading the code at `295d64b`; native probe tests in a private `git archive`
copy (`cargo test --lib sweep_`, shared `CARGO_TARGET_DIR`, copy deleted afterwards);
Boa 0.22.0 and Monty 1.6.0 sources in the cargo registry. Nothing deployed or changed.

## New candidates

| Id | Sev | Title | Evidence |
|---|---|---|---|
| SW10-1 | Low | `BoundedJobs::enqueue_job` drops every job that is not a promise or generic job, on the stated ground that "nothing in the sandbox can create them (no host async functions, no timers; Atomics and FinalizationRegistry are removed)". Dynamic `import()` creates one (`Job::AsyncJob`, Boa `vm/opcode/call/mod.rs:604-611`). The job is dropped, so the promise never settles: `try { await import("x") } catch (e) {}` cannot catch, and the run fails with "script stopped before finishing: a runtime limit ... was hit after an await, or it awaits a promise that never settles" instead of the loader's "Module could not be found." | reproduced (native): try/catch -> `Err("script stopped before finishing: ...")`; `import("x")` state after microtasks -> `"pending"`; the same TypeError delivered as a rejected promise is caught normally |
| SW10-2 | Low | Variant shorthand `"Tag"` is documented for a null payload only (README:355 "for a null payload, `"Tag"` alone also works"; SEARCH_DESCRIPTION "or "Tag" with no payload"; code comment candid_bridge.rs:281 "for null/reserved payloads"). The code passes JSON `null` to whatever the payload type is, so it is also accepted for `opt T` (and empty-record) payloads and silently sends `Tag = null` | reproduced (native): `variant { Set : opt nat; ... }` given `"Set"` -> sent `variant { Set = null }`; `{"Set":5}` -> `opt 5`; `"Num"` (nat payload) and `"Rec"` (non-empty record) correctly refused |
| SW10-3 | Low | `classify` ignores any invalid message that has a `result` or `error` key, per the comment "Responses from the client ... carry no method: ignore them"; the code never checks that `method` is absent. A request with an invalid id or `jsonrpc` gets `-32600` normally, but the same request with a stray `result`/`error` key is dropped without a reply (202 when it is the only message) | reproduced (native, `http_query`): `{"jsonrpc":"2.0","id":null,"method":"ping"}` -> 200 `-32600`; same plus `"result":0` -> 202 empty; `jsonrpc:"1.0"` + method + `error` -> 202; object id + `tools/call` + `result` -> 202 |
| SW10-4 | Low | `src/mcp.rs:4-5` module doc: the query "asks the gateway to upgrade anything containing `tools/call`" and "answers everything that is read-only". Code upgrades only an `execute_icp_script` or a search needing a fetch; a cached search and a `tools/call` with bad params (`-32602`) are answered in the query, and a read-only search that needs a fetch is upgraded | code path (`http_query` src/mcp.rs:358-367) + existing test `mcp::tests` cached-search-in-query |
| SW10-5 | Low | `certified_reads = true` is documented as "every request goes through an update call" (README:437) and "every MCP request goes through an update call" (`.did` / policy.rs:34 Limits doc). The code upgrades only requests that pass `authenticate`; wrong path (404), non-POST (405) and missing/invalid token (401) are still answered by the uncertified query, and so is the "demo page disabled" 404. (`mcp.rs:337` itself says "every authenticated request".) Related to, but not the same as, the known skip-certification finding | reproduced (native, `http_query(.., certified=true, ..)`): POST /mcp bad token -> (401, upgrade None); GET /mcp -> (405, None); POST /other -> (404, None); GET / -> (404, None) |

## Additional symptoms of known findings (not new candidates)

| Known finding | Extra symptom found by this sweep | Evidence |
|---|---|---|
| README's list of requests that write no audit entry is incomplete (remembered fetch failure; L16-12) | The comment at src/lib.rs:159-160 lists the audited cases as "sent messages ... or was refused by policy" (`calls > 0 \|\| !violations.is_empty()`). A search whose fetch took a cap slot but whose messages were both refused on the spot by the system (queue full, no cycles) has `calls == 0` and no violation, so it writes no entry while the client gets a fetch error | code path |
| Mid-search allowlist changes are trusted after a suspension | src/lib.rs:132 "After a policy refusal (call cap, budget) every further fetch would be refused too". The condition is `e.starts_with("policy:")`, which also matches "canister X is not allowlisted"; after a controller removes one canister mid-search, every later (still allowlisted) canister is reported "not fetched: policy: canister X is not allowlisted" and not tried | code path |
| JS per-round cap exceeded by a getter in call arguments | `SNAPSHOT_JS` doc (js.rs:41) says the copy takes "own data properties only"; `Object.keys` + `x[k]` also reads own enumerable accessor properties and runs their getters, which is how the known getter finding reaches `icp.call` | code path |
| L7-15 (`opt vec` given `[]`) | README:348-351 three-way rule ("An array: positional arguments") vs the array-like exception in `encode_args` (candid_bridge.rs:67-73): for a single `vec` argument, `[5]` is sent as `vec {5}` (not `5`) and `[1,2]` as one argument (not "takes 1 argument, got 2"). Same root cause as L7-15 | code path |

## Grid (pattern: exhaustive list vs code condition)

| Rows | Result |
|---|---|
| C1, C1a | C1a: per-language builds advertise all three languages (known). C1: covered by the rows below |
| C2, E20-E25 | holds: demo_store doc lists ("10 methods (7 reads, 3 writes)", "all 7 reads plus create_order" in C6, `search_products` "case-insensitive ... on name or category": category compared case-sensitively against a lowercased query, but every seeded category is lowercase and categories are fixed) match the code. Access-control claim is known |
| C3, C4, C8, S4 | holds: shim/patch comments name exactly what they replace; `.cargo/config.toml` "getrandom 0.3/0.4 use the custom backend" matches (0.2 is build-time only, L11/L15 grids) |
| C5, E26, B17, CL30 | holds: `esc` escapes `&<>` and is only used in element content, never inside attributes; storage claims known |
| C6, E27 | holds (method list matches .did) |
| C7, C9, C10, C13, C14, C15, S1-S3, S5-S10 | n/a beyond known L15 findings: no enumerated behaviour claim other than those already reported |
| C11 | holds: `.did` comments checked under AuditStatus/Limits/PolicyView rows below |
| C12, U16, O10, E28 | n/a: no exhaustive list in the video tooling beyond the known token findings |
| A1-A12 | holds: README:159-165 "other admin endpoints are controller-only" list matches the guards (`set_allowlist`, `add_api_token_hash` named earlier); `inspect_message` doc (lib.rs:268) "Only calls with a registered bearer token, or from a controller" is a summary; README:375 states the per-method rule the code applies |
| E1, E2, AD1 | holds for lists; additive re-apply is known |
| E3, CL16 | holds: README:375 per-method rule = `match msg_method_name()` arms |
| E4, E4a, E4b, CL28 | **SW10-4**, **SW10-5**; skip-certification headers go on every non-upgrade query reply (superset of "initialize, tools/list"): holds |
| E5, E5a, E5b | known (MAX_SEARCHES query path, search audit); halting-condition symptom above |
| E5c, E5d, K1-K12 | holds: AuditStatus and README:400-426 enumerations ("exactly one of these", "one of three things") match `run_script`/`run_search`/`Drop for IcBridge` except the known cases (cleanup trap, no-entry searches); the precedence PolicyViolation over Ok when a script catches a refusal is consistent with the table's PolicyViolation row |
| E6-E14, AD2-AD9, U11 | holds for lists; unvalidated values are known |
| E15 | n/a |
| E16, E17, B8, CL11 | **SW10-1**; JS removed-globals list (README:370) is a subset of what the code removes (`FinalizationRegistry` too); `ensure_can_compile_strings` covers every dynamic-function constructor (Boa `function/mod.rs:469`); snapshot "data properties only" symptom above |
| E18, B9, CL12, CL29 | holds for lists: Python OS calls other than `import icp` get `no_handler_result`; all clocks read the fixed `DateTimeSource`, `process_time` defaults to `Zero`; `random` is known |
| E19, B10, CL13, CL35 | holds: Rhai `call_canister` arities 2/3 match README; `eval` disabled, `max_modules(0)`; divergence detection compares every replayed request |
| U1-U4, B1, B2 | **SW10-3** |
| U5, B13 | holds: language aliases are a superset of the schema enum (schema non-enforcement is known) |
| U6, U7, CL10 | known (print values, UTF-16 counting) |
| U8, U9, B6 | holds: README:359 "numbers when JS-safe, else decimal strings" matches `big()` for every integer width; nat8-32/int8-32 always numbers |
| U10, U13, U14, U15 | n/a or known |
| U12 | holds |
| O1, O2, CL7 | holds: README:366 "every icp.call attempt that passes policy and every interface fetch that misses the cache" = the two `reserve` sites; slot-vs-message gap is known |
| O3, CL15, CL17-CL20 | known + no-entry symptom above |
| O4-O9 | holds |
| D1-D10 | holds for lists (run_id "own id if the run never sent a message" = `get_or_insert(entry.id)`; methods label doc is known) |
| V1-V11, CL31 | holds: `pending_run_ids` "sent messages and not ended" = insert in `record_message`, remove in `Drop`; no await between closing entry and drop. Leak on cleanup trap is known |
| LIM1-LIM32, CL9, CL14, CL21, CL22 | holds for lists except known (MAX_SEARCHES, byte caps, set_limits); `get_audit_logs` "newest first, capped at 100" matches `audit_page` |
| W1-W16 | n/a: no enumerated claims beyond limits rows |
| F1-F16 | n/a |
| B3, B4, CL23 | holds: canonicalisation list (allowlist, cache, override) — every key site uses `Principal::to_text()` (`DID_FAILED`, `evict_interface`, `set_candid_override`, search `scope`) |
| B5 | **SW10-2**; L7-15 symptom above; `decode_reply` "single unwrapped, zero null, several array" holds |
| B7, B11, B12, B14-B16 | n/a or known |
| VF1-VF9 | n/a (no enumerated claim) |
| CL1-CL6, CL8, CL24-CL27, CL32-CL34, CL36, CL37 | known findings; CL32 holds |
| T1-T12 | n/a (tests are not claims); none of SW10-1..5 has a test |
| History (register) | Register "Patterns worth sweeping for" has no entry for this pattern; it should gain one: "A documented list or absolute ('two kinds', 'every', 'nothing can') whose code condition is broader or narrower" |
