# Grid: L9 Failure handling, round 2 (Live)

Run `2026-11-17-295d64b`. Reviewer: L9 lens agent, round 2. Task: observe on-chain the L9 findings
that round 1 proved only natively or from the code path (L9-1, L9-2, L9-3/L4-5, L4-2, L6-7/L9-7,
register #29, #46, the 300 s cap, cycles held by in-flight calls), using a misbehaving target
(register #55).

## Setup

- Network: the project's local icp-cli network (port 8010), shared with other audit agents that
  were running at the same time (audit log grew from 898 to over 5,500 entries during this run;
  another agent stopped and upgraded `icp_code_mode` at least twice, changing its module hash from
  the baseline `ad3ee221...` to `224cfa7a...`, set `call_timeout_seconds = 30` for a while, and
  allowlisted its own target `4xhad-gd777-77775-aaacq-cai`). Every observation below is tied to my
  own runs by script hash, run id, method label and timestamp.
- Hostile target (not in the repo; built in a private scratch directory with the shared target
  dir, `ic-cdk 0.20.3`, `candid 0.10.40`): detached canister `4qggx-l3777-77775-aaaca-cai`, no
  `candid:service` metadata, methods:
  - `__get_candid_interface_tmp_hack` (manual-reply query) in three modes: 0 = `DIDL 01 6d7f 02 71 00
    "service : {}" leb(n)` (a `vec null` of length n with no element bytes), 1 = a valid interface,
    2 = `type T = vec vec ... nat` nested to a chosen depth;
  - `hang`: never replies until `release` (keeps its call context open by awaiting `raw_rand`
    in a loop);
  - `pending_only`: `std::future::pending().await` (as the task proposed);
  - `big_reject(n)`: rejects with an n-byte message.
  A second detached canister `46el7-ql777-77775-aaada-cai` carried a 200,047-byte nested
  `candid:service` (depth 50,000) in public metadata. Both were deleted afterwards (cycles recovered).
- Settings changed and restored: allowlist entries for the two targets and `aaaaa-aa` (all removed;
  final allowlist = ledger + demo_store, as found); `set_limits` `max_calls_per_script = 1` for one
  run, `call_timeout_seconds = 600` for one run (restored to the values found; the final state read
  back is `call_timeout_seconds = null, max_calls_per_script = 25, max_instructions = 20e9,
  certified_reads = null`). No override was set. `icp_code_mode` was not deployed or upgraded by me.

Evidence: **L** = observed on the local network in this round (request, response and audit
entries), **C** = code path.

## Live results

| Probe | Result |
|---|---|
| L9-1, `vec null` n = 2^40 (29 B) | Clean error, cached for 5 min: `Fail to decode argument 1 from table0 to reserved`. On wasm32 a vector length must fit `usize` (32 bits) (L) |
| L9-1, `vec null` n = 2^32-1 (28 B reply) | **Confirmed.** Scoped search: HTTP 503 after 9.2 s, `Canister exceeded the limit of 40000000000 instructions`. Audit: `Started` (`aaaaa-aa.canister_metadata`), `Call` (`...__get_candid_interface_tmp_hack`), `Trapped` (instructions recorded 1.94M). Repeated 3 times: each repeat sent both messages again and trapped again (`DID_FAILED` never written). 3 requests = 114.3B cycles (~38B each, local pricing; 3 trivial scripts in the same window = 0.1B). An unscoped search (`query: "balance"`, aimed at the ledger) also failed with 503 while the target was allowlisted (L) |
| L9-2 via tmp_hack | Depth 5,000, 20,000, 30,000 (120 KB) parse fine on wasm. Depth 50,000 (200 KB): **stack overflow** trap (503, 3.1 s, canister log `[TRAP]: stack overflow`), repeated twice with both messages re-sent each time. Depth 70,000 and 100,000: 40B instruction limit instead (L) |
| L9-2 via `candid:service` metadata | Depth 50,000: stack overflow on both of two consecutive searches; each sent `aaaaa-aa.canister_metadata` again and wrote `Trapped` (L) |
| `pending_only` (`std::future::pending().await`) | Does **not** hang: ic-cdk 0.20.3 traps `protected task outlived its canister method`, the caller gets IC0503 at once. A never-replying target needs an outstanding call of its own (L) |
| L9-3 / CL9, unbounded wait (timeout null) | 8 scripts each awaiting `hang`: `get_policy` showed `active_runs = 8` and 8 `pending_run_ids` (1119-1124, 1127, 1136). They stayed pinned until I released the target; all 8 HTTP requests returned after ~352 s (gateway did not time out). (First attempt's `hang` spun on self-calls, saturating the local replica, so the ninth request could not be sent then.) (L) |
| Busy refusal (CL9, CL19, register #29) | Second round (timeout 30 s set by another agent): 8 pinned, `active_runs = 8`, ninth script answered `busy: 8 scripts are already running; retry shortly` (`isError: true`) in 0.23 s; no audit entry carries its script hash (sha256 `5dbf44d3...`); the audit length rose only by the 8 `Started` entries. **Register #29 triggered live; CL19 holds** (L) |
| 300 s cap with `call_timeout_seconds = 600` | Call to `hang` failed after 301.4 s with `call rejected: 6 - Call deadline has expired. (outcome unknown: the call may have executed)`. Cap and wording hold (L) |
| `call_timeout_seconds = 30` | 8 pinned runs ended after ~33 s with the same SYS_UNKNOWN text; target kept executing (`held = 8`) until released (L) |
| Cycles held for in-flight calls (L5 "not tested") | One script with 25 concurrent `hang` calls: balance 17,452.32B -> 16,381.32B while the calls were in flight (-1,071B, ~42.8B per call), back to 17,427.08B 4 s later when they expired. Earlier, 8 runs x 1 call held ~340B for ~6 min (17,904B -> 17,564B, refunded to 17,900B on release) (L). **Candidate finding L9-r2-1** |
| Reject message size (U9) | `big_reject(8192 / 65536 / 2,000,000)`: the platform cuts the reject text at 8 KiB (script saw 8,270 chars incl. prefix). App-side bound not needed (L) |
| L4-2 | `max_calls_per_script = 1`, `icp.candid('aaaaa-aa')`: run 5531 sent `aaaaa-aa.canister_metadata` and `aaaaa-aa.__get_candid_interface_tmp_hack`, closing entry `calls = 2`. **Two messages for one cap slot confirmed** (L) |
| `DID_FAILED` for `aaaaa-aa`, L6-7/L9-7 | After the failure (IC0536, no such method), an immediate second `icp.candid('aaaaa-aa')` sent nothing (no entries). Query-path `http_request` for an unscoped search returned `upgrade = opt true`; scoped searches for the ledger and another target answered in the query (5-10 ms), the `aaaaa-aa` one needed the update (0.14 s). **Confirmed** (L) |
| Register #46, 5 min expiry | Failure written at t0 = 11:46:14. At t0+284 s: no message sent. At t0+320 s: both messages sent again (entry 5554). **Holds live** (L) |

## Cells

| Row | Cell | Reason / evidence |
|---|---|---|
| A6 allowlisted target | **findings** (round 1 L9-1, L9-2, L9-3 confirmed live; new L9-r2-1) | Hostile 28-byte reply: 40B-instruction trap per request, repeated (L). Nested interface 200 KB: stack overflow per request, both fetch paths (L). Never-replying target pins run slots (L) and holds ~42.8B cycles per in-flight call (L, L9-r2-1). Oversized reject text: holds (platform 8 KiB cap) |
| O2 interface fetch | **finding** (round 1 L9-1, L4-2 confirmed live) | Two messages per slot (L); tmp_hack reply decoded without quota (L). Uses its own 300 s bounded wait regardless of `call_timeout_seconds` (C, already reported) |
| V4 DID_FAILED | **finding** (round 1, confirmed live) | Error path: cached, nothing re-sent within 5 min, re-sent after (L). Trap path: never cached, every request re-sends and re-traps (L). Query path ignores it and upgrades (L) |
| V1 ACTIVE_RUNS | **finding** (round 1 L9-3, confirmed live) | 8 slots held for the whole wait (352 s observed with timeout null; released only by the target) (L) |
| V3 PENDING_RUNS | holds | Listed exactly the 8 pinned runs, emptied when they ended (L) |
| K5 search awaits | **finding** (round 1, confirmed) | `Started`/`Call` committed with the messages; `Trapped` written in cleanup with 1.9M instructions recorded for a 40B execution (already reported) (L) |
| LIM1 / I2 cap | **finding** (L4-2, confirmed) | `calls = 2` under a cap of 1 (L) |
| LIM3 call_timeout_seconds | holds as documented for the cap; candidate below | 600 -> expired at 301.4 s; 30 -> ~33 s; "outcome unknown" text present (L). While a 600 s call was in flight another agent's stop of the canister stayed `Stopping` and reverted to `Running` (unverified, see not-tested) |
| LIM12 MAX_CONCURRENT_RUNS | **finding** (round 1 L9-3, confirmed); busy path holds | Ninth script refused with no entry (L) |
| LIM18 TTLs | holds (5 min) | Boundary observed at +284 s / +320 s (L). 1 h TTL not observed (an upgrade by another agent emptied the cache first) |
| LIM19 MAX_DID_BYTES | **finding** (round 1 L9-2, confirmed) | 200 KB, well under 512 KiB, traps (L) |
| LIM20 / U8 | **finding** (round 1 L9-1 confirmed) | Note: on wasm32 the attack needs n < 2^32; 2^40 fails cleanly (L) |
| U9 reject text | holds | Platform caps at 8 KiB (L) |
| VF1 cycles | **findings** L9-1 (cost ~38B/request locally, L) and L9-r2-1 (held cycles, L) | |
| CL9 "At most 8 scripts run at once" | holds (L) | Busy refusal observed |
| CL19 busy writes no entry | holds (L) | No entry with the ninth script's hash |
| CL21 timeout semantics | holds for the cap and the error text (L); "that can't happen" (stop blocked) unverified | See not-tested |
| CL22 TTLs, 512 KiB | 5 min holds (L); 512 KiB **finding** (L9-2) | |
| CL31 active_runs / pending_run_ids | holds for scripts (L) | |
| E5b run_search | **finding** (confirmed) | One trapping target makes every unscoped search return HTTP 503 to every token holder (L) |
| T rows / register #55 | gap remains | This round's target lives only in a scratch directory. Note for the harness: `std::future::pending()` does not hang under ic-cdk 0.20 (traps); a self-call loop hangs but saturates a local replica; awaiting `raw_rand` in a loop works |
