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

## Findings by class

Classes are the lenses in the plan.

### L1 Access control

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 1 | R1 | High | The interface cache and admin overrides were keyed by the caller's spelling of a canister id, so a different letter case missed both | Fixed; re-checked A1 (bridge path reproduced natively, still no committed test; see 186) | search::scope_canonicalises_and_reports_unknowns (search only); bridge path: manual (A1, native) |
| 105 | A1 | Medium | A registered SHA-256 of the empty token makes an empty bearer authenticate | Open | none |
| 147 | A1 | Low | The allowlist accepts this canister's own principal and `aaaaa-aa` with `"*"` | Open | none |
| 193 | A1 | Low | A wildcard allowlist admits delegating methods (e.g. `icrc2_approve`) whose authority outlives token revocation and `remove_allowlist` | Open | none |
| 195 | A1 | Low | An empty allowlist method list is an undocumented "interface only" state | Open | none |
| 196 | A1 | Low | `canister_metadata("candid:service")` returns a private interface when this canister controls the target, and it is served to token holders | Open | none |
| 198 | A1 | Low | The wildcard `"*"` cannot be told apart from a Candid method literally named `*` | Open | none |

### L2 Atomicity and commit points

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 2 | R1 | High | A reply over the size limit trapped after calls had run, which rolled back the audit entry | Fixed; re-checked A1 (batch shrink discards results: 112) | mcp::result_size_is_bounded_after_escaping |
| 3 | R3 | High | A run's first and final audit entries were linked only by script hash, so two runs of the same script could not be told apart | Fixed; re-checked A1 (live reconciliation); guard tests the helper only (134) | policy::run_id_links_started_and_final_entries (helper only) |
| 4 | R4 | High | A batch padded with tiny invalid messages made the reply trap, erasing the run's audit entry | Fixed; re-checked A1 (10.45 MB id-echo batch) | mcp::replies_always_fit |
| 5 | R4 | High | A run that trapped after its calls went out showed "Started, 0 calls" with no methods | Fixed; re-checked A1 live; instructions on Trapped is a lower bound (83) | manual |
| 6 | R2 | Medium | A search refused by policy before any message was sent wrote no audit entry | Fixed; re-checked A1; remembered-failure searches write none (151) | manual |
| 7 | R6 | High | Message entries were written before argument encoding, so a call that failed to encode left an entry for a message never sent | Fixed; re-checked A1 (868 messages reconciled) | manual |
| 8 | R6 | Medium | A message the system refused on the spot still left an entry | Fixed; re-checked A1 live | bridge::on_first_pending_fires_once_and_only_if_pending (mechanism only) |
| 9 | R6 | Medium | A trapped run had no closing entry | Fixed; re-checked A1 live; cleanup trap leaks (150), batch misattribution (143) | manual |
| 10 | R1 | Medium | A runtime limit hit after an await was reported as a promise that never settles | Mitigated; re-checked A1 (still generic, see 59) | none |
| 11 | R4 | Low | A run that traps before sending any message leaves no audit entry | Accepted; re-checked A1: now reachable through many 40e9 traps (59-65, 78) | none |
| 76 | A1 | High | JavaScript and Python dispatch the last allowed round, then report the run failed and drop the reply | Open | none |
| 77 | A1 | High | The instruction budget is checked before the interface fetch await and not again before the send; a message sent past the budget is then reported as a policy refusal | Open | none |
| 82 | A1 | Medium | Audit entries for an interface fetch name only the management canister; the documented `canister.<candid interface>` label is never produced | Open | none |
| 83 | A1 | Medium | A Trapped entry's `instructions` omits the trapped execution (about 2-25M recorded for a run that hit 40e9) | Open | none |
| 115 | A1 | Medium | Records cannot show whether a sent message took effect, and errors raised after a target executed do not say so; README claims the outcome is in the script's result or error | Open | none |
| 120 | A1 | Medium | A trap while handling one reply of a batch frees the run slot and pending id while sibling calls are still outstanding | Open | none |
| 143 | A1 | Medium | In a batch, a trap in a later tool call closes an earlier, completed search as Trapped, and the trapping script leaves no entry | Open | none |
| 149 | A1 | Low | Audit violation lists lose information: the Trapped entry drops the "...and N more" count, and the first 16 refusals (chosen by the script) are kept | Open | none |
| 150 | A1 | Low | A trap inside the cleanup callback leaks a run slot, a pending id and the run's task until upgrade | Open | none |
| 151 | A1 | Low | Searches answered from a remembered failure, or whose fetch the system refused at send time, write no audit entry; the README's list of no-entry cases omits them | Open | none |
| 194 | A1 | Low | Audit entries record `canister.method` and a script hash but never arguments, so recipient, spender or amount cannot be reconstructed | Open | none |

