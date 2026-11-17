# Variant sweep r2-8

Pattern: "A limit counted per logical operation while the operation fans out to several
outbound messages."

Method: code reading at `295d64b`, plus one live probe of read-only requests against the local
network on port 8010. Nothing was deployed, upgraded or reconfigured. The bearer token was read
from `.env` into a shell variable and was not printed. For each inventory row the question was:
is a limit counted once for an operation that can send more than one message (or HTTP request,
run, or retry), so that the real number of messages is a multiple of the limit?

Evidence keys: **C** = the code path produces it unambiguously; **R** = reproduced.

Result: **no new candidate.** Every hit in the code is already known (the fetch slot that can
send 2 messages, the duplicate fetches on concurrent misses, Python's lack of a per-batch request
cap, the JS getter that gets past the per-round cap, the 5-searches cap that applies only on the
update path, searches taking no run slot, the 8 runs x 25 calls of held prepayment, the
fetch timeout that is not `call_timeout_seconds`, and the budget that is not re-checked before
sending). One new place where the pattern could apply, the demo page's classic side sending 12
concurrent HTTP requests against the 8-run cap, was tested live and holds on the local network.

## Outbound-message rows

| Row | Limit and its unit | Fan-out under one unit | Verdict |
|---|---|---|---|
| O2 / LIM1: an interface fetch (`IcBridge::candid`, `bridge.rs:486`) | One call-cap slot per fetch (`reserve(p, None)`) | `fetch_did` (`bridge.rs:424-437`) sends `canister_metadata` and then `__get_candid_interface_tmp_hack`. That is 2 messages for 1 slot, and the second has no `reserve` | **known** (L4-2: "one call-cap slot can send two messages"; the missing budget and allowlist re-checks before the send are already listed as separate known findings) |
| O1 + O2: `icp.call` on a cold cache (`bridge.rs:510-516`) | 2 slots (one for the call, one for the fetch inside `service()`) | Up to 3 messages for 2 slots | **known** (L4-1/L4-2; documented by CL7, "every interface fetch that misses the cache") |
| O2 from concurrent misses (JS `join_all`, Python `gather`, concurrent requests) | One slot per request | N requests send N identical fetches (no single-flight) | **known** (L4-1) |
| LIM3 `call_timeout_seconds` | Applies to each script call | A fetch makes 2 `bounded_wait` calls at ic-cdk's default 300 s each, and ignores the setting | **known** ("call_timeout_seconds is not applied to interface fetches, which use ic-cdk's default 300 s each") |
| LIM2 instruction budget | Checked once per host request in `reserve` | One request does fetch, parse, encode and then sends | **known** ("Budget checked before the interface-fetch await and not again before send") |
| LIM25 JS `MAX_PENDING` 64 per round, `MAX_ROUNDS` 64 | Counts queued requests per round | A getter in the call arguments queues more requests during dispatch | **known** ("JS per-round cap of 64 concurrent requests can be exceeded by a getter"). Without that, holds: L4 found that requests past the cap are refused in `reserve` before any await |
| LIM26 Python 256 suspensions | Counts suspensions | One `gather` suspension carries any number of requests | **known** ("Python has no per-batch request cap; JS refuses the 65th concurrent request") |
| LIM27 Rhai `MAX_REQUESTS` 64 | Counts requests (`answers.len()`, `rhai_rt.rs:208`) | One request = one `bridge.call`/`bridge.candid`. Replays serve earlier answers, errors included, from `answers` (`rhai_rt.rs:71-77`) and never resend. A request sends more than one message only through the fetch rows above | holds for this pattern (C) |
| LIM9 `MAX_SEARCHES` 5 per HTTP request (`mcp.rs:395`) | Counts searches | Each search has its own 25-slot cap. Searches in one request run one after another, so later searches see the earlier successes in `DID_CACHE` and failures in `DID_FAILED` (`bridge.rs:478-485`) and do not fetch the same canister again. At most 125 distinct interfaces (up to 250 messages) per request, and at most one fetch per canister per TTL | holds, as L4 found (E5b). The query path's 50 searches are **known** (L4-3) |
| LIM10 one `execute_icp_script` per HTTP request (`mcp.rs:389-393`) | Counts scripts | One script can call this canister's own `http_request_update` and so start nested runs | **known** (the self-allowlist finding). Otherwise holds |
| LIM12 `MAX_CONCURRENT_RUNS` 8 | Counts scripts | Each run can keep up to 25 calls in flight, each holding response prepayment. Searches take no slot | **known** (prepayment finding; L4-4) |
| LIM12 versus the demo page's classic side (`src/demo.html:312, 386-389, 447-448`) | The run cap counts scripts | One "turn" of "What needs restocking?" sends 12 concurrent `POST /mcp` requests, each running a one-call script, and any `isError` throws (`demo.html:381`), which fails the whole comparison | **holds on the local network (R)**. 12 concurrent single-call scripts (`return await icp.call(S, "get_inventory", n)`), fired as 12 separate curl processes so the browser's 6-connection HTTP/1.1 limit does not apply: all 12 succeeded in 2 of 2 trials. 30 concurrent got 1 `busy`, so the cap is reachable, but same-subnet calls finish fast enough that 12 runs do not overlap 8 deep. Mainnet (HTTP/2, a store on another subnet) was not tested |
| LIM8 `MAX_BATCH` 50 | Counts JSON-RPC messages | Only the tool messages in a batch send anything, and those are already capped by LIM9 and LIM10 | holds (C) |
| LIM7 `MAX_REPLY_BYTES` per HTTP reply | Counts the whole reply | Up to 6 tool results in one reply | **known** (oversized batch reply replaces every tool result) |
| O3 audit writes; LIM5 `MAX_AUDIT_PAGE` 100; LIM13/LIM14 | One entry per message plus one closing entry per run; page counted in entries | A run writes up to 2 x cap + 1 entries. Page and entry caps are per entry, so fan-out changes how many runs fit on a page, not the size of an entry. The largest entry is still made of 25 call labels (<= ~320 B each). Fetch labels are shorter (`aaaaa-aa.canister_metadata`, `<id>.__get_candid_interface_tmp_hack`), so the 2-messages-per-slot fan-out does not raise the entry-size bound | holds (C). Page counted by entries, not bytes: **known** |
| CL15 / audit `calls` | "`calls` counts messages sent" | Can exceed `max_calls_per_script` | **known** (L4-2 consequence) |
| E3 `inspect_message` | One check per ingress | One ingress carries up to 1 script + 5 searches. The check is authentication, not a quota | not applicable (no count is enforced) |
| E5c / K2-K5 callbacks | none | Messages handed to the system one by one. ic-cdk 0.20 `Call` does not retry, so one script call is one message | holds (C) |
| A7 management canister (`canister_metadata`) | One slot | One message to `aaaaa-aa`, routed by `canister_id` | holds for this pattern. Logging only aaaaa-aa is **known** |
| AD2 self or `aaaaa-aa` allowlisted | One slot per call | A self-call starts a whole nested run with fresh caps | **known** |
| VF1 / VF5 cycles | No per-caller rate limit | Per request at most (1 + 5) tools x 25 slots x 2 messages | **known** (register #31; L4 VF5) |

## Other rows

| Row | Verdict |
|---|---|
| E21 demo_store `create_order(items)`: stock checked per line, not per order | **known** (duplicate lines oversell). No outbound messages |
| W15 / E20 demo_store queries | not applicable: no outbound effects |
| C12 / O10 `video/tts.mjs` `speak()` retry loop (`tts.mjs:51-57`) | holds: at most 4 attempts per scene, one scene at a time; development tooling only |
| C12 `video/capture-demo.mjs` runs the 3 demo scenarios | holds: it drives the same page over local HTTP/1.1 (see the LIM12 demo row) |
| C6 `scripts/demo-setup.sh` | not applicable: one `set_allowlist` call |
| E4 / E4b query path | not applicable: sends no outbound messages. Up to 50 searches is **known** |
| V1-V11 volatile state | not applicable as limits. V2/V4 single-flight is **known** (L4-1) |
| D1-D10, K6-K12, F1-F16, B1-B17, S1-S10, T1-T12, CL1-CL37 (other than above) | searched for a count that is enforced per operation while the operation sends several messages: no other case. These rows enforce no limit on outbound messages, or the limit applies to each message |
| C1a feature subsets | not applicable: the same bridge and limits in every subset |
| C14 per-customer idea doc | not applicable: not built |

## Not tested

- The demo page's 12 concurrent classic-side requests against the 8-run cap on mainnet, where
  the gateway speaks HTTP/2, so the browser can open more than 6 concurrent requests, and the
  store may be on another subnet, so each run waits longer. This needs a mainnet deployment.
  Locally it holds (see above).
- The live audit log grew by the runs from this probe: 55 one-call scripts, of which 1 was
  refused as `busy`, which writes no entry.
