# Variant sweep r1-5 (Phase 6)

Pattern swept: **"Queued work is discarded at end of run while success is reported. Sibling
runtimes differ in eager versus lazy dispatch."** (source: L2-2, un-awaited `icp.call` /
`icp.call_async`, not re-reported). Searched for in general form: *a driver loop or
dispatcher reaches its end (completion, a count limit, a protocol path) and drops work or a
result that was already queued, sent or produced, while the status it reports does not
match what happened; equivalent paths handle the same end differently.*

Evidence: native tests added to a private copy (`/private/tmp/claude-501/sweep-r1-5.*`,
deleted afterwards), `cargo test --lib`, shared `CARGO_TARGET_DIR`. R = reproduced,
C = code path.

## New candidates

| Id | Sev | Title | Evidence |
|---|---|---|---|
| SW5-1 | Medium | JS and Python drop a finished run at their round/suspension limit after its last messages were sent, and report an error; Rhai at the same limit succeeds | R (native) |
| SW5-2 | Low | A `tools/call` sent without an `id` is silently not executed, answered `202 Accepted`, with no audit entry, on both the query and update path | R (native) |

### SW5-1

`src/runtimes/js.rs:374-409`: the loop sends round *i*'s batch, resolves the promises, then
increments; after the 64th round's replies are delivered the loop exits and returns
`Err("script exceeded 64 host round-trips")` without running the jobs or checking the
script's promise. `src/runtimes/python.rs:239-277`: same shape, `progress = next?` is
assigned on the 256th iteration and the loop ends with `Err("script exceeded 256 host
suspensions")` even when `progress` is `RunProgress::Complete`. Rhai
(`src/runtimes/rhai_rt.rs:211`) checks `answers.len() >= MAX_REQUESTS` *before* sending the
65th request, so 64 requests complete.

Native results (MockBridge, script = N sequential calls then return `"done"`):

| Runtime | N | Result | Calls sent |
|---|---|---|---|
| JS | 63 | Ok("done") | 63 |
| JS | 64 | Err("script exceeded 64 host round-trips...") | 64 |
| Python | 255 | Ok("done") | 255 |
| Python | 256 | Err("script exceeded 256 host suspensions") | 256 |
| Rhai | 64 | Ok("done") | 64 |
| Rhai | 65 | Err("script exceeded 64 icp requests") | 64 (65th not sent) |

Reachable under default limits (call cap 25): rounds/suspensions that send nothing still
count. JS: `for (63) await icp.candid(S)` (cached interface: `IcBridge::candid` returns before
`reserve`, `bridge.rs:475-480`) then `await icp.call(S, "create_order", ...)` -> the
`create_order` is sent and executed, run returns `isError:true`, closing audit entry
`Error`. Python: 255 x `icp.principal()` (each a host suspension, no message) then one
`icp.call` -> same. Both reproduced natively with one recorded call and `Err`. The client
(an AI agent) is told the script failed after its effect happened, so a retry repeats it.
README:368/371 document "64 rounds" and "256 suspensions"; the effective limits are 63 and
255 completing ones. Fix: check the promise / `progress` once more after the loop (or count
before dispatch, as Rhai does).

### SW5-2

`src/mcp.rs:252-254`: `classify` returns `Msg::Done(None)` for any message without `id`,
before looking at `method`, so `tools/call` (`execute_icp_script` or `search_icp_canisters`)
as a notification never reaches `exec`; `finish` answers `202` with an empty body
(`mcp.rs:308`). Native: single message and a two-message batch, `http_update` -> 202, `exec`
ran 0 times; `http_query` -> 202, no upgrade. No audit entry. MCP defines `tools/call` only as
a request, and JSON-RPC 2.0 lets the server skip the reply but not the processing; the
Streamable HTTP rule is "202 if the server accepts the input, else an HTTP error such as
400". The L12 grid recorded this case as holds (spec-permitted); listed here because it is
the swept pattern (submitted work dropped, success status). Verifier to decide.

## Cells