### L3 Resource bounds

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 12 | R1 | High | A deeply nested JavaScript value overflowed the stack when converted | Fixed; re-checked A1; other native stack overflows: 84, 142 | runtimes/js::deep_and_huge_values_are_rejected_not_overflowing |
| 13 | R1 | High | The Rhai replay loop had no cap on host requests | Fixed; re-checked A1; guard hangs on regression (134) | runtimes/rhai_rt::request_count_bounds_replays |
| 14 | R1 | High | Script output buffers grew without limit while a script ran (Rhai and JavaScript) | Fixed; re-checked A1: guard passes with both caps deleted (134) | runtimes/rhai_rt::print_is_capped_per_run, runtimes/js::console_output_is_capped_while_running (ineffective: pass with the caps deleted) |
| 15 | R1 | High | Deeply nested Rhai arrays cost far more engine work than the operation count showed | Fixed; re-checked A1 | manual |
| 16 | R1 | Medium | Audit error and violation strings were stored at any length | Fixed for new entries; re-checked A1 (marker overshoot 175, legacy entries 96) | none |
| 17 | R1 | Medium | A valid reply with a large unknown field was rejected, and the error embedded the whole reply | Regressed A1: holds only below ~400 KB (87) | none |
| 18 | R5 | High | A self-rescheduling promise chain bypassed the loop and recursion limits | Fixed; re-checked A1; guard hangs on regression (134) | runtimes/js::promise_chains_are_bounded |
| 19 | R5 | High | A `JSON.stringify` replacer could feed an unbounded value to the native serializer | Fixed; re-checked A1; guard aborts on regression (134); see 111 | runtimes/js::stringify_replacer_and_size_limits |
| 20 | R5 | High | Arrays of primitives were not counted against the element limit | Fixed; re-checked A1 | runtimes/js::stringify_replacer_and_size_limits |
| 21 | R5 | High | One large string referenced many times was small inside the engine and gigabytes once copied out (all runtimes) | Regressed A1: JS object keys (66), Python repr branches (64) | runtimes/js::stringify_replacer_and_size_limits, runtimes/python::values_are_bounded |
| 22 | R5 | Low | Interface text of any size was fetched and parsed | Fixed; re-checked A1 (boundary exact); cap does not bound parse cost (74), legacy overrides (80) | none |
| 23 | R1 | High | One uninterruptible built-in JavaScript operation can run past the platform's instruction limit and trap | Reopened A1: the 'one built-in' premise is false (59, 60, 61, 63, 64, 65, 70, 73, 74, 78) | none |
| 24 | R5 | Medium | JavaScript has no memory cap | Reopened A1 at High as 71: the 'trap, 503' premise is false | none |
| 59 | A1 | High | JavaScript code between host calls is never charged against the instruction budget; Boa's 1M loop limit is per call frame | Open | none |
| 60 | A1 | High | Monty native operations run uninterruptibly: bigint arithmetic, repr, regex, decimal and collection operations run past the budget to the 40e9 trap | Open | none |
| 62 | A1 | High | Python's budget is a one-time relative allowance on a clock that pauses during host work | Open | none |
| 63 | A1 | High | Untrusted source and interface text are compiled outside the budget, and the compilers are quadratic | Open | none |
| 64 | A1 | High | Python `to_json` uses `py_repr` as a catch-all: no size or cost bound, one node per repr, dispatch on a script-chosen class name | Open | none |
| 65 | A1 | High | Decimal text for `nat`/`int` is parsed quadratically inside `encode_args`, with no budget check before the send | Open | none |
| 66 | A1 | High | JavaScript object-key text is added to the 4 MiB budget but never compared against it | Open | none |
| 67 | A1 | High | Rhai's size limits ignore map keys and curried function pointers, and Rhai has no other memory cap | Open | none |
| 69 | A1 | High | Monty allocates outside its 64 MiB tracker: `json.dumps(indent=...)` and `re.compile` trap or grow the heap permanently | Open | none |
| 70 | A1 | High | Engine built-ins allocate from script-chosen sizes before any limit; on wasm32 this is an uncatchable "capacity overflow" trap | Open | none |
| 71 | A1 | High | A JavaScript run can grow the Wasm heap past `wasm_memory_limit` inside response callbacks; afterwards every update, the controller's included, is refused | Open | none |
| 78 | A1 | High | Search work is keywords x allowed methods with no keyword cap, and the budget is checked only after rendering | Open | none |
| 84 | A1 | Medium | Small inputs overflow the native stack in the Boa parser and in Boa built-ins on deep data | Open | none |
| 86 | A1 | Medium | Candid conversion cost scales with the interface's shape, not the value budget (variant tags, record fields, nested inline types, decoder memo) | Open | none |
| 92 | A1 | Medium | Call arguments are never checked against the inter-canister payload limit; JS counts text in UTF-16 units, so an argument inside the value limits traps the canister | Open | none |
| 117 | A1 | Medium | `set_candid_override` validates by parsing in an update (40e9) but query searches re-parse under the 5e9 query limit and cannot keep the result | Open | none |
| 118 | A1 | Medium | JavaScript `console.log` formats every argument in full before the 64 KiB cap; one call grew the heap by 630 MB permanently | Open | none |
| 119 | A1 | Medium | JavaScript `icp.call`/`icp.candid` copy canister-id and method strings of any size out of the sandbox | Open | none |
| 131 | A1 | Medium | The error exit of the JS and Python sandboxes skips the value limits: an `AggregateError` with a huge `errors.length` traps, and thrown error text of any size is copied | Open | none |
| 145 | A1 | Medium | Hash tables keyed by untrusted data use public or absent seeds; a hostile reply with colliding keys makes JavaScript's reply conversion quadratic | Open | none |
| 175 | A1 | Low | Truncation markers make strings exceed their caps, and LogBuf then discards the whole cut line | Open | none |
| 176 | A1 | Low | The log cap counts raw line bytes only: empty lines are free and the client-side cap uses escaped bytes | Open | none |
| 188 | A1 | Low | Rhai's expression-depth limit is consumed by flat operator chains | Open | none |
| 191 | A1 | Low | The composite_query refusal comes after a cap slot, a possible fetch and full argument encoding | Open | none |

