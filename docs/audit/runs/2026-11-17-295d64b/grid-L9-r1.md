# Grid: L9 Failure handling, round 1

Run `2026-11-17-295d64b`. Reviewer: L9 lens agent. Question: what happens when a dependency
(target canister, management canister, the platform, stable memory) is slow, wrong, hostile or
absent? Method: read `src/bridge.rs`, `src/lib.rs`, `src/mcp.rs`, `src/search.rs`,
`src/candid_bridge.rs`, `src/policy.rs`, `src/runtimes/*.rs`, `shims/web-time`, and the
dependency code they lean on (ic-cdk 0.20.3 `call.rs`, ic-cdk-macros 0.20.3, candid 0.10.40
`de.rs`, monty-types 1.6.0 `resource/`). Probes ran natively in a private copy as
`tests/l9_failure.rs` (debug, and release `--no-default-features` for the stack probe; shared
target dir, `RUST_MIN_STACK=8388608` to mirror the 8 MiB wasm stack). The live network was used
read-only (controller queries `get_audit_logs`/`get_audit_log_len`): no setting changed, nothing
deployed. Candidate findings are L9-1 ... L9-9 in the lens output.

Evidence: **R** = reproduced natively, **C** = code path unambiguous, **L** = observed on the
local network (read-only).

## Findings in this grid

