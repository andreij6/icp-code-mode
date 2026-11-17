# Grid: L12 Protocol conformance, round 4 (protocol revisions after 2025-03-26; gateway caching)

Run `2026-11-17-295d64b`. Rows: E4, E4b, U4, B11, A9, CL28, LIM8.

Scope: rounds 1-3 graded against MCP 2025-03-26 only. This round grades the same rows against the
later revisions current clients speak, and answers the open question from L10 r1 and sweep-r1-0
(do the IC HTTP gateway or the API boundary nodes cache `POST /mcp` query responses across callers?).

Specifications read (modelcontextprotocol.io, 2026-11-17): 2025-03-26 Basic (batching),
2025-06-18 Transports (Protocol Version Header), versioning page (current revision is
**2026-07-28**, a stateless, per-request-versioned revision with no `initialize` handshake),
2026-07-28 Versioning (era model, compatibility matrix) and Streamable HTTP (request metadata
headers, server validation, backward compatibility).

Gateway sources read (GitHub, default branches as of 2026-11-17; the versions running on mainnet
were not identified): `dfinity/ic-gateway` `src/routing/mod.rs`, `src/routing/middleware/cache.rs`,
`src/cli.rs`; `dfinity/ic-bn-lib` `ic-bn-lib/src/http/cache.rs`; `dfinity/ic`
`rs/boundary_node/ic_boundary/src/{core.rs,cli.rs,routes.rs,http/middleware/cache.rs}`.

Evidence: **R-live** = local icp-cli network gateway `http://icp_code_mode.local.localhost:8010/mcp`,
query-path requests only (`initialize`, `ping`, `tools/list`, `server/discover`, ping batches), token
read from `.env` into a shell variable and never printed, no setting changed, no audit write.
**code** = the code path produces it unambiguously. **source** = read in the published gateway source.
No native build was needed: the only header reader is `mcp::authenticate` (`src/mcp.rs:266-290`),
which reads only `Authorization`, and nothing in `src/` reads `_meta` or a protocol-version header
(grep).

## Observations (R-live)

| # | Request | Response |
|---|---|---|
| 1 | `initialize` asking `2025-06-18`, `2025-11-25`, `2026-07-28`, `garbage`, and with no `params` | 200, `protocolVersion: "2025-03-26"` every time |
| 2 | `ping` with `MCP-Protocol-Version: garbage` / `1900-01-01` | 200 `{"result":{}}` |
| 3 | modern `server/discover` (header `MCP-Protocol-Version: 2026-07-28`, `Mcp-Method`, `_meta` protocolVersion) | **200** `{"error":{"code":-32601,"message":"method not found: server/discover"}}` |
| 4 | modern `tools/list` (same headers and `_meta`) | **200**, full tool list |
| 5 | header/body mismatch (`Mcp-Method: ping`, body `tools/list`, `MCP-Protocol-Version: 2025-06-18`) | 200, tool list |
| 6 | batches of 49 / 50 / 51 pings with `MCP-Protocol-Version: 2025-06-18` | 200 array of 49, 200 array of 50, 400 single `-32600` with `id: null` |
| 7 | `tools/list` alternated 3x: valid token, wrong token, no token | 200 3565 B / 401 / 401 every round (no cross-Authorization reuse on the local gateway) |
| 8 | Query `POST /mcp` response headers | `content-type`, `ic-certificate`, `ic-certificateexpression`, CORS, `x-ic-canister-id`, HSTS; **no `cache-control`**, no `age`/cache-status header |
| 9 | `GET /` twice | 200 with `cache-control: no-cache` both times |

## Gateway caching (the open question from L10 r1 / sweep-r1-0)

