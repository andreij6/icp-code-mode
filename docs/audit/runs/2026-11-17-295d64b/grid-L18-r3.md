# Coverage grid: L18 Operations, round 3 (exhausted-cycles fault injection)

Run `2026-11-17-295d64b`. Lens question: can this be run safely for a year? Focus of this
round: what happens to suspended runs, records, run slots and pending ids when the canister
crosses its freezing threshold, what token holders and the AI see, and whether pinned calls,
searches or memory growth can push a canister near the threshold into freezing.

## Method

- Network: the project's local icp-cli network on port 8010 (icp-cli 1.7.0), shared with other
  audit agents running at the same time (audit log grew from 5,747 to 9,845 entries during this
  round, about 3,700 of them mine). Module hash `0x224cfa7a...ec769`. Nothing was deployed.
- Fault injection: the canister's balance (about 15-17T cycles) could not be drained, so
  "exhausted cycles" was produced by raising `freezing_threshold` (a) above the balance
  (frozen) or (b) to `(balance - margin) / idle_burn_per_second` (a chosen liquid margin).
  Caveat: a large threshold in seconds magnifies how much a memory increase raises the
  threshold in cycles, so memory-growth results below are about the rule, not the size of
  the band at the default 30 days.
- Every threshold change was restored to `2_592_000` by a shell `trap` within 1-8 s; final
  state checked: threshold 2,592,000, `pending_run_ids = vec {}`, `active_runs = 0`, limits
  25 / 20_000_000_000 / null / null (unchanged; `set_limits` was not called).
- Client: MCP JSON-RPC over the gateway (`Host: icp_code_mode.local.localhost:8010`, token
  from the `MCP_TOKEN` line of `.env`, never printed). Scripts in the scratchpad.
- Side effects left on the shared deployment: audit log +~17 MB (filler runs, needed to
  place a stable-memory bucket boundary), one extra 8 MiB stable bucket (memory_size
  372,002,600 -> 380,391,208; buckets are never released). No slot or pending id leaked.
- Evidence keys: **R** reproduced on the local network (exact request and response below),
  **C** the code path produces it unambiguously.

## Experiments

| # | Setup | Observed |
|---|---|---|
| E1 | JS run: 12 sequential `get_inventory` calls with ~1.35B instructions of work before each; threshold raised above the balance 1.2 s after start (the run was suspended between calls) | Callbacks kept executing while frozen. Calls 1-6 sent; calls 7-12 refused on the spot: `insufficient liquid cycles balance, available: 0, required: 42102440000`. Client got HTTP 200 with the result. Entries 5784 (`Started`) ... 5789 (`Call`), closing 5790 `Ok`, `calls = 6`. After restore: `pending_run_ids = vec {}`, `active_runs = 0` (R) |
| E1b | While frozen: `get_policy`, `get_audit_log_len` (queries), `icp canister status`, MCP `tools/list` (query path) and `tools/call` | All refused. Queries: `Canister ... is out of cycles: please top up the canister with at least 60_950_667_377_355 additional cycles`; `canister_status` via icp-cli refused the same way; MCP: HTTP 503 `{"error_type":"canister_frozen","description":"The canister has run out of cycles. You or others can top it up to restore functionality."}` (not a JSON-RPC body) (R) |
| E2 | As E1 (8 calls), then after the last call a catastrophic regex (`/^(a+)+$/` on 45 `a` + `!`); frozen 0.7 s after start | 6 calls sent, 2 refused, then the callback hit the 40B instruction limit while frozen. Cleanup ran: entry 5805 `Trapped`, `calls = 6`, methods listed. Pending and slot released. Client: HTTP 503 `canister_error` with the instruction-limit text (R) |
| E3 | Margin 150B (threshold 432,907,391 s); JS run allocating 300 x 1 MiB strings, no calls | Heap growth refused: 503 `Canister cannot grow memory by 16842752 bytes due to insufficient cycles`. memory_size unchanged (98,520,872). No audit entry (nothing sent; K8). Next request at the same margin: call refused with `call perform failed`, closing entry 5849 `Error` with that text (R) |
| E4 | Margin 20B, then 60B (threshold set from live balance and burn) | 20B: every update refused with IC0207 `Canister ... is out of cycles`, shown by the gateway as HTTP 503 `backend_other`, "temporarily unable to process the request. Please try again later"; `tools/list` (query) 200, `get_audit_log_len` answered. Canister status `Running`, balance above the threshold. 60B: a no-call script ran; a 1-call script got `insufficient liquid cycles balance, available: 12473539518, required: 42102440000`; a cached search answered. So one update execution holds about 47.5B and each outbound call 42.1B (R) |
| E5 | Audit log filled (filler runs, normal threshold) until the next 8 MiB bucket of the audit data memory was due, located by decoding every entry's Candid size (`get_audit_logs -o hex`) and by memory_size jumps; then margin 260B and a run whose closing entry needed the new bucket (16 refused 256-byte methods + 1 call, `PolicyViolation` closing entry 5,532 B) | Stable growth was **not** refused: the run returned HTTP 200 `"1"`, its closing entry was written, memory_size grew by exactly 8,388,608, and the canister was then frozen: the next `get_policy` returned `out of cycles: please top up the canister with at least 48_880_651_743 additional cycles`. Compare E3, where Wasm heap growth at a smaller margin was refused (R) |
| E6 | Two more runs at margin 260B after E5 | Normal: no growth, no freeze, pending/slots clean (R) |

