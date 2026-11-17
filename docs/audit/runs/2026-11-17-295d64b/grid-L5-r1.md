# Coverage grid: L5 Cost and payment, round 1

Run `2026-11-17-295d64b`. Lens question: who pays, and can someone else make them pay?
Row ids refer to `inventory.md`. Candidate ids (L5-1 ... L5-6) are listed at the end.

Evidence keys: **R** = reproduced (live on the local icp-cli network, port 8010, or a native
test in a private copy); **C** = the code path produces it unambiguously.

Live measurements: cycle balance of `icp_code_mode` read with `icp canister status` before and
after each call, one call per trial, at least three trials, minimum taken (other agents were
using the canister at the same time, so single readings can be inflated). One status read costs
the canister 1,944,000 cycles (measured: constant over 6 control trials) and is subtracted.
No settings were changed; no state other than demo_store test data in a native test copy was
written. Calls that carried the test token read it from `.env`; it was never printed.

| Row | Cell | Reason / evidence |
|---|---|---|
| VF1 cycles for update executions, messages, stable writes | **finding** L5-1, L5-2, L5-4 | The canister pays for every update it accepts. Measured: authenticated `ping` sent straight to `http_request_update` costs 8.0M cycles (small body) and 3.95B cycles (1.9 MB body, ~2,080 cycles/byte), returns 200 and writes no audit entry (R). A search with 925k matching keywords in the update path costs 8.69B cycles, writes no entry (R) |
| VF2 query calls free | holds | Queries are not charged to the canister (measured: a 1.8 MB search query that hit the 5B query instruction limit and a 0.3 MB one cost the canister nothing beyond the status read). Assumes mainnet keeps queries free |
| VF3 calls from other canisters | **finding** L5-3 | A canister without a token makes `icp_code_mode` pay 5.65M cycles per `http_request_update` call (401 reply), 5.19M per admin-method call (guard reject) and 7.14M per replicated `http_request` call (R, through the local proxy canister `4ld2s-...`). The comment "they pay for themselves" and README "before the canister is charged" are wrong for this path |
| VF4 assets held by C1's principal | holds (cycles) / n.a. (ledger funds) | No call attaches cycles (`grep` finds no `with_cycles`, `msg_cycles_*`), so no script can move the canister's cycles. Moving tokens held on a ledger needs a controller to allowlist a transfer method; all tokens act as one principal (register #49, accepted) |
| VF5 per-run work x concurrency x time | **finding** L5-4 (search path); holds per script run, open over time (#31) | Script runs: `max_instructions` checked before each host request and between rounds, so one run costs at most about the budget plus one execution's overshoot (exact overshoot is L3's). Search runs: the budget is checked only after `search::render`, so render work is bounded by the ingress size and the platform limit, not by policy (R: 12B instructions with 13 allowed methods). No rate limit across requests: #31 still open |
| VF6 demo_store stock reservation | **finding** L5-6 | Duplicate lines in one order are each checked against the same `reserved`, so `reserved` exceeds `on_hand` (R: native test, product 1 on_hand 21, reserved 1 -> 41). With `restock(p, u32::MAX)` the `reserved +=` add wraps in release (C) |
| VF7 factory fees | not applicable | Not built (C14 is an idea document) |
| VF8 Gemini API usage | not applicable | Developer tooling, developer's own key; not reachable by any actor of the deployed system |
| E3 `inspect_message` | holds for ingress | R: 6/6 tokenless ingress calls to `http_request_update` rejected at zero cost; 5/5 anonymous ingress calls to `remove_allowlist` rejected. It does not look at the body, so an authenticated caller passes with any 2 MiB payload (L5-1). Does not apply to inter-canister calls (L5-3, documented in the code comment but with a wrong cost claim) |
| E4 / E4a / E4b query path | holds | Free to the canister. `upgrade: true` is returned only after `authenticate` succeeds, so an unauthenticated caller cannot make the gateway issue a paid update. Exception: a search on a canister whose fetch failed recently is upgraded every time (L5-2) |
| W3 search keyword matching | holds in the query path (free; a 1.8 MB query trips the 5B query limit and returns 503 at no cost); **finding** L5-4 in the update path | |
| E5 `http_request_update` | **finding** L5-1 | Any authenticated request is executed and paid for; only tool runs are audited. `ping`, `initialize`, `tools/list`, notifications, parse errors, cached searches and failed-fetch searches leave no record |
| E5a `execute_icp_script` | holds per request | Always audited (closing entry), so its cost is attributable; busy refusals unaudited by design (CL19) and cheap |
| E5b `run_search` | **finding** L5-2, L5-4 | Audited only when a message was sent or policy refused |
| E5c/E5d callbacks | holds | Paid by the canister but only as continuations of an accepted, audited run |
| E6-E11 admin updates | holds for ingress (R); **finding** L5-3 for canister callers (R: 5.19M cycles per rejected call) |
| E12-E14 admin queries | holds | Queries are free; called as inter-canister updates they cost like L5-3 |
| E20-E24 demo_store | **finding** L5-5, L5-6 | No access control and no `inspect_message`: anyone's ingress `create_order`/`cancel_order`/`restock` is paid by demo_store |
| E25 demo_store lifecycle | holds | Heap re-seeded on upgrade; the only recovery path for L5-5 |
| A1 anonymous web user | holds | Page and 401/404/405 are query answers, free |
| A2 anonymous direct ingress | holds (R) | Rejected by `inspect_message` before charge (register #30 still fixed) |
| A3 token holder | **finding** L5-1, L5-2, L5-4; #31 open | |
| A8 other canisters | **finding** L5-3 | |
| A10 anyone on demo_store | **finding** L5-5 | |
| D4 audit log growth | open (#31, re-checked) | No rate limit, no pruning, no size cap: audit grew 687 -> 834 entries during this lens's session from other agents' normal use. Worst case per request: a script with 25 slots can write up to 51 message entries (an interface fetch can send 2 messages per slot) plus a closing entry; 5 searches add up to 2 entries per uncached allowlisted canister each. When stable memory cannot grow, `append_audit` traps (F4), which ends every audited request |
| D8 / V11 demo_store `STORE` | **finding** L5-5 | Grows per order from anyone, unbounded lines per order |
| W2 inspect decode + SHA-256 | holds | Runs on one replica, not charged to the canister |
| W14 PENDING_RUNS | holds | Heap set of u64, entries removed on run end or trap cleanup; bounded by runs in flight (8 scripts with unbounded wait; searches use bounded-wait fetches, at most 300 s) |
| W15 demo_store orders | **finding** L5-5 | |
| LIM8-LIM10 per-request caps | holds | Bound tool work per HTTP request; do not bound requests over time (#31) |
| LIM12 run slots | holds for scripts | Searches take no slot; their cost per request is bounded by LIM9 and L5-4 |
| LIM2 `max_instructions` | **finding** L5-4 (not applied to search rendering until afterwards) |
| I4 bounded cost | violated | #31 (open), plus L5-1, L5-4 (no record / no policy bound), L5-3 (tokenless canister callers), L5-5 (demo_store) |
| I18 tokenless updates refused before payment | holds for ingress (R); violated for inter-canister calls (L5-3) |
| CL2 / CL3 / CL36 search "fast and free", "cheap" | **finding** L5-2, L5-4 |
| CL16 unpaid-call filter | **finding** L5-3 (wording covers all update calls; only ingress is filtered) |
| CL19 "two kinds of request write no entries" | **finding** L5-1, L5-2 |
| Register #30 | still fixed for ingress (R); variant through canister callers is L5-3 |
| Register #31 | still open (C: no rate limit or pruning in `src/policy.rs`, `src/mcp.rs`) |

## Candidates

- **L5-1** (Medium, R): authenticated update requests that run no tool cost the canister up to ~4B cycles each (1.9 MB body) and write no audit entry; a search in the update path up to 8.7B. README:411 says only two kinds of request write no entries.
- **L5-2** (Medium, C): a search touching an allowlisted canister whose interface fetch failed in the last 5 minutes is always upgraded to a paid update and writes no audit entry; the tool description tells the AI model searches are free and use no consensus round.
- **L5-3** (Low, R): any canister, without a token, makes icp_code_mode pay 5.2M-7.1M cycles per call to `http_request_update`, admin methods or `http_request`; the README and code comment say tokenless update calls are refused before the canister is charged / callers pay for themselves.
- **L5-4** (Medium, R): search rendering cost grows with keyword count x allowed methods and is checked against `max_instructions` only afterwards; 925k keywords over 13 methods = ~12B instructions, 8.69B cycles, reported as success, no audit entry.
- **L5-5** (Medium, R native + C): demo_store accepts zero-quantity lines from anyone, so one ~2 MB `create_order` stores ~170k lines; two such orders make `list_orders(null)` a 4.08 MB reply (over the IC reply limit), and repeated orders exhaust its heap; demo_store pays for every call.
- **L5-6** (Low, R native): demo_store oversells: duplicate lines in one order pass the availability check against the same stock; `reserved` wraps with `restock(p, u32::MAX)`.

## Not tested

- Cycles reserved for in-flight calls (response prepayment) held by 8 scripts x 64 pending unbounded-wait calls against a target that never replies: needs a misbehaving target canister (register #55); not deployed in this phase.
- Cost to a purpose-built attacking canister per call (only the generic proxy's cost was measured, 13.3M per call), so the amplification factor of L5-3 is not established.
- Mainnet fee schedule and gateway behaviour (register #57); local fees match a 13-node subnet only by observation.
- The search 40B-trap extrapolation in L5-4 (needs an allowlist of about 45+ methods, i.e. a settings change reserved for the Live phase).
- Duplicate-line oversell and the 170k-line order were reproduced natively only (demo_store live data is shared with other agents and was left untouched).