**ic-gateway (HTTP gateway, `*.icp0.io`)** (source):
- The response cache is built only when `--cache-size` is set, with `CacheBuilder::new(KeyExtractorUriRange)`
  and no `.methods(..)` call (`src/routing/mod.rs`). `ic-bn-lib` `Opts::default()` sets
  `methods: vec![Method::GET]`, and `process_inner` bypasses any other method with
  `CacheBypassReason::MethodNotCacheable`. **`POST /mcp` is never cached.**
- The key is authority + path-and-query + `Range`; `Authorization` is not in the key, so a cached GET
  is served to every caller. The canister's only 2xx GET is the demo page (`GET /`, `/demo`): public,
  identical for everyone, and sent with `cache-control: no-cache`, which the cache obeys by default
  (`--cache-disregard-cache-control` off) as a bypass. `GET /mcp` is 405 and non-2xx responses are
  not cached. Worst case with an operator who disregards cache-control: the public page is served up
  to `--cache-ttl` (10 s default) after `certified_reads` is switched on.
- The cache layer wraps only the HTTP-to-canister fallback route.

**ic-boundary (API boundary node, `/api/v2|v3/canister/<id>/query`)** (source):
- Caches POST query calls only when `cache_size` is set, bypassing non-query calls, calls with a
  nonce and non-anonymous senders (`http/middleware/cache.rs`). The HTTP gateway's `http_request`
  queries are anonymous, so they are eligible.
- The key is `RequestContext` (manual `Hash`/`PartialEq` in `routes.rs`): canister id, sender, method
  name, `ingress_expiry`, and, for `http_request`, the decoded `HttpRequest { method, url, headers,
  body }` (derived `Hash`/`PartialEq`, headers included). **`Authorization` is part of the key**, so a
  response computed for one token is never served to a request carrying another token or none.
- TTL `cache_ttl`, default 1 s. Effect: an identical request (same token, body and rounded expiry)
  within the TTL gets the earlier answer, so a token revocation or allowlist change can lag by up to
  the configured TTL on the query path. Not a cross-caller leak.

Verdict: **holds**. A response cached for one `Authorization` cannot be served to another caller by
either gateway layer. The missing `cache-control` header on `POST /mcp` responses has no effect,
because ic-gateway never caches POST and ic-boundary keys on the full request including headers
(RFC 9111 also makes a POST response unreusable for a later request without explicit freshness and
`Content-Location`). Not verified: the deployed mainnet versions and their flags (the default branch
was read); a third-party gateway or custom-domain proxy in front of the canister.

## Rows

