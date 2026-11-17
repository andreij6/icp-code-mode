# Variant sweep r2-4: input count unbounded where only output is capped

Run `2026-11-17-295d64b`, Phase 6, round 2. Pattern (seed, known, not re-reported: search
keyword count and length are unbounded while `MAX_METHODS` caps only what is shown):

> A cap that bounds what is stored or returned, while the number (or size) of inputs that
> feed the work before the cap is applied is unbounded, so the work and memory scale with
> the input and only the output looks bounded.

Method: listed every cap in the Limits inventory and every `truncate`, `MAX_*`, `.take(`,
`join(` and early-return guard in `src/`, then asked for each one whether it counts the
inputs that do the work or only what is kept or returned. Probes ran natively in a private
copy (`cargo test --lib`, shared `CARGO_TARGET_DIR`, own test binary copied out because the
shared build dir was being overwritten by other agents' builds). The copy was deleted
afterwards. No live instance was touched.

## Caps and what they count

| Row | Cap | Counts inputs or only output? | Result |
|---|---|---|---|
| LIM32 / W3 search `MAX_METHODS` | 40 methods shown | output only; every allowed method x every keyword matched | seed (known) |
| LIM9 / LIM8 `MAX_SEARCHES`, `MAX_BATCH` | 5 / 50 | `MAX_BATCH` counts the parsed array length (notifications included); parse happens first but is bounded by ingress size | holds (query-path gap is known) |
| LIM5 `MAX_AUDIT_PAGE` | 100 | `audit_page` computes indexes from `offset` arithmetically; work is O(limit), not O(offset) (`policy.rs:261-270`) | holds (byte size known) |
| LIM14 `MAX_VIOLATIONS` | 16 kept, rest counted as `…and N more` | refusals beyond 16 are counted, not lost; refusal count bounded by JS 64x64, Rhai 64; Python known (no per-batch cap) | holds |
| LIM15 `MAX_LOG_BYTES` (JS `console_fn`, Rhai `RunState::log`, `LogBuf::push`) | 64 KiB of line content | **counts content bytes only**: a 0-byte line is free, and the line count is unbounded | **finding SW24-1** (new) |
| LIM15 JS `console_fn` formatting | 64 KiB per run | checks the cap before formatting, then formats **every argument in full** and joins them before truncating; argument count unbounded (`apply`), string arguments not held to the 4 MiB value limit | **finding SW24-2** (new) |
| LIM15 Python `CollectString` | 64 KiB | counts separators and newlines (`monty-types io.rs` `write`/`push`), so `out.lines()` is at most 65,536 lines | holds (MemoryError and format-before-cap are known) |
| LIM15 Rhai print/debug text size | 1 MiB string, 10,000 elements | Rhai's data-size check sums string bytes inside arrays, so a printed value is at most about 1 MiB | holds (empty-line count: SW24-1) |
| LIM6 `MAX_RESULT_BYTES` (result) | 1 MB escaped | input is a value already held to 200k nodes / 4 MiB | holds (truncation success known) |
| LIM6 logs block in `tool_result` | 250 KB escaped | `logs.join("\n")` of every line, then truncate; line count from SW24-1 | symptom of SW24-1 |
| LIM7 `MAX_REPLY_BYTES` | 1.9 MB | sum over at most 50 replies | holds (fallback behaviour known) |
| LIM11 rpc error 300 B | | formats then truncates; input bounded by body | holds |
| LIM13 audit error 4 KiB (`fetch_errors.join`) | | one error per allowlisted canister (admin-sized); each echo up to 512 KiB is known | holds / known |
| LIM31 principal echo 80 B | | count of `icp.call` strings bounded by `MAX_PENDING` 64; size of the canister/method string is copied (`arg_string`) before the 80 B / 256 B checks | not this pattern (size, count bounded); unverified note below |
| LIM16 value limits | per value | each value has its own budget; JS `console.log` applies one budget per argument (SW24-2) | see SW24-2 |
| LIM25 JS `MAX_PENDING`/`MAX_ROUNDS` | 64 / 64 | count inputs at enqueue | holds (getter bypass known) |
| LIM27 Rhai `MAX_REQUESTS` | 64 | counts answers before each request | holds |
| LIM26 Python suspensions 256 | | counts suspensions; per-gather fan-out known | known |
| LIM12 run slots | 8 | scripts only; searches known | known |

## Other inventory rows

| Row | Result |
|---|---|
| C1/C1a, C3, C4, C7-C11, C13, C15 | not applicable: build/config, no input-fed cap |
| C2 / E20-E25 demo_store | no output caps at all (`list_orders`, `search_products`); unbounded orders known; not this pattern |
| C5 / E26 / B17 / W16 demo page | browser-side; `slice(0, 40)` caps display only, cost of the interface regex is known |
| C6, C12, C14 | not applicable (scripts/docs) |
| A1-A12 | actors: covered through their entry points below |
| E1/E2/E6-E11 admin | inputs unbounded (known: admin inputs unbounded, `get_policy` in one reply) |
| E3 / U2 / U12 / W2 | header list scanned with `find` (first `authorization`), linear in ingress size; decode quota known |
| E4/E4a/E4b/E5/E5a-E5d | covered by the caps above |
| E12-E14 | `get_policy` known; `get_audit_logs` holds (offset arithmetic); `get_audit_log_len` O(1) |
| E16/E17 JS host functions | SW24-1, SW24-2 |
| E18 Python host functions | holds (print counts newlines) |
| E19 Rhai host functions | SW24-1 (empty lines bounded by the 1M operation limit) |
| U1, U3, U4, U5 | U5 `query` = seed; `canister_id` single value; holds otherwise |
| U6, U7 | SW24-2 (console arguments) |
| U8, U9, U10 | decoding quota, reject text and interface size known |
| U11 | known |
| U13, U14, U15, U16 | not applicable or known |
| O1-O10 | outbound effects bounded by the call cap; no output-only cap |
| D1-D10 | D4 growth known (#31); others admin-sized |
| V1-V11 | V3 PENDING_RUNS (known, searches); V2/V4/V5 keyed by allowlisted canisters (admin) |
| K1-K12 | not applicable (no caps) |
| W1 | holds (batch counted after parse, bounded by ingress) |
| W4 | `list_allowed` copies whole allowlist per search; admin-sized; known |
| W5, W6, W7, W8, W10, W11 | known |
| W9 | refusals: 64x64 policy checks max, violations counted; holds |
| W12 | holds |
| W13 | holds |
| W14, W15 | known |
| F1-F16 | not applicable to this pattern; SW24-2 can end in F13 (out of memory) |
| B1-B17 | B11 logs join = SW24-1 symptom; others not this pattern |
| VF1-VF8 | VF5 (work per run) is where SW24-1/SW24-2 cost lands |
| AD1-AD9 | known |
| S1-S10 | not applicable |
| CL14 "Captured logs <= 64 KiB, enforced while running" | contradicted in memory terms by SW24-1 (bytes hold, entry count does not) and by SW24-2 (formatting happens before the cap) |
| T1-T12 | `console_output_is_capped_while_running` / `print_is_capped_per_run` only sum line lengths, so they cannot see either finding |

## Findings

### SW24-1 (new): the log cap counts line bytes, never lines; empty lines are free

`console_fn` (`src/runtimes/js.rs:227-248`), Rhai `RunState::log`
(`src/runtimes/rhai_rt.rs:46-54`) and `LogBuf::push` (`src/bridge.rs:106-119`) add only
`line.len()` to the 64 KiB counter. `console.log()` / `console.log("")` / Rhai `print("")`
add 0 bytes and are always accepted, so the `Vec<String>` of lines grows without a count
limit while the run lasts (JS keeps it in `State` across every await). `tool_result`
(`src/mcp.rs:164-167`) then joins all of them with `\n` and truncates to 250 KB escaped.

Reproduced natively (test in the private copy, `MockBridge`):

| Script | Lines kept | Bytes counted | Console text returned | Peak RSS |
|---|---|---|---|---|
| JS `function f(){for(let i=0;i<1e6;i++) console.log()} for(let j=0;j<1;j++) f()` | 1,000,000 | 0 | 125,031 chars | 79 MB |
| same, 5 frames | 5,000,000 | 0 | 125,031 | 357 MB |
| same, 20 frames | 20,000,000 | 0 | 125,031 | 1.49 GB |
| Rhai `for i in 0..400000 { print(""); } 1` | 333,332 (op limit hit, logs still kept) | 0 | | |

On chain the only bound is the instruction budget (each `console.log` checks it); the
number of lines that fit in 20B instructions was not measured (no live run in this sweep).
Python holds: Monty counts the newline of every `print`.

### SW24-2 (new): JS `console.log` formats every argument in full before applying the cap

`console_fn` checks `log_bytes >= MAX_LOG_BYTES` once, then runs
`args.iter().map(display).collect::<Vec<_>>().join(" ")` and only then truncates the line
to the remaining budget. The number of arguments is unbounded (`console.log.apply(null, a)`),
each object argument gets its own 200k-node / 4 MiB snapshot budget, and string arguments
are copied by `to_std_string_escaped` with no size check at all. The work is one native
call with no instruction-budget check inside it.

Reproduced natively: `let s='x'.repeat(1<<19); s=s+s; const a=Array(N).fill(s);
console.log.apply(null, a); return 1` (JS heap: one 1 MiB string and an N-element array):

| N | Result | Line kept | Peak RSS | Time (debug build) |
|---|---|---|---|---|
| 1 | Ok | 21 B (`…[output truncated]`) | 33 MB | 0.05 s |
| 300 | Ok | 21 B | 662 MB | 3.8 s |
| 1000 | Ok | 21 B | 2.13 GB | 12.6 s |

About 2 MB of host memory per argument (the copies plus the joined string). On wasm32
(4 GiB address space, 3 GiB default `wasm_memory_limit`) N of about 1,500 exceeds memory. The
on-chain outcome (out-of-memory trap, or, after an await, heap growth past
`wasm_memory_limit` as in the known L3 memory finding) was not run on a live canister.

## Unverified note (not a finding)

JS `icp.call` / `icp.candid` copy the canister and method strings (`arg_string`,
`js.rs:195-200`) into up to 64 pending requests per round before `MAX_METHOD_LEN` and the
80 B principal echo apply, at dispatch. Count is bounded (64), so this is a size check after
the copy, not this pattern. Not measured.
