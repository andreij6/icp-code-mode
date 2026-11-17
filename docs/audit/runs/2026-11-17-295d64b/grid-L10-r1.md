# Coverage grid: L10 Trust and transport, round 1

Run `2026-11-17-295d64b`. Lens question: who can read or alter this in transit or at rest?
Evidence: **R** = reproduced (command or test given), **C** = code path. Candidate findings are
numbered L10-1 to L10-4 (details in the lens report returned to the workflow).

## Live observations used below (read-only, local network on port 8010)

- `curl -D -` on `GET /`, `POST /mcp` (no token, 401) and `GET /anything/else` (404): all three
  carry the same `IC-Certificate` `tree=` and `expr_path=` (`http_expr/<*>`, expression
  `no_certification`). Decoding the subnet-signed certificate gives
  `certified_data = [85, 236, 195, 28, ...]`, which is `SKIP_CERT_DATA` (`src/lib.rs:55`) and
  equals `ic_http_certification::utils::skip_certification_certified_data()` (the crate's own
  test, `ic-http-certification-3.2.0/src/utils/skip_certification.rs:96-105`).
- The local gateway serves the demo page on the certified (non-raw) domain with status 200, so it
  accepts skip-certified responses.
- `certified_reads` was not toggled (not the Live phase); its effect is read from the code.

## Grid

| Row | Cell | Entry | Reason / evidence |
|---|---|---|---|
| A9 | infrastructure reading requests | holds as documented (register #48) | Bearer token travels in the `Authorization` header to the boundary node and the replica(s); README.md:459-462 states it. Not new |
| A9 | single node altering responses | **finding L10-1** | Certified data is the all-paths skip hash at all times, so one replica can return any response for any path and the gateway accepts it, with or without `certified_reads` (C + live certified data) |
| A4 | content the AI model reads (tools/list, initialize.instructions) | **finding L10-1** (symptom) | Static text, but served from the query path with skip certification; alterable by one node even in certified mode |
| A4 | search results and interface text | **finding L10-4** | Target-authored text (quoted Candid labels, reject/error text, `icp.candid` text) reaches the model unmarked (R: unit probe) |
| E4 | `http_request` query, IC-Certificate headers | **finding L10-1** | `src/lib.rs:82-94` adds skip headers to every non-upgrade response; `src/lib.rs:64,70` set certified data once, independent of `certified_reads` |
| E4a | `GET /`, `GET /demo` | **finding L10-1** (symptom) | With `certified_reads` the canister answers 404 (`src/mcp.rs:341-344`), but the certificate still says "skip" for `<*>`, so a node can serve any page, including a token-phishing one, on the canister's origin |
| E4b | query-answered JSON-RPC, upgrade flag | **finding L10-1** (symptom) | The `upgrade: true` decision travels in the uncertified query response; a node can answer `tools/call execute_icp_script` with a fabricated result instead of upgrading. Contradicts CL28 "Script results always come from the update call" |
| E5 | `http_request_update` responses | holds | Update responses reach the gateway through consensus (read_state certificate); not alterable by one node. Only reached if the query response says upgrade (see L10-1) |
| B15 | IC-Certificate header construction | holds (format), **finding L10-1** (semantics) | Header bytes match `add_skip_certification_header` exactly (tree, expr_path, version 2, `IC-CertificateExpression` name matches the crate constant). The defect is that it is the only certification state the canister ever has |
| LIM4 | `certified_reads` | **finding L10-1** | Boundary: `None`/`false`/`true` handled by `== Some(true)` (`src/lib.rs:79`) and tested in `mcp::certified_mode_...`; but toggling it changes only what the honest canister returns, never `certified_data`. The register #47 guard cannot detect this (tests the honest function, not the certificate) |
| D3 | token hashes at rest | holds | Unsalted SHA-256 is readable by node operators from stable memory, but README tokens are `openssl rand -hex 32` (256 bits), so the hash is not invertible; plaintext is exposed in transit anyway (#48). `verify_token` hashes first, so lookup timing depends on the hash, not the token |
| B3 | caller id = first 16 hex of token hash | holds | 64-bit prefix: accidental collision needs ~2^32 tokens; a crafted collision needs a 2^64 preimage search, and only controllers register tokens. Shown only in the controller-only audit log and `get_policy` |
| D9 | demo token in browser localStorage | **finding L10-2** | Written on every request and reloaded into the field on every visit (`src/demo.html:207-213`), never cleared; any script later running on the origin (L10-1 page) reads it without user action (C) |
| D10 | `.env` with `MCP_TOKEN` | holds (repo); n/a (host hygiene) | `.env` is gitignored (`.gitignore:3`), absent from tracked files and from history (searched for the token and its hash by count, not printed). File mode 0644 on this machine is the developer's environment, not project code |
| C12 | `video/capture-demo.mjs` + `cdp.mjs` | **finding L10-3** | R: 3 of 134 leftover `$TMPDIR/cdp-*` Chrome profiles contain the plaintext token in `Local Storage/leveldb/000003.log`; `launch` never deletes its profile, and `capture-demo.mjs` has no try/finally, so a failure leaves headless Chrome with DevTools on 127.0.0.1:9333. No such process is running now |
| C12 | `video/tts.mjs` API key | holds | Key sent in the `x-goog-api-key` header, never in a URL or log |
| C5 | demo page served uncertified | holds as documented (default mode); **finding L10-1** (certified mode) | README.md:434-439 acknowledges the default; the certified-mode switch-off is not effective |
| C5 | honest page behaviour | holds | Token sent only as `Authorization` to relative `/mcp` (`rpc`, `post`); "Raw JSON-RPC exchange" shows the body, not headers; field is `type=password`. Page interpolates the search result's `canister_id` unescaped into script source (`const S = "${S}"`, `src/demo.html:299,315,329`): harmless with an honest reply (canonical principal from the allowlist), script injection under L10-1 |
| C5 | CORS | holds | Gateway adds `access-control-allow-origin: *`, but no ambient credential exists (token is a header, not a cookie), so a foreign origin cannot act as the user |
| U9/U10 | target-supplied interface and error text | **finding L10-4** | See A4 |
| O2 | interface fetch replies | holds | Inter-canister replies are replicated; not alterable in transit |
| CL21 | `certified_reads` semantics | **finding L10-1** | README.md:434-439 and `src/policy.rs:34-35` promise certification by consensus and the page switched off |
| CL28 | skip-certification claim | **finding L10-1** | README.md:497-499, `src/lib.rs:51-54` |
| CL30 | demo page holds no secrets; token only as header | holds | See C5 |
| I22 | query answered only when nothing must be fetched; certified mode upgrades all | holds for the canister's own output; **violated** end-to-end (L10-1) | |
| I25 | demo page holds no secrets, nothing from a reply runs as script | holds for honest replies; see C5 interpolation under L10-1 | |
| #47 | uncertified queries alterable; certified mode opt-in | **regressed / never effective: L10-1** | |
| #48 | tokens visible to infrastructure | still accurate (accepted) | Query path exposes the token to the single answering node, update path to the subnet; README says so |
| #49 | all tokens one identity | still accurate (accepted) | `caller` hash reaches only `IcBridge` audit fields; outbound calls carry no per-token identity |
| E6-E14 | admin calls | n/a for transport beyond #48 | Candid over the agent; `icp canister call` defaults to update (certified), `--query` is opt-in, so README's "read the audit log with an update call" is the default behaviour |
| CL26 | getrandom stream never used for secrets | n/a | No secret is generated on-chain (tokens are generated off-chain) |
