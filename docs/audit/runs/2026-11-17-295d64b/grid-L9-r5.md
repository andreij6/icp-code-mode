# Grid: L9 Failure handling, round 5 (a run that outlives the transport)

Run `2026-11-17-295d64b`. Question: when a script runs longer than the HTTP gateway or the MCP
client is willing to wait, what does the client get, does the run still commit, who learns the
outcome, and what does a retry do? Not tried before (L5-r2 "Not tested"; L5r2-1 covers only the
round/suspension-limit trigger).

## Method

- **Network:** the project's local icp-cli network (port 8010, pocket-ic network launcher
  v16.0.0, gateway built from `ic-gateway` with `ic-agent 0.49.1`). No second network.
- **Why a separate instance:** at the start of this round the shared `icp_code_mode`
  (`4caro-…`) ran module `89b4c117…`, built from the `audit-fixes` branch (it has
  `tool_calls_per_minute` and makes every call `bounded_wait` with a 300 s default), so it was
  not the audited code. Instead of replacing someone else's deployment, the audited module was
  installed in a **detached canister on the same network**: `5iptu-f3777-77775-aaaga-cai`.
- **Build:** `git archive HEAD` into a private temp copy, sources touched,
  `CARGO_TARGET_DIR` = the shared directory, then icp.yaml's own `ic-wasm` steps. Result
  sha256 `224cfa7a…c769`, byte-identical to the HEAD artifact of L13-r2/L13-r4.
- **Install:** `InitArgs { token_sha256 = [hash of a fresh test token kept in the scratchpad,
  never printed], allowlist = [(fixture, ["hang"]), (demo_store 4fbx2-…, ["create_order",
  "list_orders"])], limits = null }` (defaults: 25 calls, 20e9, **unbounded wait**), plus
  `set_candid_override` for the fixture.
- **Fixture** (temp dir only): detached `5pova-id777-77775-aaagq-cai`. `hang(n)` logs
  `hang-enter`, keeps its call context open with one self-call per round until
  `release(true)`, then logs `hang-done` and replies `n`; `events()` returns the log.
- **Script** (one JSON-RPC body, file sha256 `bad9403c…78e9`, JSON-RPC `id` 7, sent
  byte-identical every time):
  `const h = await icp.call(FIX,"hang",1); const o = await icp.call(DS,"create_order",["ix6jd-voa77-xagai-camca-kbq",[{product_id:4,quantity:1}]]); return {h,o};`
- **Client:** Python `urllib` POST to `127.0.0.1:8010/mcp` with `Host: 5iptu-….localhost:8010`
  and the bearer header. "SDK-like" means a 60 s client timeout followed by
  `notifications/cancelled {requestId: 7}`, which is what the TypeScript SDK does.
- **Cleanup:** both detached canisters stopped and deleted (the fixture with
  `--no-recover-cycles`: its self-call loop had burned its cycles below the recovery cost).
  The shared `icp_code_mode`'s allowlist, limits and overrides were never touched. Orders 61-63
  stay in `demo_store` (test data, status `Pending`; 3 units of product 4 reserved). Note: while
  this round ran, someone else redeployed the shared `icp_code_mode` to `224cfa7a…`. That was
  not this lens.

## Measurements

| # | Step (local time) | Client sees | Canister/target |
|---|---|---|---|
| T1 | Gateway timeout, source: pocket-ic's gateway submits through ic-agent; `ic-agent` 0.49.x `poll_backoff` bounds the **cumulative sleep** by `max_polling_time` (default 300 s, `agent_config.rs:56`) and restarts that budget once the status is `Processing` (`agent/mod.rs:840-851, 876-890`) | n/a | The source does not settle it, so it was measured (A) |
| A | 11:22:38 send, no client timeout | **HTTP 200 after 716.4 s** with the full result (`order id 61`) once the fixture was released | Run 3: `Started` (hang) at 11:22:38, then `Call` (create_order) and `Ok` after release. **The local gateway did not time out in 716 s**, and it held the connection for the whole run |
| C1 | 11:22:50, during A: `{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":7,...}}` | HTTP 202, empty body, 0.5 s | `pending_run_ids = [3]`, `active_runs = 1` unchanged; run 3 later sent `create_order` and closed `Ok` |
| B | 11:32:55 send, client timeout 60 s (MCP SDK default) | `TimeoutError: timed out` after 60.0 s. Nothing else ever arrives | Run 4 kept running: `pending_run_ids = [3,4]`, `active_runs = 2`, fixture `hang-enter` #2 |
| C2 | 11:33:55 `notifications/cancelled` for id 7 (as the TS SDK sends on timeout) | 202, empty | Run 4 unaffected (see below) |
| R | 11:34:09 **byte-identical** retry of B, timeout 60 s. Fixture released at 11:34:29 | HTTP 200 after 22.8 s, `isError:false`, `{"h":1,"o":{"Ok":{"id":62,...}}}` | Run 5 |
| End | After release | | Fixture events: 3 x `hang-enter 1`, `release true`, 3 x `hang-done 1`. Audit: runs 3, 4 and 5 each `Started` -> `Call create_order` -> **`Ok`** (entries 9-11), with identical `script_sha256 cff20eb2…`, caller, methods and `calls = 2`. demo_store: customer `ix6jd-…` orders 8 -> **11** (ids 61, 62, **63**). Order 63 is run 4's, the run whose client got a timeout. `pending_run_ids = []`, `active_runs = 0` |
| G | `GET /mcp` and `DELETE /mcp` with the token (resumable stream / session) | 405 `only POST is supported (no SSE stream)` | No endpoint returns a past result. The audit (controllers only) has the status but not the result, the JSON-RPC id or the order id |