### L4 Multiplication

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 25 | R1 | Medium | Interface fetches did not count toward the per-script call cap | Fixed; re-checked A1; one slot can send two messages (152) | manual |
| 26 | R1 | Medium | Scripts batched in one request shared one instruction counter | Fixed for scripts; re-checked A1; searches plus a script share one execution (126) | mcp::one_tool_call_per_request |
| 27 | R2 | Low | A search kept attempting fetches after the call cap was reached | Fixed; re-checked A1 | manual |
| 28 | R5 | Medium | A batch of searches multiplied outbound calls and audit entries | Fixed on update; re-checked A1; query path lacks the cap (89) | mcp::one_tool_call_per_request |
| 29 | R5 | Medium | Unlimited concurrent scripts could exhaust memory | Fixed; re-checked A1 live (cap triggered); no per-caller share (121) | lib::run_slots_are_capped_and_returned (helper only; cap triggered live in A1) |
| 88 | A1 | Medium | Concurrent cache misses on one interface each send their own fetch (no single-flight) | Open | none |
| 121 | A1 | Medium | Run slots are one global pool of 8 with no per-caller share | Open | none |
| 126 | A1 | Medium | Each tool call has its own instruction budget, but up to 5 searches and a script in one HTTP request share one execution | Open | none |
| 152 | A1 | Low | One interface-fetch cap slot can send two messages, so the audit `calls` field exceeds `max_calls_per_script` | Open | none |
| 153 | A1 | Low | Searches take no run slot: concurrent searches suspended on fetches are uncapped and not counted in `active_runs` | Open | none |
| 168 | A1 | Low | A getter in JavaScript call arguments bypasses the 64-concurrent-request cap | Open | none |