Byproduct (already reported, not repeated): two pre-#16 audit entries (ids 36 and 60) are
8,388,879-byte replies each and can never be returned by `get_audit_logs` (reply limit
2,097,152), so every page that includes them fails; the reader has to page around them with
`limit = 1`.

## Cells

| Row | Verdict | Reason / evidence |
|---|---|---|
| VF1 cycles paid by the canister | **finding L18-r3-1, L18-r3-3**; holds for calls, execution and heap | Holds: an outbound call can never take the balance below the threshold (ic-cdk liquid pre-check, then the replica's `call_perform`, E3/E4), and an update cannot start without ~47.5B liquid (E4); Wasm heap growth that would cross the threshold traps instead (E3). Finding r3-1: audit-log (stable) growth is allowed to cross it, so an ordinary run froze the canister (E5). Finding r3-3: cycles reserved by in-flight calls (42.1B each) take the canister into a state where every update is refused while it is not frozen (E4) |
| D7 canister settings / cycles | **finding L18-r3-2** (plus r3-1, r3-3) | Default freezing threshold 30 days, no setting in `icp.yaml`. While frozen the controller cannot read anything: queries and `canister_status` are refused (E1b); the only figure available is the top-up amount in the rejection text. The known L18-2 (no signal before freezing) is not repeated |
| K1 first-pending commit | holds (R) | Frozen or near the threshold, a refused send resolves immediately with an error and writes no entry; `calls` equals the messages that left (E1: 6 entries, 6 refusals not logged; E3: `calls = 0`). If the `Started`/`Call` append itself needs a new stable bucket, growth succeeds (E5 mechanism), so the message and its entry are not rolled back by the freezing threshold |
| K6 closing entry | holds for the freezing threshold (R); r3-1 | A run suspended across the freeze writes its closing entry from a frozen canister (E1, entry 5790). A closing entry that needs a new 8 MiB bucket near the threshold is written and the growth then freezes the canister (E5) instead of trapping, so the F4 `expect` (L2-5, already reported) is not reachable through cycles on this replica. Its other triggers (storage reservation / `reserved_cycles_limit`, subnet memory, 256 GiB) were not tested |
| K7 cleanup after a trap | holds (R) | Cleanup ran in a frozen canister after a 40B-instruction trap: `Trapped` entry 5805 with the six methods, pending id removed, slot returned (E2) |
| CL18 "a script run ends with exactly one of these" | holds under freezing (R) | E1 (`Ok`), E2 (`Trapped`), E3 (`Error`), E5 (`PolicyViolation`): one closing entry each. A heap-growth refusal before any send leaves no entry, which the README covers ("not in the log at all") |
| CL20 "pending or unknown, never proof of a trap; a trap is recorded as Trapped" | holds (R); r3-2 | Trap while frozen recorded as `Trapped` (E2). But `pending_run_ids`, the tool the README gives for telling pending from unknown, cannot be read while the canister is frozen (E1b) |
| I3 records match reality | holds under freezing (R) | No entry for refused sends; a closing entry or `Trapped` for every run that sent (E1, E2, E5) |
| I5 clean failure | partly; see r3-2, r3-3 | Requests refused while frozen or liquid-starved get gateway HTTP 503 bodies, not JSON-RPC responses; at the liquid floor the body calls a cycles shortage "temporary" (E4) |

## Candidate findings

### L18-r3-1 Audit-log growth may cross the freezing threshold: an ordinary run froze the canister (Low)

- Invariant: I4 (the party that asked pays, or the cost is refused before it is incurred), I5.
- Where: `src/policy.rs:247-254` (`append_audit` -> `StableLog::append`), ic-stable-structures 0.7.2
  `memory_manager.rs:340-380` (a new 8 MiB bucket grows stable memory by 128 pages); replica
  behaviour: stable growth is not checked against the freezing threshold, while Wasm heap growth is.
- Evidence: R (E5). At a 260B liquid margin, a script run whose closing entry needed the next
  bucket succeeded (HTTP 200), memory_size went 372,002,600 -> 380,391,208, and the canister
  was frozen right after (`please top up ... at least 48_880_651_743`). Heap growth at a
  smaller margin was refused instead (E3).
- Scenario: a canister running down its cycles gets frozen by a token holder's audited request
  (scripts and fetching searches append entries) at the moment the log needs a bucket, instead
  of the request being refused. Every 8 MiB of log raises the threshold permanently, since
  buckets are never released (L18-1). At the default 30 days the jump per bucket is about
  2.6B cycles at the mainnet storage rate and 6.4B at the local rate (arithmetic), so the band
  is narrow; graded Low.
- Fix: check `canister_liquid_cycle_balance` against a floor before audited work and refuse
  with a clear error; size the floor to cover a bucket; expose the balance (L18-2).

### L18-r3-2 While frozen, nothing can be read: audit log, pending runs and status are refused (Low)

- Invariant: I10 (the README tells operators to read `get_policy().pending_run_ids` and the
  audit log to judge runs), observability.
- Where: README.md:414-426 and the "immutable audit log" claims; `get_policy`,
  `get_audit_logs`, `get_audit_log_len` (`src/lib.rs:346-350`); no export path (L18-1).
- Evidence: R (E1b). All three queries, `icp canister status` and both MCP routes were
  refused while frozen; the rejection text carried only the top-up amount.
- Scenario: the canister freezes; the operator can neither check which runs were in flight
  nor copy the log before deciding whether to fund it. If it is left frozen until the balance
  reaches zero it is uninstalled (platform rule, not tested) and the log was never
  exportable. Overlaps L18-1 (no export) and L18-2 (no signal); merge if the verifier finds
  the same root cause.
- Fix: document that a frozen canister answers nothing and must be topped up first; provide an
  export/archive path (L18-1) so the log exists outside the canister before it is needed.

### L18-r3-3 At the liquid-cycles floor every update is refused as "temporary" while the canister reports Running (Medium)

- Invariant: I5 (documented refusal), I10; operator misled.
- Where: `src/bridge.rs:521-528` (unbounded wait by default, 42.1B reserved per in-flight
  call, no global cap on in-flight calls), `src/lib.rs:121-190` (searches not slot-capped,
  each fetch reserves the same); no liquid check or floor in the app.
- Evidence: R (E4) for the state: with liquid margin 20B (balance above the threshold, status
  `Running`) every update was refused with IC0207 and the gateway answered HTTP 503
  `backend_other` "temporarily unable ... try again later", while `tools/list` and audit
  queries still answered. With 60B one update ran and its call was refused
  (`available: 12473539518, required: 42102440000`). How pinned calls reach that state is
  the reservation arithmetic (C, plus L9-r2's live measurement of 42.8B held per hung call):
  up to 8 runs x 25 calls = 200 unbounded calls = about 8.4T cycles held indefinitely by
  one token holder with a never-replying allowlisted target; searches add more, uncapped,
  for each fetch's timeout. The reservation can never cross the threshold, but it does not
  need to.
- Scenario: a canister holding less than about 8.5T above its threshold is wedged: no script
  or uncached search runs, the platform status shows Running with a healthy-looking
  balance, the AI is told to retry, and the cycles stay held until the target answers (for
  unbounded calls, possibly never; the canister also cannot be stopped, register #44).
- Fix: a global cap on in-flight calls and fetches, a liquid-balance floor checked before
  accepting work, `call_timeout_seconds` on by default, and the liquid balance in
  `get_policy`. May merge with L9-r2-1 (cycles held by in-flight calls) as the same root cause.

## Not tested

- A real drain to zero (uninstall, outstanding calls rejected): would take days of idle burn
  or deleting cycles; only the platform rule is known.
- Pinned calls and search floods actually holding cycles: needs a never-replying target
  (L9-r2's was deleted; no Wasm build in this round). The resulting state was reproduced by
  margin instead (E4).
- Stable growth failure for other reasons (storage reservation with `reserved_cycles_limit`,
  subnet memory full), which would reach the F4 `expect` (L2-5): not reproducible on a
  local replica with an empty subnet.
- Mainnet: rejection texts, gateway bodies, whether `canister_status` is refused for a frozen
  canister, and storage rates may differ from the local replica.
