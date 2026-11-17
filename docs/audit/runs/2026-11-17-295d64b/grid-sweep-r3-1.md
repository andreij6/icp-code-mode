# Variant sweep r3-1

Run `2026-11-17-295d64b`, Phase 6, round 3. Pattern (seed, known, not re-reported: the
query-path search ignores `DID_FAILED`, upgrades, and the update path then sends nothing and
writes no entry):

**"Two equivalent paths (query and update) judge the same state differently. One escalates
to paid work, and the other then does nothing and records nothing."**

Question asked of every inventory row: is there a pair of paths that should reach the same
verdict on the same state (query `http_request` vs `http_request_update`, `inspect_message`
vs the method, `search_cached` vs `run_search`, `IcBridge::candid` vs `IcBridge::service`,
script vs search), where the cheap/first path ignores or misjudges some state, escalates
(upgrade to a paid update, a call-cap slot, a fetch), and the second path, honouring that
state, refuses or no-ops without an audit entry?

Evidence: reading the code at `295d64b` (`src/mcp.rs`, `src/lib.rs`, `src/search.rs`,
`src/bridge.rs`, `src/policy.rs`, `src/candid_bridge.rs`, `src/runtimes/mod.rs`). Nothing
compiled, deployed or sent: every hit below is unambiguous from the code path. C = code path.

## New candidates

| Id | Sev | Title | Ev |
|---|---|---|---|
| SW3-1-1 | Low (may merge with L5-1 / L5-2) | The query path upgrades every `execute_icp_script` without looking at `ACTIVE_RUNS`; when 8 scripts are running, the paid update then answers "busy ... retry shortly" and writes no audit entry. The query could have refused it for free, and the error text invites the client/AI to retry, each retry another paid, unaudited update | C |

### SW3-1-1 (busy refusal is judged only after a paid upgrade)

- Query: `http_query` (`src/mcp.rs:366`) returns `HttpResponse::upgrade()` for any
  `ToolRequest::Execute`, consulting no state. `ACTIVE_RUNS` is a heap `thread_local`
  (`src/lib.rs:200`) that a query can read (`get_policy`, a query, already reads it at
  `src/lib.rs:341`).
- Update: the gateway sends `http_request_update`; `inspect_message` lets it in (valid token,
  `src/lib.rs:276-277`), so the canister pays induction and execution. `run_script`
  (`src/lib.rs:219-221`) finds `RunSlot::take()` is `None` and returns
  `busy: 8 scripts are already running; retry shortly` before creating an `IcBridge`; no
  `append_audit` runs on this branch.
- Scenario: a token holder keeps 8 runs in flight (any long or unbounded-wait script; the
  slot pinning itself is known). Every other `execute_icp_script` from any client is then
  upgraded, paid by the canister (cost class measured in L5-1: a few M cycles for a small
  body, up to ~4B for a 1.9 MB body, which `parse_body` accepts before the tool runs), and
  answered "busy" with no record. The message tells the AI model to "retry shortly", so an
  agent loop multiplies the paid, unrecorded updates.
- Same shape as the seed: query ignores state the update honours (`DID_FAILED` there,
  `ACTIVE_RUNS` here); update no-ops and records nothing. The "no entry" part is documented
  (README:411-412); the escalation is not, and the cost lands on the canister.
- Refute attempts: (1) Could the query not know? It reads the same committed heap; a run
  started after the query's state would only make the query optimistic, never wrongly
  pessimistic beyond a race. (2) Is it covered by L5-1? L5-1 is "authenticated update
  requests that run no tool" (ping/initialize/invalid bodies, mostly direct ingress); this one
  is the normal gateway flow for the main tool, and a tool call is made. Listed as a possible
  merge. (3) Is it free on the IC? No: upgraded calls are ordinary ingress updates paid by the
  receiving canister.
- Fix direction: in `http_query`, answer the `Execute` with the same busy tool result when
  `ACTIVE_RUNS >= MAX_CONCURRENT_RUNS` (non-authoritative pre-check; the update keeps its own
  check), or record busy refusals.

## Grid (pattern: query/update or pre-check/act judge the same state differently)

