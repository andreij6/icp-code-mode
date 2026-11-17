# Grid: L12 Protocol conformance, round 3 (Streamable HTTP transport rules)

Run `2026-11-17-295d64b`. Scope: the MCP 2025-03-26 Streamable HTTP transport rules that the
round 1 and 2 grids left without a cell: Origin validation (DNS rebinding), `GET /mcp` (SSE),
the `Accept` header, and `Mcp-Session-Id`. Spec: MCP 2025-03-26, Basic / Transports /
Streamable HTTP (the version the server advertises, `src/mcp.rs:16`). L10 graded CORS only.

Evidence: **R-live** = reproduced against the local icp-cli network gateway
`http://icp_code_mode.local.localhost:8010/mcp` (canister `4caro-hl777-77775-aaaba-cai`) with
query-path requests only (`initialize`, `ping`, `tools/list`, a notification; no script run, no
setting changed, no audit write). Token read from `.env` (`MCP_TOKEN`) into a shell variable,
never printed. **code** = the code path produces it unambiguously. No native build was needed:
`mcp::authenticate` (`src/mcp.rs:266-290`) is the only reader of request headers, and it reads
only `Authorization`.

Candidate findings: **L12r3-1** (described at the end).

## Observed behaviour (R-live)

| # | Request | Response | Spec rule (2025-03-26) | Grade |
|---|---|---|---|---|
| A | `POST /mcp` `initialize`, valid token, `Origin: http://evil.example` | 200 `application/json`, full `InitializeResult`, `access-control-allow-origin: *` | "Servers MUST validate the Origin header on all incoming connections to prevent DNS rebinding attacks" | **violated (L12r3-1)** |
| B | `POST /mcp` `tools/list`, valid token, `Origin: http://attacker.test:8010` | 200, 3565 B tool list | same | violated (L12r3-1) |
| C | `POST /mcp` `ping`, valid token, `Origin: null` | 200 `{"id":3,"jsonrpc":"2.0","result":{}}` | same | violated (L12r3-1) |
| D | foreign Origin, no token | 401 `www-authenticate: Bearer` | authentication SHOULD | holds |
| E-H | DNS-rebinding shape: request to `127.0.0.1:8010` with `Host: attacker.test:8010` (with and without `?canisterId=4caro-...`, with and without token, `POST /mcp` and `GET /`) | 400 `client_unknown_domain` from the gateway; the canister is never reached | (threat the Origin rule exists for) | mitigated by the gateway (A9), not by the canister |
| I | `GET /mcp`, token, `Accept: text/event-stream` | 405, `allow: POST`, JSON body "only POST is supported (no SSE stream)" | "The server MUST either return Content-Type: text/event-stream ... or else return HTTP 405 Method Not Allowed" | holds |
| J | `GET /mcp`, no token | 405 (method checked before the token, `src/mcp.rs:271`) | same | holds |
| K | `GET /mcp` with `Last-Event-ID: 1` | 405 | resumability is optional (MAY) | holds |
| L | `DELETE /mcp` with `Mcp-Session-Id: abc` | 405, `allow: POST` | "The server MAY respond to this request with HTTP 405" | holds |
| M | `POST` `ping`, no `Accept` header | 200 `application/json` | client MUST send `Accept: application/json, text/event-stream`; the spec sets no server duty on a missing one | holds (lenient) |
| N | `POST` `ping`, `Accept: text/event-stream` only | 200 `application/json` | server "MUST either return text/event-stream ... or application/json"; RFC 9110 lets a server ignore `Accept` instead of sending 406 | holds (lenient; a conforming client always lists both) |
| O | `POST` `ping`, `Accept: text/html` | 200 `application/json` | as N | holds (lenient) |
| P | `POST` `ping`, `Content-Type: text/plain` | 200 | no server rule; without a token a CORS-simple request gets 401, so no CSRF | holds |
| Q | `POST` `ping` with an unknown `Mcp-Session-Id` | 200 | the 404-for-unknown-session rule applies only to session ids the server issued; none are issued | holds |
| R | `initialize` response headers | no `Mcp-Session-Id` | session id is MAY | holds (stateless server) |
| S | notification only | 202, empty body, no content type | "MUST return HTTP status code 202 Accepted with no body" | holds |
| T | `OPTIONS /mcp` preflight, `Origin: http://evil.example`, requesting `authorization,content-type,mcp-session-id` | 200 from the gateway, `access-control-allow-origin: *`, `access-control-allow-headers` list without `authorization` or `mcp-session-id` | no MCP rule (CORS) | observation (A9), not a finding |

## Rows

