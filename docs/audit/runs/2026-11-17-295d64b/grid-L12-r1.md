# Grid: L12 Protocol conformance, round 1

Run `2026-11-17-295d64b`. Specifications checked against: JSON-RPC 2.0, MCP 2025-03-26 (base
protocol, lifecycle, Streamable HTTP transport, tools), ICP HTTP gateway interface (query
`http_request`, `upgrade`, `http_request_update`, response verification v2 / skip
certification), Candid (method modes, class types, value encoding).

Evidence: **R-native** = reproduced with a probe test added to a private copy of the tree
(`cargo test --lib l12_probe` / `l12_candid_probe`, calling the production `mcp::http_query`,
`mcp::http_update`, `mcp::tool_result`, `candid_bridge::Service`, `search::render`; script
execution through the real engines with `MockBridge`). **R-live** = reproduced against the local
network gateway `http://icp_code_mode.local.localhost:8010/mcp` with query-path requests only
(no script run, no state change). **code** = the code path produces it unambiguously.

Candidate findings are numbered L12-1 .. L12-7 and described at the end.

## Rows

| Row | Verdict | Why |
|---|---|---|
| C1 | finding | Hosts E4/E5; see L12-1..L12-7 |
| C1a | not applicable | Feature subsets change engines, not the protocol surface (`mcp.rs` is unconditional) |
| C2 | not applicable | demo_store speaks plain Candid; no protocol layer of its own. Its interface is consumed through B5-B7 (holds, below) |
| C3, C4, C8, C9, C10, C12, C13, C14 | not applicable | No protocol surface |
| C5 | holds | `GET /` and `GET /demo` (query string ignored) return 200 `text/html; charset=utf-8`, `cache-control: no-cache`; in certified mode 404 JSON (`mcp::demo_page_is_served_without_auth`, `certified_mode_...`) |
| C7 | holds | `ic-wasm metadata candid:service -v public` publishes the interface that `canister_metadata` fetches read (O2); `init_args "(null)"` matches `opt InitArgs` |
| C11 | holds | `.did` equals `export_candid!` (T2). `http_request_update` returns `HttpResponse` with an extra `upgrade : opt bool` versus the gateway's `HttpUpdateResponse`; an extra opt field is a Candid subtype, so the gateway decodes it (R-live: upgraded calls work in the baseline deployment) |
| C15 | not applicable | Not callable as messages |
| A1 | holds | Anonymous GET of the page works; anything else gets 404/405/401 JSON with `allow: POST` / `www-authenticate: Bearer` (R-live) |
| A2 | holds | Direct `http_request_update` callers go through the same `authenticate` + JSON-RPC handling as the gateway path (`mcp::http_update`) |
| A3 | finding | L12-1, L12-2, L12-3, L12-4, L12-5, L12-6 |
| A4 | finding | L12-3 (wrong-language run reported as success), L12-4 (truncated result as success), L12-7 (search annotations) |
| A5 | not applicable | Admin methods are plain Candid; no JSON-RPC |
| A6, A7 | holds | Replies decoded against declared return types with quotas (B6); oneway refused before sending; composite_query refused before sending (code, `bridge.rs:518`) |
| A8 | holds | Same handler as A2 |
| A9 | holds | Gateway honours `upgrade`, verifies skip-certification headers (R-live: every query response delivered with `ic-certificate` + `ic-certificateexpression`). Note: the local gateway answers `OPTIONS /mcp` itself with 200 and an empty body (R-live); the canister would answer 405 (R-native). Gateway behaviour, not canister code |
| A10, A11, A12 | not applicable | No protocol surface in scope |
| E1, E2 | holds | `opt InitArgs` decodes `(null)`; Candid record extension rules apply to `Limits` (optional fields) |
| E3 | holds | Decodes `HttpUpdateRequest` with the gateway's field set; extra fields would be ignored by Candid; refusal = no `accept_message` (the IC's documented mechanism) |
| E4 | holds | Query returns `upgrade = opt true`, status 200, empty body, no certificate headers for anything needing an update; otherwise adds `IC-Certificate` (`certificate`, `tree`, `expr_path`, `version=2`) and `IC-CertificateExpression`. Verified the fixed tree: CBOR decodes to `labeled("http_expr", labeled("<*>", labeled(H, leaf(""))))`, H = SHA-256 of `default_certification(ValidationArgs{no_certification:Empty{}})` = `c31abadb...b33723`, and the tree's root hash equals `SKIP_CERT_DATA` byte for byte (recomputed with the IC hash-tree domain separators). `expr_path` = CBOR `["http_expr","<*>"]` |
| E4a | holds | As C5 |
| E4b | finding | L12-1 (ids), L12-2 (oversized batch), L12-5 (query path ignores the five-search cap), L12-6 (error codes). Holds for: `initialize` (protocolVersion `2025-03-26` returned also when the client asks for `2024-11-05`, which the spec allows: server answers with a version it supports), `capabilities.tools`, `serverInfo`, `ping` -> `{}`, `tools/list` shape, unknown method -32601, parse error -32700 + HTTP 400 + id null, empty batch -32600 single object + HTTP 400, invalid batch items each -32600 with id null, invalid request with valid id echoes the id, notifications/responses only -> 202 empty body (also `notifications/cancelled`, client responses, and a `tools/call` without id, which is not executed), batch order preserved (R-native, R-live) |
| E5 | finding | Same handler as E4b: L12-1, L12-2, L12-3, L12-6. The update path does enforce the five-search cap (contrast L12-5) and replaces oversized tool results per message before the whole-reply fallback |
| E5a | finding | L12-3 (unvalidated arguments), L12-4 (truncation reported as success), L12-6 (-32600 for the one-script rule). Unknown tool / missing `code` / bad `language` -> -32602 (spec: unknown tools and invalid arguments are -32602) holds; script errors -> `isError: true` holds |
| E5b | finding | L12-3 (misspelled `query` silently ignored, returns everything), L12-5, L12-6, L12-7 |
| E5c | holds | `decode_reply` decodes against `func.rets` with skipping/decoding quotas; 0 returns -> null, 1 -> value, n -> array (T4) |
| E5d | not applicable | No protocol output |
| E6-E14 | not applicable | Plain Candid methods; guard errors use the CDK's reject mechanism |
| E15 | not applicable | Not messages |
| E16-E19 | holds | SDK argument conventions (null = no args, array = positional, bare value for one argument) follow B5; see B5 |
| E20-E23 | not applicable | demo_store business methods; no protocol beyond Candid |
| E24 | holds | `__get_candid_interface_tmp_hack : () -> (text) query` matches what `fetch_did` decodes (`candid::<String>()`) |
| E25-E28 | not applicable | No protocol surface |
| U1 | holds | Path compared after stripping `?`/`#` and trailing `/`; method compared case-insensitively (`post` accepted, R-native). RFC 9110 methods are case-sensitive, but the gateway normalises; no failure shown |
| U2 | holds | First `Authorization` header, scheme `Bearer` case-insensitive, token trimmed; missing/invalid -> 401 + `www-authenticate: Bearer` (R-live) |
| U3 | finding | L12-1 (number ids above u64 lose precision; `1e400` is valid JSON but rejected as -32700). Nesting over serde_json's 128 levels -> -32700 "recursion limit exceeded" (a documented server limit, holds) |
| U4 | finding | L12-1; id null/object/bool -> -32600 with id null (register #50 still fixed, R-live) |
| U5 | finding | L12-3 |
| U6, U7, U9, U11, U13-U16 | not applicable | Not protocol fields (covered by other lenses) |
| U8 | holds | See E5c |
| U10 | holds | `canister_metadata` reply decoded as `record { value : blob }`, then UTF-8 checked; fallback query decoded as `text` |
| U12 | holds | See E3 |
| O1 | holds | `Call::*_wait(..).with_raw_args(bytes)` where bytes come from `to_bytes_with_types(env, func.args)`; oneway and composite_query refused before the call (R-native for the mode detection; the refusal itself is code: `candid_bridge.rs:60`, `bridge.rs:518`) |
| O2 | holds | Management `canister_metadata` argument `record { canister_id; name = "candid:service" }` matches the management canister interface |
| O3-O5, O7-O10 | not applicable | No protocol output |
| O6 | finding | L12-2 |
| LIM6 | finding | L12-4 |
| LIM7 | finding | L12-2 (fallback object breaks batch shape and drops ids) |
| LIM8 | holds | 51 messages -> single -32600 object, HTTP 400 (T5 `replies_always_fit`) |
| LIM9 | finding | L12-5 |
| LIM10 | finding | L12-6 (enforced, but with -32600) |
| LIM11 | holds | Error messages truncated to 300 B (T5) |
| LIM20, LIM21 | holds | Applied in `decode_reply` / `idl_to_json` (T4) |
| LIM22 | holds | Integers above 2^53-1 become decimal strings (T4) |
| other LIM rows | not applicable | Not protocol limits |
| W1 | holds | Parse before the batch check is bounded by the ingress size; outcome conforms (single error object) |
| W2-W16 | not applicable | Cost rows (L3) |
| F6 | holds | `method.unwrap()` guarded by `valid` |
| other F rows | not applicable | Not protocol |
| B1 | finding | L12-1, L12-2, L12-6 |
| B2 | holds | See E3 |
| B3, B4, B8-B10, B12, B14, B17 | not applicable | Not protocol conversions (L7) |
| B5 | holds | Records by name / id / `_n_`, unknown keys refused, tuples from arrays with exact length, variants `{"Tag":v}` or `"Tag"`, opt null, blob hex; oneway refused (T4 `errors_are_descriptive`); class unwrapping and service-by-name both resolve methods (R-native: `service : (nat) -> {..}`, `service : (nat) -> S`, `service : S` all give the same methods and modes) |
| B6 | holds | Tuple records <-> arrays consistently with B5's `is_tuple`; variants single-key objects; nat64/int64 safe-range rule (T4). `opt opt` ambiguity is register #42 (accepted) |
| B7 | holds | `check_prog`, class unwrapped; `TypeEnv::as_service` also traces `Var` and `Class`, so `get_method` and `search::render` agree (R-native) |
| B11 | finding | L12-4. String results are passed raw, so `return "42"` and `return 42` produce identical `CallToolResult`s (R-native); MCP does not constrain text content, so this is recorded as a fidelity note for L7, not an L12 finding |
| B13 | finding | L12-3 (aliases and case-insensitivity accepted beyond the advertised enum is harmless; the defect is the missing check for unknown keys) |
| B15 | holds | See E4 (tree and root hash recomputed) |
| B16 | holds (note) | Search lists `oneway` and `composite_query` methods with their mode in the signature but does not say they are refused (R-native). Signature text is accurate; no spec violated |
| VF1-VF8 | not applicable | Not protocol |
| AD1-AD9 | not applicable | `certified_reads` effect on routing is covered under E4 (holds, T5) |
| S1-S10 | not applicable | L15 |
| CL9 | finding | L12-5 ("five searches per HTTP request" holds only on the update path) |
| CL14 | finding | L12-2 (the 1.9 MB fallback is "a small error", but it is a single object with id null even for batches) |
| CL21, CL28 | holds | Certified mode upgrades every authenticated request; script results come from the update call (T5, code) |
| CL3 | finding | L12-7 (protocol-level annotation; the description text itself is L16's) |
| other CL rows | not applicable | Not protocol claims |
| T4, T5 | holds (gaps) | No test covers: id precision, oversized batch shape, query-path search cap, unknown tool arguments, truncation `isError`, the certificate tree constant, composite_query refusal |
| Register #50 | holds (still fixed) | null / object / bool ids -> -32600 with id null (R-live, `mcp::ids_must_be_string_or_number`) |

## Candidate findings

- **L12-1** (Low, R-native + R-live) Numeric request ids are not echoed exactly: `18446744073709551616` -> `1.8446744073709552e+19`, `123456789012345678901234567890` -> `1.2345678901234568e+29`, `-0` -> `-0.0`, `1e2` -> `100.0`; `1e400` -> whole body refused as -32700. Cause: `serde_json::Value` without `arbitrary_precision`.
- **L12-2** (Low, R-native + R-live) When a reply exceeds 1.9 MB, `finish` replaces it with one error object with `id: null`, even for a batch (JSON-RPC requires an array of responses with the request ids). Live: 50 pings with 40 KB string ids -> `{"error":{"code":-32603,...},"id":null}`.
- **L12-3** (Medium, R-native) Tool arguments are not validated against the advertised `inputSchema` (`additionalProperties: false`). `{"code": "print(1+1)\n1+1", "lang": "python"}` runs as JavaScript and returns `null` with `isError: false`; `{"querry": "balance"}` returns the unfiltered search.
- **L12-4** (Medium, R-native) A result over 1 MB escaped is cut mid-value, suffixed `…[truncated]`, and returned with `isError: false`; the text no longer parses as JSON.
- **L12-5** (Medium, R-native + R-live) The query path does not enforce `MAX_SEARCHES`: 7 cached searches in one batch all succeed on the query path (live), while the update path answers 5 and refuses 2. README says five per HTTP request.
- **L12-6** (Low, R-native) Server policy refusals (second `execute_icp_script`, sixth search, batch over 50) use -32600 "Invalid Request", which JSON-RPC defines as "not a valid Request object"; server-defined errors belong in -32000..-32099. Documented in README:368.
- **L12-7** (Low, code) `search_icp_canisters` is annotated `readOnlyHint: true` / `idempotentHint: true`, but on a cache miss it runs as an update that sends `canister_metadata` / `__get_candid_interface_tmp_hack` messages, takes call-cap slots and appends audit entries (`src/lib.rs:121-190`). Same root cause as CL3 if L16 reports it.