### L5 Cost and payment

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 30 | R4 | High | Anyone could call the update endpoint directly, without a token, and make the canister pay for it | Fixed for ingress; re-checked A1; canister callers bypass (154) | manual |
| 31 | R4 | Medium | A token holder can grow the audit log without limit; nothing rate-limits requests | Open; re-checked A1; no retention, append traps on growth failure (180) | none |
| 90 | A1 | Medium | The update path accepts and pays for any authenticated body with no size cap; work that runs no tool leaves no record, and the parser holds three full copies | Open | none |
| 91 | A1 | Medium | The query path ignores the failed-fetch cache, so every search touching an unfetchable interface becomes a paid, unaudited update | Open | none |
| 125 | A1 | Medium | Cycles reserved by in-flight calls have no global cap; at a thin liquid margin other callers' sends fail while the canister reports Running | Open | none |
| 148 | A1 | Low | `demo_store` accepts unauthenticated, unbounded writes and its arithmetic oversells, wraps and saturates | Open | none |
| 154 | A1 | Low | Calls from other canisters bypass the `inspect_message` cost filter, so a tokenless caller costs ~5.6M cycles per call | Open | none |

### L6 Consistency across variants

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 32 | R5 | Medium | Python used a fixed instruction budget instead of the configured one | Fixed, incomplete: host work uncharged (62); docs stale (109) | manual |
| 33 | R5 | High | Python returned deep data truncated with a marker and reported success, where the other runtimes refuse it | Regressed A1 at deep call stacks (64) | runtimes/python::values_are_bounded |
| 81 | A1 | Medium | An un-awaited `icp.call` (JS) or `icp.call_async` (Python) is never sent but the run reports Ok; Rhai sends it | Open | none |
| 89 | A1 | Medium | The five-searches-per-request cap is enforced only on the update path | Open | none |
| 93 | A1 | Medium | Python fails the whole run with a MemoryError once output passes 64 KiB, where JS and Rhai truncate; printed values are formatted in full first | Open | none |
| 94 | A1 | Medium | Rhai applies its engine size limits to host replies, summed over the whole nested value; valid replies are refused after the call ran | Open | none |
| 95 | A1 | Medium | Per-language builds advertise, default to and accept languages that are not compiled in | Open | none |
| 139 | A1 | Medium | JavaScript `icp.call`/`icp.candid`/`icp.principal` silently ignore extra arguments; Python and Rhai refuse them | Open | none |
| 156 | A1 | Low | The 128-level depth limit is off by one between runtimes | Open | none |
| 162 | A1 | Low | Python sees block time truncated to whole seconds while JavaScript sees milliseconds | Open | none |
| 185 | A1 | Low | Python has no per-batch request cap; JavaScript refuses the 65th concurrent request | Open | none |
| 186 | A1 | Low | Search trims a whitespace-padded `canister_id` that `icp.call`/`icp.candid` refuse | Open | none |
| 187 | A1 | Low | `inspect_message` decodes the request without the skipping quota the method uses, admitting ingress that then traps on decode | Open | none |

