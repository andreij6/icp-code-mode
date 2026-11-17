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

## Findings by class

Classes are the lenses in the plan.

### L1 Access control

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 1 | R1 | High | The interface cache and admin overrides were keyed by the caller's spelling of a canister id, so a different letter case missed both | Fixed | search::scope_canonicalises_and_reports_unknowns (search only); bridge path: none |

### L2 Atomicity and commit points

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 2 | R1 | High | A reply over the size limit trapped after calls had run, which rolled back the audit entry | Fixed | mcp::result_size_is_bounded_after_escaping |
| 3 | R3 | High | A run's first and final audit entries were linked only by script hash, so two runs of the same script could not be told apart | Fixed | policy::run_id_links_started_and_final_entries |
| 4 | R4 | High | A batch padded with tiny invalid messages made the reply trap, erasing the run's audit entry | Fixed | mcp::replies_always_fit |
| 5 | R4 | High | A run that trapped after its calls went out showed "Started, 0 calls" with no methods | Fixed | manual |
| 6 | R2 | Medium | A search refused by policy before any message was sent wrote no audit entry | Fixed | manual |
| 7 | R6 | High | Message entries were written before argument encoding, so a call that failed to encode left an entry for a message never sent | Fixed | manual |
| 8 | R6 | Medium | A message the system refused on the spot still left an entry | Fixed | bridge::on_first_pending_fires_once_and_only_if_pending (mechanism only) |
| 9 | R6 | Medium | A trapped run had no closing entry | Fixed | manual |
| 10 | R1 | Medium | A runtime limit hit after an await was reported as a promise that never settles | Mitigated: the message now names both causes | none |
| 11 | R4 | Low | A run that traps before sending any message leaves no audit entry | Accepted | none |

### L3 Resource bounds

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 12 | R1 | High | A deeply nested JavaScript value overflowed the stack when converted | Fixed | runtimes/js::deep_and_huge_values_are_rejected_not_overflowing |
| 13 | R1 | High | The Rhai replay loop had no cap on host requests | Fixed | runtimes/rhai_rt::request_count_bounds_replays |
| 14 | R1 | High | Script output buffers grew without limit while a script ran (Rhai and JavaScript) | Fixed | runtimes/rhai_rt::print_is_capped_per_run, runtimes/js::console_output_is_capped_while_running |
| 15 | R1 | High | Deeply nested Rhai arrays cost far more engine work than the operation count showed | Fixed | manual |
| 16 | R1 | Medium | Audit error and violation strings were stored at any length | Fixed | none |
| 17 | R1 | Medium | A valid reply with a large unknown field was rejected, and the error embedded the whole reply | Fixed | none |
| 18 | R5 | High | A self-rescheduling promise chain bypassed the loop and recursion limits | Fixed | runtimes/js::promise_chains_are_bounded |
| 19 | R5 | High | A `JSON.stringify` replacer could feed an unbounded value to the native serializer | Fixed | runtimes/js::stringify_replacer_and_size_limits |
| 20 | R5 | High | Arrays of primitives were not counted against the element limit | Fixed | runtimes/js::stringify_replacer_and_size_limits |
| 21 | R5 | High | One large string referenced many times was small inside the engine and gigabytes once copied out (all runtimes) | Fixed | runtimes/js::stringify_replacer_and_size_limits, runtimes/python::values_are_bounded |
| 22 | R5 | Low | Interface text of any size was fetched and parsed | Fixed | none |
| 23 | R1 | High | One uninterruptible built-in JavaScript operation can run past the platform's instruction limit and trap | Accepted | none |
| 24 | R5 | Medium | JavaScript has no memory cap | Accepted | none |

### L4 Multiplication

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 25 | R1 | Medium | Interface fetches did not count toward the per-script call cap | Fixed | manual |
| 26 | R1 | Medium | Scripts batched in one request shared one instruction counter | Fixed | mcp::one_tool_call_per_request |
| 27 | R2 | Low | A search kept attempting fetches after the call cap was reached | Fixed | manual |
| 28 | R5 | Medium | A batch of searches multiplied outbound calls and audit entries | Fixed | mcp::one_tool_call_per_request |
| 29 | R5 | Medium | Unlimited concurrent scripts could exhaust memory | Fixed | lib::run_slots_are_capped_and_returned (cap never triggered on a live network) |

### L5 Cost and payment

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 30 | R4 | High | Anyone could call the update endpoint directly, without a token, and make the canister pay for it | Fixed | manual |
| 31 | R4 | Medium | A token holder can grow the audit log without limit; nothing rate-limits requests | Open | none |

