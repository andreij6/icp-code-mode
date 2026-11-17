# Findings register

The app-specific half of the audit ([PLAN.md](PLAN.md) is the generic half). Every audit
appends to this file and re-checks what is already in it. Each row is also a regression
check: the next audit confirms that fixed findings stay fixed and that open ones are still
accurately described.

**Guard** says what would catch a regression:

- a test name: an automated test covers it;
- **manual**: it was verified by hand on a local network and has no automated test;
- **none**: nothing guards it.

Rows marked manual or none are the automation backlog.

**Status:** fixed, mitigated (reduced, not removed), open (not fixed), or accepted (a known
limit, stated in the README).

## Rounds so far

| Round | Date | Source |
|---|---|---|
| R1 | 2026-11-17 | Internal review from five angles, each finding re-checked by a second reviewer |
| R2 | 2026-11-17 | Outside review of the search tool |
| R3 | 2026-11-17 | Reader of the audit code, on X |
| R4 | 2026-11-17 | Internal follow-up hunt; findings 4, 5 and 30 reproduced on a local network |
| R5 | 2026-11-17 | Outside review of the whole repository |
| R6 | 2026-11-17 | Reader on X, plus an internal pass over the audit path against the platform's commit rules |
| A1 | 2026-11-17 | First full run of PLAN.md at `295d64b`, depth full, 5 rounds, Live phase on a disposable local deployment. Report: [runs/2026-11-17-295d64b/REPORT.md](runs/2026-11-17-295d64b/REPORT.md) |
| F1 | 2026-11-17 | Fix round for A1 (branch `fix-cross`): nine fix groups and one cross-runtime group, each fix reviewed and repaired once; native tests only, nothing deployed. Boa, Monty and candid are now vendored with patches (`vendor/*/PATCHES.md`). In the status column, "F1" means changed in this round and "before F1" means fixed by the commits between A1 and this round (fc76d85, 4565e4e, 91409d1). Live checks of the F1 fixes are left to the next audit |

## Findings by class

Classes are the lenses in the plan.

### L1 Access control

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 1 | R1 | High | The interface cache and admin overrides were keyed by the caller's spelling of a canister id, so a different letter case missed both | Fixed; re-checked A1; search now parses ids exactly as calls do (186) | search::scope_canonicalises_and_reports_unknowns, search::scope_parses_ids_exactly_as_calls_do; bridge path: manual (A1, native) |
| 105 | A1 | Medium | A registered SHA-256 of the empty token makes an empty bearer authenticate | Fixed before F1 | none (the empty-hash refusal in `add_token_hash` has no test) |
| 147 | A1 | Low | The allowlist accepts this canister's own principal and `aaaaa-aa` with `"*"` | Fixed F1 | policy::privileged_targets_are_refused |
| 193 | A1 | Low | A wildcard allowlist admits delegating methods (e.g. `icrc2_approve`) whose authority outlives token revocation and `remove_allowlist` | Fixed F1: `"*"` excludes delegating methods; they must be listed by name | policy::wildcard_excludes_delegating_methods |
| 195 | A1 | Low | An empty allowlist method list is an undocumented "interface only" state | Fixed F1 (documented in the README and `.did`) | policy::interface_fetch_and_overrides_and_token_edge_cases |
| 196 | A1 | Low | `canister_metadata("candid:service")` returns a private interface when this canister controls the target, and it is served to token holders | Fixed F1: `canister_metadata` is skipped when this canister controls the target | bridge::concurrent_requests_share_one_fetch_and_its_failure, bridge::fallback_failure_after_a_failed_canister_info_is_not_remembered |
| 198 | A1 | Low | The wildcard `"*"` cannot be told apart from a Candid method literally named `*` | Accepted (README): telling them apart needs a breaking allowlist type | none |

### L2 Atomicity and commit points

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 2 | R1 | High | A reply over the size limit trapped after calls had run, which rolled back the audit entry | Fixed; re-checked A1; a batch reply can no longer approach the size cap (112, before F1) | mcp::result_size_is_bounded_after_escaping |
| 3 | R3 | High | A run's first and final audit entries were linked only by script hash, so two runs of the same script could not be told apart | Fixed; re-checked A1 (live reconciliation); F1 adds a test of the production path | policy::run_id_links_started_and_final_entries, bridge::a_sent_call_opens_one_started_entry_and_a_pending_run |
| 4 | R4 | High | A batch padded with tiny invalid messages made the reply trap, erasing the run's audit entry | Fixed; re-checked A1 (10.45 MB id-echo batch) | mcp::replies_always_fit |
| 5 | R4 | High | A run that trapped after its calls went out showed "Started, 0 calls" with no methods | Fixed; re-checked A1 live; instructions on Trapped is a documented lower bound (83) | manual |
| 6 | R2 | Medium | A search refused by policy before any message was sent wrote no audit entry | Fixed; re-checked A1; F1 audits remembered-failure searches too (151) | manual |
| 7 | R6 | High | Message entries were written before argument encoding, so a call that failed to encode left an entry for a message never sent | Fixed; re-checked A1 (868 messages reconciled) | manual |
| 8 | R6 | Medium | A message the system refused on the spot still left an entry | Fixed; re-checked A1 live | manual; bridge::a_sent_call_opens_one_started_entry_and_a_pending_run covers the accepted case only (the old mechanism test was removed in F1) |
| 9 | R6 | Medium | A trapped run had no closing entry | Fixed; re-checked A1 live; F1 fixed the cleanup trap (150) and batch misattribution (143) | manual; policy::append_over_full_memory_is_an_error (cleanup append) |
| 10 | R1 | Medium | A runtime limit hit after an await was reported as a promise that never settles | Fixed F1: a stop inside the VM or after an await reports the limit (59) | runtimes/js::deadline_interrupts_vm_and_native_loops, runtimes/js::per_run_memory_cap |
| 11 | R4 | Low | A run that traps before sending any message leaves no audit entry | Accepted; re-checked A1; F1 bounds the 40e9 traps A1 found (59-65, 78); one built-in call can still trap (README) | none |
| 76 | A1 | High | JavaScript and Python dispatch the last allowed round, then report the run failed and drop the reply | Fixed before F1 | runtimes/js::pending_and_round_limits_are_enforced |
| 77 | A1 | High | The instruction budget is checked before the interface fetch await and not again before the send; a message sent past the budget is then reported as a policy refusal | Fixed F1: over budget after a send closes as `Error` listing the messages, not a policy refusal | lib::budget_overrun_after_sends_is_not_a_policy_refusal, bridge::budget_refusal_after_sends_is_not_a_violation |
| 82 | A1 | Medium | Audit entries for an interface fetch name only the management canister; the documented `canister.<candid interface>` label is never produced | Fixed: fetch labels before F1; F1 refuses the management canister as a call target | policy::privileged_targets_are_refused; fetch labels: none |
| 83 | A1 | Medium | A Trapped entry's `instructions` omits the trapped execution (about 2-25M recorded for a run that hit 40e9) | Accepted F1: documented as a lower bound (README, `.did`, the entry's error text) | lib::did_file_matches_exported_interface (keeps the comment) |
| 115 | A1 | Medium | Records cannot show whether a sent message took effect, and errors raised after a target executed do not say so; README claims the outcome is in the script's result or error | Fixed F1: decode and value-limit errors say the call executed; audit entries still record no reply status (README) | bridge::undecodable_reply_names_the_call_and_says_it_executed, candid_bridge::replies_over_the_value_limits_are_refused_after_decoding |
| 120 | A1 | Medium | A trap while handling one reply of a batch frees the run slot and pending id while sibling calls are still outstanding | Fixed F1: a trapped run keeps its slot and pending id for 6 minutes (README) | lib::run_slots_linger_after_a_trap_and_expire_when_stale (helper only; the trap path needs a replica) |
| 143 | A1 | Medium | In a batch, a trap in a later tool call closes an earlier, completed search as Trapped, and the trapping script leaves no entry | Fixed before F1 (one tool call per request) | mcp::one_tool_call_per_request |
| 149 | A1 | Low | Audit violation lists lose information: the Trapped entry drops the "...and N more" count, and the first 16 refusals (chosen by the script) are kept | Fixed F1: first 8 and latest 8 kept, then "…and N more" | bridge::late_violations_are_kept |
| 150 | A1 | Low | A trap inside the cleanup callback leaks a run slot, a pending id and the run's task until upgrade | Fixed F1: the cleanup append never traps; lost entries are counted | policy::append_over_full_memory_is_an_error (the Drop path needs a replica) |
| 151 | A1 | Low | Searches answered from a remembered failure, or whose fetch the system refused at send time, write no audit entry; the README's list of no-entry cases omits them | Fixed F1: both cases write an `Error` entry; README updated | none (needs a replica) |
| 194 | A1 | Low | Audit entries record `canister.method` and a script hash but never arguments, so recipient, spender or amount cannot be reconstructed | Fixed F1: `args` holds the encoded argument (hex, or a digest past 1 KiB) | policy::arg_digest_is_bounded |

