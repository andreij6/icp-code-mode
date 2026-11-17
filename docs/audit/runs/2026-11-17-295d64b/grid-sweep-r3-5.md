# Variant sweep r3-5: per-item check against batch-changed state, plus unchecked counter arithmetic

Run `2026-11-17-295d64b`, Phase 6, round 3. Pattern (seed: demo_store oversells because
duplicate lines in one order are each checked against the same stock, and `reserved +=`
wraps in the release Wasm; both known, not re-reported):

> A check made per item against state that the batch itself changes, combined with
> unchecked arithmetic on a counter.

Searched in its two halves and their combination: (a) a loop or batch that validates each
item against a shared quantity (stock, slots, budget, bytes, count) that earlier items of the
same batch consume, without the batch's own consumption being counted; (b) `+`, `+=`, `-`,
`*` on a counter in non-test code where the release profile (overflow checks off) wraps
instead of trapping; (c) a consumer of such a counter that reports the wrapped value.

Method: `grep -n "+= \|+ 1\|-= \|saturating\|checked_\|wrapping\|MAX_"` over `src/`,
`src/runtimes/`, `demo_store/src/`, `shims/`, then reading every hit and every loop that
checks a limit per item. One native reproduction (R) in a private copy of the tree with the
shared target dir: `cargo test -p demo_store --release --lib` (release profile = the shipped
overflow semantics). The copy's only source change besides the added test was
`created_at: ic_cdk::api::time()` -> `0` in `create_order` (ic0 is unavailable natively).
The copy was deleted afterwards. No running instance was contacted.

C = code path, R = reproduced.

## Cells

