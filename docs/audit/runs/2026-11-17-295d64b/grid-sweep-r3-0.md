# Variant sweep r3-0: cost per accepted request, record per message or run

Run `2026-11-17-295d64b`, Phase 6, round 3. Pattern under test (from confirmed L5-1):

> Cost is incurred per accepted request but recorded per outbound message or tool run, so
> paid work that sends nothing leaves no trace (a record keyed to a different unit than the
> cost).

Method: code reading at `295d64b` plus the ic-cdk 0.20.3 call future
(`ic-cdk-0.20.3/src/call.rs:896-940`, `get_cost` at 323, liquid-balance check at 753). No
build, no live request; nothing deployed or changed. For each inventory row the question was:
does this row incur a cost (update execution, ingress bytes, outbound message, slot, shared
state change) whose record is written only when some *other* unit (a sent message, a policy
refusal, a tool run) occurs?

Known and not re-reported: L5-1 (authenticated update requests that run no tool; cached
searches in the update path incl. `certified_reads`), L5-2 / L2-6 (searches refused by a
remembered fetch failure), L5-3 (tokenless canister callers), L5-5 (demo_store), register
#11 (trap before first send), busy refusals (documented, CL19), upgrades leave no trace,
DID_FAILED caching transient/self-caused send failures, Trapped-entry undercounts.

## Grid

