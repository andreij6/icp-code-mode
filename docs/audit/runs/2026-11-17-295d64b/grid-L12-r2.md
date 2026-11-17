# Grid: L12 round 2 — differential test of the query route against the update route

Run `2026-11-17-295d64b`. Lens: "No systematic differential test of the query route against the
update route" (L12 Protocol conformance, applied with the Phase 4 differential technique).

## Harness

A private copy of the tree (`/private/tmp/claude-501/l12r2/repo`, version string changed so the
shared build directory does not mix it with other copies; deleted after the run) with one added test module,
`src/l12_diff.rs`, run with `cargo test --lib l12r2 -- --nocapture` (`RUST_MIN_STACK=256 MiB`,
see "Not tested").

- **Real code:** `mcp::http_query`, `mcp::http_update`, `mcp::tool_result`, `search::scope`,
  `search::render`, `candid_bridge::Service::parse`, the Rhai engine through `MockBridge`.
- **Modelled:** `lib.rs::search_cached` and `lib.rs::run_search` (the production `IcBridge` cannot
  run off-chain, register #58). Three allowlisted canisters: A cached, B not cached (the update
  "fetches" it, takes calls and writes an audit entry), C in `DID_FAILED` (update answers the cached
  error without sending, no entry). The interface of A has a ~450 KB record type so that searches for
  `big` produce ~760 KB results. The instruction budget is not modelled (counter is 0 natively).
- **Generator:** random batches of 1-12 messages (or single messages) mixing `initialize`, `ping`,
  `tools/list`, notifications, client responses, null/object/bool ids, bad search params, unknown
  tool, unknown method, at most one Rhai script, and searches with scope A / B / C / all /
  not-a-principal / not-allowlisted and five queries including `big`. Each batch goes through
  `http_query` with `certified_reads` off and on. If the query answers, the same body goes through
  `http_update` on a fresh state and must give the identical status and body with no audit write.
  If the query upgrades, every message is also sent alone to the query; whatever the query answers
  alone must equal the update's answer for that id in the batch. Plus six HTTP-level cases (bad
  token, wrong path, GET /mcp, GET /, GET /demo, `post /mcp/`) in both modes.
- **Runs:** 400 batches unbiased (seed `0x9e3779b97f4a7c15`) and 300 batches biased to cached
  scopes (`L12_BIAS=1`), each x 2 modes.

Results (counts are batch x mode):

| Outcome | Unbiased | Biased |
|---|---|---|
| answered in query, update identical, no audit write | 123 | 235 |
| answered in query, update differs: search cap (L12-5, known) | 0 | 19 |
| answered in query, update differs: oversized-reply shape (L12-2, known) | 0 | 1 |
| upgraded (certified off) | 277 | 45 |
| upgraded (certified on, every authenticated request) | 400 | 300 |
| per id: answered alone in query, refused in the batch by the search cap | 26 | 6 |
| per id: answered alone in query, replaced in the batch by the reply-size fallback | 35 | 0 |

No other divergence: ids, notifications, client responses, invalid ids, bad params, unknown
tool/method, error codes and messages, and HTTP status were identical in every answered batch.

Live (R-live, local network, direct `http_request` query and `http_request_update` calls with the
same Candid argument, built from the token in `.env` without printing it): see the rows below.

## Rows

| Row | Verdict | Why |
|---|---|---|
| E4b | finding | Known divergences re-found by the generator: L12-5 (query ignores `MAX_SEARCHES`), L12-2 (query answers an oversized cached-search batch with one `id: null` error; the update answers the same batch with an array and per-id fallbacks). New: L12r2-3 (no instruction budget on the query search path). Holds for everything else: 358 answered batches identical to the update (R-native). R-live: a batch of `initialize`, `ping`, notification, client response, null-id ping and a cached `demo_store` search was answered in the query and the update returned byte-identical body and status, audit log length unchanged |
| E4 (query, certified off/on) | holds | Certified on: every authenticated request upgraded (700/700 batches); 401/404/405 identical in both routes and both modes (R-native). Uncertified 401/404/405 is SW10-5 (known) |
| E4a demo page | holds (note) | `GET /` and `GET /demo` sent directly to `http_request_update` return 404 "not found; the MCP endpoint is POST /mcp", while the query returns 200 HTML (certified off) or 404 "the demo page is disabled..." (certified on) (R-native). The gateway only calls the update after `upgrade: true`, which the page never sets, so the divergence is reachable only by a direct canister call and harms no one |
| E5 | finding | L12r2-1 (fallback replaces every tool result, including a script that ran), L12r2-2 (size check ignores the array framing). Known: L12-2, L12-5, L12-6 |
| E5b | finding | L12r2-3 (budget applied only on the update path). Holds: failed scope C answered by the update from `DID_FAILED` with no message and no entry, while the query upgrades (L6-7, known); uncached scope fetched once per batch and later searches in the batch see the cache (R-native model) |
| E5a | finding | L12r2-1: the script's result is discarded by the batch fallback while its audit entry says `Ok` |
| B1 | holds | Classification is one shared function (`classify`); generator found no id, notification, client-response or error-code difference between routes (R-native). Id precision (L12-1) is the same in both routes |
| LIM7 reply size | finding | L12r2-1, L12r2-2; L12-2 (known) |
| LIM8 `MAX_BATCH` | holds | Shared `parse_body`: 51 messages -> 400 + single -32600 in both routes (T5; generator stays within 12) |
| LIM9 `MAX_SEARCHES` | finding (known) | L12-5 / L4-3 / L6-8 re-found: biased run, 19 answered batches with 6-11 searches answered in full by the query and partly refused by the update; per id, 32 searches answered alone and refused inside the batch |
| LIM10 one script per request | holds | Only the update executes; the query upgrades any batch with a script, so the rule cannot diverge (R-native) |
| LIM2 `max_instructions` | finding | L12r2-3 (R-live: cached heavy search, query traps at 5e9 -> gateway 503; update answers 200) |
| I22 | finding | "Query responses are answered only when nothing must be fetched or written": holds for writes (no answered batch caused an audit write in the update model; R-live audit length unchanged). Does not hold as "the query answer is the update answer": L12-5, L12-2 (known), L12r2-3 |
| Test suite (T5) | finding | L12r2-4: no test compares the two routes; `tools_call_upgrades_from_query` and `search_is_answered_in_query_unless_a_fetch_is_needed` check the upgrade flag only |

