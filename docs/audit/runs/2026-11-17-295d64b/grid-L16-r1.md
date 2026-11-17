# Coverage grid: L16 Claims, round 1

Run `2026-11-17-295d64b`. Rows from `inventory.md` section 16 (CL1-CL37), plus I10 and
register #52/#53. Each cell: **finding** (candidate id below), **holds** (why), or **n/a**.

Evidence used: reading the code at `295d64b`; native probe tests in a private `git archive`
copy (`cargo test --lib l16_`, shared target dir, copy deleted afterwards); read-only queries
to the running local network (`icp canister status icp_code_mode`, `get_policy`,
`get_audit_logs` pages 0-800 of 839 entries). Nothing was deployed or changed.

## Candidate findings (ids used in the grid)

| Id | Sev | Title | Evidence |
|---|---|---|---|
| L16-1 | High | JS synchronous code has no instruction-budget check, and Boa's "1M loop iterations" is per call frame, so loops spread over function calls run until the IC 40B trap; README says every path but one built-in stops cleanly | native repro (4.5M iterations over 5 frames succeed; 4.5M in one frame refused) + code path for the trap |
| L16-2 | Medium | Search described to the AI (and in README:14, search.rs:5-7) as query-only, free, unaudited and outside the call cap; fetching searches are update calls that write audit entries and take cap slots, and a search over a canister whose fetch failed is upgraded on every call | live audit log: entries 696/697 `language = "search"`, `Started` + `Ok`; code path |
| L16-3 | Medium | README:367 "Python has its own 10B VM budget" and `EXECUTION_BUDGET` doc (python.rs:24-28) contradict README:371 and `vm_budget` (remaining policy budget on wasm; the constant is used off-chain only) | code path |
| L16-4 | Medium | "Calls per script 25" / `DEFAULT_MAX_CALLS` "cap on inter-canister calls per script": the cap counts slots, one interface-fetch slot can send 2 messages, so a run can send up to 2x the cap and its closing entry's `calls` (messages, since #53) can exceed `max_calls_per_script` | code path |
| L16-5 | Medium | README:493-496 lists `set_candid_override` as the third, fallback interface source; code consults the override first (candid, service, cached_service), so it shadows fetched interfaces and the 1-hour refresh | code path |
| L16-6 | Medium | `.did`/policy.rs `methods` doc: interface fetch logged as `canister.<candid interface>`; actual labels are `aaaaa-aa.canister_metadata` / `<target>.__get_candid_interface_tmp_hack`, and when metadata succeeds the fetched target appears nowhere in the entry (`canisters = ["aaaaa-aa"]`). README:74 diagram still says "one entry per run" | live audit log entries 680, 696, 697 vs older 476/477; code path |
| L16-7 | Medium | Python fails the whole script (with a `MemoryError: memory limit exceeded` message) when output passes 64 KiB, while JS and Rhai truncate and succeed; tool description promises identical SDK semantics and that output "is captured and returned" | native repro |
| L16-8 | Low | Tool description (shown to AI) says "No ... randomness"; Python's `random` works and returns the same sequence on every run (seed 0x1C9); README:371 says "seeded random" | native repro |
| L16-9 | Low | README:369 "Any value leaving a script (result, call arguments, printed values) is limited to 128 levels ... in all three languages": Python and Rhai `print` are not held to the value limits (150-deep list printed), JS prints `<unprintable value>` | native repro |
| L16-10 | Low | Stated byte caps are exceeded by the truncation markers: violation 526 B (claim 512), error 4,110 B (claim 4 KiB), log buffer 65,556 B (claim 64 KiB); violations list can hold 17 entries (claim <= 16) | native repro + code path (17th entry) |
| L16-11 | Low | README:468-470 says the local replica does not implement `canister_metadata`; the live log shows fetches answered by `canister_metadata` alone (no fallback message, status Ok) | live audit log |
| L16-12 | Low | "every run is audited" (README:25, TOOL_DESCRIPTION) and "Two kinds of request write no entries" (README:411-412) omit runs that leave no entry: trap before the first message (README:422 documents it) and an update-path search answered from the failed-fetch cache (no message, no entry) | code path |
| L16-13 | Low | "immutable audit log" (README:42): controllers can reinstall (wipes stable memory) or upgrade to code that rewrites it; append-only holds only for this code | code path / platform |
| L16-14 | Low | per-customer-canisters.md: idle cost "mostly storage for the 12 MB Wasm" and "a build without Python is about half that size"; live: memory size 1,246,514,630 B, idle burn 32,710,006,708 cycles/day, module 11.4 MB (< 1 % of memory) | read-only `icp canister status` |

## Grid