| Rows | Result |
|---|---|
| E4, E4b, E5, E5a | **SW3-1-1** (`ACTIVE_RUNS`). Other states an Execute can meet in the update (code over 100 KiB, language not compiled in, `max_calls_per_script = 0`, `max_instructions = 0`) are also not judged by the query, but `run_script` always writes a closing entry, so the escalation is recorded: holds for "records nothing" |
| E4b, E5b, V4 | known seed (`DID_FAILED` ignored by the query); not re-reported |
| E4b, E5b, V5, D5, AD7 | latent, not a hit at this commit: `cached_service` turns an override parse error into `None` (query upgrades) while `run_search` calls `IcBridge::candid`, which returns the override text unparsed, then `cached_service` is `None` again, so the entry renders "interface not fetched", no fetch, no audit. Unreachable today because `set_candid_override` validates with the same `Service::parse` and the parser is deterministic; becomes reachable if an upgrade makes `candid_parser` stricter than the version that accepted a stored override. The 5B-query-limit variant is known |
| E4b, E5b, V2, LIM18 | holds: both paths use `cache_get` with the same TTL; the update's block time is never earlier than the query's state time, so an expired entry is expired in both and the update fetches (audited). Expired + failed is the known re-fetch finding |
| E4b, E5b, LIM2 | known (query search has no budget, L5-4); in the update a budget overrun is a recorded violation |
| E4b, E5b, LIM1 | holds: with `max_calls_per_script = 0` the query upgrades an uncached search and the update records a `PolicyViolation` |
| E4b, E5b, AD2 (empty method list) | not the pattern: an allowlisted canister with `[]` methods makes the query upgrade and the update fetch an interface none of whose methods can be shown; the fetch is paid but audited (wasted work, recorded) |
| E4b, E5b (keyword filter) | not the pattern: the query cannot tell whether an uncached interface matches, so it upgrades; the update fetches and audits |
| E4b, LIM9 | known (MAX_SEARCHES in update only) |
| E4, LIM4, CL28 | known (certified_reads upgrades ping/initialize/tools/list and cached searches to updates that record nothing: L5-1, SW10-5) |
| E4a, A1 | holds: the demo page and 401/404/405 are answered in the query and never upgraded |
| E3, E5, B2, U12 | known (inspect_message vs method decode quota; requests that pass inspect_message and run no tool: L5-1) |
| E5a vs E16-E19 (`icp.candid` vs `icp.call` on a remembered failure) | not the pattern: `IcBridge::candid` checks `failed_get` before `reserve`; `call` reserves a call-cap slot first and then fails in `service()` on the same `DID_FAILED` entry, so the two judge it in different order and `call` burns a slot. No message and no extra cost follows, and the run's closing entry records the error: holds |
| E5c, E5d, K1-K12 | n/a: commit points have no query twin; the trap-in-cleanup cases are known |
| E6-E14, AD1-AD9, U11 | n/a: admin methods have one path each (controller guard on update and query alike); `get_audit_logs` as a query vs an update read (CL21) does not escalate anything |
| E1, E2 | n/a: lifecycle, single path |
| E15 | n/a |
| E16-E19, B8-B10, W8-W11 | n/a for query/update; cross-language differences are known (L6) and none escalates to paid work that another runtime refuses silently |
| E20-E25, C2, W15, VF6 | n/a: demo_store queries never escalate; its updates have no pre-check twin (no inspect_message) |
| E26-E28, C5, C6, C12, B17 | n/a: client tooling; the demo page posts to `/mcp` and follows the same query/upgrade flow (covered by E4/E5) |
| C1, C1a, C3, C4, C7-C11, C13-C15, S1-S10 | n/a: build and supply-chain rows have no request paths |
| A1-A12 | covered through the entry-point rows above (A3/A4 via SW3-1-1) |
| U1-U16 | U1-U5: same `authenticate`/`parse_body`/`classify` in both paths, so the verdict is identical except for the rows above; U6-U16: n/a (no query twin) |
| O1, O2 | covered under E5a/E5b; the synchronous send-refusal no-entry case is the known L16-12 symptom (SW10 additional symptoms) |
| O3-O10 | n/a |
| D1-D10 | D1 (`certified_reads`) known; D4 audit is the "records" side checked above; others n/a |
| V1 | **SW3-1-1** |
| V3, V6-V11 | n/a: not consulted by either path's escalation decision |
| LIM3, LIM5-LIM8, LIM10-LIM17, LIM19-LIM32 | n/a: enforced once on one path, or no escalation hinges on them; LIM10/LIM12 are the update-side checks the query does not mirror (LIM12 = SW3-1-1; LIM10 refusal is -32600 after the first script, which does run and is audited) |
| W1-W7, W12-W16 | W3 known (L5-4); others n/a |
| F1-F16 | n/a |
| B1-B7, B11-B16 | n/a beyond rows above |
| VF1-VF8 | VF1: SW3-1-1 adds a paid path; VF3 known (L5-3); others n/a |
| CL1-CL37 | CL19 (busy writes no entry) is accurate but omits that the busy refusal is reached only through a paid upgrade; CL3/CL2 known |
| T1-T12 | no test drives `http_query` with all run slots taken, or compares a query verdict with the update verdict on the same state (known testability item "No test compares the query route with the update route") |

## Not tested

- SW3-1-1 was not reproduced live: holding 8 run slots on the shared local network is a
  Live-phase action. The cost figure is borrowed from L5-1's measurement of an update that
  runs no tool; the busy branch does strictly less work than that plus one `RunSlot::take`.