| Row | Verdict | Why |
|---|---|---|
| E4 `http_request` (query) | holds | Gateway caching: see above (source; R-live #7, #8). Version handling on this route is E4b's |
| E4b `POST /mcp` JSON-RPC | **finding L12r4-1**, **finding L12r4-2** | L12r4-1: no protocol-version gate (R-live #2-#5). L12r4-2: search annotations beyond `readOnlyHint` (extension of L12-7). Holds: `initialize` answers `2025-03-26` to any requested version (#1), which every handshake revision 2024-11-05..2025-11-25 allows ("respond with another protocol version it supports"); clients that support 2025-03-26 then send `MCP-Protocol-Version: 2025-03-26`, which is ignored harmlessly. Batches: 2025-03-26 says servers "MUST support receiving JSON-RPC batches", and 2025-03-26 is the only version the server ever negotiates, so accepting batches is required, and the removal of batching in 2025-06-18 does not make L12-2 (oversized-batch `id: null` fallback) moot. `structuredContent`/`outputSchema`: no tool declares `outputSchema`, so 2025-06-18's "MUST return structuredContent" rule cannot trigger, and 2025-03-26 has neither field (not applicable). 2025-11-25's "input validation errors as tool errors" is a SHOULD in a revision the server does not negotiate (its `-32602` for bad arguments conforms to 2025-03-26) |
| U4 JSON-RPC `id`, `method`, `params` | finding L12r4-1 (same root); holds otherwise | `params._meta` (where 2026-07-28 puts `io.modelcontextprotocol/protocolVersion`) is never read (code; R-live #3-#4). `initialize` params are not validated: no params at all still gives a full `InitializeResult` (#1). Lenient, and no rule requires a refusal. Id rules did not change in later revisions; L12-1 (precision) stays as reported |
| B11 script result -> `CallToolResult` | holds (for the rules in scope) | 2025-06-18 `structuredContent` is required only when the tool has an `outputSchema`; none has one, so text-only content conforms. `isError` next to `structuredContent` is not applicable (no structured content). Known issues stay as reported: L12-4 (truncated result with `isError: false`), the string-vs-JSON ambiguity (L7) |
| A9 gateway / boundary nodes | holds | See "Gateway caching". The local gateway behaves the same (R-live #7). The local CORS preflight list lacks `authorization` (L12r3 A9 note); it lacks `mcp-protocol-version` too, which matters only for browser clients, and those are already blocked by the missing `authorization` |
| CL28 skip-certification claim | holds | R-live #8: query replies carry `ic-certificate` + `ic-certificateexpression: default_certification(ValidationArgs{no_certification:Empty{}})`; scripts always upgrade (`src/mcp.rs:366`). Gateway caching does not widen it (above). The fact that one node can alter skip-certified replies is L10-1 (known) |
| LIM8 `MAX_BATCH` | holds | Boundary test (R-live #6): 49 and 50 -> 200 arrays of the same length; 51 -> 400 single `-32600` `id: null`. Valid under 2025-03-26 as a server limit; the error code choice is L12-6 (known) |

## Candidate findings

- **L12r4-1** (Low; R-live + code) **The server never reads the protocol version a request
  declares, so it cannot refuse a version it does not implement, and it answers requests in the
  current (2026-07-28) shape with 200 under 2025-03-26 rules.** Nothing in `src/` reads the
  `MCP-Protocol-Version` header or `params._meta["io.modelcontextprotocol/protocolVersion"]`;
  `authenticate` (`src/mcp.rs:266-290`) reads only `Authorization`, and `classify`
  (`src/mcp.rs:236-263`) dispatches on `method` alone. Live: `ping` with `MCP-Protocol-Version:
  garbage` or `1900-01-01` -> 200; a modern `tools/list` (header `2026-07-28`, `_meta`) -> 200 with
  the tool list; a modern `server/discover` -> **HTTP 200** with `-32601`.
  Against what spec: for the revision it advertises (2025-03-26) nothing here is violated; the
  "MUST respond with 400 Bad Request" for an unsupported `MCP-Protocol-Version` exists from
  2025-06-18. It matters because of how current clients detect old servers: the 2026-07-28
  backward-compatibility section has a dual-era HTTP client "attempt a modern request first" and
  fall back to `initialize` on a 4xx (on 400, after checking the body is not a modern error). The
  compatibility matrix lists Dual-era x Legacy as working because the legacy server answers the
  modern request with a 4xx. Every handshake server from 2025-06-18 on does that through the
  version-header rule. This server answers 200 instead, so a dual-era client never falls back. It
  keeps speaking 2026-07-28 to a server that implements none of it (no `server/discover`, no
  `UnsupportedProtocolVersionError`, no header/body validation: R-live #5). The matrix row Modern x
  Legacy describes this outcome ("process an era-ambiguous method under legacy semantics"). The
  server also answers `server/discover` with `-32601` on HTTP 200: the modern contract for an
  unknown method is 404 + `-32601`, and the legacy-fallback signal is a 4xx without a modern error
  body, so this answer is neither.
  Impact not demonstrated: the server is stateless, so `tools/list` and `tools/call` happen to work
  when sent in the modern shape; no real 2026-07-28 or dual-era client was run. Graded Low
  (interop hardening, no demonstrated failure); a verifier who reads "differs from a specification"
  against 2025-06-18+ would give Medium, but the server does not claim those revisions.
  Refutation tried: the README and `PROTOCOL_VERSION` claim only 2025-03-26 (README.md:8;
  `src/mcp.rs:16`), so no "honest interface" claim is broken; the demo page sends no version header
  (`src/demo.html:221, 367`), so a gate would not break it.
  Fix: in `authenticate` (both routes), read `MCP-Protocol-Version` and, when present, the `_meta`
  protocol version. If either is present and not `2025-03-26`, answer 400 with a JSON-RPC error that
  is not a recognised modern error (for example `-32600 "unsupported protocol version X; this server
  speaks 2025-03-26 (initialize first)"`, `id: null`). When both are absent, assume 2025-03-26, as
  2025-06-18 and 2026-07-28 allow. Optionally answer `initialize` with an explicit error when the
  requested version is not a handshake revision. Regression test: `http_query`/`http_update` with
  header `2026-07-28` -> 400 non-modern body; header `2025-03-26` or absent -> 200; a modern
  `server/discover` -> 400.
  Pattern: version negotiation implemented only in the handshake reply, with nothing on the request
  path that enforces the version agreed.

- **L12r4-2** (Low; code; extension of L12-7, same root, merge) **`idempotentHint: true` and
  `openWorldHint: false` on `search_icp_canisters` also describe only the cache-hit path.**
  `src/mcp.rs:115`. On a miss, `run_search` (`src/lib.rs:121-190`) sends `canister_metadata` and
  `__get_candid_interface_tmp_hack` messages to third-party canisters through `bridge.candid` and
  appends an audit entry. The positive cache expires after 1 h (`DID_CACHE_TTL_NS`,
  `src/bridge.rs:180`) and the failure cache after 5 min (`DID_FAIL_TTL_NS`, `:183`), so repeating
  the same search after those windows sends new messages and writes new audit entries: each repeat
  has an additional effect, which `idempotentHint` denies. `openWorldHint: false` says "domain of
  interaction is closed". But the miss path messages external canisters, and the result embeds
  interface text written by those canisters' controllers (method and type names; the
  prompt-injection side is already reported for `src/search.rs:84-159`). Per 2025-03-26
  `ToolAnnotations`, `idempotentHint` is meaningful only when `readOnlyHint` is false, so this
  stands only together with L12-7. Holds for `execute_icp_script`: it declares no annotations, so
  clients apply the defaults `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: false`
  and `openWorldHint: true`, all accurate for a tool that can call allowlisted update methods.
  Fix: as L12-7. Either make the annotations describe the worst path (`readOnlyHint: false`,
  `idempotentHint: false`, `openWorldHint: true`), or make search never fetch.

## Phase 4 techniques applied

- Boundary: LIM8 at 49/50/51 live (#6).
- Differential: the same body with valid / wrong / no token, alternated, to catch reuse by a gateway
  cache (#7); `initialize` across five requested versions (#1).
- Fuzz of the version header: not needed beyond #2/#5, because the code reads no such header (code).

## Not tested

- A real 2026-07-28 or dual-era MCP client (SDK) against the server, so the client-side
  consequence of L12r4-1 is inferred from the spec's compatibility rules, not observed.
- Mainnet ic-gateway / ic-boundary versions and flags (`--cache-size`, `--cache-ttl`,
  `--cache-disregard-cache-control`): read from the default branch; which release mainnet runs and
  with what flags is not public in the repos read. Third-party gateways or custom-domain proxies
  (for example a CDN in front of a custom domain) are outside this source and would need their own
  check; the canister sends no `cache-control: no-store, private` on `POST /mcp` to protect against
  such a proxy (no RFC-conforming cache reuses a POST response without explicit freshness, so this
  is noted, not graded).
- The replica's own query cache was not read; its key is the full query payload, so it is not
  expected to differ from ic-boundary's (unverified).
- Update route (`http_request_update`) live for the version probes: it needs a script or uncached
  search (audit writes); the code path (`authenticate` + `classify`) is shared.
