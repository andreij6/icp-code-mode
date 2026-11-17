# Coverage grid: L2 Atomicity and commit points, round 1

Run `2026-11-17-295d64b`. Lens question: if execution stops here, what has already been
committed and what is lost?

Dynamic work was done on a **private, disposable local network** (icp-cli 1.7.0, gateway port
8047, own state directory in the scratchpad, stopped and deleted afterwards) running the exact
deployed artifacts copied from the shared network's state (`icp_code_mode` sha256
`ad3ee221…2411`, `demo_store` `5ed9b423…a029`), with a freshly generated test token. The shared
network on port 8010 was not touched. Fault injection used: a reply-callback trap at the 40B
instruction limit (catastrophic regex backtracking, register #23), the "insufficient liquid
cycles" on-the-spot refusal (freezing threshold raised), and attempts to make stable-memory
growth fail (see L2-5).

Candidate ids (L2-n) refer to the findings returned with this grid.

| Row | Verdict | Reason / evidence |
|---|---|---|
| K1 first-pending commit (`IcBridge::sent` / `record_message`) | holds | ic-cdk 0.20.3 `CallFuture::poll` returns `Pending` only after `call_perform` returned 0, and `Ready(Err)` on `call_perform` failure or on the liquid-cycles pre-check (`call.rs:899-935`); `record_message` runs inside that first poll, in the same execution as `call_perform`. Reproduced: with liquid cycles below the call's reservation the call failed with "insufficient liquid cycles balance" and the run's entries showed `calls = 0`, no `Started`. A normal call wrote `Started` with `calls = 1` |
| K1 (record content) | finding L2-1 | For an interface fetch the record names `aaaaa-aa.canister_metadata` and `canisters = ["aaaaa-aa"]`; the canister whose interface was read appears nowhere. Contradicts `.did`/`policy.rs` "`canister.<candid interface>` for an interface fetch". Reproduced |
| K2 JS awaits (`join_all` per round) | holds / finding L2-2 | Commit per round holds (all K1 entries of a round commit with their sends). But requests still queued in `State.pending` when the script's promise settles are dropped without being sent, error or warning (`js.rs:377-383`); run reports `Ok`. Reproduced (L2-2) |
| K3 Python awaits | holds / finding L2-2 | Same as K2 for `rebased(host)`/`rebased(join_all)`; `icp.call_async` requests left in `queued` at `RunProgress::Complete` are dropped (`python.rs:241, 249`). Reproduced (L2-2) |
| K4 Rhai replay awaits | holds | One request per replay, awaited before the next replay; answers kept per run. Differential: an unused `icp::call_canister` statement *is* executed (order created), unlike K2/K3 |
| K5 `run_search` awaits / two sequential awaits in `fetch_did` | holds (with L2-1) | Each fetch message is recorded at its own first poll; a refused-on-the-spot `canister_metadata` falls through to the hack call with no entry for the first. Label issue as L2-1 |
| K6 closing entry + reply | holds | `finish()` and `http_update` re-measure keep the reply under 1.9 MB (register #2/#4). If the execution traps after the closing entry, the entry and `finished = true` roll back together and K7 writes `Trapped` (code path; reproduced for the trap-in-callback case) |
| K7 cleanup (`Drop for IcBridge`, `RunSlot`) | holds / findings L2-3, L2-4, L2-5 | Reproduced: a 40B-instruction trap in the reply callback produced a `Trapped` entry with the right `run_id`, `calls = 1`, `methods`; `active_runs` back to 0; `pending_run_ids` empty, so the cleanup gets a fresh instruction budget. But the `Trapped` entry under-reports `instructions` (L2-3) and drops the "…and N more" violation count (L2-4). If the cleanup itself traps, the slot and the pending id leak (L2-5, code path) |
| K8 trap before first send | holds (accepted, register #11) | Whole request rolled back; no entry; documented in README "Not in the log at all" |
| K9 policy refusal before any send | holds (with #11 caveat) | Refusals are recorded only in the closing entry; if that execution traps, nothing is recorded (register #11, accepted, documented). After a send, refusals made in the final (trapped) execution are lost from `Trapped` (documented: "violations recorded up to the last message"); reproduced (`late_bad` absent) |
| K10 admin updates | not applicable | Single execution, no await, no outbound effect: all or nothing |
| K11 upgrade while runs are suspended | holds (as documented, not tested live) | Heap `PENDING_RUNS`/`ACTIVE_RUNS` reset; runs keep `Started`/`Call` without a closing entry; README lists this as "Unknown … canister was upgraded while the run was waiting". Not reproduced: needs a target that stays busy across an upgrade |
| K12 demo_store updates | not applicable | Single execution each, no await |
| O3 stable audit writes | holds | Every append is in the same execution as the effect it describes (K1) or the closing step (K6/K7); `append_audit` assigns `id = len` and appends in one borrow |
| D4 `AUDIT` StableLog | holds (with F4) | Index and data writes happen in one execution, so a trap cannot leave a half-written entry. Growth failure traps (F4, see L2-5) |
| E5a `run_script` closing entry | holds | Exactly one closing entry per run that reaches the end; written before `finish()`; `run_id` links (reproduced) |
| E5b `run_search` closing entry | finding L2-6 (claims) | Entry written only if a message was sent or a policy refusal happened. A search answered from cache in the update path (e.g. `certified_reads = true`) or refused by a remembered fetch failure (`DID_FAILED`) writes no entry, a case missing from README's "Two kinds of request write no entries". Code path. Records still match reality (nothing was sent) |
| E5c reply callbacks | holds | One callback per reply; each commits; a callback after cancellation traps "Call already trapped" and its cleanup returns early (`call.rs:961-1040`), so no second `Trapped` entry |
| E5d cleanup callback | see K7 | |
| V3 `PENDING_RUNS` | holds / finding L2-5 | Inserted with `Started` (K1), removed in `Drop` (normal end and successful cleanup, reproduced). Never removed if the cleanup traps (L2-5); empty after upgrade (documented) |
| F4 `append(..).expect` | finding L2-5 (consequence) | In K1 the trap rolls back the message too (consistent). In K6 it rolls back the closing entry and the cleanup's own append fails the same way, so the cleanup traps and its slot return and pending removal are discarded. Trigger not reproduced locally: `memory_allocation` did not cap growth on this replica (usage grew from 75.7 MB to 84.1 MB with an 80.2 MB allocation) and a raised freezing threshold let the 8 MiB bucket grow and then froze the canister ("out of cycles") instead of failing the grow |
| I3 Records match reality | (a) holds, reproduced; (b) holds except L2-5 and documented upgrade case; (c) count holds, content fails for fetches (L2-1); (d) violated by design (#11), plus L2-4 | |
| Register #2 reply over size trapped | still fixed | `finish` guard + `http_update` re-measure (`mcp.rs:306-318, 405-410`); test `mcp::result_size_is_bounded_after_escaping` |
| Register #3 run_id linking | still fixed | Reproduced: `Started`, `Call`, closing and `Trapped` entries share `run_id` |
| Register #4 padded batch | still fixed | `MAX_BATCH` + `finish` guard; test `mcp::replies_always_fit` |
| Register #5 "Started, 0 calls" | still fixed | Reproduced: `Trapped` entry `calls = 1`, `methods` filled |
| Register #6 search refused before send | still fixed | `run_search` writes an entry when `violations` is non-empty (code path) |
| Register #7 entry before encoding | still fixed | `sent()` wraps only the call future, after `encode_args` (`bridge.rs:517-528`) |
| Register #8 refused on the spot | still fixed | Reproduced with the liquid-cycles refusal: no entry, `calls = 0` |
| Register #9 trapped run without closing entry | still fixed, with exception L2-5 | Reproduced `Trapped` entry |
| Register #10 limit after await vs never-settling promise | still mitigated | `js.rs:385-389` names both causes |
| Register #11 trap before send leaves no entry | still accepted, accurately described | |
| Register #52 "proof of a trap" | still fixed | README "never proof of a trap"; `.did` text agrees |
| Register #53 `calls` counted attempts | still fixed | Reproduced: fetch + call gives `calls = 2`, refused calls not counted |