Docs check (step 5): `git grep -i 'idempot|created_at_time|retr(y|ies)|dedup|cancel'` over
README.md, `src/mcp.rs`, `src/lib.rs`, `src/bridge.rs`, `icp_code_mode.did` and `src/demo.html`
at HEAD finds: only `idempotentHint: true` on the **search** tool (`mcp.rs:115`); "retry
shortly" in the busy error (`lib.rs:220`, correct, since a busy run did nothing); the demo
page's transfer example with `created_at_time: null` (`demo.html:199`). Nothing in the README or
in the `execute_icp_script` description (`mcp.rs:77-101`, shown to the AI) says that a run
continues after the client gives up, that a retry repeats every effect, or how to make a call
idempotent. `execute_icp_script` has no annotations (`mcp.rs:116-133`), so the MCP defaults
apply (`idempotentHint: false`, `destructiveHint: true`).

SDK timeouts: TypeScript SDK 1.32.1 (installed copy under `/opt/homebrew/lib/node_modules/...`):
`DEFAULT_REQUEST_TIMEOUT_MSEC = 60000` (`dist/esm/shared/protocol.js:8`). On timeout it rejects
with `RequestTimeout` "Request timed out" and sends `notifications/cancelled`
(`protocol.js:698-713, 740-742`). Python SDK: not installed locally, so not checked.

## Cells