| Row(s) | Result | Reason |
|---|---|---|
| E1/E2 init, post_upgrade | n/a | Controller-initiated install; no per-request cost from other actors. "Upgrades leave no audit trace" is known |
| E3 inspect_message | holds | Runs at no charge; refuses tokenless ingress before cost |
| E4/E4a/E4b query path | holds | Queries are free to the canister on mainnet; `upgrade` only after `authenticate` |
| E5 http_request_update (non-tool messages, parse errors, revoked-token race, 1.9 MB padding around a tool call) | known (L5-1) | Paid per accepted request; entries exist only for tool runs. The non-tool part of a tool request (body parse, reply re-serialisation at `mcp.rs:405`, `finish`) is outside the entry's `instructions` window (`IcBridge::new` .. closing entry); the ingress byte fee is recorded nowhere. Same root as L5-1; the entry itself exists, so no new trace gap |
| E5a run_script | holds | `append_audit` at `lib.rs:246` is unconditional after `RunSlot::take`; a run that sends nothing (policy refusal, sync send failure, script error, oversized code, unknown language reaching `execute`) still gets a closing entry. Busy refusal (`lib.rs:219-221`) is the documented exception (CL19) |
| E5b run_search, cached / DID_FAILED / certified_reads | known (L5-1, L5-2, L2-6) | Condition `bridge.calls() > 0 \|\| !violations.is_empty()` (`lib.rs:161`) |
| E5b run_search, fetch passes policy, send refused by the system | **candidate SW3-0-1** | `reserve` takes a slot (`bridge.rs:486`), both `self.sent(...)` futures resolve `Ready(Err)` on first poll (`InsufficientLiquidCycleBalance` or `CallPerformFailed`), so `record_message` never runs and `calls()` stays 0; the error does not start with `policy:` so it goes to `fetch_errors`, not `violations`; `lib.rs:161` skips the entry. Yet `DID_FAILED` is written (`bridge.rs:504`) for every caller for 5 min. README:405 says a search that could not fetch closes with `Error` |
| E5b run_search, override that no longer parses (e.g. after a candid_parser change on upgrade) | same root as SW3-0-1 / L2-6 (edge, not separately reported) | `cached_service` returns `None` (`bridge.rs:210`, `r.ok()`), query upgrades; update `candid()` returns the raw override text with no send (`bridge.rs:475`), service `Ok(None)`, calls 0 -> no entry, on every search, with no TTL. Needs a parser behaviour change across an upgrade; not demonstrated |
| E5c reply/reject callbacks | holds | Work in a callback belongs to a run whose closing entry is unconditional (scripts) or already has a `Started` entry (searches that sent) |
| E5d cleanup callback | known | Trapped-entry undercount and cleanup-trap leak are known |
| E6-E11 admin updates | n/a for this pattern | Canister pays for controller ingress and policy changes write no audit entry, but the audit log documents runs, not admin actions (README:374), and the payer is the trusted actor. Not a record keyed to a different unit than a cost a third party imposes |
| E12-E14 admin queries | holds | Free as queries; replicated callers are L5-3 |
| E15 non-method exports | n/a | Not callable |
| E16-E19 script SDK (JS/Python/Rhai) | holds | Refused requests are recorded as violations (16 + "...and N more"); sent ones per message; replays and rounds are inside the run's `instructions`. Un-awaited calls (L2-2), dropped last-round replies and slot-vs-message counting are known |
| E20-E25 demo_store | known (L5-5) | No records of any kind; every accepted update paid by demo_store |
| E26-E28 demo page, setup script, video | n/a | Clients of E5 or dev tooling; the costs they cause land in E5 (covered above). Gemini calls in `video/tts.mjs` are developer-paid and outside the product |
| U1-U5 HTTP inputs | known (L5-1) | Bytes accepted per request, unrecorded unless a tool runs |
| U6-U7 script source, script values | holds | Inside a run with an unconditional entry |
| U8-U10 replies, rejects, interface text | holds / candidate | Replies belong to recorded messages; a fetch with no message is SW3-0-1 |
| U11 admin inputs | n/a | Controller-paid |
| U12 inspect_message bytes | holds | Free |
| U13 demo_store args | known (L5-5) | |
| U14-U16 | n/a | Browser, platform, dev tooling |
| O1 script calls | holds | Recorded at `on_first_pending`, closing entry unconditional |
| O2 interface fetch | **SW3-0-1** for searches; holds for scripts | Script closing entries cover a refused-at-send fetch; search entries do not |
| O3-O5 stable writes, certified data | holds | Each write is itself the record or admin-only |
| O6 HTTP responses | known (L5-1) | |
| O7-O10 | n/a | No cost to a third party / dev only |
| D1-D3, D5-D7 | n/a | Admin-written |
| D4 audit log | **SW3-0-1** | The log has no entry for a search attempt that changed shared state |
| D8 demo_store store | known (L5-5) | |
| D9-D10 | n/a | Client-side / disk |
| V1 ACTIVE_RUNS | holds | Busy is documented |
| V2 DID_CACHE | holds | Filled only after messages that were recorded (or from override text, no cost) |
| V3 PENDING_RUNS | holds | Keyed to `Started` entries, i.e. to sent messages, which is its documented unit |
| V4 DID_FAILED | **SW3-0-1** | Written on a zero-message fetch failure from a search with no entry; the transient-cause caching itself is known |
| V5 OVERRIDES | edge (see E5b override row) | |
| V6-V11 | n/a | No per-request cost or record |
| K1-K12 commit points | holds except SW3-0-1 | K9 (policy refusal before send) is recorded in the closing entry for both tools; the system-refusal-before-send case is recorded for scripts only |
| LIM1 call cap | known | Slots vs messages known; for searches the slot taken by a refused-at-send fetch leaves no record (SW3-0-1) |
| LIM2 instruction budget | holds | Violations recorded |
| LIM3-LIM32 | holds / n/a | Size and depth limits; refusals land in recorded runs or are free (query) or are L5-1 (`-32600`/`-32602` replies, batch errors) |
| W1-W2 | known (L5-1) / holds (free) | |
| W3 search render | known (L5-4) | |
| W4-W13 | holds | Inside recorded runs or admin |
| W14 | known (#31) | |
| W15-W16 | known (L5-5) / n/a | |
| F1-F16 | known (#11, cleanup traps) / n/a | |
| B1-B17 | holds / n/a | Conversions inside recorded runs or free paths |
| VF1 | known (L5-1) + **SW3-0-1** | |
| VF2 | holds | Queries free |
| VF3 | known (L5-3) | |
| VF4-VF5 | holds | Per-message and per-run records exist |
| VF6-VF8 | known (L5-5) / n/a | |
| AD1-AD9 | n/a | Controller-paid configuration |
| S1-S10 | n/a | No per-request cost |
| CL15, CL17 | holds | Entries per message sent; none for unsent |
| CL18 / README:405 "Error ... a search could not fetch an interface" | **SW3-0-1** | A search that could not fetch because the system refused the send writes no closing entry |
| CL19 | known (L5-1/L2-6) + SW3-0-1 | Third unlisted kind of no-entry search |
| CL1 "every run is audited" | known | |
| Other CL rows | n/a for this pattern | |
| T1-T12 | gap | No test drives `run_search` through `IcBridge` (register #58); the audit condition at `lib.rs:161` is untested for every no-message outcome |

## Candidate

### SW3-0-1 A search whose interface fetch is refused at send writes no audit entry but caches the failure for every caller (Low, code path)

- Where: `src/lib.rs:159-187` (condition at 161), `src/bridge.rs:424-437, 483-506`;
  ic-cdk 0.20.3 `CallFuture::poll` (`call.rs:899-922`).
- Path: a search reaches an uncached, unfailed, allowlisted canister. `reserve` passes and
  takes a call-cap slot. `fetch_did` builds `canister_metadata`; on first poll ic-cdk
  returns `Ready(Err(InsufficientLiquidCycleBalance))` when the liquid balance is below
  `cost_call` (which includes the max-response prepayment), or `Ready(Err(CallPerformFailed))`
  when `call_perform` refuses (queue full, out of cycles). `on_first_pending` never fires,
  so no `Started` entry; the fallback `__get_candid_interface_tmp_hack` fails the same way.
  `candid()` writes `DID_FAILED[key]` and returns an error without the `policy:` prefix, so
  it lands in `fetch_errors`. `calls() == 0` and `violations` is empty, so no closing entry.
- Outcome: for 5 minutes every script and search touching that canister, from any caller,
  gets "could not fetch candid for X (...)", and the audit log holds nothing that shows which
  request caused it or when. A script hitting the same refusal does get a closing `Error`
  entry (`lib.rs:246`), so the two tools disagree. README:405 says a search that could not
  fetch closes with `Error`; README:411-412 does not list this case.
- Reachability: needs the canister's liquid balance under the call cost or a full output
  queue. The known finding "Cycles reserved by in-flight calls and fetches have no global
  cap" shows a token holder can push the canister into that band.
- Refutation tried: (1) does `reserve` record the slot anywhere durable? No, `slots` is a
  `Cell` on the bridge. (2) Does the error reach `violations`? Only `policy:` errors are
  routed there (`lib.rs:139`), and `deny` is called only from `reserve` and the budget check.
  (3) Is `fetch_errors` enough to force an entry? No, it only selects the status inside the
  `if`. Not reproduced: needs a low-cycles canister (a settings change outside this phase)
  and `IcBridge` has no native harness (register #58).
- Root: the same condition as L2-6 (entry keyed to messages sent or policy refusals, not to
  attempts); the new part is that this outcome mutates shared state. A verifier may merge it
  into L2-6 as a further symptom.
- Fix: write the search's closing entry whenever a slot was taken (`slots > 0`) or a
  fetch error occurred, as `run_script` does unconditionally; or simply always write it in
  the update path.