### L7 Data fidelity

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 34 | R1 | High | JavaScript integers above 2^31 were rejected as natural numbers | Fixed; re-checked A1; values >= 2^53 rounded (114) | runtimes/js::big_js_integers_pass_as_nat |
| 35 | R1 | Medium | Misspelled record fields were silently dropped, so an optional field was sent as null | Fixed for records; re-checked A1; aliases 160, func keys 159, tool args 103 | candid_bridge::errors_are_descriptive |
| 36 | R1 | Medium | Replies nested too deep came back truncated and marked as success | Fixed; re-checked A1; opt counted twice (127), stack overflow first (142) | candid_bridge::deep_replies_error_instead_of_truncating |
| 37 | R1 | Low | For single-argument methods, `[x]` and `[]` were interpreted inconsistently | Regressed A1 (135) | candid_bridge::errors_are_descriptive (partial) |
| 38 | R1 | Low | Python bytes and Rhai blobs could not be passed as binary data | Fixed; re-checked A1; JS typed arrays (155) | runtimes/python::determinism_and_sandbox, runtimes/rhai_rt::limits_and_sandbox |
| 39 | R1 | Low | Non-finite numbers decoded to a value that could not be sent back | Fixed; re-checked A1; strings undocumented (155) | candid_bridge::js_style_floats_and_non_finite |
| 40 | R5 | Low | Negative zero was rejected as a natural number | Fixed; re-checked A1; JS float -0 sent as 0 (155) | candid_bridge::js_style_floats_and_non_finite |
| 41 | R2 | Low | Search results cut by the method cap showed an empty method list | Fixed; re-checked A1 | search::unfetched_and_failed_interfaces_and_cap |
| 42 | R1 | Low | A doubly optional value cannot be told apart from a missing one | Accepted; re-checked A1 | none |
| 68 | A1 | High | Rhai `sort`/`dedup` swallow comparator errors, returning wrong data as success or trapping on replay | Open | none |
| 87 | A1 | Medium | The reply decoding quota admits only ~400 KB because untyped decoding costs 50x; larger valid replies fail with a misleading type-mismatch error | Open | none |
| 97 | A1 | Medium | Results over 1 MB are cut to partial JSON and returned with `isError: false`, and audited Ok | Open | none |
| 98 | A1 | Medium | JavaScript call arguments go through JSON text and back: depth 128 becomes `null`, and floats can be sent 1 ulp off | Open | none |
| 111 | A1 | Medium | The JavaScript value snapshot skips the spec's unwrapping of boxed primitives and applies an array replacer only after walking every key | Open | none |
| 114 | A1 | Medium | Integers at or above 2^53 cross the JSON boundary as if exact: JS numbers are sent rounded, and Python/Rhai results above 2^53 leave as numbers | Open | none |
| 122 | A1 | Medium | A stale or overridden interface makes Candid's `opt` rule turn real reply data into `null`, audited Ok | Open | none |
| 127 | A1 | Medium | `MAX_DEPTH` counts Candid levels in the bridge but JSON levels in the runtimes; `opt`-recursive lists are refused from 65 nodes, replies only after the call ran | Open | none |
| 135 | A1 | Medium | The single-argument `[x]` heuristic changes a reply passed back unchanged; `[]` for a single `opt vec` argument is sent as `opt vec {}` | Open | none |
| 155 | A1 | Low | Values with no JSON form are dropped, nulled, stringified or refused differently in each runtime | Open | none |
| 157 | A1 | Low | `float32` arguments saturate silently and decimal or integer input is rounded twice | Open | none |
| 159 | A1 | Low | The JSON-to-Candid converter accepts and discards data for some types and accepts malformed text | Open | none |
| 160 | A1 | Low | The mapping between Candid labels and JSON keys is not one-to-one: aliases collapse and distinct fields print the same | Open | none |
| 161 | A1 | Low | String results are returned raw, so `"42"` and `42` produce identical tool text | Open | none |
| 177 | A1 | Low | JavaScript `console.log` and uncaught-error text pass values through the JSON snapshot | Open | none |
| 189 | A1 | Low | Blob replies cost one value-budget node per byte, so a blob over 200,000 bytes cannot be passed back | Open | none |
| 200 | A1 | Low | Candid interface text with `import` declarations is accepted and the imports silently dropped | Open | none |

### L8 Concurrency and shared state

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 43 | R1 | Medium | Python's memory limit used a process-wide baseline that concurrent runs overwrote | Fixed; re-checked A1: guard never interleaves (134); replies uncounted (100) | runtimes/python::memory_limit_survives_interleaved_runs (ineffective: never interleaves) |
| 99 | A1 | Medium | Boa's process-wide GC heap charges one run's instructions for other runs' objects | Open | none |
| 100 | A1 | Medium | Python's memory limit does not count host replies, so a run can hold several times 64 MiB | Open | none |
| 141 | A1 | Medium | Allowlist and token checks made before a suspension are trusted after it | Open | none |
| 163 | A1 | Low | Within one run the instruction budget and call timeout are snapshots, while other checks read live limits | Open | none |