| Row | Verdict | Why |
|---|---|---|
| E4b | **finding L12r3-1** | No Origin check anywhere: `authenticate` reads only `Authorization`, `http_query` and `http_update` read no other header (code; R-live A-C). Holds for the rest of the transport section: `GET /mcp` 405 with `allow: POST` (I-K), `DELETE` 405 (L), 202 for notification-only input (S), `application/json` replies to requests, no session id issued and an unsolicited one ignored (Q, R), `Accept` not enforced (M-O, lenient but allowed) |
| E4 (query, both modes) | finding L12r3-1 (same) | `certified_reads` changes nothing here: `authenticate` runs before the upgrade decision (`src/mcp.rs:347-352`), so a foreign Origin with a token is upgraded and a `GET /mcp` gets the same 405 (code) |
| E5 | finding L12r3-1 (same) | `http_update` calls the same `authenticate` (`src/mcp.rs:375`); no header but `Authorization` is read (code; not exercised live because only a script or uncached search reaches it, and that writes audit entries) |
| U1 | holds | Method and path: `GET`/`DELETE`/`OPTIONS` (when the canister sees them) -> 405 before auth; `POST /mcp` only; covered for case and trailing slash in round 1 |
| U2 | **finding L12r3-1** (Origin); holds for the rest | `Origin` ignored (finding). `Accept`, `Content-Type`, `Mcp-Session-Id`, `Last-Event-ID` and `MCP-Protocol-Version` are ignored; for 2025-03-26 none of these is a server MUST (M-R). `Authorization` handling as round 1 |
| C5 | holds | The demo page posts to relative `/mcp` from the canister's own origin, so it would pass any Origin allowlist that includes the canister's gateway origins. Note for the fix: the page has several valid origins (`<id>.icp0.io`, `<id>.raw.icp0.io`, `ic0.app` forms, custom domains, `*.localhost:8010` locally), so an allowlist must be derived, not hard-coded to one |
| A9 | holds (with note) | The local gateway refuses any `Host` it does not serve (`400 client_unknown_domain`, E-H), which is the defence that actually stops DNS rebinding: a rebound name never reaches the canister, so the missing Origin check has no demonstrated effect. The gateway also answers CORS preflights itself, and its `access-control-allow-headers` omits `authorization` (T): per the Fetch standard a browser page on another origin cannot send the bearer token at all, so cross-origin browser MCP clients cannot use this server, and L10's C5 CORS cell is stronger than it states. Both are gateway behaviour, observed on the local gateway only |

## Candidate findings

- **L12r3-1** (Low, R-live + code) **The server never validates the `Origin` header, which
  MCP 2025-03-26 Streamable HTTP makes a server MUST.** `mcp::authenticate`
  (`src/mcp.rs:266-290`) reads only `Authorization`; `http_query` (`:340-370`) and `http_update`
  (`:374-412`) read no other header. Live: `initialize`, `tools/list` and `ping` with a valid token
  and `Origin: http://evil.example`, `http://attacker.test:8010` or `null` all return 200 and the
  full result. The README claims "Model Context Protocol (2025-03-26, Streamable HTTP via the ICP
  HTTP gateway)" (README.md:8-9) and lists no deviation. Impact not demonstrated: the DNS-rebinding
  attack the rule targets is refused by the gateway's Host check before the canister (live E-H),
  and auth is a bearer header with no cookie or network-position trust, so a rebound or foreign
  page gains nothing without the token. Graded Low as in the precedent L12-6 (spec divergence with
  no demonstrated harm); a literal reading of the severity table ("differs from a specification")
  would give Medium. Refuted attempts: tried to reach the canister under a rebound Host with and
  without `?canisterId=` (gateway 400); tried a token-less foreign Origin (401). Fix: if an `Origin`
  header is present and not in an allowed set (the canister's own gateway origins plus an
  operator-configurable list), answer 403 before authentication, in `authenticate` so both routes
  get it; absent Origin (non-browser clients, direct canister calls) stays allowed. Or, if the
  operator relies on the gateway, state the deviation in the README. Regression test: `http_query`
  and `http_update` with a foreign `Origin` and a valid token return 403; same-origin and absent
  Origin return 200.
  Pattern: a transport-level MUST of the advertised protocol version delegated implicitly to
  infrastructure, with no check or statement in the server.

## Not tested

- Mainnet gateway (`icp0.io`, custom domains): whether it rejects unknown Hosts and answers
  preflights with the same `allow-headers` list. Local gateway only.
- Browser enforcement of the preflight result (T) was not run in a browser; the conclusion
  follows from the header list and the Fetch standard.
- Origin on the update route live (needs a script or an uncached search, which writes audit
  entries); the code path is shared.
- Whether the gateway forwards `Origin` to the canister at all (the canister has no echo); it does
  not matter for the verdict, since the canister code reads no such header.