| Row(s) | Verdict for this pattern | Reason |
|---|---|---|
| C1, C1a | finding SW5-1 | runtimes are the sibling variants |
| C2 (demo_store) | holds | each update is one synchronous execution; nothing queued |
| C3, C4, C8-C11, C15 | n/a | build inputs; no queued work |
| C5 / E26 demo page | holds | `tool()` throws on `isError` (`demo.html:381`); every `fetch`/`tool` promise is awaited or joined with `Promise.all` |
| C6 / E27 `demo-setup.sh` | holds | `set -euo pipefail`; the one `icp canister call` failing exits non-zero before the success line |
| C7 | holds for this pattern | build steps run sequentially; failure stops `icp build` (stale-wasm copy is a known finding) |
| C12 / E28 video | holds | every `b.eval`, `fetch`, `execFileSync` awaited or synchronous; no detached promise found by grep |
| C13, C14 | n/a | docs/tooling, not product behaviour |
| A1-A12 | n/a | actors; covered through their entry points |
| E1, E2 | holds | synchronous `apply`; a bad hash traps and rolls back (no silent drop) |
| E3 | holds | filter only, no queued work |
| E4, E4a | holds | stateless answers |
| E4b | finding SW5-2 | `tools/call` without id dropped with 202; client responses and real notifications dropped by design |
| E5 | finding SW5-2 | same; extra scripts/searches over the per-request caps get explicit -32600 (not silent); oversized replies replaced by an error (known) |
| E5a | finding SW5-1 | via the runtimes |
| E5b `run_search` | holds | canisters after a policy halt are returned as `not fetched: ...` per entry (`lib.rs:134`); every fetch is awaited in sequence |
| E5c | holds | replies are delivered to every batch member (`js.rs:401-407`, `python.rs:262-264`); a `from_json` failure aborts the rest but returns `Err` |
| E5d | holds for this pattern | `Trapped` entry written; leaks on a second trap are known |
| E6-E14 | holds | single synchronous executions |
| E15 | n/a | not callable |
| E16 JS `icp.call`/`icp.candid` | finding SW5-1; L2-2 (known) | 64th round's sends executed then result dropped; un-awaited requests at completion: known |
| E17 JS console | holds | `State.logs` flushed to the bridge on every path (`js.rs:355-357`), including errors |
| E18 Python | finding SW5-1; L2-2 (known) | 256th suspension's result dropped; `queued` at `Complete` dropped: known. `call_async` queued and later followed by any blocking await IS sent (whole `queued` batch, `python.rs:258`) |
| E19 Rhai | holds | eager dispatch; limit checked before sending (`rhai_rt.rs:211`); 64 requests complete (R) |
| E20-E25 | holds | demo_store synchronous handlers |
| U1-U16 | n/a | inputs; the pattern is about dispatch/completion, tracked at E/K rows |
| O1 | finding SW5-1 | last-round O1 messages are sent and executed for a run reported as failed |
| O2 | holds | fetch result cached even if the run is later dropped; nothing discarded |
| O3 | holds | `Call` entries for the last round are written (K1); closing entry says `Error` with the round/suspension message: records match the sends, status contradicts completion |
| O4-O10 | n/a / holds | synchronous writes; O6 replies built from what ran |
| D1-D3, D5-D7 | n/a | no queue |
| D4 | holds | see O3 |
| D8-D10 | n/a | |
| V1 | holds | `RunSlot` dropped on every path |
| V2, V4, V5 | holds | caches written in the same execution as the fetch result |
| V3 | holds | removed in `IcBridge::drop` on every path |
| V6-V11 | n/a | no queued work |
| K1 | holds | entry written when the message is accepted |
| K2 | finding SW5-1 (+ L2-2 known) | `drive` exits after the 64th round without a final `run_jobs`/state check |
| K3 | finding SW5-1 (+ L2-2 known) | `for _ in 0..MAX_SUSPENSIONS` discards a `Complete` assigned on the last iteration |
| K4 | holds | Rhai: checked before the send; final replay after the 64th answer still runs |
| K5 | holds | search awaits each fetch in turn |
| K6 | holds for this pattern | oversized reply replacement is known |
| K7 | holds for this pattern | known leak on second trap |
| K8, K9, K10, K12 | holds | nothing sent or single execution |
| K11 | known | upgrade discards in-flight runs ("Upgrades leave no audit trace") |
| LIM25 (`MAX_ROUNDS`) | finding SW5-1 | effective 63 completing rounds; 64th round's messages sent then reported as an error |
| LIM26 (suspensions) | finding SW5-1 | effective 255 completing suspensions; also counts `NameLookup`/`OsCall` progress steps as iterations |
| LIM27 (`MAX_REQUESTS`) | holds | 64 complete, 65th refused before sending (R) |
| LIM9, LIM10 | holds | over-cap tool calls get an explicit -32600, not a silent drop |
| LIM8 | holds | over-size batch refused whole with 400 before any work |
| LIM6, LIM7 | known | truncation with `isError:false` and oversized fallback already reported |
| LIM15 | known | Python MemoryError vs truncate (known) |
| LIM25 `MAX_PENDING`, `MAX_PROMISE_JOBS` | holds | refused before queuing (RangeError at `icp.call`), job queue cleared on error and `Err` returned |
| LIM1-LIM5, LIM11-LIM14, LIM16-LIM24, LIM28-LIM32 | n/a / holds | refuse before the work, or not a queue/loop-exit limit |
| W1-W8, W10, W12-W16 | n/a | cost rows; no queued work dropped |
| W9 | finding SW5-1 | rounds that are refused or free still count toward the 64 |
| W11 | holds | replays bounded; answers kept |
| F1-F16 | n/a | aborting constructs report failure, not success |
| B1 | finding SW5-2 | classification by `id` presence before `method` |
| B2-B17 | n/a | conversions (fidelity findings are L7, known) |
| VF1 | finding SW5-1 | canister pays for messages of a run reported failed; a retry pays again |
| VF2-VF8 | n/a | |
| AD1-AD9 | n/a | admin settings do not change the round/suspension constants; `max_calls_per_script` does not reach them because free rounds exist |
| S1-S10 | n/a | |
| CL9 | finding SW5-1 | "JS ... 64 rounds", "Python 256 suspensions": one fewer completes |
| CL12 | finding SW5-1 | same |
| CL1, CL15, CL17, CL18 | finding SW5-2 (CL1) | a tools/call notification is not run and not audited; CL17/CL18 hold for SW5-1 (messages logged, one closing entry) |
| CL29 "identical semantics in every language" | finding SW5-1 | same script of 64 calls succeeds in Rhai, fails in JS |
| other CL rows | n/a | not about completion or dispatch |
| T1-T12 | gap | no test reaches exactly `MAX_ROUNDS` / `MAX_SUSPENSIONS` with a completing script, nor a `tools/call` notification |

## Not tested

Live reproduction of SW5-1 on the local network (native only; the live path differs only in
the bridge, and the free-round mechanisms are read from `IcBridge::candid` and the Python
host). Engine-internal queues (Boa async/timer jobs ignored in `enqueue_job`, `js.rs:145-147`):
the comment says nothing in the sandbox can create them; not verified against Boa 0.22's
built-ins.