| Id | Sev | Title |
|---|---|---|
| L9-1 | High | `__get_candid_interface_tmp_hack` reply decoded with no quota: a 27-byte hostile reply forces unbounded work; the trap rolls back the failure cache, so it repeats on every request |
| L9-2 | High | A nested interface well inside the 512 KiB limit overflows the 8 MiB stack while parsing; same no-memory-of-traps repetition |
| L9-3 | High | Default `unbounded_wait`: a target that never replies holds a run slot forever; 8 such runs refuse every script until an upgrade, and revoking the token does not help |
| L9-4 | Medium | The reply decoding quota is effectively 400,000, not 20,000,000 (candid's 50x untyped penalty): valid replies over ~400 KB, ~36k nat64, ~9k small records fail with a misleading error; register #17 regressed above ~400 KB |
| L9-5 | Low | A transient failure while re-fetching an expired interface discards the still-held interface and blocks every caller for 5 minutes, with an error that blames a missing interface |
| L9-6 | Low | Interface parse errors echo up to 512 KiB of target-controlled text into search results and script errors |
| L9-7 | Low | One allowlisted target without a retrievable interface makes every unscoped search a paid update call |
| L9-8 | Low | `call_timeout_seconds` is not validated (0 accepted: calls time out at once, outcome unknown) and is not applied to interface fetches (always 300 s, twice) |
| L9-9 | Low | When stable memory cannot grow, audit appends trap; a trap in the cleanup callback also loses the `Trapped` entry and leaks the run slot |

## Cells

### Components, actors

| Row | Cell | Reason / evidence |
|---|---|---|
| C1 icp_code_mode | **findings** L9-1..L9-9 | See rows below |
| C1a feature subsets | holds | Failure handling lives in `bridge.rs`/`lib.rs`, shared by every subset; no runtime-specific dependency calls |
| C2 demo_store | not applicable as a component | Synchronous methods, no outbound calls; as a target it always replies (L9 treats it under A6) |
| C3 web-time shim | holds | `Instant + Duration` (`expect` on overflow) is never used: Monty only calls `Instant::now`/`elapsed` (`monty-types resource/execution_clock.rs:35,70,76`, `resource.rs:842` sleep loop, `SleepMode::Zero`); Rhai built with `no_time`. So F9 is unreachable (C) |
| C4 vendored chrono | not applicable | No failure-handling surface (no I/O; `wasmbind` removed) |
| C5 demo page | not applicable | L17 owns rendering; it only shows whatever `/mcp` returns |
| C6 demo-setup.sh | not applicable | Operator script; fails visibly in the shell |
| C7 icp.yaml | not applicable | Build config |
| C8 rustflags (8 MiB stack) | **finding** L9-2 | 8 MiB is not enough for `candid_parser` on nesting allowed by `MAX_DID_BYTES` (R) |
| C9-C11, C13-C15 | not applicable | Toolchain, profile, interface files, audit tooling, docs, exports: no dependency calls |
| C12 video pipeline | not applicable | Dev-only tooling outside the deployed trust boundary (Gemini/Chrome failures stop a local script) |
| A1-A5, A8 | not applicable | Callers, not dependencies; their failures are handled by authentication (L1/L12) |
| A6 allowlisted target | **findings** L9-1, L9-2, L9-3, L9-4, L9-5, L9-6, L9-7 | Never replies: L9-3. Replies late: L9-3, L9-8. Replies hostile bytes: L9-1 (fetch path), L9-4 (call path quota mis-sized, fails closed). Lies about its interface: holds (the interface only shapes calls to that same target, which the controller allowlisted; it cannot widen the allowlist). Hostile interface text: L9-2, L9-6. Absent/broken: L9-5, L9-7. Rejects: error text passed through (U9) |
| A7 management canister | holds | Only `canister_metadata`, bounded wait; reply structure trusted and decoded with ic-cdk defaults; on the local network the fetch used `canister_metadata` and no fallback (L: 4 entries `aaaaa-aa.canister_metadata`, 0 `__get_candid_interface_tmp_hack` in the newest ~780 entries). Any failure or undecodable reply falls back to the target (which reaches L9-1) |
| A9 infrastructure | not applicable here | Altered query responses are L10 (#47) |
| A10 anyone on demo_store | not applicable | No dependency |
| A11 supply chain | holds for this lens | Dependency defaults that matter here are recorded under L9-1 (ic-cdk `Response::candid` has no quota) and L9-4 (candid untyped penalty) |
| A12 factory | not applicable | Not built |

### Entry points

| Row | Cell | Reason / evidence |
|---|---|---|
| E1/E2 init/post_upgrade | holds | Bad token hash traps the install/upgrade (F1): fails closed and rolls back. Heap caches (`DID_CACHE`, `DID_FAILED`) and `ACTIVE_RUNS` reset, which is also the only recovery for L9-3 |
| E3 inspect_message | holds (fails closed); variant note | Decode failure or limit -> not accepted. Variant of L9-1's pattern: `candid::decode_one::<HttpUpdateRequest>` uses the default config (no skipping quota, unlike the ic-cdk method wrapper's 10,000), so a hostile ingress argument with a huge skipped `vec null` spends inspect_message's whole budget before being refused (C; replica cost only, no canister charge). Handed to L3 |
| E4/E4a/E4b http_request | **finding** L9-7 | `search_cached` ignores `DID_FAILED`: a recent failure forces an upgrade (C). `policy::limits()` decode `expect` (F3) holds for current shapes (`policy::old_audit_entries_still_decode`) |
| E5 http_request_update | **findings** L9-1, L9-3, L9-9 | |
| E5a run_script | **finding** L9-3 | `_slot` lives across `LanguageRouter::execute(..).await` (`lib.rs:219-225`); with `call_timeout_seconds = null` every call is `Call::unbounded_wait` (`bridge.rs:524-527`) (C) |
| E5b run_search | **findings** L9-1, L9-2, L9-6, L9-7 | A trap in fetch processing fails the whole unscoped search, not one entry; errors rendered untruncated (`search.rs:98-100`) |
| E5c reply/reject callbacks | **findings** L9-1, L9-2, L9-4 | Work done in the callback on target-supplied bytes |
| E5d cleanup | **finding** L9-9 | Cleanup that traps rolls back its own `Trapped` entry and `RunSlot` decrement (C). Whether cleanup gets enough instructions after an instruction-limit trap is platform behaviour: not tested |
| E6 set_allowlist | holds | Does not evict `DID_FAILED`; harmless because re-adding is not a fix for a failing target |
| E7 remove_allowlist | holds | Evicts `DID_CACHE`, `DID_FAILED`, `OVERRIDES` |
| E8 set_limits | **finding** L9-8 | No validation of `call_timeout_seconds` |
| E9, E10, E12-E14 | not applicable | No dependency calls |
| E11 set_candid_override | holds | Admin text checked for size and parsed before storing; a nested override that overflows the stack only traps the admin call (state rolled back). It is also the operator's escape hatch for L9-1/L9-2/L9-5 |
| E15 exports | not applicable | |
| E16 JS icp.call/icp.candid | **findings** L9-3, L9-4 | `join_all` per round waits for the slowest call; failed calls reject the promise with the error text (catchable); reply conversion failure ends the run (loud) |
| E17 console | not applicable | |
| E18 Python icp.* | **findings** L9-3, L9-4 | Same bridge; errors surface as Python exceptions |
| E19 Rhai icp::* | **findings** L9-3, L9-4 | Failed answers are cached in `answers` and re-thrown on every replay, never re-sent (holds: no retry loop) |
| E20-E25 demo_store | not applicable | No dependencies |
| E26-E28 | not applicable | |

### Untrusted inputs

| Row | Cell | Reason / evidence |
|---|---|---|
| U1-U7, U11-U14 | not applicable | Caller/admin/script inputs: other lenses (L3, L7, L12) |
| U8 Candid reply bytes | **findings** L9-1, L9-4 | Call path: quotas present but mis-sized (L9-4, R). Fetch path: no quota at all (L9-1, R) |
| U9 reject codes/messages | holds (app side), size not tested | Embedded as `call {p}.{method} failed: call rejected: {code} - {message}` (`bridge.rs:528-531`) and in `could not fetch candid for {key} (...)` (`bridge.rs:503`). App truncation: audit 4 KiB, client 1 MB, search entry none. The platform's bound on a canister-supplied reject message was not tested (needs a hostile target) |
| U10 interface text | **findings** L9-2, L9-6 | 512 KiB checked after receipt (bounded by the 2 MiB reply); parse can overflow the stack (L9-2) and its error can echo the text (L9-6) |
| U15 platform values | not applicable | Trusted |
| U16 video tooling | not applicable | Dev only |

### Outbound effects

| Row | Cell | Reason / evidence |
|---|---|---|
| O1 script call | **findings** L9-3, L9-8 | Default unbounded wait (register #44 still accurately described: opt-in). With a timeout, `SysUnknown` is flagged "outcome unknown" (holds: `!is_clean_reject()` with a timeout set) |
| O2 interface fetch | **findings** L9-1, L9-8 | Bounded (300 s default, `ic-cdk call.rs:180-186`) but ignores the configured timeout; up to 2 messages per fetch for one cap slot (cap counts fetch attempts as documented in CL7; noted for L4/L16) |
| O3 audit writes | **finding** L9-9 | `expect("audit log out of stable memory")` |
| O4-O7, O9, O10 | not applicable | |
| O8 traps | **findings** L9-1, L9-2, L9-9 | |

### Durable state

| Row | Cell | Reason / evidence |
|---|---|---|
| D1 LIMITS | holds (F3) | Read on every request; a future non-optional field change would trap every request (L13 hazard); current shapes tested |
| D2, D3, D5 | holds | Decode `expect` only on stored, app-written values |
| D4 AUDIT | **finding** L9-9 | Storage exhaustion |
| D6-D10 | not applicable | |

### Volatile shared state

| Row | Cell | Reason / evidence |
|---|---|---|
| V1 ACTIVE_RUNS | **findings** L9-3, L9-9 | Slots held forever by hung calls; leaked when cleanup traps |
| V2 DID_CACHE (1 h) | **finding** L9-5 | Expired entry kept in memory but unusable when the refresh fails. No single-flight: concurrent misses each send their own fetch (C; cost bounded by concurrency, noted for L4) |
| V3 PENDING_RUNS | holds | Hung runs stay listed, which is the intended signal (CL20) |
| V4 DID_FAILED (5 min) | **findings** L9-1, L9-2, L9-5, L9-7 | Shared by all callers by design; but a *trap* is never recorded here (rolled back), so the most expensive failures are the ones never remembered (L9-1, L9-2) |
| V5 OVERRIDES | holds | Admin text, validated on write |
| V6-V11 | not applicable | No dependency results stored |

### Commit points

| Row | Cell | Reason / evidence |
|---|---|---|
| K1 first pending | holds | Entry written with the message; a hung call leaves `Started`/`Call` entries plus `pending_run_ids` (correct) |
| K2-K4 engine awaits | **finding** L9-3 | Each await on an unbounded call can last forever |
| K5 search awaits | **findings** L9-1, L9-2 | Trap after the fetch message: `Started` entry committed, `Trapped` written in cleanup, failure not cached |
| K6 closing entry | **finding** L9-9 | |
| K7 cleanup | **finding** L9-9 | |
| K8-K10, K12 | not applicable | |
| K11 upgrade while suspended | holds | Only recovery for L9-3; drops callbacks (CL20 documents unknown runs) |

### Limits

| Row | Cell | Reason / evidence |
|---|---|---|
| LIM1 call cap | holds for this lens | Refused before any await; a failed fetch is cached so retries in one script do not send again |
| LIM2 max_instructions | holds for this lens | Not checked inside decode or parse: L9-1/L9-2 run to the IC's own 40B limit regardless of the 20B budget |
| LIM3 call_timeout_seconds | **finding** L9-8 | Boundary: `null` -> unbounded (L9-3); `0` accepted (ic-cdk documents "would most likely time out", `call.rs:297-300`); `>300` silently capped (documented); fetches ignore it |
| LIM12 MAX_CONCURRENT_RUNS | **finding** L9-3 | |
| LIM13 audit error 4 KiB | holds | Applied to run and search closing entries |
| LIM18 TTLs | **findings** L9-5, L9-7 | Boundary arithmetic holds (`saturating_sub`, `<=`); semantics are the findings |
| LIM19 MAX_DID_BYTES | **finding** L9-2 | 512 KiB admits ~130k nesting levels; parser overflows at 10k-20k (R) |
| LIM20 DECODING_QUOTA | **finding** L9-4 | Effective 400,000 (R) |
| LIM21 MAX_DEPTH | holds | `idl_to_json` errors past 128; candid's own recursion guard applies in decode |
| LIM30 IC limits | **findings** L9-1, L9-2 | The app relies on the IC trap instead of its own bound |
| LIM31 truncate 80 B | holds | Invalid canister ids shortened before echo |
| other LIM rows | not applicable | Not about dependency failures |

### Unbounded work, failure sites

| Row | Cell | Reason / evidence |
|---|---|---|
| W6 decode_reply | **finding** L9-4 | |
| W7 Service::parse | **findings** L9-2, L9-6 | |
| W11 Rhai answers | holds for this lens | Failed answers cached, not retried; size is L3's |
| other W rows | not applicable | Caller-driven, not dependency-driven |
| F3 decode expect | holds | See D1 |
| F4 append expect | **finding** L9-9 | |
| F9 web-time overflow | holds | Unreachable: no `Instant + Duration` in Monty or (no_time) Rhai (C) |
| F12 native stack | **finding** L9-2 | |
| F13/F14 | **finding** L9-1 (F14 via decode), others L3 | |
| F1, F2, F5-F8, F10, F11, F15, F16 | not applicable | Not dependency failures |

### Boundary conversions, value flows, admin, supply chain, claims, tests

| Row | Cell | Reason / evidence |
|---|---|---|
| B5 | not applicable | Script input |
| B6 Candid -> JSON | **finding** L9-4 | |
| B7 .did -> TypeEnv | **findings** L9-2, L9-6 | |
| B11 result text | holds | Errors truncated to 1 MB escaped |
| B16 search JSON | **finding** L9-6 | Entry errors not truncated |
| other B rows | not applicable | |
| VF1 cycles | **findings** L9-1, L9-7 | 40B instructions per hostile fetch, repeated; unscoped searches as updates |
| VF4 assets via targets | **finding** L9-8 | Timeout 0: calls may execute while reported failed, inviting retries |
| other VF rows | not applicable | |
| AD4 set_limits | **finding** L9-8 | |
| AD7 overrides | holds | Escape hatch; validated |
| other AD rows | not applicable | |
| S2 ic-cdk / candid | see L9-1, L9-4 | Default `decode_one` has no quota; IDLValue decoding is charged 50x |
| other S rows | not applicable | |
| CL2/CL3 "fast and free" | **finding** L9-7 | Fails when one target has no retrievable interface |
| CL21 timeout semantics | **finding** L9-8 | 0 undocumented; fetches not covered |
| CL22 TTLs and 512 KiB | **findings** L9-2, L9-5 | 512 KiB is accepted but not parseable at depth; failures cached 5 min holds as written |
| CL31 active_runs | **finding** L9-3 | Slots can stay taken forever |
| CL34 8 MiB stack | **finding** L9-2 | The comment's goal (no trap) is not met for nested interfaces |
| other CL rows | not applicable | |
| T rows | gap | No test drives a failing, slow, hostile or absent target (register #55, still open); no test for `DID_FAILED`/TTLs (#45, #46 guards: none); no test of `decode_reply` near the quota |

### Register re-check (L9 class)

| Id | Re-check | Result |
|---|---|---|
| #44 unbounded wait default | `bridge.rs:524-527`, `policy.rs:28-33` | Still accurately described (opt-in). New consequence: L9-3 |
| #45 interfaces cached until upgrade | `cache_get` TTL 1 h | Still fixed; side effect L9-5 |
| #46 failed fetches repeated every request | `failed_get` 5 min | Fixed for errors; **not** for traps (L9-1, L9-2 repeat on every request) |
| #17 (L3) large unknown field rejected | probe `register_17_recheck` | **Regressed above ~400 KB** (L9-4): 390,000-byte unknown field decodes, 410,000 fails |

## Appendix: probes (private copy, `tests/l9_failure.rs`, not committed)

```rust
// L9-1: what ic-cdk Response::candid::<String>() does (candid::decode_one, default config).
// table0 = vec null; args: (text "service : {}", table0 of length n, no element bytes)
let mut b = vec![0x44,0x49,0x44,0x4c, 1, 0x6d,0x7f, 2, 0x71, 0];
leb(12, &mut b); b.extend_from_slice(b"service : {}"); leb(n, &mut b);
candid::decode_one::<String>(&b)
// n=1e6: Ok in 0.06 s; 1e7: 0.58 s; 1e8: 5.72 s (debug) - linear, 27-byte reply

// L9-2: Service::parse(&format!("type T = {}nat; service : {{ m : () -> (T) query }}", "vec ".repeat(d)))
// release, 8 MiB stack: d=10,000 (40 KB) ok; d=20,000 (80 KB) stack overflow (SIGABRT)

// L9-4: svc.decode_reply(..) largest accepted reply per type (DECODING_QUOTA = 20M):
// text 399,998 chars (400,008 B wire); blob 399,997 B; vec nat64 36,363 (290,916 B);
// vec record { id : nat64; memo : text } 9,090 (172,733 B); record { body : blob; status } 399,968 B.
// Error just above: "candid decoding failed: Fail to decode argument 0 from text to text".
// Unknown field of 390,000 B: Ok({"a":1}); 410,000 B: Err(... to record { a : nat64 }).

// L9-6: Service::parse("type T = CCC...(500,000 C) ; service : {}")
// -> error of 500,050 bytes: "invalid candid interface: Unbound type identifier CCCC..."
```
