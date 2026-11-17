# Grid: L1 (access control), round 2: identifiers compared in the caller's spelling

Scope: register class #1 swept beyond principals. Question per row: is any spelling accepted
on one route (query `http_request` E4, update `http_request_update` E5, `inspect_message` E3)
and refused or read differently on another?

Live probes ran against the local icp-cli network (port 8010, gateway
`icp_code_mode.local.localhost`), read-only except one trivial Rhai script (`40+2`, one
`Ok` audit entry, 0 calls) and two direct `http_request_update` ingress calls with a `ping`
body (one returned 200, one trapped on decode; neither changes state). No setting was
changed, nothing was deployed. Token read from `.env` (MCP_TOKEN line) and never printed.

| Row | Cell | Evidence |
|---|---|---|
| U1 path, query vs update | **holds** | One function, `mcp::authenticate` (`src/mcp.rs:266-270`), gates E4, E5 and E3. Gateway forwards the raw path unchanged (no decoding, no `//` or dot-segment folding). Live, POST without token: `/mcp`, `/mcp/`, `/mcp//`, `/mcp?a=1` -> 401; `/MCP`, `/Mcp`, `//mcp`, `/./mcp`, `/x/../mcp`, `/%6Dcp`, `/%6dcp`, `/mcp%2F`, `/mcp;x`, `/mcp%3Fa`, `/mcp%20` -> 404. Through the upgrade: POST `/mcp//?x=1` with a `tools/call` reached the update and returned the script result, so the update call saw the same URL. No spelling reaches any handler without the token check |
| U1 path, demo route | **holds** (cosmetic asymmetry, no access effect) | `demo_page` (`src/mcp.rs:323-330`) matches `/` and `/demo` exactly, while `authenticate` folds trailing slashes: `/demo/` -> 404, `/mcp/` -> accepted. Live: `/`, `/demo`, `/?x` -> 200; `/demo/`, `/DEMO`, `//demo`, `/%64emo` -> 404. The page is unauthenticated and holds no secret; the update path never serves it; certified mode 404s both spellings it matches. No spelling serves the page when it should be off |
| U1 HTTP method | **holds** | `eq_ignore_ascii_case` for POST (`authenticate`) and GET (`demo_page`), same function on all three entry points |
| U2 `Authorization` header, query vs update vs inspect_message | **holds** | `authenticate` takes the first header whose name equals `authorization` ignoring case; scheme `bearer` ignoring case; token trimmed. Same function in E3/E4/E5 on the same decoded header list. Live (query): bad-then-valid -> 401, valid-then-bad -> 200, valid / `X-A` / lowercase bad -> 200, lowercase bad / `X-A` / valid -> 401, empty `Authorization;` then valid -> 401, comma-merged -> 401, tab separator -> 401, two spaces -> 200, trailing space -> 200. Live (update): valid then `X-A` then lowercase bad, through an upgrade, ran the script: the gateway kept the header order between the query and the update call. No ordering or merge difference observed |
| U4 JSON-RPC `method`, `jsonrpc` | **holds** | Exact comparison in `classify` (`src/mcp.rs:240, 256-261`), same parser on both routes; live `Tools/Call` -> -32601 in the query |
| U5 `params.name` (tool name) | **holds** | Exact match in `parse_tool_call` (`src/mcp.rs:213-226`), same function on both routes. Live: `Execute_ICP_Script` and `search_icp_canisters ` -> -32602 `unknown tool` in the query, never upgraded. Duplicate JSON keys: serde_json keeps the last on both routes (same parser; the gateway does not parse the body) |
| U5 `arguments.language` / B13 | **holds** | `Language::from_str` (`src/runtimes/mod.rs:34-44`) lower-cases and maps `js`/`py`; the audit entry stores `Language::as_str()` (`src/lib.rs:224, 250`), the canonical name. Live: `language: "RHAI"` ran, newest audit entry has `language = "rhai"`. `" js"` is refused on both routes (no trim; consistent). Lenient beyond the schema enum but never misread |
| U5 `arguments.canister_id` / B4 search vs bridge | **finding** (low, reproduced) | `search::scope` (`src/search.rs:39`) parses `id.trim()`; `bridge::parse_principal` (`src/bridge.rs:464-468`) parses the text untrimmed. Same id accepted by search, refused by `icp.call`/`icp.candid`. Case canonicalisation is consistent on both (register #1). See candidate 1 |
| B4 principal canonicalisation in policy, caches, overrides | **holds** | `check_call`/`check_interface_fetch` look up by `Principal` (`src/policy.rs:193-203`); bridge keys `p.to_text()` after parsing (`src/bridge.rs:472-474, 440`); overrides stored/read with `to_text()`/`to_string()` (same Display) (`src/policy.rs:234-241`); `evict_interface` uses `to_text()` (`src/bridge.rs:270-274`); `run_search` passes `canister.to_text()` (`src/lib.rs:137`). The identifier checked is the one sent |
| B4 method names | **holds** | Allowlist compares `m == method` exactly (`src/policy.rs:177`) and the same `method` string is encoded and sent (`src/bridge.rs:510-517`). Candid method names are case-sensitive, so exact comparison is the canonical form |
| V2 `DID_CACHE` key | **holds** | Written and read only with `Principal::to_text()` (`src/bridge.rs:208, 287, 474, 499`) |
| V4 `DID_FAILED` key | **holds** | Same canonical key (`src/bridge.rs:279, 498, 504`) |
| V5 `OVERRIDES` key | **holds** | Same canonical key, from `cached_service` (`src/bridge.rs:208-216`) and policy override keyed `to_text()` |
| E3 `inspect_message` vs `http_request_update` decode | **finding** (low, reproduced) | E3 decodes with `candid::decode_one` (no skipping quota, `src/lib.rs:276`); the method decodes with ic-cdk's `skipping_quota(10000)` (ic-cdk-macros-0.20.3 `src/export.rs:258-260`). See candidate 2 |
| Tokens (D-row, hashing) | **holds** | `add_token_hash`/`revoke_token_hash` trim and lower-case the hex; `verify_token` hashes to lower-case hex (`src/policy.rs:208-227`) |

Not applicable: E6-E14 identifiers (Candid `principal` arguments are already canonical on the wire; covered by register #1 and round 1).
