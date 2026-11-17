# Coverage grid: Testability (register class), round 5

Run `2026-11-17-295d64b`, fifth round for this lens. Question: does any register guard fail
when its fix is reverted, and does any test detect a mutated limit comparison? Rows: T1-T12,
the register rows whose Guard names a test (#1, #2, #3, #4, #8, #12, #13, #14, #18-#21, #26,
#28, #29, #33-#41, #43, #47, #50), and the limits named in the task (LIM rows for
`ValueBudget`, `LogBuf`, `bridge::truncate`, `mcp::finish`/re-measure, `authenticate`,
`authorize`/`check_call`, `RunSlot`, `audit_page`, Rhai `MAX_REQUESTS`, JS `MAX_PENDING`/
`MAX_ROUNDS`) plus neighbouring limits.

## Method

- Private copy: `git archive 295d64b` into a temp directory, `CARGO_TARGET_DIR` = the shared
  `/private/tmp/claude-501/audit-target`. **The package version was changed to a unique
  string in the copy** (`0.1.0-auditt5`): with the original version, the shared directory gave
  every copy the same test-binary hash, and the first `cargo test` here silently ran another
  agent's binary (84 tests, 14 failing, from sources that are not in `295d64b`). Baseline
  after the change: 57 passed (identical to the baseline report).
- Harness (scratch, not in the repository): `/private/tmp/claude-501/audit-t5-harness/`
  `mutants.py` (110 mutants as exact text replacements), `run.py` (apply, build
  `cargo test -p icp_code_mode --lib --no-run`, run the test binary under a 300 s alarm, record
  failing tests, `git checkout`), `results.jsonl`, `probes_audit_t5.rs` (boundary probes).
- 40 register-guard mutants (R*: revert or neutralise the fix, at the helper and at the call
  site) and 70 limit mutants (L*: `>`/`>=` flips, +1, deletion of the check). No cargo-mutants
  (not installed; installing it would download and run new code).
- Boundary probes (P1-P8) in a second copy: values just under, at and over each limit, a
  real interleaving for the Python memory baseline, and a log sink without a cap.

Evidence key: **R** reproduced (mutant run or probe); **C** code path; **K** already reported.

## Answer to "do the 59 tests detect anything at all?"