### L9 Failure handling

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 44 | R4 | High | Calls wait indefinitely, so a target that never replies can block stopping and upgrading. A timeout option exists; it is off by default | Reopened A1: default still unbounded (75, 116, 125, 181) | manual, success path only |
| 45 | R4 | Medium | Fetched interfaces were cached until the next upgrade | Fixed; expiry observed live A1 (re-fetch at 3,620 s) | manual (observed live in A1) |
| 46 | R5 | Low | Failed interface fetches were repeated on every request | Fixed; expiry observed live A1; transient errors cached (136) | manual (observed live in A1) |
| 73 | A1 | High | The interface-fetch fallback decodes `__get_candid_interface_tmp_hack` with no quota; the trap is never negative-cached, so every request and every unscoped search repeats it | Open | none |
| 74 | A1 | High | Interface text well under 512 KiB makes `candid_parser` overflow the stack or exceed 40e9; the trap is not negative-cached | Open | none |
| 75 | A1 | High | Calls wait unboundedly by default: a silent allowlisted target pins all 8 run slots, holds cycles, and blocks stopping | Open | none |
| 116 | A1 | Medium | A call timeout bounds each call, not a stop: a run keeps sending new messages while the canister is Stopping | Open | none |
| 136 | A1 | Medium | Any interface-fetch error, including transient and self-caused ones, is cached for 5 minutes as the target's failure and served to every caller | Open | none |
| 142 | A1 | Medium | Deeply nested reply bytes overflow the Wasm stack inside candid's decoder before the 128-level check | Open | none |
| 164 | A1 | Low | Error messages echo caller- or target-supplied text without the truncation used elsewhere | Open | none |
| 165 | A1 | Low | `call_timeout_seconds` is not applied to interface fetches | Open | none |

### L10 Trust and transport

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 47 | R4 | Medium | Tool lists and search results come from uncertified queries that a single node could alter. A certified mode exists; it is off by default | Reopened A1: certified mode does not stop forgery (101) | mcp::certified_mode_upgrades_everything_and_disables_the_demo_page |
| 48 | R4 | Medium | Bearer tokens are visible to the infrastructure that carries requests | Accepted; re-checked A1 | none |
| 49 | R4 | Low | All tokens act as one on-chain identity | Accepted; re-checked A1; delegated authority outlives revocation (193) | none |
| 101 | A1 | Medium | `certified_reads` does not stop a single node from forging query responses: certified data always says "skip verification" on every path | Open | none |
| 129 | A1 | Medium | Audit free-text fields embed caller- and target-authored text in server sentences and labels without escaping | Open | none |
| 166 | A1 | Low | The video tooling leaves the plaintext MCP token in Chrome profiles in TMPDIR, can leave a DevTools port open, sends the token to whatever answers on port 9333, and misreads a quoted `.env` token | Open | none |

### L11 Sandbox and determinism

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 61 | A1 | High | Python `eval`/`exec` compile a run-time string of any size in one unchecked step | Open | none |
| 72 | A1 | High | Boa's process-global `Symbol.for` registry keeps every key across runs; a token holder can grow the heap until upgrade | Open | none |
| 85 | A1 | Medium | At 65,536 pages of Wasm memory a shadow-stack overflow wraps below 2^32 silently and overwrites other runs' heap data | Open | none |
| 113 | A1 | Medium | Rhai `sleep()` and the `sync` lock's spin call `std::thread::sleep`, which panics on wasm32 | Open | none |
| 123 | A1 | Medium | A sort comparator that is not a total order makes the canister trap | Open | none |
| 124 | A1 | Medium | `String.prototype.at` with an index at or below -2^63 traps the canister | Open | none |
| 179 | A1 | Low | Dynamic `import()` enqueues a job the executor drops, so the run fails with a misleading "never settles" error | Open | none |

