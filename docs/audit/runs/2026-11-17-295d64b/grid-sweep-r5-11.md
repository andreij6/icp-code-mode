# Variant sweep r5-11

Pattern: "Per-run cost or limit accounting done on a process-wide resource (thread-local GC
heap, global GC threshold) that concurrent or earlier runs also occupy." Stated generally: a
number that is meant to describe one run (its instruction budget, memory limit, audit
`instructions`, call cap, log cap) is read from a counter or structure that other runs, or
process-wide one-time work, also move.

Method: code reading at `295d64b` (C = code path). For every place a per-run limit or audit
figure is computed, I traced which counter it reads and who else moves that counter:
`grep` for `instruction_counter`, `performance_counter`, `call_context`, `LIVE_MEMORY`,
`BASELINE`, `force_collect`, `thread_local!`, `static` over `src/`, `shims/`, `demo_store/src/`;
and for process-wide statics (`thread_local!`, `static` atomics/`LazyLock`/`OnceLock`/
`OnceCell`/`Mutex`) in the shipped engines and libraries: `boa_engine`/`boa_gc`/`boa_string`
0.22.0, `monty`/`monty-types` 1.6.0, `rhai` 1.27.0, `candid` 0.10.40, `candid_parser` 0.4.1,
`ic-cdk-executor` 2.0.0. Nothing was built or run (no step needed it).

Result: **no new candidate.** Known hits: the Boa GC finding (L8-1, threshold and
`force_collect`), the Python reply credit (L8-2), the shared 40B per-message limit for a batch,
the process-wide run-slot pool, the Symbol.for registry, the getrandom stream position, the
shared negative interface cache. Everything else either reads a per-run or per-call-context
counter that only this run moves, or is a one-time, bounded process-wide cost.

## Counters behind per-run limits and audit figures

| Row | Per-run figure | Counter / structure it reads | Who else moves it | Verdict |
|---|---|---|---|---|
| LIM2 instruction budget, JS (`check_budget` `js.rs:388`, `BoundedJobs::tick` `js.rs:115`, `budget()` in host fns) | `deadline = base + max` | `call_context_instruction_counter` (`bridge.rs:128-130`) | Only this HTTP request's call context. Batch items run strictly in sequence (`mcp.rs:386-402`, `exec(..).await` per item), and `base_instructions` is taken when each item's `IcBridge` is created (`bridge.rs:318`) | holds for this pattern, except work done inside the run for other runs: Boa GC = **known** (L8-1) |
| LIM2, Rhai (`on_progress` -> `budget_error(deadline)` `rhai_rt.rs:146-155`) | same deadline | same counter | same | holds (C). No shared GC: Rhai values are refcounted per run; `Engine` and `StandardPackage` are built per run (`rhai_rt.rs:118-120`) |
| LIM2/LIM26, Python (`vm_budget` `python.rs:37-43`, Monty clock = web-time `Instant`) | feed/turn budget | web-time shim `Instant::now()` = `performance_counter(1)` (`shims/web-time/src/lib.rs:15-18`), i.e. the same call-context counter; Monty only measures its running windows | Nothing else runs inside a Monty running window (single thread, synchronous) | holds for this pattern; one-time budget capture and turn-cap resets are **known** |
| LIM2, search (`run_search` `lib.rs:153-156`) | `instructions_used()` vs `max_instructions` | call-context counter from the search's own `IcBridge` | Sequential with the other batch items | holds (C) |
| Audit `instructions` (scripts `lib.rs:229`, searches `lib.rs:180`) | `instructions_used()` | same | JS: includes `force_collect` over other runs' objects = **known** (L8-1); Trapped entry = **known** | otherwise holds |
| LIM30 40B per message | per tool call budgets | IC per-execution counter | other items of the same HTTP request | **known** ("each tool call has its own instruction budget, but ... share one message execution") |
| LIM26 Python 64 MiB (`probe_memory` = `LIVE_MEMORY - BASELINE_MEMORY`, `monty-types resource.rs:825-829`) | live bytes | process-wide `LIVE_MEMORY` fed by the counting allocator (`lib.rs:357-425`) | Every allocation of every run and of the host. `rebased` (`python.rs:151-157`) re-bases around every await; nothing else executes inside a Monty synchronous window; `allocate_into_baseline` is not called anywhere in monty 1.6.0 | holds for other runs' allocations (register #43 fix); own reply excluded = **known** (L8-2) |
| LIM1 call cap | `slots` per `IcBridge` | per-run `Cell` (`bridge.rs:311-360`) | none | holds (C); cache-state dependence = **known** (concurrent cache misses / fetch slot) |
| LIM12 run slots | `ACTIVE_RUNS` | process-wide by design | every caller | **known** (one token holder can fill the pool; slots held during unbounded waits) |
| LIM14, LIM15, LIM16, LIM25, LIM27, Rhai `MAX_REQUESTS` | violations, log bytes, value budgets, promise jobs, rounds, replays | per-run structs (`IcBridge`, JS `State`/`BoundedJobs`, Rhai `RunState`, `ValueBudget` per conversion, Monty `PrintWriter` limit) | none | holds (C) |
| LIM24 Boa loop/recursion/stack limits | per run | Boa `RuntimeLimits` and VM stack per `Context` | none; per-frame loop counting is **known** | holds for this pattern |
| LIM20/LIM21 decode quota/depth | per decode | candid `DecoderConfig` per call | none | holds (C) |
| LIM18 / V2 / V4 / V5 interface caches | cost of fetch + parse charged to whichever run misses | process-wide caches | earlier runs warm them | holds for this pattern: the run that misses really does the work (not work done for another run). Outcome depending on cache state is the known call-cap/no-single-flight finding; negative cache shared across callers is **known** |