## Candidate findings

- **L12r2-1** (Medium, R-native). `http_update` replaces the result of *every* message that has
  `/result/content` with "reply exceeded 1900000 bytes" (`isError: true`) when the summed reply is
  too large (`src/mcp.rs:405-410`), including a script whose result is two bytes. Batch
  `[execute_icp_script "40 + 2", 5 x search "big"]`: the script ran (the model records its audit write;
  in production `run_script` writes `Started`/`Ok`), and the client receives
  `{"id":"script","result":{"content":[{"text":"reply exceeded 1900000 bytes"}],"isError":true}}`.
  The script's own reply would have fitted; the searches caused the overflow. A client cannot tell a
  failed script from one whose effects committed, and may run it again.
- **L12r2-2** (Low, R-native). The update's size check sums the serialized messages
  (`src/mcp.rs:405`) without the array's brackets and commas; `finish` then measures the real body.
  Batch `[search big, search big, ping with padded id]` with summed size exactly 1,900,000: body is
  1,900,000 + 4 bytes, the per-message fallback is skipped, and `finish` returns one
  `{"error":{"code":-32603},"id":null}` object (92 bytes) for the whole batch. One byte more input
  (sum 1,900,001) gives the array with both searches replaced and the ping answered. Shares its root
  with L12-2 (two size guards with different shapes); may merge.
- **L12r2-3** (Medium, R-live for the default settings, code path for a lowered budget). The query
  search path has no instruction budget of its own, so the route decides the limit. R-live (defaults,
  `max_instructions` 20e9): one cached search of `demo_store` with 600,000 `->` keywords (1.8 MB body)
  sent to `http_request` (query) traps at the platform's 5e9 query limit; the gateway returns 503
  `canister_error` and never upgrades. The same body sent to `http_request_update` returns 200 with
  the 8 methods, `isError: false`, and no audit entry (log length unchanged). The same search for an
  interface that is not yet cached is upgraded before rendering and succeeds, so it fails only once
  the cache is warm. Code path for the reverse: `search_cached` renders and replies (`src/lib.rs:111-117`, `src/mcp.rs:362-363`), while
  `run_search` checks `instructions_used() > max_instructions` after the same render and, if over,
  returns `policy: instruction budget exceeded` and appends a `PolicyViolation` entry
  (`src/lib.rs:150-154, 161-186`). With `max_instructions` set below a search's render cost, the
  same cached search is a free success in the query and a refused, audited run in the update.
  Same pattern as L12-5 (a per-request limit enforced on one route only); the cost side of the render
  is L5-4 and W3 (known).
- **L12r2-4** (Low, testability, R-native). No test in the suite runs the same request through both
  routes; every divergence above and L12-2/L12-5/L6-7 was found by hand or by this harness. Fix: keep
  `src/l12_diff.rs` (generator + oracle "answered in query => identical update answer, no audit write;
  per id, answered alone => same answer in the batch") with the production `search_cached`/`run_search`
  behind a trait so the real glue, not a model, is compared.

## Not tested

- The production `search_cached`/`run_search`/`run_script` glue natively (modelled; register #58).
- Instruction budgets natively (counter is 0 off-chain). The reverse direction of L12r2-3 (query
  succeeds, update refused) needs `set_limits` with a low `max_instructions`, a settings change
  reserved to the Live phase; only the default-settings direction was run live.
- Certified-mode behaviour live (needs `set_limits`; Live phase).
- A real interface large enough for a ~760 KB search result (used a synthetic `.did` in the model).
- Parsing that synthetic 40,000-field `.did` overflowed the default 2 MiB native test stack;
  the harness runs with a 256 MiB stack. Parser recursion on large `.did` text is known (F12).