| Mutant set | Killed by a failing assertion | Killed only by hang/crash | Survived |
|---|---|---|---|
| Register guards (40) | 28 | 3 (#13 and #18: 300 s hang; #19: stack-overflow abort of the whole binary) | 9 |
| Limit comparisons (70, 4 of them controls) | 6 (4 controls + `authorize` cap + `RunSlot` drop) | 0 | 64 |
| Total (110) | 34 | 3 | 73 |

The suite detects reversions of the exact input a past finding used. It does not detect
boundary errors (no `>`/`>=` flip on any limit was caught), deletion of most checks that no
past finding exercised, or reversion at the call site when the test drives a helper.

## Register guards

| Row | Mutant | Result | Cell |
|---|---|---|---|
| #1 search | R1a `scope` compares spelling | killed by the named test | holds |
| #1 bridge path | R1b `IcBridge::candid` keys by caller spelling | survived | known (register: none; #58) |
| #2 | R2a/R2b byte truncation, R2c control char = 1 B | killed by the named test | holds |
| #3 | R3a `append_audit` no `run_id` default | killed | holds for the helper |
| #3 | R3b `run_script` writes `run_id: None`; R3c `Call` entries open their own run | **survived** | **finding T5-3** |
| #4 | R4a batch cap x1000, R4b `finish` guard removed | killed by the named test | holds (boundary: L52, L12 survive) |
| #8 | R8a hook fires when ready | killed | holds (mechanism) |
| #8/#7 | R8b record before send | survived | known (register "mechanism only", #58) |
| #12 | R12 JS depth check removed | killed | holds |
| #13 | R13 request cap removed | detected only as a hang (no test timeout) | **finding T5-5**; boundary L29 survives (T5-4) |
| #14 | R14a Rhai in-run cap removed, R14b JS in-run cap removed | **survived** | **finding T5-1** |
| #18 | R18 promise-job cap removed | detected only as a hang | **finding T5-5**; L64, L65 survive |
| #19 | R19 replacer to native serializer | detected only by aborting the test binary (stack overflow) | **finding T5-5** |
| #20 | R20 primitives not counted | killed | holds |
| #21 | R21a JS bytes, R21b `ValueBudget::bytes` | killed (JS and Python tests) | holds; Rhai not asserted (T5-4) |
| #26 | R26 second script allowed | killed | holds |
| #28 | R28 search cap `>` | killed | holds (update path only; query path K) |
| #29 | R29a `RunSlot::take` `<=`, L26 drop no-op | killed | holds for `RunSlot` |
| #29 | R29b `run_script` drops its slot at once | **survived** | **finding T5-3** |
| #33 | R33 Python depth check removed | killed | holds |
| #34 | R34a `normalize_numbers`, R34b `num_text` | killed | holds |
| #35 | R35 unknown field accepted | killed | holds |
| #36 | R36 reply depth check removed | killed | holds (L51 `>=` survives) |
| #37 | R37b `[x]` arm removed: killed by other tests, not the named one; R37a `is_array_like` always false | **survived** | partial, as the register says; vec-typed single argument untested (T5-4) |
| #38 | R38a Python bytes, R38b Rhai blob | killed | holds |
| #39 | R39 NaN -> null | killed | holds |
| #40 | R40 `-0` | killed | holds |
| #41 | R41 per-canister count dropped | killed | holds |
| #43 | R43 `rebased` removed | **survived** | **finding T5-2** |
| #47 | R47 certified mode no upgrade | killed | holds |
| #50 | R50 id type check removed | killed | holds |

## Tests (inventory rows)

| Row | Cell |
|---|---|
| T1 `run_slots_are_capped_and_returned` | holds for `RunSlot` (R29a, L26 killed); does not cover the caller (R29b survived): T5-3 |
| T2 `did_file_matches_exported_interface` | not applicable to mutation (asserts generated text, not a limit); it was green under all 110 mutants as expected |
| T3 `on_first_pending_fires_once_and_only_if_pending` | holds for the combinator (R8a); the call site (R8b) is #58, K |
| T4 `candid_bridge::tests` (7) | holds for #34-#36, #39, #40 (R); finding T5-4 for the limits: L49 (decoding/skipping quotas deleted), L50 (`json_to_idl` depth check deleted), L51 (reply depth `>=`) and R37a survive |
| T5 `mcp::tests` (13) | holds for #2, #4, #26, #28, #47, #50 and the routing codes; finding T5-4: L12 `finish` `>=`, L13 re-measure deleted, L14, L15 any auth scheme, L16 no trim, L17 last header, L52 batch `>=`, L53 survive |
| T6 `policy::tests` (7) | holds for the `authorize` cap (L20 killed) and run_id default (R3a); finding T5-4: L21 budget `>=`, L22 cap n+1 in `check_call`, L23 budget ignored in `check_call`, **L24 `check_interface_fetch` admits any canister**, L27 page cap deleted, L28 page cap +1 survive |
| T7 `cross_language_tests` (5) | holds for what it asserts; L34 `MAX_CODE_BYTES` `>=` survives (T5-4) |
| T8 `js::tests` (8) | holds for #12, #19-#21, #34 (R); #18/#19 detected only by hang/crash (T5-5); T5-1 (console cap); T5-4: L30/L31 `MAX_PENDING` flipped/deleted, L32/L33 `MAX_ROUNDS` +1/x1000, L58 key bytes, L59 depth `>`, L64/L65 promise-job boundary and budget, L70 survive |
| T9 `python::tests` (5) | holds for #21, #33, #38; **T5-2** (`memory_limit_survives_interleaved_runs` never interleaves); T5-4: L35 suspensions x1000, L36 print cap removed, L56 key bytes, L63 per-argument budget, L68 VM budget survive |
| T10 `rhai_rt::tests` (5) | holds for #38 blob; T5-1 (print cap); #13 detected only by hang (T5-5); T5-4: L29 `MAX_REQUESTS` `>`, L57 key bytes, L60/L61 depth `>=`/deleted, L62 call args unbounded, L66 budget check removed, L69 survive |
| T11 `search::tests` (4) | holds (#1 search, #41, L55 control) |
| T12 `demo_store::tests::seed_is_consistent` | not applicable (demo fixture; not in `icp_code_mode --lib`, no limit or guard of the audited canister) |

## Limits (task list)

| Limit | Mutants | Boundary probe (R) | Cell |
|---|---|---|---|
| `ValueBudget::node` | L01 `>=`, L02 +1: survived | node 200,000 ok, 200,001 error; Python/JS 199,999-element list ok, 200,000 error (= 200,001 nodes) | behaviour holds; untested: T5-4 |
| `ValueBudget::bytes` | L03, L04: survived | 4,194,304 ok, +1 error; Python 4 x 1 MiB ok, +1 B error | holds; untested: T5-4 |
| depth 128 (all runtimes) | L51, L59, L60: survived | 128 levels ok, 129 error in Python, JS and Rhai | holds; untested: T5-4 |
| `LogBuf::push` | L05, L06, L07 (cap deleted): survived | a 65,536 B line kept, next dropped; 65,535 + 2 B gives a 21 B marker line | holds; untested: T5-4 (never reached by any test: every runtime caps first) |
| `bridge::truncate` | L08-L11: survived | 300 at 300 unchanged; 301 -> 314 B; multibyte cut on a boundary | overshoot K; untested (K, variant) |
| `mcp::finish` / `http_update` re-measure | L12, L13 (deleted), L14: survived | not run (needs > 1.9 MB of tool results in one request) | untested: T5-4 |
| `mcp::authenticate` | L15 any scheme, L16 no trim, L17 last header: survived; L18, L19 (header/scheme case) killed | `bearer` lower-case accepted; padded token accepted; `Basic` 401; no scheme 401; two headers: the first decides | behaviour holds; scheme, trim and duplicate-header rules untested: T5-4 |
| `authorize` / `check_call` | L20 cap `>` killed; L21, L22, L23, L24 survived | n/a | cap holds; budget boundary, call-site arguments and the interface-fetch allowlist untested: T5-4 |
| `IcBridge::reserve` | L25 slot never consumed: survived | n/a | #58 K; listed in T5-4 |
| `RunSlot::take` | R29a, L26 killed | n/a | holds |
| `audit_page` | L27 cap deleted, L28 +1: survived | 150 entries: `limit` 1000 -> 100, 101 -> 100 | behaviour holds; cap untested: T5-4 |
| Rhai `MAX_REQUESTS` | L29 `>`: survived; R13 deletion -> hang | 64 requests ok, 65 error | holds; boundary untested: T5-4, T5-5 |
| JS `MAX_PENDING` | L30, L31: survived | 64 concurrent ok, 65 `RangeError` | holds; untested: T5-4 |
| JS `MAX_ROUNDS` | L32, L33: survived | 64 sequential host requests already fail (the first round runs the script body, so 63 is the most) | behaviour matches README "64 rounds"; untested: T5-4 |
| others: `deny` (L37, L38), audit error (L39), DID size/TTL (L40-L42), method length (L43), `Trapped` status (L44), search audit condition (L45), override size (L46), `inspect_message` (L47), post-run budget (L48), `budget_error` (L67) | all survived | n/a | K (tests per past finding, not per limit; #58; T-1 counter seam) |

## Candidate findings

**T5-1 (Medium, testability). The register #14 guards pass with both in-run output caps
deleted.** R14a removes the cap in Rhai's `RunState::log` (`src/runtimes/rhai_rt.rs:46-54`);
R14b disables the cap in JS `console_fn` (`src/runtimes/js.rs:237-243`). All 57 tests pass,
including `rhai_rt::print_is_capped_per_run` and `js::console_output_is_capped_while_running`.
Cause: both tests run on `MockBridge`, whose `log()` is a `LogBuf` that caps again at 64 KiB
after the run (`src/bridge.rs:106-119`), and the tests measure only that final buffer. Register
#14 was about buffers growing *while the script runs*; under R14a a run holds 1,000 x 100 KB
lines in `RunState.logs` before the bridge sees them. Probe P8 (a bridge whose `log` keeps
everything) fails under R14a and R14b and passes on `295d64b` (65,550 B in one line for JS
and Rhai). Fix: run the two tests against a sink without a cap (P8), or assert on the
runtime's own buffer.

**T5-2 (Medium, testability). `python::memory_limit_survives_interleaved_runs` never
interleaves, so the register #43 fix can be deleted.** Its script's first statement
`x = 'y' * 100000000` is already over the 64 MiB limit on its own (probe: `MemoryError` with
no call), so the run fails before its first `await` and the second run never overlaps it.
R43 (`rebased` reduced to `f.await`, `src/runtimes/python.rs:151-157`) passes all tests.
Probe P5 splits the allocation (40 MB, call, 40 MB; each part fits, together 80 MB do not):
on `295d64b` it fails alone and interleaved (correct); under R43 the interleaved run returns
success with 80 MB live, i.e. the memory limit is bypassed and only P5 fails. Fix: replace the
test's sizes with ones that fit before the await (P5).

**T5-3 (Low, testability/claims). Register guards #3 and #29 test a helper, not the place
the fix lives.** R3b (`run_script` writes `run_id: None`, `src/lib.rs:258`), R3c (`Call`
entries written with `run_id: None`, `src/bridge.rs:412`) and R29b (`let Some(_) =
RunSlot::take()`, which drops the slot at once and disables the 8-run cap,
`src/lib.rs:219`) all pass the 57 tests. `policy::run_id_links_started_and_final_entries`
builds entries with hand-written run ids and `run_slots_are_capped_and_returned` takes slots
directly. The register lists both as automated guards with no qualifier (unlike #8, "mechanism
only"). Same root as the reported "glue not testable off-chain" / #58: merge, but correct the
register's Guard column for #3 and #29 (and #37: R37a survives, R37b is caught by other
tests, not by `errors_are_descriptive`).

**T5-4 (Medium, testability; variant of the reported "tests were added per past finding, not
per inventory limit"; merge and extend its location list).** 64 of 70 limit mutants survive,
including every `>`/`>=` flip and every +1. New locations beyond the reported ones: an
authorization check, `check_interface_fetch` admitting any canister
(`src/policy.rs:201`, L24); `check_call` ignoring the budget or counting one call too many
(`src/policy.rs:195`, L22, L23); `authorize` budget `>=` (`:184`, L21); `audit_page` cap
deleted (`:265`, L27); `authenticate` scheme filter, `trim` and first-header rule
(`src/mcp.rs:276-281`, L15-L17); `http_update` re-measure deleted (`:405-410`, L13);
`finish` and `MAX_BATCH` `>=` (L12, L52); `ValueBudget` boundaries (`src/bridge.rs:59-73`,
L01-L04); `LogBuf` deleted (`:106-119`, L05-L07); JS `MAX_PENDING` deleted and `MAX_ROUNDS`
x1000 (`src/runtimes/js.rs:254, 374`, L30-L33); Rhai `MAX_REQUESTS` `>`, depth check deleted,
call arguments unbounded, budget check removed (`src/runtimes/rhai_rt.rs:89, 160, 148, 211`,
L29, L61, L62, L66); Python suspensions x1000, per-argument budget (`python.rs:239, 244`, L35,
L63); key bytes uncounted in all three runtimes (L56-L58); Candid decoding/skipping quotas
deleted and `json_to_idl` depth deleted (`src/candid_bridge.rs:100-101, 196`, L49, L50). The
boundary behaviour itself holds where probed (table above); nothing pins it. Fix: one table-
driven boundary test per limit (under / at / over), the probes in `probes_audit_t5.rs` are a
start.

**T5-5 (Low, testability). Three guards detect a regression only by hanging or crashing the
test run.** With the fix of #13 or #18 removed, `request_count_bounds_replays` and
`promise_chains_are_bounded` spin until killed (300 s alarm here; `cargo test` has no
timeout, so a CI job would hang). With #19 reverted, `stringify_replacer_and_size_limits`
overflows the native stack and aborts the test binary (`fatal runtime error: stack
overflow`), so every test after it reports nothing. Fix: give these tests a bounded
input or an instruction/iteration budget the mock can trip (see T-1's counter seam), and a
per-test timeout in CI.

## Surviving mutants (all 73)

Register: R1b, R3b, R3c, R8b, R14a, R14b, R29b, R37a, R43.
Limits (64): L01 L02 L03 L04 L05 L06
L07 L08 L09 L10 L11 L12 L13 L14 L15 L16 L17 L21 L22 L23 L24 L25 L27 L28 L29 L30 L31 L32 L33
L34 L35 L36 L37 L38 L39 L40 L41 L42 L43 L44 L45 L46 L47 L48 L49 L50 L51 L52 L53 L56 L57 L58
L59 L60 L61 L62 L63 L64 L65 L66 L67 L68 L69 L70. Descriptions in `mutants.py`.

## Not tested, and why

- Mutants inside `IcBridge`, `run_script`/`run_search` glue, `inspect_message` and the wasm-only
  budget branches can only be detected by the on-chain harness (#54, #58) or a counter/clock
  seam (T-1); they were run to confirm they survive, not to diagnose them.
- `finish`/re-measure boundaries were not probed (needs a request whose tool results exceed
  1.9 MB, i.e. a bridge that returns large replies).
- No cargo-mutants exhaustive run: not installed, and installing it means downloading and
  running new code. The 110 hand mutants cover the task's list.
- Unverified side observation (L3/L6 to decide): `'x'.repeat(1048576)` in JS ends with
  `RuntimeLimitError: reached the maximum number of iteration loops`, so the at-limit 4 MiB
  text probe could not be built in JS that way.
