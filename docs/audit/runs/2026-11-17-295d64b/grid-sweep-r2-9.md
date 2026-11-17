# Variant sweep r2-9: per-request limit enforced in one of two sibling handlers

Run `2026-11-17-295d64b`, Phase 6, round 2. Pattern (seed: known finding "MAX_SEARCHES is
enforced only in http_update; the query path answers up to 50 cached searches"):

> A per-request limit enforced in one of two sibling handlers for the same request.

Generalised for the sweep: two or more handlers that serve the same logical request (query
and update routes, `inspect_message` and the method, sibling SDK functions or sibling
arguments of one SDK call, the three runtimes, admin endpoint and `InitArgs`, the
`canister_metadata` and `__get_candid_interface_tmp_hack` fetch paths, `rpc()` and `post()` on
the demo page), where a size, count, depth, budget or cap check exists in one and is missing
or different in the other.

Method: listed every limit constant and check site (`grep` for `MAX_`, `len() >`, `>=`,
`truncate`, `budget`, `quota`, `set_max_`, `limit`) in `src/`, `src/runtimes/`,
`demo_store/src/`, `src/demo.html`; for each, found every handler that serves the same
request and compared. One native reproduction (below) in a private copy built with the shared
`CARGO_TARGET_DIR`; the copy was deleted afterwards. Known findings listed in the task were
not re-reported.

## Grid

| Row | Sibling handlers compared | Verdict | Reason |
|---|---|---|---|
| C1 / E4 / E5 / LIM9 | `http_query` vs `http_update`: `MAX_SEARCHES` | known | Seed finding |
| LIM8 `MAX_BATCH` | query vs update | holds | Both go through `parse_body` (`src/mcp.rs:293-304`) |
| LIM10 one script per request | query vs update | holds | Query upgrades on any `Execute` (`mcp.rs:366`); update refuses the second (`mcp.rs:392`) |
| LIM7 `MAX_REPLY_BYTES` | query (`finish` only) vs update (per-result replacement, then `finish`) | known | Query batch of large cached searches collapses to one `id:null` error, update replaces each result: covered by "Oversized-reply fallback answers a batch with one error object and id null" and the http_update replacement findings |
| LIM2 instruction budget for search | `search_cached` vs `run_search` | known | "The query search path applies no instruction budget" |
| LIM18 failed-fetch TTL | `search_cached` vs `run_search` / `IcBridge::candid` | known | "query-path search ignores the failed-fetch cache" |
| LIM12 run slots | `run_script` vs `run_search` | known | "Searches take no run slot" |
| LIM13 audit error cap | `run_script` vs `run_search` | holds | Both truncate to `MAX_AUDIT_ERROR_BYTES` (`lib.rs:169`, `:243`); the `Trapped` entry's error is fixed text |
| LIM14 violation caps | script vs search vs `Trapped` | known (count dropped in `Trapped`) | `deny` is shared; the `…and N more` omission in `Trapped` is a known finding |
| LIM6 result cap | script vs search | holds | Both use `tool_result` (`mcp.rs:156`) |
| LIM4 `certified_reads` | query vs update | known | "unauthenticated, wrong-path and wrong-method requests are still answered by the uncertified query" |
| E3 vs E5 (`inspect_message` vs method) | decode quota | known | "inspect_message decodes HttpUpdateRequest without the skipping quota" |
| E3 vs E5 | authentication | holds | Same `mcp::authenticate`; admin methods: controller check in both `inspect_message` and the guard |
| E1/E2 vs E9/E6/E8 (InitArgs vs admin endpoints) | token hash format, allowlist, limits | holds | `apply` calls the same `add_token_hash`, `set_allowed`, `set_limits`; label and list sizes unbounded in both (known) |
| E11 vs O2 (`set_candid_override` vs fetched interface) | `MAX_DID_BYTES`, parse | holds | Override checked at set time (`lib.rs:326-329`), fetched text after the reply (`bridge.rs:490-494`); same limit, same parser |
| O2 `canister_metadata` vs `__get_candid_interface_tmp_hack` | decode quota, size, timeout | known | No-quota decode of the fallback and default 300 s timeout are known; `MAX_DID_BYTES` applied to both after the reply |
| LIM17 `MAX_METHOD_LEN` | `IcBridge::call` vs `IcBridge::candid` | holds | `candid` takes no method; Rhai 2/3-arg `call_canister`, Python `call`/`call_async`, JS `icp.call` all reach `IcBridge::call` |
| LIM31 principal echo cap | `parse_principal` vs `candid_bridge` / search note | known | "Caller-supplied principal ... echoed capped at 80 B by parse_principal but in full ..." |
| LIM16 value limits (U7) | sibling arguments of one SDK call: JS `icp.call(canister, method, args)` / `icp.candid(canister)` | **finding (reproduced), candidate 1** | Only `args` goes through `to_json_safe` (`js.rs:262`); `canister` and `method` go through `arg_string` (`js.rs:195-200, 259-261`), which copies a string of any length out of the engine. Python counts all positional args, canister and method included, in one `ValueBudget` (`python.rs:243-244`); Rhai caps every string at 1 MiB (`rhai_rt.rs:126`) |
| LIM16 value limits | runtimes: printed values | known | JS `display` copies a string argument in full before the 64 KiB cap; Python formats in full (known); outcome capped by the log limit, no new impact |
| LIM25 / LIM26 / LIM27 request caps | JS `MAX_PENDING` vs Python batch vs Rhai `MAX_REQUESTS` | known | "Python has no per-batch request cap"; getter overrun of `MAX_PENDING` known |
| LIM2 budget checks before a request | JS `request_fn` vs Python `FunctionCall` vs Rhai `on_progress` | known | Python budget findings (VM clock, native operations) cover the difference |
| LIM15 log cap | JS vs Python vs Rhai | known | Python `MemoryError`, JS/Rhai marker overflow are known |
| LIM21 depth | encode vs decode, runtimes | known | "Depth limit off by one between runtimes"; L7-2 |
| LIM20 decode quota | `decode_reply` vs `fetch_did` vs `inspect_message` | known | both other sites are known findings |
| LIM5 `MAX_AUDIT_PAGE` | `get_audit_logs` vs `get_policy` | known | `get_policy` unbounded reply is "Admin inputs are unbounded ... get_policy returns them in one reply" |
| LIM23 `MAX_CODE_BYTES` | three runtimes | holds | Checked once in `LanguageRouter::execute` (`runtimes/mod.rs:52`) before dispatch |
| LIM32 `MAX_METHODS` | query vs update search | holds | Same `search::render` |
| LIM1 call cap | `icp.call` vs `icp.candid` | holds | `call` takes a slot per attempt even when the cached failure then refuses it; documented in CL7 ("every icp.call attempt that passes policy") |
| LIM3 call timeout | script calls vs fetches | known | "call_timeout_seconds is not applied to interface fetches" |
| E20-E23 demo_store | `create_order` vs `restock` vs `cancel_order` | known | No per-request item cap anywhere (known unbounded order lines); `restock` saturates, `create_order` wrap is known |
| C5 / E26 demo page | `rpc()` vs `post()` | holds | Neither applies a size or time limit, so no asymmetric limit; both send the same body shape |
| C12 video | capture vs render/stills | not applicable | No per-request limits |
| C2-C4, C6-C11, C13-C15 | build and tooling | not applicable | No request handlers |
| A1-A12 | actors | not applicable | Pattern is about handlers; A3 and A8 reach the same `http_update` |
| U1-U16 | inputs | covered above | U5 `query`/`canister_id` unbounded in both routes (known); U7 candidate 1 |
| D1-D10, V1-V11 | state | not applicable | No per-request limit is stored; V2/V4 consulted asymmetrically (known) |
| K1-K12 | commit points | not applicable | |
| W1-W16 | unbounded work | covered above | W3 keyword count unbounded in both routes (known) |
| F1-F16 | failure sites | not applicable | |
| B1-B17 | conversions | covered above | B8 is candidate 1's site |
| VF1-VF8 | value flows | not applicable | |
| AD1-AD9 | admin | covered above | |
| S1-S10 | supply chain | not applicable | |
| CL1-CL37 | claims | CL10 violated by candidate 1 | CL10: "Any value leaving a script (results, call arguments, printed values) is limited to ... 4 MiB of text, in all three languages" |
| T1-T12 | tests | gap | T7/T8 test the value limits only on the `args` value, never on canister or method strings |

## Candidates

### 1. JS `icp.call` / `icp.candid` copy canister-id and method strings of any size out of the sandbox; only `args` is held to the value limits

- **Where:** `src/runtimes/js.rs:195-200` (`arg_string`), `:259-268` (`request_fn`).
- **Sibling handlers:** the three arguments of one `icp.call` request (only `args` is
  snapshotted); and the same request in Python (`python.rs:243-244` counts every positional
  argument in one `ValueBudget`) and Rhai (1 MiB string cap).
- **Reproduced** natively (MockBridge, default features), private copy, one added test:
  - JS `icp.call(L, <5 MiB string>, null)`: Ok, the bridge received a 5,242,880-byte method.
  - Python `icp.call(L, 'm' * 5 MiB)`: `value holds more than 4194304 bytes of text`, no call.
  - Rhai: `Length of string too large`, no call.
  - JS `icp.candid(<5 MiB string>)`: the 5 MiB id reached the bridge (error text 5,242,923 bytes
    from MockBridge's echo; Python refuses at 4 MiB).
  - JS `Promise.all` of 64 `icp.call(L, m, null)` sharing one 5 MiB string `m`: Ok, 64 calls,
    335,544,320 bytes of method text handed to the host in one round. That is the
    amplification `MAX_VALUE_BYTES` exists to stop (`bridge.rs:43-46`: "one 1 MB string
    referenced 5,000 times is a small value inside the engine and a 5 GB one once copied out").
- **On-chain effect (code path):** `IcBridge::call` refuses methods over 256 B and
  `parse_principal` truncates the echoed id to 80 B, so nothing oversized is sent or audited.
  The copies are still made, one Rust `String` per pending request, and held until the
  round is dispatched: up to 64 x the string's size in heap per round, outside every value
  limit. JS has no memory cap (register #24), so the reachable size is bounded only by the
  Wasm memory limit.
- **Severity:** Medium (a documented limit is inconsistent between handlers; CL10 is false
  for JS call arguments). Not graded higher: no trap or wrong effect was demonstrated on-chain.
- **Fix:** count `canister` and `method` in the same `ValueBudget` (or refuse
  canister ids over the 63-char principal text maximum and methods over `MAX_METHOD_LEN`)
  inside `request_fn`, before `to_std_string_escaped`.