| Row(s) | Result | Reason |
|---|---|---|
| E21 `create_order` stock check | known (seed) | Per-line `quantity > on_hand - reserved` against pre-batch state; `reserved += q` wraps in release (R: `reserved_after` 4,294,487,298 after 160k lines of 4,294,967,293) |
| E21 `create_order` `total += cents * q` (u64) | holds | Max per order: ~174k lines fit in 2 MiB ingress (12 B each); 174,762 x 12,900 x (2^32-1) = 9.68e18 < u64::MAX (C). R: largest order total 8.86e18 with 160k lines |
| E20 `sales_summary` `revenue += line`, `e.2 += line` (u64) | **finding SW3-5-1** | Sums every non-cancelled order. Unreachable without the oversell (12 products x u32::MAX units x 38,850 cents total price = 1.67e14), reachable with it: five orders of 160k duplicate lines (one product each, after `restock(p, u32::MAX)`), each accepted, make the exact revenue 1.94e19; `sales_summary` reports 9.32e17 in the release build (R) |
| E20 `sales_summary` `count`, `e.1` units (u64) | holds | Needs ~4e9 orders / units beyond u64; not reachable |
| E22 `cancel_order` `reserved.saturating_sub(q)` | holds as written; consequence of the seed | Saturation hides the seed's wrap: after an oversold order, cancelling sets `reserved` to 0 while other pending orders still hold stock, so `available` is overstated. Same root cause as the known oversell/wrap; not a new finding |
| E23 `restock` `saturating_add` | holds | Saturates at u32::MAX; no wrap |
| D8/V11 demo_store `orders.len() + 1` ids | holds | `usize`/`u64`, unreachable |
| E5 `http_update` batch: 1 script + up to 5 searches per HTTP request, each with its own instruction budget | **finding SW3-5-2** | Each tool call's budget is measured from its own `IcBridge` start (`bridge.rs:318`, `instructions_used`), and `run_search` checks it only after render (`lib.rs:150-154`). Cached searches never await, and a script with no `icp` calls never awaits, so all items of one request run in the same message execution, whose 40e9 limit is shared. No check bounds the sum: items that each pass their own check (search <= 20e9, Rhai script stopped at 20e9) trap the message together (C) |
| LIM9 `MAX_SEARCHES`, LIM10 one script per request | holds | Counters `searches`/`ran_script` updated per item before the next item is classified (`mcp.rs:395-400`) |
| LIM8 `MAX_BATCH`, LIM7 reply sum | holds / known | Batch length checked once on the parsed array; reply-size sum: known findings (framing, collapse to one error) |
| LIM1 call cap (`reserve`) | holds | `slots` read and incremented synchronously in each request's first poll, so concurrent items of a `join_all` batch each see the earlier items' slots (`bridge.rs:370-380`). Slots vs messages: known (L4-2) |
| LIM2 instruction budget (`reserve`, `check_budget`, Rhai `on_progress`, JS jobs) | holds per run | `instruction_deadline` uses `saturating_add`, `instructions_used` `saturating_sub`. Batch-level sum: SW3-5-2 |
| V4 `DID_FAILED` / V2 `DID_CACHE` in a batch | known | Per-item `failed_get`/`cache_get` before an await whose result the batch's other items cannot see: L4-1 / r2-7 |
| LIM25 JS `MAX_PENDING` per round | known | Per-request `pending.len()` check; getter re-entry bypass is a known finding. `MAX_PROMISE_JOBS` counter u64: holds |
| LIM25 JS 64 x 4 MiB args per round | known (accepted #24) | r2-5 |
| LIM26 Python suspensions, gather batch | known | No per-batch cap (known). `queued` args are host allocations made outside an await, so they count in `LIVE_MEMORY`: holds for memory |
| V6 `LIVE_MEMORY` / `BASELINE_MEMORY` (`saturating_sub`) | known | Replies excluded and their frees credited to the run: L8-2 |
| LIM27 Rhai `MAX_REQUESTS`, `seq` | holds | One request per replay; `answers.len()` checked before each send |
| LIM14 `violations_dropped + 1` (u32, wraps in release) | holds | 2^32 denials need far more than the 20e9 budget (each denial formats a message and reads stable memory); unreachable |
| LIM15 `LogBuf` / JS / Rhai `log_bytes` | holds / known | `usize` sums of byte lengths below the heap size; marker overshoot is known |
| LIM16 `ValueBudget` `nodes += 1`, `bytes += n` (usize = u32 on wasm32) | holds | `n` is one string's length (< 3 GiB heap) added to <= 4 MiB; cannot wrap |
| W12 `truncate_escaped` `used` | holds | Returns at the first char past 1,000,000 |
| LIM5 `audit_page` | holds | `checked_sub` / `checked_add` |
| V1 `ACTIVE_RUNS` | holds | `+1` only when `< 8`; `saturating_sub` on drop |
| V3 `PENDING_RUNS`, V7 getrandom (`wrapping_*`, intended) | holds | No counter arithmetic at risk |
| W3 search `budget -= 1`, `omitted += cut` | holds | Decrement guarded by `budget == 0` |
| W5 `json_to_idl` record/variant per-field checks | known | Duplicate aliased keys: L7-13 |
| F9 web-time `Instant + Duration` | holds (traps, does not wrap) | `checked_add(...).expect`; reachability is the known set_limits validation finding |
| E1/E2 `apply`, E6-E11 admin | n/a | No per-item limit against a shared quantity; duplicates overwrite |
| E3, E4, B1-B17, U1-U16, O1-O10, K1-K12, CL*, S*, C2-C15, A*, AD*, VF1-VF8, T*, W1-W2, W4, W6-W11, W13-W16 | n/a | No batch whose items are checked against a shared consumable quantity, and no counter arithmetic outside the rows above (grep over all non-test code) |

## New candidates

### SW3-5-1 demo_store `sales_summary` wraps its u64 revenue sum after an oversell (Low)

- Where: `demo_store/src/lib.rs:246-257` (`revenue += line`, `e.2 += line`, u64), release
  profile without overflow checks (`Cargo.toml:44-49`).
- Scenario (R): `restock(p, 4294967295)` for products 11, 10, 1, 2, 9, then one
  `create_order` per product with 160,000 identical lines, each asking for the full
  available quantity (~4,294,967,29x); every order is accepted (the seed oversell). Exact
  revenue 19,378,892,426,896,652,550; `sales_summary().revenue_cents` = 932,148,353,187,100,934.
  Each order's own `total_cents` is correct. Wrong data returned as if right.
- Root cause: same class as the known release-profile finding (overflow checks off, the
  sum is `u64` while the output type is `Nat`); reachable only through the known oversell.
  Proposed handling: merge into the release-profile overflow finding as another site.
- Fix: accumulate in `u128` or `Nat` (the reply type is already `Nat`), and fix the oversell
  by summing quantities per product before the check.

### SW3-5-2 Per-tool-call instruction budgets share one 40e9 message execution (Medium)

- Where: `src/mcp.rs:386-404` (items run in sequence), `src/lib.rs:121-154` (search checks
  its own budget only after render), `src/bridge.rs:318, 351-354` (each `IcBridge` measures
  from its own start), `src/runtimes/rhai_rt.rs:143-152` (Rhai stops at its own deadline).
- Scenario (C): one HTTP request with up to five `search_icp_canisters` calls answered from
  cache and one `execute_icp_script`. None of them awaits, so all run in one message
  execution. Each item is checked against `max_instructions` (20e9) from its own start; the
  sum is never checked against the 40e9 per-message limit. A Rhai script that loops to its
  20e9 stop, plus searches whose renders together exceed 20e9 while each stays under 20e9,
  traps the message: the client gets a gateway error instead of the script's clean
  "instruction budget exceeded", and nothing is recorded. When the script ran earlier in the
  request after awaits, the trap lands in its last callback (the known batch-trap record
  consequence). Reachability: search render costs about 240 instructions per keyword x
  allowed method (L3-7: 2.4e9 for 1M keywords x 10 methods), and the HTTP body caps the
  keywords for the whole request at about 1M, so the searches' sum exceeds 20e9 once about
  85 methods are allowed. That is far more than the test deployment's 8, but realistic for a
  deployment that allowlists a ledger, index and governance with `"*"`. Splitting the keywords
  over five searches gets past each search's own check, which refuses the same work done as
  one search.
- Distinct from the known "search rendering cost is not bounded by max_instructions": if each
  search were stopped at its own budget, five searches and a script could still use
  6 x 20e9 in one execution.
- Fix: measure one budget per HTTP request (base taken in `http_update`), or give later items
  only what is left of the message's headroom, and check search render cost while it runs.