### L3 Resource bounds

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 12 | R1 | High | A deeply nested JavaScript value overflowed the stack when converted | Fixed; re-checked A1; other native stack overflows: 84 (mitigated F1), 142 (fixed F1) | runtimes/js::deep_and_huge_values_are_rejected_not_overflowing |
| 13 | R1 | High | The Rhai replay loop had no cap on host requests | Fixed; re-checked A1; guard hangs on regression (134) | runtimes/rhai_rt::request_count_bounds_replays |
| 14 | R1 | High | Script output buffers grew without limit while a script ran (Rhai and JavaScript) | Fixed; guards made effective in F1 (R14: the mock no longer re-applies the cap) | runtimes/rhai_rt::print_is_capped_per_run, runtimes/js::console_output_is_capped_while_running, runtimes::output_caps_are_measured_alike |
| 15 | R1 | High | Deeply nested Rhai arrays cost far more engine work than the operation count showed | Fixed; re-checked A1 | manual |
| 16 | R1 | Medium | Audit error and violation strings were stored at any length | Fixed for new entries; marker overshoot fixed F1 (175); legacy entries readable F1 (96) | bridge::truncation_stays_within_max, policy::oversized_entries_are_readable_and_pages_bounded |
| 17 | R1 | Medium | A valid reply with a large unknown field was rejected, and the error embedded the whole reply | Fixed again F1: decoding quota raised to the full 2 MiB reply (87) | candid_bridge::large_replies_decode_and_over_quota_says_so |
| 18 | R5 | High | A self-rescheduling promise chain bypassed the loop and recursion limits | Fixed; re-checked A1; guard hangs on regression (134) | runtimes/js::promise_chains_are_bounded |
| 19 | R5 | High | A `JSON.stringify` replacer could feed an unbounded value to the native serializer | Fixed; re-checked A1; guard aborts on regression (134); see 111 | runtimes/js::stringify_replacer_and_size_limits |
| 20 | R5 | High | Arrays of primitives were not counted against the element limit | Fixed; re-checked A1 | runtimes/js::stringify_replacer_and_size_limits |
| 21 | R5 | High | One large string referenced many times was small inside the engine and gigabytes once copied out (all runtimes) | Fixed again F1 (Python to_json rewrite 64; JS key bytes 66 fixed before F1) | runtimes/js::stringify_replacer_and_size_limits, runtimes/js::snapshot_counts_object_key_bytes_against_limit, runtimes/python::values_are_bounded, runtimes/python::exports_dispatch_on_the_value_kind |
| 22 | R5 | Low | Interface text of any size was fetched and parsed | Fixed; re-checked A1; F1 bounds parse cost by shape (74) and ignores oversized legacy overrides (80) | candid_bridge::interface_shape_limits, policy::overrides_need_the_allowlist_and_the_size_cap |
| 23 | R1 | High | One uninterruptible built-in JavaScript operation can run past the platform's instruction limit and trap | Accepted F1: the A1 causes are fixed (59-65, 70, 73, 74, 78); one built-in call (a backtracking JS regex, a linear Python operation on a huge value) can still trap (README) | runtimes/js::deadline_interrupts_vm_and_native_loops, runtimes/python::engine_work_is_bounded, runtimes/python::expensive_operations_poll_the_time_limit |
| 24 | R5 | Medium | JavaScript has no memory cap | Fixed F1: 96 MiB heap cap per JavaScript run (71) | runtimes/js::per_run_memory_cap, runtimes/js::dropped_run_frees_its_heap |
| 59 | A1 | High | JavaScript code between host calls is never charged against the instruction budget; Boa's 1M loop limit is per call frame | Fixed F1: patched Boa polls the budget every 64 VM steps and inside built-in loops | runtimes/js::deadline_interrupts_vm_and_native_loops, runtimes/js::default_sort_polls_the_interrupt, runtimes::instruction_deadline_stops_every_runtime |
| 60 | A1 | High | Monty native operations run uninterruptibly: bigint arithmetic, repr, regex, decimal and collection operations run past the budget to the 40e9 trap | Fixed F1: patched Monty caps big ints, decimal, regex backtracking and hash probes, and polls time | runtimes/python::engine_work_is_bounded, runtimes/python::expensive_operations_poll_the_time_limit |
| 62 | A1 | High | Python's budget is a one-time relative allowance on a clock that pauses during host work | Fixed F1: a fresh allowance from the live counter before every resume; turns capped at 20B | runtimes/python::host_time_is_deducted_from_the_vm_budget, runtimes/python::native_vm_budget_is_capped_by_the_wall_budget |
| 63 | A1 | High | Untrusted source and interface text are compiled outside the budget, and the compilers are quadratic | Fixed before F1 (16 KiB source cap, budget check after compile) | runtimes::unknown_language_and_oversized_code |
| 64 | A1 | High | Python `to_json` uses `py_repr` as a catch-all: no size or cost bound, one node per repr, dispatch on a script-chosen class name | Fixed F1: export dispatches on the node kind; no repr catch-all | runtimes/python::exports_dispatch_on_the_value_kind |
| 65 | A1 | High | Decimal text for `nat`/`int` is parsed quadratically inside `encode_args`, with no budget check before the send | Fixed before F1 (decimal text capped at 1,000 digits before parsing) | candid_bridge::deep_replies_error_instead_of_truncating |
| 66 | A1 | High | JavaScript object-key text is added to the 4 MiB budget but never compared against it | Fixed before F1 | runtimes/js::snapshot_counts_object_key_bytes_against_limit |
| 67 | A1 | High | Rhai's size limits ignore map keys and curried function pointers, and Rhai has no other memory cap | Mitigated F1: 128 MiB per-run memory cap counting map keys, captures and cached replies; `pad` checked. Remains: nesting depth (`x = [x]`) can still overflow the stack (README) | runtimes/rhai_rt::memory_cap_counts_map_keys_and_captures |
| 69 | A1 | High | Monty allocates outside its 64 MiB tracker: `json.dumps(indent=...)` and `re.compile` trap or grow the heap permanently | Accepted F1 (README): a rolled-back trap, not a Python error | none (in the harness corpus, which has not run) |
| 70 | A1 | High | Engine built-ins allocate from script-chosen sizes before any limit; on wasm32 this is an uncatchable "capacity overflow" trap | Fixed F1: pre-sizing from script lengths capped in patched Boa; native loops poll the heap cap | runtimes/js::per_run_memory_cap, runtimes/js::deadline_interrupts_vm_and_native_loops |
| 71 | A1 | High | A JavaScript run can grow the Wasm heap past `wasm_memory_limit` inside response callbacks; afterwards every update, the controller's included, is refused | Fixed F1: 96 MiB per-run heap cap, the last run frees the shared heap, busy past half the ceiling, 3 GiB `wasm_memory_limit` | runtimes/js::per_run_memory_cap, runtimes/js::dropped_run_frees_its_heap (the busy path and low-memory hook: none) |
| 78 | A1 | High | Search work is keywords x allowed methods with no keyword cap, and the budget is checked only after rendering | Fixed before F1 (query capped at 256 bytes on both routes) | mcp::long_search_queries_are_refused |
| 84 | A1 | Medium | Small inputs overflow the native stack in the Boa parser and in Boa built-ins on deep data | Mitigated F1: source nesting refused by a lexical scan; native stack floor in patched Boa. Remains: the scan can be misled at `/` after `)` or `}` (README) | runtimes/js::deep_nesting_is_refused |
| 86 | A1 | Medium | Candid conversion cost scales with the interface's shape, not the value budget (variant tags, record fields, nested inline types, decoder memo) | Fixed F1: binary-search label lookup, interface shape caps, opt memo charged in patched candid | candid_bridge::label_lookup_is_not_keys_times_fields, candid_bridge::opt_values_pay_for_the_subtype_memo |
| 92 | A1 | Medium | Call arguments are never checked against the inter-canister payload limit; JS counts text in UTF-16 units, so an argument inside the value limits traps the canister | Fixed before F1 (encoded arguments over 2,000,000 bytes refused before sending) | none |
| 117 | A1 | Medium | `set_candid_override` validates by parsing in an update (40e9) but query searches re-parse under the 5e9 query limit and cannot keep the result | Fixed before F1 (the override is parsed once, in the update) | none |
| 118 | A1 | Medium | JavaScript `console.log` formats every argument in full before the 64 KiB cap; one call grew the heap by 630 MB permanently | Fixed before F1 (formatting stops at the remaining log room) | runtimes/js::console_output_is_capped_while_running |
| 119 | A1 | Medium | JavaScript `icp.call`/`icp.candid` copy canister-id and method strings of any size out of the sandbox | Fixed before F1 (id and method length checked before copying) | runtimes/js::audit_round_two_js_fixes |
| 131 | A1 | Medium | The error exit of the JS and Python sandboxes skips the value limits: an `AggregateError` with a huge `errors.length` traps, and thrown error text of any size is copied | Fixed F1: thrown values formatted bounded, cut to 8 KiB in both runtimes | runtimes/js::thrown_values_are_bounded, runtimes/python::raised_error_text_is_bounded |
| 145 | A1 | Medium | Hash tables keyed by untrusted data use public or absent seeds; a hostile reply with colliding keys makes JavaScript's reply conversion quadratic | Fixed F1: reply conversion within the value budget; at most 16,384 keys across objects over 64 keys | candid_bridge::reply_conversion_is_within_the_value_budget |
| 175 | A1 | Low | Truncation markers make strings exceed their caps, and LogBuf then discards the whole cut line | Fixed F1: truncation never exceeds its cap; a cut line keeps its prefix | bridge::truncation_stays_within_max, bridge::log_lines_keep_the_prefix_charge_newlines_and_mark_drops, runtimes::output_caps_are_measured_alike |
| 176 | A1 | Low | The log cap counts raw line bytes only: empty lines are free and the client-side cap uses escaped bytes | Fixed F1: one measure (JSON-escaped, plus 2 per newline) in every runtime and the transport | runtimes::output_caps_are_measured_alike |
| 188 | A1 | Low | Rhai's expression-depth limit is consumed by flat operator chains | Fixed F1: 128 levels (64 in a function), with an error that says how to split | runtimes/rhai_rt::long_expressions_parse_and_too_deep_ones_say_how_to_split |
| 191 | A1 | Low | The composite_query refusal comes after a cap slot, a possible fetch and full argument encoding | Fixed F1: refused before argument encoding | bridge::call_refuses_composite_query_before_encoding, bridge::composite_query_refused_before_encoding |