### L6 Consistency across variants

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 32 | R5 | Medium | Python used a fixed instruction budget instead of the configured one | Fixed | manual |
| 33 | R5 | High | Python returned deep data truncated with a marker and reported success, where the other runtimes refuse it | Fixed | runtimes/python::values_are_bounded |

### L7 Data fidelity

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 34 | R1 | High | JavaScript integers above 2^31 were rejected as natural numbers | Fixed | runtimes/js::big_js_integers_pass_as_nat |
| 35 | R1 | Medium | Misspelled record fields were silently dropped, so an optional field was sent as null | Fixed | candid_bridge::errors_are_descriptive |
| 36 | R1 | Medium | Replies nested too deep came back truncated and marked as success | Fixed | candid_bridge::deep_replies_error_instead_of_truncating |
| 37 | R1 | Low | For single-argument methods, `[x]` and `[]` were interpreted inconsistently | Fixed | candid_bridge::errors_are_descriptive (partial) |
| 38 | R1 | Low | Python bytes and Rhai blobs could not be passed as binary data | Fixed | runtimes/python::determinism_and_sandbox, runtimes/rhai_rt::limits_and_sandbox |
| 39 | R1 | Low | Non-finite numbers decoded to a value that could not be sent back | Fixed | candid_bridge::js_style_floats_and_non_finite |
| 40 | R5 | Low | Negative zero was rejected as a natural number | Fixed | candid_bridge::js_style_floats_and_non_finite |
| 41 | R2 | Low | Search results cut by the method cap showed an empty method list | Fixed | search::unfetched_and_failed_interfaces_and_cap |
| 42 | R1 | Low | A doubly optional value cannot be told apart from a missing one | Accepted | none |

### L8 Concurrency and shared state

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 43 | R1 | Medium | Python's memory limit used a process-wide baseline that concurrent runs overwrote | Fixed | runtimes/python::memory_limit_survives_interleaved_runs |

### L9 Failure handling

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 44 | R4 | High | Calls wait indefinitely, so a target that never replies can block stopping and upgrading. A timeout option exists; it is off by default | Fixed (opt-in) | manual, success path only |
| 45 | R4 | Medium | Fetched interfaces were cached until the next upgrade | Fixed | none (the expiry was never observed) |
| 46 | R5 | Low | Failed interface fetches were repeated on every request | Fixed | none |

### L10 Trust and transport

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 47 | R4 | Medium | Tool lists and search results come from uncertified queries that a single node could alter. A certified mode exists; it is off by default | Fixed (opt-in) | mcp::certified_mode_upgrades_everything_and_disables_the_demo_page |
| 48 | R4 | Medium | Bearer tokens are visible to the infrastructure that carries requests | Accepted | none |
| 49 | R4 | Low | All tokens act as one on-chain identity | Accepted | none |

### L12 Protocol conformance

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 50 | R1 | Low | A null request id was accepted, and invalid ids were echoed back | Fixed | mcp::ids_must_be_string_or_number |

### L13 Upgrade and compatibility

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 51 | R4–R6 | Medium | Adding audit statuses broke the interface for existing readers three times | Open | none |

### L16 Claims

| Id | Round | Severity | Finding | Status | Guard |
|---|---|---|---|---|---|
| 52 | R6 | Medium | The docs called a run without a final entry proof of a trap; it can also be pending | Fixed | manual |
| 53 | R6 | Low | The audit `calls` field counted attempts, not messages sent | Fixed | manual |

### Testability

Gaps in the tooling, each of which hides a class of issue from every audit until closed.

| Id | Round | Gap | Status |
|---|---|---|---|
| 54 | R4 | No automated integration tests against a local network. Every "manual" guard above depends on this | Open |
| 55 | R4 | No misbehaving test target: one that never replies, replies late, returns oversized data, traps, or lies about its interface | Open |
| 56 | R5 | No fuzz or property tests for the parsers and value converters, where most high-severity findings were | Open |
| 57 | R4 | Nothing has run on the real network; gateway behaviour, costs and installing the large artifact there are unverified | Open |
| 58 | R6 | Tests cannot drive the production bridge off-chain, so audit-entry behaviour is only checked by hand | Open |

## Refuted

Kept so the next audit does not spend time on them again without new evidence.

| Round | Claim | Why it was refuted |
|---|---|---|
| R1 | Returning large numbers as either a number or a string is a defect | Deliberate and documented: numbers stay numbers while they are safe for JavaScript |
| R5 | A Rhai value of 900,000 elements passes the limits | Rhai's own size limit counts nested elements and rejects it |

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