### L12 Protocol conformance

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 50 | R1 | Low | A null request id was accepted, and invalid ids were echoed back | Fixed; re-checked A1; out-of-range ids altered (158) | mcp::ids_must_be_string_or_number |
| 103 | A1 | Medium | Tool arguments are not checked against the advertised `inputSchema`: unknown keys, non-object `arguments` and `language: null` are accepted | Open | none |
| 112 | A1 | Medium | When a batch reply is too big, `http_update` replaces every tool result with an error, including a committed, audited script's result | Open | none |
| 133 | A1 | Medium | `execute_icp_script` has no outcome channel besides the synchronous reply and no idempotency; a timed-out client's retry repeats every effect, and cancellation is ignored | Open | none |
| 158 | A1 | Low | JSON-RPC ids beyond i64/u64 or in exponent form are echoed altered | Open | none |
| 169 | A1 | Low | JSON-RPC edge cases: server limits reported as -32600, and invalid requests carrying a `result` key are silently dropped | Open | none |
| 190 | A1 | Low | The server never validates the `Origin` header (an MCP 2025-03-26 Streamable HTTP MUST), and the README's deviations section does not say so | Open | none |

### L13 Upgrade and compatibility

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 51 | R4–R6 | Medium | Adding audit statuses broke the interface for existing readers three times | Open; re-checked A1: two breaks in git history (Call, Trapped), whole page lost; rollback makes pages trap | none |
| 79 | A1 | High | Upgrading with install arguments re-registers revoked tokens and undoes allowlist narrowing; `icp deploy` re-sends `init_args` on every upgrade | Open | none |
| 128 | A1 | Medium | In-flight bookkeeping lives only in the heap: an upgrade without a stop or a snapshot load with calls outstanding leaves phantom or orphaned runs | Open | none |

### L14 Configuration

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 104 | A1 | Medium | `set_limits` (and `InitArgs.limits`) replace the whole record, so omitting the optional fields silently turns `certified_reads` and `call_timeout_seconds` off | Open | none |
| 106 | A1 | Medium | Limits are accepted unvalidated: `max_instructions` above ~30-40e9 turns clean budget stops into 40e9 traps in every runtime, and `call_timeout_seconds = 0` is accepted | Open | none |
| 138 | A1 | Medium | Token ids shown to the controller (16-hex prefix) cannot be used to revoke; revocation needs the full hash, which nothing returns | Open | none |
| 144 | A1 | Medium | Install, upgrade and admin arguments under `opt` that do not fit their type decode as null and are silently not applied | Open | none |
| 170 | A1 | Low | Admin and install inputs are unbounded and unchecked for duplicates | Open | none |

### L15 Supply chain and build

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 107 | A1 | Medium | `icp.yaml` copies the Wasm from a hard-coded `./target` path, so a build directory elsewhere ships a stale or different Wasm and reports success | Open | none |
| 171 | A1 | Low | Release builds are not reproducible elsewhere: absolute builder paths are embedded (and reach callers in trap text), and `ic-wasm` is unpinned | Open | none |
| 172 | A1 | Low | The shipped profile differs from the tested one: release has no overflow checks, and dependencies gate guards by target | Open | none |
| 183 | A1 | Low | `scripts/demo-setup.sh` parses human-readable icp-cli output and overrides `ICP_ENVIRONMENT` | Open | none |
| 184 | A1 | Low | Two unmaintained crates ship in the Wasm (smartstring via rhai; the vendored chrono via monty) and nothing checks advisories | Open | none |