| Row | Cell | Reason |
|---|---|---|
| CL1 "every run is audited" | finding L16-12 | Absolute; runs that trap before the first message, and failed-cache update searches, leave no entry. Also see L16-1 (a JS run that burns 40B instructions and traps before sending leaves nothing) |
| CL2 README:14 search "answered by the query call, so it is fast and free" | finding L16-2 | Contradicted by README:271-272 and by `run_search` |
| CL3 SEARCH_DESCRIPTION (shown to AI) | finding L16-2 | "no consensus round, no audit entry, does not count toward the call cap" false for any search that fetches (`src/lib.rs:121-190`); live entries 696/697 |
| CL4 "append-only" / "immutable" | finding L16-13 (immutable); append-only holds | No delete/truncate on `AUDIT` in app code (`policy.rs:247-270`); immutability against controllers not provided |
| CL5 survive upgrades | holds | All of D1-D5 in `ic-stable-structures`, no heap copy; `old_audit_entries_still_decode`. Not across reinstall (not claimed) |
| CL6 allowlist checked before any message, incl. interface fetch | holds | `reserve` precedes `fetch_did` (`bridge.rs:486`) and `service()` (`bridge.rs:515`); override/cached text returned without a check sends no message, so the sentence holds literally |
| CL7 call-cap counting | finding L16-4 | Counting rule as written holds (`reserve` before await); the "Calls per script 25" label and `DEFAULT_MAX_CALLS` doc are false about messages |
| CL8 instruction budget sentence | finding L16-3 | Python part contradicted. Rest holds: checked in `reserve`, JS rounds, Rhai `on_progress`; but see L16-1 for what "checked between JS rounds" leaves out |
| CL9 request limits | holds | `MAX_BATCH` (`mcp.rs:295`), one script + 5 searches (`mcp.rs:392-397`, test `one_tool_call_per_request`), `RunSlot` cap 8 (`lib.rs:207`), JS `MAX_PENDING`/`MAX_ROUNDS`, Rhai `answers.len() >= 64` |
| CL10 value limits in all three languages | finding L16-9 (printed values only) | Results and call args hold (`ValueBudget` in all three, tests in T8/T9/T10) |
| CL11 JS limits | finding L16-1 ("1M loop iterations" is per frame); rest holds | `Math.random` deleted, `eval`/`Function` via `ensure_can_compile_strings`, Atomics/SAB/WeakRef/FinalizationRegistry deleted, `FixedClock(now_ms)`, timezone 0, stringify wrapper (tests `determinism_hardening`, `stringify_replacer_and_size_limits`) |
| CL12 Python limits | holds | recursion 200, 64 MiB, 256 suspensions, fixed datetime, zero sleep, seeded random, OS calls `no_handler_result` (`python.rs:212-273`; tests). Budget part is L16-3 |
| CL13 Rhai limits | holds | `MAX_OPERATIONS` applies per replay (`rhai_rt.rs:19`), so a script's logical execution is held to 1M; replay overhead is the documented O(n^2) limitation. 32 call levels, eval disabled, modules 0 |
| CL14 output caps; reply never traps | finding L16-10 (marker overshoot), L16-7 (Python print); reply part holds | `finish` + `http_update` re-measure, tests `replies_always_fit`, `result_size_is_bounded_after_escaping`; source <= 100 KiB holds (`len() > MAX_CODE_BYTES`) |
| CL15 audit fields and caps; one entry per message + one closing | finding L16-10 (caps), L16-6 (methods doc); per-message rule holds | `record_message` in `on_first_pending`; closing entry in `run_script`/`run_search`/`Drop` |
| CL16 inspect_message filter before charging | holds (untested) | `lib.rs:273-283` accepts only token-bearing `http_request_update` or controllers; "not charged" is a platform property, not verified here (register #30 manual, I18 gap) |
| CL17 log never shows unsent / never misses sent | holds | ic-cdk 0.20.3 `CallFuture::poll` returns Pending iff `call_perform` returned 0 (`call.rs:899-920`); entry written in that same execution; Drop writes `Trapped` in cleanup (executor cancels the method's tasks under `is_recovering_from_trap`). Untested on a replica (register #58) |
| CL18 exactly one closing entry | holds | Normal end writes it before `finish()`; a trap after a send rolls it back and cleanup writes `Trapped`; exceptions are the documented pending/unknown cases |
| CL19 two kinds of request write no entries | finding L16-12 | Third kind: update-path search where every uncached canister is in `DID_FAILED` (`lib.rs:161` skips the entry: no calls, no violations) |
| CL20 missing closing entry = pending or unknown, never proof of trap | holds | Matches `PENDING_RUNS` insert/remove (`bridge.rs:245-247, 415-418`) and `.did` comments. Register #52 still fixed |
| CL21 timeout and certified_reads semantics | holds (timeout cap unverified) | `bounded_wait().change_timeout(secs)`; "outcome unknown" suffix when `!is_clean_reject`; certified mode upgrades every authenticated request and 404s the page (test `certified_mode_...`). The 300 s cap is a platform claim, not tested |
| CL22 cache 1 h, failures 5 min, > 512 KiB refused | holds (untested) | `cache_get`/`failed_get` TTL filters, `n > MAX_DID_BYTES` after fetch and in `set_candid_override`. See L16-5: an override suppresses the refresh |
| CL23 canonical ids before allowlist, cache, override | holds | `parse_principal(..).to_text()` keys; `search::scope` compares `Principal`s; test `scope_canonicalises_and_reports_unknowns` |
| CL24 known limitations | finding L16-1 ("Every other path stops with a clean error"), L16-11 (pocket-ic sentence); others hold | #23/#24/#42/#48/#49/#51 statements match code |
| CL25 release Wasm imports only ic0 | holds (default build) | baseline: 36 imports, all `ic0.*`; subsets not linked (not claimed separately) |
| CL26 getrandom only for hash seeding; never for secrets | holds in effect | Also feeds Monty's boundary uuids (`monty-1.6.0/src/boundary_uuid.rs:16`), not observable by scripts (probe: `A()` repr has no id). Python `random` uses its own seed, not this stream |
| CL27 shim makes Monty limits instruction budgets | holds | `Instant::now()` = `performance_counter(1)` (`shims/web-time/src/lib.rs:15-17`) |
| CL28 skip-certification; script results from update | holds | headers only when `data_certificate()` present; every `execute_icp_script` upgrades (`mcp.rs:366`) |
| CL29 "identical semantics in every language"; "No ... randomness" | finding L16-7, L16-8 | Python print overflow aborts; Python `random` present |
| CL30 demo page: no secrets, token only as Authorization header, only hash on-chain | holds | `demo.html:204-231`: token in localStorage, sent only in the `Authorization` header to same-origin `/mcp`; raw panel shows the body, not headers; `add_api_token_hash` stores hashes |
| CL31 `active_runs` at most 8; `pending_run_ids` waiting on a reply | holds (literal) | `active_runs` counts scripts only (searches take no `RunSlot`, so it can read 0 while searches are pending); `pending_run_ids` includes searches. Uncapped concurrent searches are an L4 matter, not a false sentence |
| CL32 audit page newest first, cap 100 | holds | `audit_page`, test `audit_log_pages_newest_first` |
| CL33 benchmarks; 12 MB / 3.6 MB gz; JS + Rhai about 6 MB | holds (sizes); not tested (ratios, 6 MB) | cached artifact 11,402,905 B, `gzip -9` 3,641,028 B. Benchmark ratios need the live demo page; JS+Rhai size needs a wasm build (Live only) |
| CL34 stack 8 MiB rationale | holds | `.cargo/config.toml:9`; rationale consistent |
| CL35 Rhai replay determinism; divergence reported | holds (divergence path untested) | Rhai has no time/random (`no_time`, raw engine), maps ordered, answers keyed by sequence and compared (`rhai_rt.rs:71-75`) |
| CL36 searches "cheap and size-capped", exempt from one-script rule | holds (loosely) | Each search has its own cap and fetches each canister at most once per TTL (success and failure cached), output capped at 40 methods. Not a promise to the user |
| CL37 per-customer cost figures | finding L16-14 | Live memory 1.25 GB dominates idle burn; module is < 1 % |
| I10 Honest interface | violated | L16-2, L16-3, L16-4, L16-5, L16-6, L16-7, L16-8 (and Low ones). No test compares docs/descriptions with behaviour (gap) |
| Register #52 | still fixed | README:414-426, `.did` `Started` comment, `bridge.rs:196-197` all say pending or unknown |
| Register #53 | still fixed, sibling open | `calls()` = `messages.len()` (`bridge.rs:332-334`); the cap still counts slots, so `calls` and the cap measure different things (L16-4) |

## Not tested

- On-chain trap for L16-1 (needs a wasm build and a script run on the local network: Live phase).
  Suggested script: `function f(){for(let i=0;i<999999;i++){}} for(let k=0;k<999999;k++) f(); return 1;`.
- L16-4 live: needs an allowlisted target whose `canister_metadata` fails (no public `candid:service`) so the fallback sends a second message.
- `inspect_message` not charging (CL16), the IC 300 s timeout cap (CL21), certified-reads latency figures, benchmark ratios (CL33), JS+Rhai build size.
- Idle-cost attribution was observed on one local network whose memory may include this audit's own stress runs; the attribution to module size is wrong regardless of how the 1.25 GB arose.