## Process-wide state in the engines and libraries (V7-V10)

| Row | Item | Can it charge one run for others, or carry a per-run limit over? | Verdict |
|---|---|---|---|
| V8 | `boa_gc` `BOA_GC` heap and threshold (`boa_gc-0.22.0/src/lib.rs:44-45`) | yes | **known** (L8-1, L8-6) |
| V10 | Boa `GLOBAL_SYMBOL_REGISTRY` (`builtins/symbol/mod.rs:41`) | grows across runs | **known** (Symbol.for registry) |
| V10 | Boa `FutexWaiters` static (`builtins/atomics/futex.rs:253`) | would hold waiters across runs | holds: unreachable, `Atomics` and `SharedArrayBuffer` are deleted from the global (`js.rs:314`, test `js.rs:428`); no other path reaches `Atomics` |
| V10 | Boa `SYMBOL_HASH_COUNT` atomic, `CODEBLOCK_ID_COUNTER`, `CANNOT_BLOCK_COUNTER`, `RecursionLimiter::SEEN` (Debug fmt only) | ids/counters, no limit is computed from them | holds (no accounting reads them) |
| V10 | Boa `RAW_STATICS_CACHE` (`boa_string common.rs:231`), Monty `CORE_ENTRIES` (`intern.rs:2823`), `INT_MAX_STR_DIGITS_THRESHOLD` (`long_int.rs:60`), `DEFAULT_OS_POLICY`, `RESERVED_STRING_HASHES` (129 atomics), Rhai `HASHING_SEED`/`INDEXER_HASHES` | built lazily by the first run that needs them, so that run's instructions (and, for Monty, its 64 MiB count) include a one-time cost later runs skip | holds (low, no failure): fixed size, built from constants, once per install/upgrade; far below the 20B budget and 64 MiB. Not a candidate under the evidence rule |
| V10 | candid `ENV`/`ID`/`NAME` thread-locals (`types/internal.rs:747-752`) | used by typed `CandidType` encoding; the app decodes/encodes untyped `IDLArgs` against parsed `TypeEnv`s | holds (no per-run figure depends on them) |
| V9 | `ic-cdk-executor` `TASKS`/`PROTECTED_WAKEUPS`/`MIGRATORY_WAKEUPS` | could poll another task inside this run's execution | holds: the app spawns no tasks (`grep spawn` empty in `src/`); each HTTP request is one protected task, woken only by its own call futures, so the call-context counter of a run never includes another request's polling |
| V7 | getrandom `STATE` (`lib.rs:437`) | shared stream | **known** (position carries over between runs); no limit reads it |
| V6 | Monty `LIVE_MEMORY` / `BASELINE_MEMORY` | see LIM26 above | holds / **known** L8-2 |
| V1, V3 | `ACTIVE_RUNS`, `PENDING_RUNS` | process-wide by design | **known** findings only |

## Other inventory sections

| Section | Rows | Verdict |
|---|---|---|
| Components | C1, C1a | covered above; C2 demo_store is synchronous (no awaits), so its stock checks see no concurrent run (duplicate-line oversell is **known**); C3 shim covered (LIM2 Python); C4-C15 compute no per-run limit: n/a |
| Actors | A1-A12 | n/a: the pattern is about accounting, not who acts; the shared-principal and shared-pool cases are **known** |
| Entry points | E1-E3, E6-E15, E20-E28 | n/a: no per-run limit or cost figure. E4 query search: no budget at all (**known**); E5/E5a/E5b/E5c/E5d covered by the counter table; E16-E19 host functions charge per-run structs only |
| Untrusted inputs | U1-U16 | n/a: inputs, not accounting |
| Outbound effects | O1-O2 per-run cap (holds), O3 audit `instructions` (covered), O4-O10 n/a |
| Durable state | D1-D10 | n/a: no per-run accounting reads stable state; audit growth is **known** |
| Commit points | K1-K12 | K7 Trapped instructions **known**; others n/a |
| Limits | LIM1-LIM32 | covered above; LIM3-LIM11, LIM13, LIM17, LIM19, LIM22, LIM23, LIM28, LIM31, LIM32 are per-request/per-value constants with no shared counter: holds; LIM29 (3 GiB) has no per-run share for JS/Rhai: **known** (#24, Rhai size-limit finding); Wasm memory never shrinking is **known** (L8-6, parser high-water mark) |
| Unbounded work | W1-W16 | n/a for this pattern (work size, not accounting); W3 search cost **known** |
| Failure sites | F1-F16 | n/a |
| Boundary conversions | B1-B17 | n/a: each conversion has its own `ValueBudget` |
| Value flows | VF1-VF8 | VF1/VF5 cycles: no per-run cycles accounting exists at all (audit records instructions only); cycles reservation is **known** |
| Admin surface | AD1-AD9 | n/a; limits captured at bridge creation is **known** |
| Supply chain | S1-S10 | n/a |
| Claims | CL7, CL8, CL31 | CL7/CL8 known; CL31 known; others n/a |
| Tests | T9 `memory_limit_survives_interleaved_runs` | known to not interleave; n/a here |