### L16 Claims

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 52 | R6 | Medium | The docs called a run without a final entry proof of a trap; it can also be pending | Fixed; re-checked A1 | manual |
| 53 | R6 | Low | The audit `calls` field counted attempts, not messages sent | Fixed; re-checked A1; the cap counts slots (152) | manual |
| 80 | A1 | Medium | The Candid override layer is undocumented in precedence, invisible to operators and not tied to the allowlist | Open | none |
| 102 | A1 | Medium | The tool description tells the AI model there is no randomness; Python's `random` returns the same fixed-seed sequence every run | Open | none |
| 108 | A1 | Medium | The search tool's description shown to the AI and README:14 say search is free, unaudited and outside the call cap; a fetching search is an audited, capped update | Open | none |
| 109 | A1 | Medium | README:367 and the `EXECUTION_BUDGET` comment say Python has its own 10B on-chain budget; on chain it uses what is left of the policy budget | Open | none |
| 130 | A1 | Medium | The demo comparison and README benchmark count all 10 store methods as Classic tools while Code Mode sees 8; on equal surfaces the restocking result flips | Open | none |
| 137 | A1 | Medium | The documented MCP endpoint `http://icp_code_mode.local.localhost:8010/mcp` does not resolve for Node or Python clients on macOS | Open | none |
| 140 | A1 | Medium | Records whose labels are 0..n-1 are returned as JSON arrays, while the README and search description say records are objects | Open | none |
| 173 | A1 | Low | Minor documentation mismatches | Open | none |
| 174 | A1 | Low | The audit log is called "immutable"/"append-only", but controllers can rewind it by restoring a snapshot or reinstalling, and ids are then re-issued | Open | none |
| 178 | A1 | Low | Search audit entries use an undocumented language value and hash the caller's raw spelling | Open | none |
| 192 | A1 | Low | Search output does not match what the converter accepts or what `icp.call` allows | Open | none |
| 199 | A1 | Low | The token-hash recipe in the `.did` and InitArgs comments (`echo -n ... | shasum`) gives a different digest under POSIX sh | Open | none |

### L17 Client and presentation

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 167 | A1 | Low | Text written by a target canister reaches the AI model verbatim with nothing marking it as third-party data | Open | none |
| 197 | A1 | Low | The demo page splices a reply value into script source, parses `.did` text with a quadratic regex, and keeps the token in localStorage | Open | none |

### L18 Operations

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 96 | A1 | Medium | `get_audit_logs` pages are bounded by count, not bytes; a legacy 8.39 MB entry is unreadable through the API | Open | none |
| 180 | A1 | Low | The audit log has no retention or export path; appends trap if stable memory cannot grow, and each new bucket raises the freezing threshold | Open | none |
| 181 | A1 | Low | No in-app view of cycles or memory, no low-memory hook, and no runbook; frozen or stuck-stopping canisters expose nothing | Open | none |
| 182 | A1 | Low | Audit paging is offset-from-newest, so pages shift as the log grows | Open | none |

### Testability

Gaps in the tooling, each of which hides a class of issue from every audit until closed.

| Id | Round | Gap | Status |
|---|---|---|---|
| 54 | R4 | No automated integration tests against a local network. Every "manual" guard above depends on this | Open (re-checked A1) |
| 55 | R4 | No misbehaving test target: one that never replies, replies late, returns oversized data, traps, or lies about its interface | Open (re-checked A1) |
| 56 | R5 | No fuzz or property tests for the parsers and value converters, where most high-severity findings were | Open (re-checked A1) |
| 57 | R4 | Nothing has run on the real network; gateway behaviour, costs and installing the large artifact there are unverified | Open (re-checked A1) |
| 58 | R6 | Tests cannot drive the production bridge off-chain, so audit-entry behaviour is only checked by hand. A1: `IcBridge` runs natively up to the first platform read; the missing seams are time, the call primitive and `canister_self` (see 110) | Open |
| 110 | A1 | Platform reads have no test seam, so the instruction budget, interface TTLs and every send path cannot be exercised natively (Medium) | Open |
| 132 | A1 | The audit workflow turns missing agent output into "nothing found" and dedupes findings with a lossy 160-character key (Medium) | Open |
| 134 | A1 | Register guards do not detect regressions: 64 of 70 limit mutants survive, some guards pass with the fix deleted, and others hang or abort (Medium) | Open |
| 146 | A1 | Nothing runs the built Wasm and there is no CI, so wasm32-only traps and build invariants are never checked (Medium) | Open |
| 201 | A1 | Audit method gaps: failure sites were inventoried from app source only, and the shared build directory served stale binaries (Low) | Open |

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