### L4 Multiplication

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 25 | R1 | Medium | Interface fetches did not count toward the per-script call cap | Fixed; re-checked A1; every fetch message takes its own slot (152, fixed before F1) | manual |
| 26 | R1 | Medium | Scripts batched in one request shared one instruction counter | Fixed; re-checked A1; one tool call per request (126, fixed before F1) | mcp::one_tool_call_per_request |
| 27 | R2 | Low | A search kept attempting fetches after the call cap was reached | Fixed; re-checked A1 | manual |
| 28 | R5 | Medium | A batch of searches multiplied outbound calls and audit entries | Fixed; re-checked A1; both routes allow one tool call per request (89, fixed before F1) | mcp::one_tool_call_per_request |
| 29 | R5 | Medium | Unlimited concurrent scripts could exhaust memory | Fixed; re-checked A1 live; F1 adds the per-token share and a 600 s run limit (121) | lib::run_slots_are_capped_and_returned, lib::fetching_searches_take_run_slots |
| 88 | A1 | Medium | Concurrent cache misses on one interface each send their own fetch (no single-flight) | Mitigated F1: one fetch per run, shared with its outcome. Remains: separate requests each fetch (ic-cdk cannot resume one update from another's reply; README) | bridge::concurrent_requests_share_one_fetch_and_its_failure |
| 121 | A1 | Medium | Run slots are one global pool of 8 with no per-caller share | Fixed F1: 4 slots per token and a 600 s limit per run | bridge::run_time_limit_cuts_timeouts_then_refuses, lib::run_slots_are_capped_and_returned |
| 126 | A1 | Medium | Each tool call has its own instruction budget, but up to 5 searches and a script in one HTTP request share one execution | Fixed before F1 (one tool call per request) | mcp::one_tool_call_per_request |
| 152 | A1 | Low | One interface-fetch cap slot can send two messages, so the audit `calls` field exceeds `max_calls_per_script` | Fixed before F1 (each fetch message reserves its own slot) | none |
| 153 | A1 | Low | Searches take no run slot: concurrent searches suspended on fetches are uncapped and not counted in `active_runs` | Fixed F1: fetching searches take a run slot | lib::fetching_searches_take_run_slots |
| 168 | A1 | Low | A getter in JavaScript call arguments bypasses the 64-concurrent-request cap | Fixed F1: the cap is rechecked right before each push | runtimes/js::pending_cap_holds_against_reentrant_calls |

### L5 Cost and payment

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 30 | R4 | High | Anyone could call the update endpoint directly, without a token, and make the canister pay for it | Fixed for ingress; re-checked A1; canister callers pay to be refused, documented (154) | manual |
| 31 | R4 | Medium | A token holder can grow the audit log without limit; nothing rate-limits requests | Open; re-checked A1; F1 counts every authenticated update against the rate and adds a cycles floor; no retention or export (180) | none |
| 90 | A1 | Medium | The update path accepts and pays for any authenticated body with no size cap; work that runs no tool leaves no record, and the parser holds three full copies | Fixed F1: only a parsing `tools/call` passes the filter, every authenticated update is rate-counted, and the update path refuses requests without a tool call | mcp::update_refuses_requests_without_a_valid_tool_call, mcp::only_upgradable_requests_pass_the_cost_filter |
| 91 | A1 | Medium | The query path ignores the failed-fetch cache, so every search touching an unfetchable interface becomes a paid, unaudited update | Mitigated: fixed before F1; since F1 (151) a remembered failure is answered by an update so it is audited, which costs an update call | none |
| 125 | A1 | Medium | Cycles reserved by in-flight calls have no global cap; at a thin liquid margin other callers' sends fail while the canister reports Running | Fixed F1: at most 64 messages in flight, 32 per token, and no sends under 100B liquid cycles | bridge::in_flight_messages_are_capped_per_token_and_overall, lib::runs_need_a_cycles_margin |
| 148 | A1 | Low | `demo_store` accepts unauthenticated, unbounded writes and its arithmetic oversells, wraps and saturates | Fixed F1: orders capped at 20 lines and 1,000 per store, overflow checked; no access control by design (README, local demo only) | demo_store::orders_are_bounded, demo_store::read_error_and_write_edge_cases |
| 154 | A1 | Low | Calls from other canisters bypass the `inspect_message` cost filter, so a tokenless caller costs ~5.6M cycles per call | Accepted F1 (README, inspect_message doc) | none |

### L6 Consistency across variants

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 32 | R5 | Medium | Python used a fixed instruction budget instead of the configured one | Fixed; host work is charged since F1 (62); docs fixed before F1 (109) | runtimes/python::host_time_is_deducted_from_the_vm_budget, runtimes/python::budget_spans_turns_split_by_host_calls |
| 33 | R5 | High | Python returned deep data truncated with a marker and reported success, where the other runtimes refuse it | Fixed again F1 (R33: deep values are exact or refused) | runtimes/python::values_are_bounded, runtimes/python::deep_call_arguments_are_exact_or_refused |
| 81 | A1 | Medium | An un-awaited `icp.call` (JS) or `icp.call_async` (Python) is never sent but the run reports Ok; Rhai sends it | Fixed: the main case before F1; F1 reports requests queued by result conversion or before a throw. Rhai sends synchronously by design | runtimes/js::requests_queued_by_result_conversion_are_reported |
| 89 | A1 | Medium | The five-searches-per-request cap is enforced only on the update path | Fixed before F1 (one tool call per request on both routes) | mcp::query_allows_one_tool_call_like_update |
| 93 | A1 | Medium | Python fails the whole run with a MemoryError once output passes 64 KiB, where JS and Rhai truncate; printed values are formatted in full first | Fixed F1: cut with the marker, later prints not formatted | runtimes/python::print_past_the_cap_is_not_formatted, runtimes/python::audit_round_two_python_fixes |
| 94 | A1 | Medium | Rhai applies its engine size limits to host replies, summed over the whole nested value; valid replies are refused after the call ran | Fixed F1: replies are held to the value limits in the bridge, so every runtime accepts the same replies | runtimes/rhai_rt::replies_within_the_value_limits_fit_rhai, candid_bridge::replies_over_the_value_limits_are_refused_after_decoding |
| 95 | A1 | Medium | Per-language builds advertise, default to and accept languages that are not compiled in | Fixed F1 | runtimes::only_compiled_languages_parse |
| 139 | A1 | Medium | JavaScript `icp.call`/`icp.candid`/`icp.principal` silently ignore extra arguments; Python and Rhai refuse them | Fixed F1 | runtimes/js::extra_arguments_are_refused |
| 156 | A1 | Low | The 128-level depth limit is off by one between runtimes | Fixed F1: one depth rule in every runtime and the bridge | runtimes::nesting_depth_is_the_same_everywhere, bridge::shared_value_rules |
| 162 | A1 | Low | Python sees block time truncated to whole seconds while JavaScript sees milliseconds | Fixed F1 | runtimes/python::block_time_keeps_milliseconds |
| 185 | A1 | Low | Python has no per-batch request cap; JavaScript refuses the 65th concurrent request | Fixed F1: 64 per gather batch, as in JS | runtimes/python::gather_batches_are_capped_like_js |
| 186 | A1 | Low | Search trims a whitespace-padded `canister_id` that `icp.call`/`icp.candid` refuse | Fixed F1 | search::scope_parses_ids_exactly_as_calls_do |
| 187 | A1 | Low | `inspect_message` decodes the request without the skipping quota the method uses, admitting ingress that then traps on decode | Fixed F1 | lib::inspect_message_decoder_matches_the_method |

### L7 Data fidelity

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 34 | R1 | High | JavaScript integers above 2^31 were rejected as natural numbers | Fixed; re-checked A1; F1 refuses floats beyond 2^53 and returns big Python/Rhai ints as strings (114) | runtimes/js::big_js_integers_pass_as_nat, runtimes::big_integers_leave_as_strings |
| 35 | R1 | Medium | Misspelled record fields were silently dropped, so an optional field was sent as null | Fixed for records; re-checked A1; F1 fixed aliases (160), func keys (159) and tool args (103) | candid_bridge::errors_are_descriptive |
| 36 | R1 | Medium | Replies nested too deep came back truncated and marked as success | Fixed; re-checked A1; F1 fixed the double opt count (127) and the decoder stack overflow (142) | candid_bridge::deep_replies_error_instead_of_truncating |
| 37 | R1 | Low | For single-argument methods, `[x]` and `[]` were interpreted inconsistently | Fixed again F1 (135) | candid_bridge::single_array_argument_round_trips |
| 38 | R1 | Low | Python bytes and Rhai blobs could not be passed as binary data | Fixed; re-checked A1; JS typed arrays leave as arrays since F1 (155) | runtimes/python::determinism_and_sandbox, runtimes/rhai_rt::limits_and_sandbox |
| 39 | R1 | Low | Non-finite numbers decoded to a value that could not be sent back | Fixed; re-checked A1; the strings are documented since F1 (155) | candid_bridge::js_style_floats_and_non_finite |
| 40 | R5 | Low | Negative zero was rejected as a natural number | Fixed; re-checked A1; JS -0 keeps its sign since F1 (155) | candid_bridge::js_style_floats_and_non_finite |
| 41 | R2 | Low | Search results cut by the method cap showed an empty method list | Fixed; re-checked A1 | search::unfetched_and_failed_interfaces_and_cap |
| 42 | R1 | Low | A doubly optional value cannot be told apart from a missing one | Accepted; re-checked A1 | none |
| 68 | A1 | High | Rhai `sort`/`dedup` swallow comparator errors, returning wrong data as success or trapping on replay | Fixed before F1 | runtimes/rhai_rt::sort_comparator_errors_are_not_swallowed |
| 87 | A1 | Medium | The reply decoding quota admits only ~400 KB because untyped decoding costs 50x; larger valid replies fail with a misleading type-mismatch error | Fixed F1: quota sized for a full 2 MiB reply, errors name the cause | candid_bridge::large_replies_decode_and_over_quota_says_so |
| 97 | A1 | Medium | Results over 1 MB are cut to partial JSON and returned with `isError: false`, and audited Ok | Fixed before F1 (an oversized result is an error, audited as Error) | mcp::result_size_is_bounded_after_escaping |
| 98 | A1 | Medium | JavaScript call arguments go through JSON text and back: depth 128 becomes `null`, and floats can be sent 1 ulp off | Fixed before F1 (arguments pass as values, not text) | none |
| 111 | A1 | Medium | The JavaScript value snapshot skips the spec's unwrapping of boxed primitives and applies an array replacer only after walking every key | Fixed: boxed primitives before F1; F1 makes an array replacer read only the listed keys | runtimes/js::stringify_array_replacer_reads_only_listed_keys |
| 114 | A1 | Medium | Integers at or above 2^53 cross the JSON boundary as if exact: JS numbers are sent rounded, and Python/Rhai results above 2^53 leave as numbers | Fixed F1: inexact floats beyond 2^53 refused on input; Python/Rhai results beyond 2^53 leave as strings | runtimes::big_integers_leave_as_strings, candid_bridge::deep_replies_error_instead_of_truncating |
| 122 | A1 | Medium | A stale or overridden interface makes Candid's `opt` rule turn real reply data into `null`, audited Ok | Mitigated F1: reply mismatches are caught and named. Remains: an argument with a changed `opt` type can reach an upgraded target as null for up to an hour (cache not keyed on module hash; README) | candid_bridge::reply_type_errors_name_the_mismatch |
| 127 | A1 | Medium | `MAX_DEPTH` counts Candid levels in the bridge but JSON levels in the runtimes; `opt`-recursive lists are refused from 65 nodes, replies only after the call ran | Fixed F1 | candid_bridge::opt_cycles_and_list_depth_limit |
| 135 | A1 | Medium | The single-argument `[x]` heuristic changes a reply passed back unchanged; `[]` for a single `opt vec` argument is sent as `opt vec {}` | Fixed F1: an array is first read as the argument itself | candid_bridge::single_array_argument_round_trips |
| 155 | A1 | Low | Values with no JSON form are dropped, nulled, stringified or refused differently in each runtime | Fixed F1: one rule for values with no JSON form; functions remain the documented difference | runtimes::values_without_a_json_form_follow_one_rule |
| 157 | A1 | Low | `float32` arguments saturate silently and decimal or integer input is rounded twice | Fixed F1 | candid_bridge::floats_round_once_and_never_overflow_silently |
| 159 | A1 | Low | The JSON-to-Candid converter accepts and discards data for some types and accepts malformed text | Fixed F1 | candid_bridge::strict_null_numbers_and_func_objects |
| 160 | A1 | Low | The mapping between Candid labels and JSON keys is not one-to-one: aliases collapse and distinct fields print the same | Fixed F1 | candid_bridge::field_keys_are_unambiguous_and_round_trip |
| 161 | A1 | Low | String results are returned raw, so `"42"` and `42` produce identical tool text | Accepted F1: documented in the tool description and README | mcp::string_results_are_raw_text |
| 177 | A1 | Low | JavaScript `console.log` and uncaught-error text pass values through the JSON snapshot | Fixed F1 | runtimes/js::console_and_uncaught_text |
| 189 | A1 | Low | Blob replies cost one value-budget node per byte, so a blob over 200,000 bytes cannot be passed back | Mitigated F1: `["<hex>"]` works for a blob argument. Remains: blob replies over ~200 KB are refused (one element per byte; a contract decision; README) | candid_bridge::a_wrapped_hex_string_is_a_blob_argument, candid_bridge::replies_over_the_value_limits_are_refused_after_decoding |
| 200 | A1 | Low | Candid interface text with `import` declarations is accepted and the imports silently dropped | Fixed F1: imports are refused | candid_bridge::interface_shape_limits |

### L8 Concurrency and shared state

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 43 | R1 | Medium | Python's memory limit used a process-wide baseline that concurrent runs overwrote | Fixed; guard made effective in F1 (R43); replies counted since F1 (100) | runtimes/python::memory_limit_survives_interleaved_runs |
| 99 | A1 | Medium | Boa's process-wide GC heap charges one run's instructions for other runs' objects | Mitigated F1: a run collects only when no other JS run is live. Remains: automatic collections still bill whoever triggers them (README) | runtimes/js::only_the_last_run_collects |
| 100 | A1 | Medium | Python's memory limit does not count host replies, so a run can hold several times 64 MiB | Fixed F1 | runtimes/python::kept_replies_count_against_memory |
| 141 | A1 | Medium | Allowlist and token checks made before a suspension are trusted after it | Fixed F1: allowlist and token rechecked before every send and before caching | bridge::removal_and_revocation_during_an_await_are_honoured, bridge::candid_does_not_cache_for_a_canister_removed_during_the_fetch |
| 163 | A1 | Low | Within one run the instruction budget and call timeout are snapshots, while other checks read live limits | Fixed F1: one limits snapshot per run | bridge::run_keeps_the_limits_it_started_with |

### L9 Failure handling

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 44 | R4 | High | Calls wait indefinitely, so a target that never replies can block stopping and upgrading. A timeout option exists; it is off by default | Fixed F1: every message is bounded-wait (300 s default) and a run lasts at most 600 s (75, 116, 125) | bridge::run_time_limit_cuts_timeouts_then_refuses |
| 45 | R4 | Medium | Fetched interfaces were cached until the next upgrade | Fixed; expiry observed live A1 | bridge::interface_ttls_hold_to_the_nanosecond |
| 46 | R5 | Low | Failed interface fetches were repeated on every request | Fixed; expiry observed live A1; only definitive failures are remembered since F1 (136) | bridge::only_definitive_fetch_failures_are_remembered, bridge::interface_ttls_hold_to_the_nanosecond |
| 73 | A1 | High | The interface-fetch fallback decodes `__get_candid_interface_tmp_hack` with no quota; the trap is never negative-cached, so every request and every unscoped search repeats it | Fixed F1: a trap in a fetch's own reply callback is remembered for 5 minutes (README) | bridge::only_a_trap_in_this_fetchs_reply_is_remembered |
| 74 | A1 | High | Interface text well under 512 KiB makes `candid_parser` overflow the stack or exceed 40e9; the trap is not negative-cached | Fixed F1: shape caps make parsing linear; a trap while parsing is remembered (73) | candid_bridge::interface_shape_limits |
| 75 | A1 | High | Calls wait unboundedly by default: a silent allowlisted target pins all 8 run slots, holds cycles, and blocks stopping | Fixed F1: 600 s per run; each timeout cut to the time left | bridge::run_time_limit_cuts_timeouts_then_refuses |
| 116 | A1 | Medium | A call timeout bounds each call, not a stop: a run keeps sending new messages while the canister is Stopping | Fixed F1: the canister status is checked right before every send | bridge::run_time_limit_cuts_timeouts_then_refuses (gate logic; the stop itself needs a replica) |
| 136 | A1 | Medium | Any interface-fetch error, including transient and self-caused ones, is cached for 5 minutes as the target's failure and served to every caller | Fixed F1: only lasting failures are remembered; an expired copy is served otherwise | bridge::only_definitive_fetch_failures_are_remembered, bridge::expired_interface_is_served_when_the_refetch_fails_transiently |
| 142 | A1 | Medium | Deeply nested reply bytes overflow the Wasm stack inside candid's decoder before the 128-level check | Fixed F1: patched candid bounds recursion at 1,024 on every target | candid_bridge::deeply_nested_reply_is_refused_inside_the_decoder |
| 164 | A1 | Low | Error messages echo caller- or target-supplied text without the truncation used elsewhere | Fixed F1: quoted text cut to a short prefix | candid_bridge::errors_quote_only_a_prefix |
| 165 | A1 | Low | `call_timeout_seconds` is not applied to interface fetches | Fixed F1 | bridge::run_keeps_the_limits_it_started_with |

### L10 Trust and transport

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 47 | R4 | Medium | Tool lists and search results come from uncertified queries that a single node could alter. A certified mode exists; it is off by default | Fixed (101, before F1); no test checks a response against the gateway | mcp::certified_mode_upgrades_everything_and_disables_the_demo_page |
| 48 | R4 | Medium | Bearer tokens are visible to the infrastructure that carries requests | Accepted; re-checked A1 | none |
| 49 | R4 | Low | All tokens act as one on-chain identity | Accepted; re-checked A1; delegated authority outlives revocation (193) | none |
| 101 | A1 | Medium | `certified_reads` does not stop a single node from forging query responses: certified data always says "skip verification" on every path | Fixed before F1 (certified mode certifies nothing) | none (needs a replica) |
| 129 | A1 | Medium | Audit free-text fields embed caller- and target-authored text in server sentences and labels without escaping | Fixed F1: authored text quoted and escaped in labels and violations; control and bidi characters escaped in `error` | policy::script_text_is_escaped_in_refusals_and_labels |
| 166 | A1 | Low | The video tooling leaves the plaintext MCP token in Chrome profiles in TMPDIR, can leave a DevTools port open, sends the token to whatever answers on port 9333, and misreads a quoted `.env` token | Fixed F1 | video/test-tools.mjs |

### L11 Sandbox and determinism

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 61 | A1 | High | Python `eval`/`exec` compile a run-time string of any size in one unchecked step | Fixed F1: run-time compiled text capped at 16 KiB with time polled; annotation text still evaluates (README) | runtimes/python::engine_work_is_bounded |
| 72 | A1 | High | Boa's process-global `Symbol.for` registry keeps every key across runs; a token holder can grow the heap until upgrade | Fixed before F1 (`Symbol.for`/`keyFor` removed) | runtimes/js::audit_round_two_js_fixes |
| 85 | A1 | Medium | At 65,536 pages of Wasm memory a shadow-stack overflow wraps below 2^32 silently and overwrites other runs' heap data | Fixed before F1 (1 GiB heap ceiling in every build) | runtimes/python::heap_ceiling_bounds_single_builtin_allocations |
| 113 | A1 | Medium | Rhai `sleep()` and the `sync` lock's spin call `std::thread::sleep`, which panics on wasm32 | Fixed before F1 (`sync` dropped, `sleep` replaced) | runtimes/rhai_rt::sleep_is_disabled_in_sandbox |
| 123 | A1 | Medium | A sort comparator that is not a total order makes the canister trap | Fixed F1: patched Boa sorts with a merge sort that cannot panic | runtimes/js::inconsistent_comparators_do_not_panic |
| 124 | A1 | Medium | `String.prototype.at` with an index at or below -2^63 traps the canister | Fixed before F1 | runtimes/js::string_at_negative_overflow_returns_undefined_without_panicking |
| 179 | A1 | Low | Dynamic `import()` enqueues a job the executor drops, so the run fails with a misleading "never settles" error | Fixed F1 | runtimes/js::dynamic_import_is_rejected |

### L12 Protocol conformance

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 50 | R1 | Low | A null request id was accepted, and invalid ids were echoed back | Fixed; re-checked A1; out-of-range ids refused since F1 (158) | mcp::ids_must_be_string_or_number |
| 103 | A1 | Medium | Tool arguments are not checked against the advertised `inputSchema`: unknown keys, non-object `arguments` and `language: null` are accepted | Fixed F1 | mcp::protocol_edge_cases_and_truncate_escaped |
| 112 | A1 | Medium | When a batch reply is too big, `http_update` replaces every tool result with an error, including a committed, audited script's result | Fixed: structurally before F1; F1 gives server limits their own code (-32001) | mcp::replies_always_fit |
| 133 | A1 | Medium | `execute_icp_script` has no outcome channel besides the synchronous reply and no idempotency; a timed-out client's retry repeats every effect, and cancellation is ignored | Mitigated F1: the tool description and README warn that retries repeat effects. Remains: no idempotency key or stored outcome | mcp::descriptions_state_retries_tuples_and_untrusted_text |
| 158 | A1 | Low | JSON-RPC ids beyond i64/u64 or in exponent form are echoed altered | Fixed F1 | mcp::ids_must_be_string_or_number |
| 169 | A1 | Low | JSON-RPC edge cases: server limits reported as -32600, and invalid requests carrying a `result` key are silently dropped | Fixed F1 | mcp::protocol_edge_cases_and_truncate_escaped |
| 190 | A1 | Low | The server never validates the `Origin` header (an MCP 2025-03-26 Streamable HTTP MUST), and the README's deviations section does not say so | Fixed before F1 | mcp::authenticate_rejects_untrusted_origin |

### L13 Upgrade and compatibility

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 51 | R4–R6 | Medium | Adding audit statuses broke the interface for existing readers three times | Open; re-checked A1: two breaks in git history (Call, Trapped), whole page lost; rollback makes pages trap. F1 changes are additive (README) | none |
| 79 | A1 | High | Upgrading with install arguments re-registers revoked tokens and undoes allowlist narrowing; `icp deploy` re-sends `init_args` on every upgrade | Fixed: `post_upgrade` ignores its arguments (before F1); F1 fixed the stale docs | none (post_upgrade needs a replica) |
| 128 | A1 | Medium | In-flight bookkeeping lives only in the heap: an upgrade without a stop or a snapshot load with calls outstanding leaves phantom or orphaned runs | Fixed F1: bookkeeping expires when stale, `pre_upgrade` refuses while runs are pending unless stopped, upgrades are audited | lib::run_slots_linger_after_a_trap_and_expire_when_stale |

### L14 Configuration

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 104 | A1 | Medium | `set_limits` (and `InitArgs.limits`) replace the whole record, so omitting the optional fields silently turns `certified_reads` and `call_timeout_seconds` off | Fixed F1: an omitted or null field keeps its value | lib::set_limits_keeps_omitted_options |
| 106 | A1 | Medium | Limits are accepted unvalidated: `max_instructions` above ~30-40e9 turns clean budget stops into 40e9 traps in every runtime, and `call_timeout_seconds = 0` is accepted | Fixed F1: limits are clamped on read too | policy::stored_unsafe_limits_are_clamped_on_read, lib::set_limits_rejects_unsafe_instruction_and_timeout_bounds |
| 138 | A1 | Medium | Token ids shown to the controller (16-hex prefix) cannot be used to revoke; revocation needs the full hash, which nothing returns | Fixed F1: the shown 16-character id revokes | policy::revoke_by_shown_prefix |
| 144 | A1 | Medium | Install, upgrade and admin arguments under `opt` that do not fit their type decode as null and are silently not applied | Fixed F1: install and admin arguments are decoded strictly | lib::install_and_admin_args_are_decoded_strictly |
| 170 | A1 | Low | Admin and install inputs are unbounded and unchecked for duplicates | Fixed F1: caps on labels, tokens, methods, canisters, allowlist bytes and overrides; duplicates refused | policy::admin_input_is_capped, policy::candid_overrides_are_capped, lib::duplicate_allowlist_principal_is_refused |

### L15 Supply chain and build

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 107 | A1 | Medium | `icp.yaml` copies the Wasm from a hard-coded `./target` path, so a build directory elsewhere ships a stale or different Wasm and reports success | Fixed F1: README build and worker steps use the pinned `target` dir | none |
| 171 | A1 | Low | Release builds are not reproducible elsewhere: absolute builder paths are embedded (and reach callers in trap text), and `ic-wasm` is unpinned | Mitigated F1: build paths remapped, ic-wasm pinned to 0.11.1. Remains: no published hash; not yet compared across machines | CI reproducible-build step (has not run) |
| 172 | A1 | Low | The shipped profile differs from the tested one: release has no overflow checks, and dependencies gate guards by target | Fixed F1: release keeps overflow checks; candid's depth guard applies on wasm (patched); field id overflow refused | candid_bridge::interface_shape_limits, candid_bridge::deeply_nested_reply_is_refused_inside_the_decoder |
| 183 | A1 | Low | `scripts/demo-setup.sh` parses human-readable icp-cli output and overrides `ICP_ENVIRONMENT` | Fixed F1 | scripts/test-demo-setup.sh |
| 184 | A1 | Low | Two unmaintained crates ship in the Wasm (smartstring via rhai; the vendored chrono via monty) and nothing checks advisories | Fixed F1: CI checks advisories (deny.toml); both crates accepted and documented. CI has not run yet | CI advisories job (has not run) |

### L16 Claims

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 52 | R6 | Medium | The docs called a run without a final entry proof of a trap; it can also be pending | Fixed; re-checked A1 | manual |
| 53 | R6 | Low | The audit `calls` field counted attempts, not messages sent | Fixed; re-checked A1; the cap counts slots (152) | manual |
| 80 | A1 | Medium | The Candid override layer is undocumented in precedence, invisible to operators and not tied to the allowlist | Fixed F1: served only while allowlisted, oversized legacy overrides ignored, precedence documented | policy::overrides_need_the_allowlist_and_the_size_cap |
| 102 | A1 | Medium | The tool description tells the AI model there is no randomness; Python's `random` returns the same fixed-seed sequence every run | Fixed before F1 | none |
| 108 | A1 | Medium | The search tool's description shown to the AI and README:14 say search is free, unaudited and outside the call cap; a fetching search is an audited, capped update | Fixed before F1 | none |
| 109 | A1 | Medium | README:367 and the `EXECUTION_BUDGET` comment say Python has its own 10B on-chain budget; on chain it uses what is left of the policy budget | Fixed before F1 | none |
| 130 | A1 | Medium | The demo comparison and README benchmark count all 10 store methods as Classic tools while Code Mode sees 8; on equal surfaces the restocking result flips | Fixed before F1 (both sides see the 8 allowlisted methods) | none |
| 137 | A1 | Medium | The documented MCP endpoint `http://icp_code_mode.local.localhost:8010/mcp` does not resolve for Node or Python clients on macOS | Fixed F1: client URLs use `localhost:8010/mcp?canisterId=` | scripts/check-readme-urls.mjs |
| 140 | A1 | Medium | Records whose labels are 0..n-1 are returned as JSON arrays, while the README and search description say records are objects | Fixed F1 (documented in the tool descriptions and README) | mcp::descriptions_state_retries_tuples_and_untrusted_text |
| 173 | A1 | Low | Minor documentation mismatches | Fixed F1 | none |
| 174 | A1 | Low | The audit log is called "immutable"/"append-only", but controllers can rewind it by restoring a snapshot or reinstalling, and ids are then re-issued | Accepted F1 (README): the canister code only appends; controllers can rewind with snapshots | none |
| 178 | A1 | Low | Search audit entries use an undocumented language value and hash the caller's raw spelling | Fixed F1: documented, and the key uses the canonical id | lib::search_key_canonicalises_the_canister_id |
| 192 | A1 | Low | Search output does not match what the converter accepts or what `icp.call` allows | Mitigated F1: uncallable methods are not listed and doc comments are searched. Remains: candid's printer leaves keyword-like names unquoted | search::lists_only_callable_methods_and_searches_docs, candid_bridge::uncallable_methods |
| 199 | A1 | Low | The token-hash recipe in the `.did` and InitArgs comments (`echo -n ... | shasum`) gives a different digest under POSIX sh | Fixed F1 | lib::did_file_matches_exported_interface |

### L17 Client and presentation

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 167 | A1 | Low | Text written by a target canister reaches the AI model verbatim with nothing marking it as third-party data | Mitigated F1: descriptions and instructions call target text untrusted data. Remains: the text itself is not marked or separated | mcp::descriptions_state_retries_tuples_and_untrusted_text |
| 197 | A1 | Low | The demo page splices a reply value into script source, parses `.did` text with a quadratic regex, and keeps the token in localStorage | Fixed F1: values passed as JSON literals, ids checked, no `.did` regex, token in sessionStorage | scripts/test-demo-page.mjs |

### L18 Operations

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 96 | A1 | Medium | `get_audit_logs` pages are bounded by count, not bytes; a legacy 8.39 MB entry is unreadable through the API | Fixed F1: entries capped on read, pages capped at 1.5 MB | policy::oversized_entries_are_readable_and_pages_bounded |
| 180 | A1 | Low | The audit log has no retention or export path; appends trap if stable memory cannot grow, and each new bucket raises the freezing threshold | Mitigated F1: a failed closing append is counted, not a trap; no new runs under 0.1T liquid cycles. Remains: no retention or export | policy::append_over_full_memory_is_an_error, lib::runs_need_a_cycles_margin |
| 181 | A1 | Low | No in-app view of cycles or memory, no low-memory hook, and no runbook; frozen or stuck-stopping canisters expose nothing | Fixed F1: `get_metrics`, a low-memory hook, thresholds and a 2 MiB log in icp.yaml, an Operations section | lib::icp_yaml_sets_a_larger_canister_log (settings only) |
| 182 | A1 | Low | Audit paging is offset-from-newest, so pages shift as the log grows | Fixed F1: `get_audit_logs_before` and `get_audit_run` | policy::pages_before_an_id_do_not_shift, policy::a_run_is_read_by_its_id |

### Testability

Gaps in the tooling, each of which hides a class of issue from every audit until closed.

| Id | Round | Gap | Status |
|---|---|---|---|
| 54 | R4 | No automated integration tests against a local network. Every "manual" guard above depends on this | Mitigated F1: harness/ installs the release Wasm under PocketIC and sends a trap corpus; it has not run yet, and there are no tests against a local network |
| 55 | R4 | No misbehaving test target: one that never replies, replies late, returns oversized data, traps, or lies about its interface | Open (re-checked A1); F1 bridge tests use a native fake network for stopped, silent and failing targets |
| 56 | R5 | No fuzz or property tests for the parsers and value converters, where most high-severity findings were | Open (re-checked A1) |
| 57 | R4 | Nothing has run on the real network; gateway behaviour, costs and installing the large artifact there are unverified | Open (re-checked A1) |
| 58 | R6 | Tests cannot drive the production bridge off-chain, so audit-entry behaviour is only checked by hand. A1: `IcBridge` runs natively up to the first platform read; the missing seams are time, the call primitive and `canister_self` (see 110) | Fixed F1 (110): IcBridge runs natively with a fake network, test clock and instruction counter; audit entries are checked by tests |
| 110 | A1 | Platform reads have no test seam, so the instruction budget, interface TTLs and every send path cannot be exercised natively (Medium) | Mitigated F1: test clock, fake network, instruction counter and `canister_self` stand-ins drive every runtime and send path. Remains: JS keeps its own poll-count seam |
| 132 | A1 | The audit workflow turns missing agent output into "nothing found" and dedupes findings with a lossy 160-character key (Medium) | Fixed F1 (guard: scripts/test-audit-workflow.mjs) |
| 134 | A1 | Register guards do not detect regressions: 64 of 70 limit mutants survive, some guards pass with the fix deleted, and others hang or abort (Medium) | Mitigated F1: print-cap and interleaving guards made effective (14, 43). Remains: run-slot and pending-run wiring through `run_script` untested, no timeout on tests that can hang, no query/update differential test, no mutation gate |
| 146 | A1 | Nothing runs the built Wasm and there is no CI, so wasm32-only traps and build invariants are never checked (Medium) | Mitigated F1: CI workflow (tests, clippy, release Wasm, import check, hash check) and a PocketIC harness exist; neither has run remotely |
| 201 | A1 | Audit method gaps: failure sites were inventoried from app source only, and the shared build directory served stale binaries (Low) | Fixed F1 (PLAN.md) |

## Refuted

Kept so the next audit does not spend time on them again without new evidence.

| Round | Claim | Why it was refuted |
|---|---|---|
| R1 | Returning large numbers as either a number or a string is a defect | Deliberate and documented: numbers stay numbers while they are safe for JavaScript |
| R5 | A Rhai value of 900,000 elements passes the limits | Rhai's own size limit counts nested elements and rejects it |
| A1 | The demo page keeps the token in localStorage with no way to clear it | Emptying the field overwrites it; persistence is disclosed (hardening note kept in 197) |
| A1 | Search annotations `readOnlyHint`/`idempotentHint` are wrong because fetches send messages | Fetches are reads and the writes are bookkeeping; the description text is wrong instead (108) |
| A1 | A `tools/call` without an id is silently not executed and answered 202 | It is a notification: JSON-RPC forbids a reply and the MCP transport requires 202 |
| A1 | A bare variant tag `"Tag"` is accepted for `opt` and empty-record payloads | `null` legitimately stands for those payloads; tags with no null form are refused |
| A1 | Search results are unbounded because `MAX_METHODS` counts methods, not bytes | Both search paths pass through `tool_result`, which caps the text at 1 MB |
| A1 | Boa Map/Set hash flooding from the shared getrandom seed | Map/Set use std seeding; not reproduced. JS reply conversion is 145 |
| A1 | No MCP protocol-version gate | Only 2025-03-26 is negotiated and answered per that version; no client failure shown |
| A1 | Python dict hash flooding during reply conversion | Monty compares full 64-bit hashes and builds no table during conversion; measured linear |
| A1 | `ahash compile-time-rng` makes builds non-reproducible | `runtime-rng` is enabled alongside it, so the constant branch is compiled out |
| A1 | web-time `Instant + Duration` overflow with huge budgets | Monty never adds a Duration to an Instant |
| A1 | Vendored chrono drifted from upstream | Byte-identical to 0.4.45 apart from documented feature edits |
| A1 | 500+ concurrent searches fill the output queue and poison the failure cache | 800 concurrent searches never had more than 25 fetches open |
| A1 | Stopping a target poisons its cached interface | `canister_metadata` is answered by the management canister while the target is stopped |
| A1 | 8 interleaved Rhai runs retaining replies reach 3 GiB | The instruction budget bounds retention to ~90 MB per run |

## Patterns worth sweeping for

Each confirmed finding above reduces to one of these. The plan's variant sweep searches the
whole codebase for each.

- A record written in a different step from the action it describes.
- A limit checked after the work it is meant to prevent, or on one dimension only.
- A limit enforced in one runtime, route or path and missing in its sibling.
- A per-item budget that a batch, a retry or concurrency multiplies.
- Work accepted before the caller is known to be allowed to ask for it.
- Recursion or a copy inside a dependency, outside the app's own limits.
- An identifier compared in the caller's spelling instead of its canonical form.
- Partial or altered data returned as success.
- Process-wide state read before a suspension and trusted after it.
- A sentence in the docs that uses "never", "always" or "proof" without a test behind it.
- Work sized by untrusted input that runs in one uninterruptible step (an engine with no
  interrupt hook, a dependency's native operation, a compile or parse step) while the budget
  is checked only between steps. (A1)
- A budget turned into a relative allowance once, measured by a clock that pauses during host
  work. (A1)
- A cost limit measured in bytes or nodes standing in for work that is superlinear in the
  input's structure (nesting depth, digit count, declarations, keywords x methods). (A1)
- A platform resource limit used as the only bound, without checking where the platform
  enforces it (not in reply callbacks) or what happens once it is crossed. (A1)
- A dependency allocating from a script-chosen size, or keeping script-writable state in a
  process-global static, outside the app's accounting. (A1)
- A safety guard that exists only on the host target (a dependency's native-only depth guard,
  `catch_unwind` under `panic=abort`, `thread::sleep`, overflow checks off in release). (A1)
- An untrusted reply or interface parsed after the commit point, so a trap rolls back the
  failure record and the work repeats on every request. (A1)
- A check made before an await (or before costly work) and trusted for the effect after it;
  a cache filled after an await with no in-flight marker or generation check. (A1)
- A failure reported after an effect as if nothing happened (a loop bound checked after the
  last dispatch, a post-run budget check replacing a received reply). (A1)
- A shared negative cache that stores every error as a fact about the remote party. (A1)
- A running total added in one branch and compared in another; the same limit measured in
  different units by sibling implementations. (A1)
- A converter's catch-all branch that stringifies what it does not recognise, or dispatches on
  a name the value controls. (A1)
- A whole-record setter where an omitted optional field means "off"; install-time
  configuration re-applied on upgrade. (A1)
- In-flight work tracked only in volatile state that lifecycle operations reset. (A1)
- A test that checks a limit through a mock that re-applies the same limit, or whose first
  step already trips the guard. (A1)
- Text from a third party passed verbatim to an AI model or into a trusted record without
  marking or escaping. (A1)
