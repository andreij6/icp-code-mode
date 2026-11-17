# Variant sweep r3-8: one limit, measured in different units by sibling implementations

Run `2026-11-17-295d64b`, Phase 6, round 3. Pattern:

> The same limit measured in different units by sibling implementations.

Seeds (known, not re-reported): JS counts value text in UTF-16 units while Python and Rhai
count UTF-8 bytes; Boa's 1M loop limit counts per call frame; Rhai's size limits are summed
over the whole nested value; the log cap counts line bytes and not lines; the call cap counts
slots, not messages; `DECODING_QUOTA` is scaled by 50 for untyped values; Python's clock is in
seconds while JS uses milliseconds.

Method: for every inventory row that holds or applies a limit, find each place that enforces
or depends on it and compare the unit each one counts: bytes or UTF-16 units, raw or
JSON-escaped bytes, JSON levels or Candid levels, per call frame, per replay, per Monty turn or
per IC message, slots or messages, elements or bytes. Reproductions were native
(`cargo test --lib`) in a private copy with the shared `CARGO_TARGET_DIR`. The copy was deleted
afterwards. Nothing was deployed and no live setting was changed.

Evidence keys: **R** = reproduced natively; **C** = the code path produces it unambiguously.

## New candidates

| Id | Severity | Candidate | Evidence |
|---|---|---|---|
| SW3-8-1 | Medium | `MAX_DEPTH` (128) counts Candid levels in `candid_bridge` and JSON levels in the runtimes. `opt` adds a Candid level but no JSON level (`json_to_idl` `src/candid_bridge.rs:231-233`, `idl_to_json` `:336`). So a recursive list (`type List = opt record { head; tail : List }`, or Motoko's `List` = `opt record { T; List }`) is refused from 65 elements (JSON depth 65), in both directions. The runtimes accept 128 levels, and README:369 promises 128. A reply is refused only after the call has run, with the misleading `reply nested deeper than 128 levels` | **R**: for n = 63 and 64, encode and decode succeed. For n = 65, 66 and 100, encode fails with `value nested too deeply` and decode fails with `reply nested deeper than 128 levels`. Same for the record list and the Motoko tuple list. JS returns a 100-node list as its result without error |
| SW3-8-2 | Medium (operator setting; merge candidate with L14-2) | The Python turn cap (`MAX_TURN_INSTRUCTIONS` 30e9, "below the IC's 40B limit per message, whatever the policy budget is", `src/runtimes/python.rs:29-31, 224`) counts per Monty turn. Monty resets the turn clock on every resume (`monty-1.6.0 run_progress.rs:444, 859`; `repl.rs:1368`). `drive` resumes without an IC await after `icp.principal()`, a `NameLookup`, an `OsCall` (`import icp`), a refused or cached request, and every queued `call_async` (`python.rs:248-273`). So one message can hold any number of turns. With `max_instructions` >= ~40e9 the feed budget no longer stops the run either, and `chunk; icp.principal(); chunk`, with each chunk 25e9, reaches the 40B trap with no audit entry. L14-2 recorded "Python unaffected (turn capped at 30e9)", and that is wrong | **R** (mechanism, wall-clock natively): turn limit 400 ms, 10 chunks of 133 ms. With no separator: `TimeoutError: turn time limit exceeded`. With `icp.principal()` or a caught `NameError` between chunks: `Ok` after 1.33 s / 1.29 s in one drive (9 synchronous host events). The on-chain trap was not run: it needs `set_limits` above 40e9, which only the Live phase may set |
| SW3-8-3 | Low | The log cap is 64 KiB of raw bytes in the runtimes (`js.rs:237`, `rhai_rt.rs:51`, Monty `io.rs:133`). `tool_result` cuts the same log text again, to 250,000 JSON-escaped bytes (`src/mcp.rs:165-166`). A control character costs 6 escaped bytes, so a log well inside the documented "Captured logs <= 64 KiB" is cut further, with `isError:false` | **R**: JS `console.log('\x01'.repeat(60000))` captures 60,000 bytes and the client gets 41,666 of them, followed by `…[truncated]` |

## Limits and their units, per sibling

| Row | Limit | Unit in each sibling | Verdict |
|---|---|---|---|
| LIM16 text 4 MiB (`ValueBudget::bytes`, `SNAPSHOT_JS`) | value text | JS: UTF-16 units for strings and keys. Python and Rhai: UTF-8 bytes | **known** (JS UTF-16) |
| LIM16 200k nodes | value elements | JS, Python and Rhai count one node per value. Python `bytes` and Rhai blobs count 1 node + N text bytes. A JS typed array is an object of index keys, so it costs N nodes plus the key text | Python bytes: **known** ("bytes elements not charged"). Rhai blobs are capped at 10,000 by `set_max_array_size`, so nothing beyond 10,000 is reachable: holds. JS typed array to object: L7 noted it holds (fails loudly for a blob) |
| LIM16 / LIM21 depth 128 | nesting | JS: `d >= 128` on JSON objects/arrays. Python/Rhai: `depth > 128`. `candid_bridge`: `depth > 128` on **Candid** levels, `opt` included | runtime off-by-one **known**; Candid levels vs JSON levels: **SW3-8-1** (R) |
| LIM15 log 64 KiB | captured output | JS: formatted line bytes, `[level] ` prefix included, separators not counted. Rhai: line bytes, `[debug pos] ` prefix included. Python (Monty): bytes of the whole buffer, `sep`/`end` newlines included, then split with `lines()`. `LogBuf`: line bytes | Python raises where the others truncate, and empty lines are free outside Python: **known** (both). Python counting `\n` is the same root cause as the known lines finding. Escaped re-cap: **SW3-8-3** (R) |
| LIM6 result 1 MB | tool result | `truncate_escaped` counts escaped bytes. The input is held to 4 MiB of text by the value budget | Cutting is **known** (L7-1). The units are documented ("escaped") |
| LIM6 logs block 250k | console block | escaped bytes, over a log capped in raw bytes | **SW3-8-3** |
| LIM7 reply 1.9 MB | HTTP body | Update path: sum of serialized replies (framing ignored). `finish`: body bytes. Query path: body only | **known** (framing) |
| LIM11 rpc error 300 B, LIM13 audit error 4096 B, LIM14 violation 512 B, LIM31 principal echo 80 B | echoed text | all count raw UTF-8 bytes through `bridge::truncate` | holds for this pattern. Echo-length differences are **known** |
| LIM2 `max_instructions` | instructions per run | JS and Rhai: call-context counter (`performance_counter(1)`) against `base + max`. Python: the remaining feed budget on Monty's clock (the same counter, paused during host work). Search: checked after render | Python's paused clock: **known** (SW1-1). Search: **known** |
| LIM30 IC 40B per message vs LIM26 Python turn 30e9 | instructions per message | IC: per message execution. Python: per Monty turn, reset at every resume, several per message | **SW3-8-2** (R mechanism) |
| LIM24 JS loop 1M | iterations | per call frame (Boa) | **known** |
| LIM27 Rhai 1M operations | operations | per replay. Each replay re-executes the same prefix, so the logical run is held to 1M | holds (L16 CL13 agrees). The O(n^2) replay cost is register-accepted |
| LIM1 call cap 25 | outbound | slots (JS/Python/Rhai reserve alike) vs messages (fetch = up to 2) | **known** |
| LIM25 JS 64/64, LIM26 Python 256 suspensions, LIM27 Rhai 64 requests | host requests | JS: per round and rounds. Python: host loop counts every progress event (`NameLookup`, `OsCall`, queued `call_async`), and Monty's stored limit is the same 256. Rhai: requests, refused ones included | documented as different. The missing per-batch cap in Python and the round off-by-one are **known**. Python's event count vs suspension count: both 256, and the host loop is the tighter one. Nothing beyond the call cap is reachable: holds |
| LIM9 5 searches / LIM8 50 messages / LIM10 1 script | per HTTP request | update path counts tool calls. Query path does not cap searches | **known** |
| LIM12 8 runs | concurrent | scripts only; searches not counted | **known** |
| LIM3 `call_timeout_seconds` | seconds | script calls: `change_timeout(secs)`. Fetches: ic-cdk default 300 s | **known** |
| LIM18 TTLs | ns | `ic_cdk::api::time()` ns on both paths (`failed_get`, `cache_get`) | holds |
| B14 script clocks | time | JS ms, Python s, Rhai none (`no_time`) | **known** (Python seconds) |
| LIM19 512 KiB | interface text | fetch: `did.len()` UTF-8 bytes after `from_utf8` / `candid::<String>`. Override: `d.len()` bytes (`src/lib.rs:326`) | holds (same unit). Cost vs bytes is **known** |
| LIM20 decoding quota | decode cost | candid scales untyped values x50 | **known** |
| LIM22 2^53 | JS-safe ints | `idl_to_json` and `normalize_numbers` <= 2^53-1. `num_text` accepts integral floats < 1e19. Python/Rhai results are not converted | **known** (L7-3, L7-17) |
| LIM23 100 KiB code | source | `code.len()` UTF-8 bytes in `LanguageRouter::execute`, the same for all three languages. The JS wrapper adds bytes after the check | holds |
| LIM17 method 256 B | method name | `method.len()` in the bridge. JS copies the name with `to_std_string_escaped` (a lone surrogate becomes 6 bytes) | holds; the surrogate text is **known** (L7-10) |
| LIM5 audit page 100 | entries | entries, not bytes | **known** |
| LIM29 memory | heap | Python: counting allocator, 64 MiB (replies not counted). JS: none. Rhai: engine size limits only | **known** |
| LIM32 search 40 methods | methods shown | the same `render` on the query and update paths | holds |
| LIM28 stack 8 MiB vs LIM24 Boa stack 10,240 | stack | Boa counts VM stack slots. The wasm stack counts bytes. Native overflows are **known** (parser, built-ins) | not a new case |
| CL33 / B17 demo comparison | "chars" | both sides use `.length` (UTF-16) of the same kinds of text (tool definitions, tool-call JSON, result text). Tokens = chars/4 on both sides | holds (C) |
| CL37 / C14 idea doc | cycles, MB | figures are **known** (misattributed); MB vs MiB used loosely, no limit derived from them | holds for this pattern |

## Other rows

| Rows | Verdict |
|---|---|
| C1-C15 | Only C1 enforces limits; covered above. C2 demo_store enforces none (stock checks are per line, **known**). C3 shim: one unit (instructions as ns), holds. C5/C12/C13 tooling and page: holds/n.a. |
| A1-A12 | n.a.: actors carry no limits |
| E1-E28 | Entry points route into the limits above. E3 `inspect_message` decodes without a quota (**known**). E20-E23 demo_store: no limits |
| U1-U16 | Bounded by the limits above. U5 `query`/`canister_id` unbounded (**known**) |
| O1-O10 | Outbound units covered under LIM1/LIM3 |
| D1-D10 | n.a. (no size limit on stored records apart from LIM13/LIM14, same unit) |
| V1-V11 | n.a.: caches keyed by canonical text; V6 Python memory **known** |
| K1-K12 | n.a.: commit points carry no limit |
| W1-W16 | Work bounded by limits above; unit mismatches only as listed |
| F1-F16 | n.a. |
| B1-B17 | B5/B6: SW3-8-1. B8-B10: value budget units above. B11: SW3-8-3 |
| VF1-VF8 | n.a. (cost accounting uses one unit: cycles charged by the platform) |
| AD1-AD9 | AD4 `max_instructions` > 40e9 is the precondition of SW3-8-2 (L14-2) |
| S1-S10 | n.a. |
| CL1-CL37 | CL10 (README:369, "128 levels ... in all three languages"): violated for `opt`-recursive types by SW3-8-1. CL12/CL24 Python budget: SW3-8-2. CL14 ("Captured logs <= 64 KiB"): SW3-8-3. Others as listed above |
| T1-T12 | No test covers any of the three candidates. T4 tests recursive types only to shallow depth |

## Not tested

- SW3-8-2 on chain. It needs `set_limits` with `max_instructions` >= 40e9, which this phase
  must not set. The mechanism was reproduced natively on Monty's wall clock with the same
  resume path as `drive`.
- SW3-8-1 against a live Motoko canister that returns a `List` of 65 or more elements. The
  decode path was reproduced natively with candid's own encoder building the reply bytes.
