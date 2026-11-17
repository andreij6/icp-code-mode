# Variant sweep r3-4: unauthenticated, unbounded writes

Run `2026-11-17-295d64b`, Phase 6, round 3. Pattern (from the confirmed demo_store
findings, not re-reported):

> Unauthenticated writes with no per-item or total bound, so storage and reply size grow
> with cheap anonymous requests.

Method: code reading at `295d64b` (nothing compiled or run, no live instance touched). Every
inventory row was asked: (a) can a caller with no credential (A1, A2, A8, A10) cause a write
that outlives the request, (b) is that write bounded per item and in total, (c) does any
reply or iteration grow with it. Round 2 swept the broader "cheap or unauthenticated
growth" pattern in `grid-sweep-r2-6.md`; this round re-checked its load-bearing cells
independently instead of trusting them, and looked for anonymous paths that sweep did not
name (canister-to-canister callers, demo_store reply echoes, query-path side effects).

Evidence key: **CP** = the code path makes it unambiguous; **ref** = cell taken from the
cited grid after re-reading the code it names.

Result: **no new candidate.** The only anonymous write path in the repository is demo_store
`create_order` / `cancel_order` / `restock`, and its growth (`orders`) is the known finding.
icp_code_mode takes no write before `authenticate`.

## Anonymous entry points (re-checked)

| Row | Anonymous write? | Bound | Verdict |
|---|---|---|---|
| E3 `inspect_message` (`src/lib.rs:273-283`) | none: decode + `authenticate`, then `accept_message`; inspect state is never committed | n/a | holds (CP) |
| E4/E4a/E4b `http_request` query (`src/lib.rs:76-96`) | none persists: query execution is discarded, including `OVERRIDES` inserts from `override_service` | reply: demo page fixed, 401/404/405 fixed, errors <= 300 B (LIM11) | holds (CP) |
| E5 `http_request_update` via ingress (A2) | refused by E3 unless the decoded body carries a registered token | n/a | holds (CP); a dishonest single replica skipping E3 still reaches only the next row |
| E5 via another canister (A8), no token | `mcp::http_update` calls `authenticate` first and returns 401 before `parse_body` and before any tool (`src/mcp.rs:378-385`); no audit entry, no cache write, no `PENDING_RUNS`/`ACTIVE_RUNS` change | n/a | holds for storage (CP); the execution cost is **known** ("Canisters without a token make icp_code_mode pay") |
| E5 attached cycles from A8 | no `msg_cycles_accept` in app code; unaccepted cycles are refunded | n/a | holds (CP) |
| E6-E11, E12-E14 | `guard = "controller"`; non-controller ingress refused by E3 | n/a | not cheap |
| E1/E2 | installer/controller | n/a | not cheap |
| E15 | not callable as messages | n/a | not applicable |
| E16-E19 | reachable only inside E5 after `authenticate` | n/a | not anonymous (token-holder growth: #31, Symbol.for, PENDING_RUNS all **known**) |
| E20 demo_store queries | none (queries) | `list_orders` and `sales_summary` grow with `orders`; `get_customer` returns a fixed-size `Customer` (no order list, `demo_store/src/lib.rs:231-234`); `search_products`/`list_products`/`get_product`/`get_inventory` read the seeded, non-growing vectors | **known** (same root cause as the seed) |
| E21 `create_order` | appends to `orders` (`demo_store/src/lib.rs:307`) | none per order (items count), none in total | **known** (seed). `customers`, `products`, `inventory` do not grow: unknown customer -> `UnknownCustomer`, unknown product -> `UnknownProduct`. The reply echoes the order, so its size is request-sized, not storage-sized |
| E22 `cancel_order` | flips status, `reserved.saturating_sub` | no growth | holds for this pattern (access control **known**) |
| E23 `restock` | `on_hand.saturating_add` (`demo_store/src/lib.rs:334`) | value saturates at u32::MAX, no growth | holds for this pattern (access control **known**) |
| E24 | fixed text | fixed | holds |
| E25 | heap reseeded on install/upgrade | resets `orders` | holds (it is also the only reclaim path for the known growth) |
| E26 demo page | writes one browser `localStorage` key on the viewer's own machine | one key | holds (retention **known**) |
| E27/E28 | local operator/developer tools | n/a | not anonymous (TMPDIR Chrome profiles **known**) |

## Other rows

| Rows | Verdict |
|---|---|
| C1, C1a, C3-C15 | No anonymous write path besides those above (C1 is gated by `authenticate`/controller guard in every build variant: the gate is in `mcp.rs`/`lib.rs`, not behind a language feature). C2 is the known seed |
| A1, A2, A8 | Cannot write C1 state (rows E3-E5 above). A9 (infrastructure) does not write app state; tampering is **known** (#47/#48) |
| A10 | demo_store writes: **known** |
| A3-A7, A11, A12 | Authenticated, controller, target or not built: outside "anonymous" |
| U1-U4 | Read only after `authenticate` for writes; on the query path they shape a fixed-size reply |
| U5-U11, U14-U16 | Reach storage only behind a token or controller (`grid-sweep-r2-6.md`, ref) |
| U12 | inspect decode, nothing stored (decode-trap gap **known**) |
| U13 | demo_store arguments: **known** |
| O1-O8, O10 | Every C1 effect is behind `authenticate` or the controller guard; O9 is demo_store (**known**) |
| D1-D7, D10 | Writers are controllers, token holders or the system; none anonymous |
| D8 | **known** (seed) |
| D9 | per viewer, one key |
| V1-V10 | Written only from E5 after `authenticate`, or from query executions that are discarded; V11 = D8 (**known**). Engine statics checked in `grid-sweep-r2-6.md` (ref) |
| K1-K12 | Commit points only exist in authenticated updates and demo_store updates (K12, **known**) |
| LIM1-LIM32 | No limit gates an anonymous store; the missing-kind list's demo_store order count is the seed |
| W1, W2 | Bounded by ingress size, nothing stored |
| W3-W14, W16 | Behind a token or controller, or browser-side; W15 is the seed |
| F1-F16 | F16 (`reserved` wraps) is **known**; no failure site writes anonymous data |
| B1-B17 | No conversion stores data across requests |
| VF1-VF8 | VF1 cost of unauthenticated canister calls **known**; VF6 is demo_store (**known**) |
| AD1-AD9 | Controller only |
| S1-S10 | Not applicable (no runtime write path) |
| CL1-CL37 | No claim says demo_store or C1 bounds anonymous writes beyond what was checked; CL16 (`inspect_message` before charging) is the ingress row above, whose canister-call gap is **known** |
| T1-T12 | No test exercises demo_store writes or an unauthenticated canister caller (inventory §17 no-test list; not a new gap) |

## Not tested

Nothing was run. The claim that a canister caller without a token writes nothing rests on
reading `http_update` and `authenticate`; no inter-canister probe was sent. Platform-side
growth (canister log ring buffer, ingress history) was taken as platform-bounded and not
measured.