| Row | Verdict | Reason / evidence |
|---|---|---|
| O1 inter-canister call | **finding L9r5-1** | R: the `create_order` of run 4 executed after its client had received a timeout, and the retry executed it again (orders 62, 63). Policy and the call cap are per run, so nothing links the retry to the first run |
| E5a `execute_icp_script` | **finding L9r5-1** | R: the run's lifetime is set by its targets, not by the transport. No request id, script hash or idempotency key is remembered between requests (`run_script`, `lib.rs:218-263`, keeps only per-run state). B and R were byte-identical, including JSON-RPC id 7, and both executed |
| K1 first send (Started) | holds | R: entries 3/4/5 were written as each `hang` was handed over, before any client timed out, and stayed (CL17 literally holds) |
| K2 JS `await` per round | holds (records); part of L9r5-1 (outcome) | R: after 12 min suspended, run 3 resumed from the committed state and continued with `create_order`. The continuation does not know whether anyone is still listening, and the protocol gives it no way to find out |
| K3 Python await / K4 Rhai replay | holds (C) | Same suspension model (`rebased(host(...))`, replay loop). Not run live in this round, and no different transport behaviour is expected: the timeout is outside the canister |
| K5 search awaits | not applicable | A search's fetches have no target-side effects and are cached; a retried search costs one more fetch at most (register #46 TTL) |
| K6 end of `http_request_update` | **finding L9r5-1** | R: run 4 wrote `Ok` (entry 11) and built a reply with order 63. The reply went to a gateway request whose client had already gone, and was discarded. `Ok` here means "the script returned", not "the caller received it". The audit cannot tell runs 3, 4 and 5 apart (same hash, caller, methods). L5r2-3 already covers the missing arguments and reply, so this is not repeated |
| VF4 assets moved by allowlisted methods | **finding L9r5-1 (C, consequence)** | Same mechanism as `create_order`. With `icrc1_transfer` allowlisted, a transfer behind any slow call is re-executed by a client retry unless the script hard-codes `created_at_time` (L5-r2 M6/M8-M10 show the ledger side). Not re-run with the ledger in this round |
| CL15 "One entry per outbound message ... plus one closing entry per run" | holds | R: 3 runs, 3 x (`Started`, `Call`, `Ok`) |
| CL17 "never shows a message that was not sent, never misses one" | holds | R: every `hang` and `create_order` sent has its entry. The gap is the outcome reaching the client (L9r5-1), not the record of the send |
| LIM3 `call_timeout_seconds` | holds as documented; does not address L9r5-1 | Default `null` lets one call wait indefinitely (register #44; A waited 716 s). A configured timeout (at most 300 s per call, 25 calls) still allows a run far longer than a 60 s client timeout, so L9r5-1 is independent of LIM3. The README presents the option only as a stop/upgrade safeguard (README:430-433) |
| MCP `notifications/cancelled` (E4b/E5, `classify` `mcp.rs:248-250`) | **finding L9r5-2** | R: 202, and the run continued and committed. The spec lets a server ignore a cancellation it cannot honour, so the defect is that nothing documents this, and the client SDK's timeout-plus-cancel suggests to the user that the work stopped |

## Candidates

### L9r5-1: a run that outlives the client's timeout commits its effects, and nobody learns the outcome; a retry repeats them

- **Severity:** High (a caller with ordinary access gets a failure for a run that changed state
  on a target, and the standard client reaction, a retry, duplicates every effect; with a
  value-moving method allowlisted (VF4) that is the canister's funds sent twice).
- **Invariant:** I3 (the record and what the client is told disagree), I10 (nothing documents
  it), I4 (a retry multiplies every effect: L4 pattern "a per-item budget that a retry
  multiplies").
- **Where:** `src/mcp.rs:374-412` (`http_update`: a reply is the only outcome channel),
  `src/lib.rs:218-263` (`run_script`: no request id or idempotency key, nothing kept after
  the reply), `src/bridge.rs:524-527` (unbounded wait by default), `src/mcp.rs:77-133` (tool
  description and missing annotations), README "How to read the audit log" and "Options".
- **Evidence:** reproduced (B, R, End above). A client with the MCP SDK's 60 s timeout got
  `timed out`. Its run (4) waited, created order 63, and closed `Ok`. A byte-identical retry
  created order 62. A third send (A) created 61. That is 3 orders and 3 `Ok` runs for one
  intended action, and the client that timed out had no way to learn that its order existed.
  The local gateway itself held a connection for 716 s and delivered the result (A), so locally
  only the client's own timeout triggers this. The mainnet boundary-node timeout was not
  measured.
- **Who learns the outcome:** the target (it executed) and a controller reading the audit log
  (`Ok`, without the result, the arguments or the JSON-RPC id). The token holder, who cannot
  read the audit, and the AI client learn nothing: no result endpoint (`GET /mcp` 405, no SSE,
  no session), no receipt id, and the late reply is dropped with the gateway request.
- **Refuted first:** (a) "the gateway would time out and tell the client": locally it never did
  (716 s). (b) "the cancel stops it": C1/C2 had no effect. (c) "a retry is deduplicated":
  identical body and id, executed again. (d) "the docs warn": they do not (step 5).
- **Distinct from:** L5r2-1 (the server itself reports failure after an effect because of
  round/suspension limits) and #44 (stop/upgrade blocked). Same symptom (failure reported,
  effect committed, retry repeats it), different trigger: the transport.
- **Pattern:** an operation whose effects outlive the request that started it, with the
  response as the only channel for the outcome and no idempotency key.
- **Fix:** give every run a stable id returned at once and store each run's result. Two ways:
  (1) a `get_run_result(run_id)` tool or JSON-RPC method that answers from the audit log plus
  a bounded result store, or (2) the MCP Tasks utility / SSE with resumability. Accept an
  optional client idempotency key (or the JSON-RPC id plus the token hash) and answer a repeat
  within a window with the stored outcome instead of running again. Document in the tool
  description that effects may have happened when a call times out, and that ledger calls
  should carry `created_at_time`. Add `annotations: { idempotentHint: false, destructiveHint:
  true }` explicitly. Regression test: PocketIC, a client times out on a held target, the
  target is released, a retry with the same key returns the first result and does not send a
  second message.

### L9r5-2: `notifications/cancelled` is silently ignored, and nothing says so

- **Severity:** Low (the spec allows ignoring a cancellation, and the outcome is covered by
  L9r5-1. The fault is the missing documentation and the misleading client experience).
- **Invariant:** I10.
- **Where:** `src/mcp.rs:248-250` (`classify`: every notification -> `Done(None)`, in both
  query and update paths). The cancel arrives as a separate HTTP request, answered by the
  query path, so even a cooperative check inside the run could never see it.
- **Evidence:** reproduced (C1, C2): 202 with an empty body, the runs continued, sent
  `create_order` and closed `Ok`.
- **Fix:** document in the tool description and README that a sent script cannot be cancelled.
  If cancellation is wanted, record cancel requests by (token hash, request id) in an update
  and check them before each send (`reserve`).

## Not tested

- **Mainnet / boundary-node gateway timeout:** only the local pocket-ic gateway was measured
  (no timeout in 716 s). Whether `ic-gateway` on boundary nodes cuts a long
  `http_request_update` (and with what status) needs the real network (register #57).
- **Python MCP SDK default timeout:** the package is not installed locally, and installing it
  means downloading. TypeScript checked in source (60 s).
- **Ledger re-run:** the value consequence (VF4) was not re-run here; it rests on L5-r2 M6/M8-M10
  and the identical code path.
- **K3/K4 live:** Python and Rhai suspensions under a held target were not run in this round.
- **Gateway-initiated retries:** no automatic re-submission by the local gateway was seen
  (3 sends -> 3 runs, all mine). Boundary-node behaviour is unknown.
