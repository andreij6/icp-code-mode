# Variant sweep r3-7: budget checked between interpreter steps, not inside one native operation

Run `2026-11-17-295d64b`, Phase 6, round 3. Pattern under test:

> A budget enforced between interpreter steps but not inside one uninterruptible native
> operation inside a dependency.

"Budget" covers every per-run limit the app relies on: the policy instruction budget
(`max_instructions`), engine step limits (Rhai operations, Boa loop limit, Monty VM budget),
and engine size limits (Rhai string/array/map size, Monty 64 MiB). The question for each row:
is there a single call into a dependency, reachable from script input or a target's reply,
whose cost or allocation is decided before the next check runs?

Known and not repeated (from the task's list): Boa built-ins and sync JS (register #23 and the
reported Boa items: repeat/pad/join capacity overflow, sort, `String.at`, compile quadratic,
parser depth, `Symbol.for`, console formatting), every Monty native operation in grid-L3-r3
(`**`, gcd/lcm, isqrt, `*`, `//`, 3-arg pow, decimal sqrt, regex, NFKC, zfill, replace, hex,
list search), Monty `eval`/`exec` compile, `json.dumps(indent)`, `re.compile`, Python
`to_json`/`py_repr`, candid_parser, `json_to_idl`, candid encode depth^2 and `from_radix`,
candid decode gamma clone, search rendering, Rhai sort/sleep/size-limit gaps.

## How it was tested

- **Native.** Private copy of the tree at
  `/private/tmp/claude-501/sweep-r3-7.LvFO/repo` (no `target/`, no `design/`), build dir
  `CARGO_TARGET_DIR=/private/tmp/claude-501/audit-target`, release profile, one feature at a
  time. Throwaway examples (not in the repository): `examples/sweep_r3_7.rs` (Rhai engine
  configured exactly as `rhai_rt::build_engine`), `sweep_r3_7_js.rs` (Boa `JsValue::from_json`),
  `sweep_r3_7_py.rs` (Monty `MontyRun::new` compile), `sweep_r3_7_nat.rs`
  (`candid_bridge::Service::decode_reply` on large LEB128 `nat` replies).
- **Live.** The project's local network (port 8010), canister `icp_code_mode`
  (`4caro-hl777-77775-aaaba-cai`), default limits, nothing deployed or changed. Each probe is
  one `execute_icp_script`; the cost is the `instructions` field of its audit entry, matched by
  `script_sha256`. Helper `scratchpad/r37/probe.py` (token read from `.env`, never printed).
  Traps happen before any outbound message, so they roll back and write no entry
  (register #11). Canister memory after the probes: 380,391,208 bytes, status Running.

## Results

### Rhai compile (`engine.compile` in `rhai_rt::run`, `src/runtimes/rhai_rt.rs:187`)

Runs once per run, before the first `on_progress` callback. Native cost of 102 KB sources
(under `MAX_CODE_BYTES` = 102,400):

| Source shape | Optimizer on (as shipped) | `OptimizationLevel::None` | Cause |
|---|---|---|---|
| `let a = 1;{}{}{}...` (empty blocks) | 0.11 s / 0.45 s / 1.76 s / 7.45 s at 12.5 / 25 / 50 / 102 KB | 6.8 ms at 102 KB | optimizer: pure statements removed one at a time with `Vec::remove(index)` (`rhai optimizer.rs:245-268`) |
| `x;x;x;...` / `1;1;1;...` | 0.31 s at 102 KB | 12 ms | same loop |
| `let f = \|\| { b0;b1;...;bN }; 1` (distinct captured names) | 1.69 s at 102 KB | 1.74 s | parser: each new name scans `external_vars` (`rhai parser.rs:548-549`) |
| `let v0=1;let v1=1;...` | 0.43 s | 0.46 s | parser variable stack |

Live (wasm, audit `instructions`; no outbound call, so this is compile plus a trivial run):

| Script | Bytes | Instructions | Outcome |
|---|---|---|---|
| empty blocks | 12,500 | 2,805,398,460 (entry 9878) | Ok |
| empty blocks | 25,000 | 11,111,435,992 (9879) | Ok |
| empty blocks | 37,000 | 24,267,157,933 (9880) | `policy: instruction budget exceeded (24267157933 > 20000000000)`, reported after the fact |
| empty blocks | 60,000 | | HTTP 503 `Canister exceeded the limit of 40000000000 instructions for single message execution`, no entry |
| closure with captured names | 50,000 | 27,905,427,073 (9881) | budget exceeded, after the fact |
| closure with captured names | 100,028 | | HTTP 503, 40e9 limit, no entry |
| `let v0=1;...` | 102,000 | 16,407,038,034 (9882) | `Too many variables defined` raised at run time, after compile was paid |

Cost is quadratic: twice the source, about 4x the instructions (2.81e9 to 11.1e9).

### Rhai `parse_json` (`rhai packages/lang_core.rs:180-186` -> `Engine::parse_json`, `api/json.rs:68-131`)

A script-callable native function that runs the Rhai parser on a runtime string of up to
1 MiB (the string limit), with closures allowed, in one call. Same parser cost as above, but
the source is built at run time, so it is not bounded by `MAX_CODE_BYTES`:

| Script (9 lines, builds `[|| [[b0,b1,...]]]` in a loop) | Text | Rhai operations | Outcome |
|---|---|---|---|
| 4,000 names | 22,900 B | ~24k | 5,997,200,229 instructions (entry 9896), error `Variable not found: b0` |
| 12,000 names | ~75 KB | ~72k | HTTP 503, 40e9 limit, no entry |
| 30,000 names | ~200 KB | ~180k | HTTP 503, 40e9 limit |

### Rhai string `replace` (`rhai packages/string_more.rs:1172-1239`, four overloads)

`*string = string.replace(find, sub).into()` has no size check; the 1 MiB `max_string_size`
is applied by `check_data_size` after the native call returns (`func/call.rs`,
`func/func_call.rs`). The result can be `len(s) / len(find) * len(sub)`, up to 1 MiB x 1 MiB.

- Native (`sweep_r3_7 replace <sub>`; `s` = 1 MiB of `a`, `t` = `sub` bytes): sub 64 / 256 /
  1024 -> max RSS 171 MB / 574 MB / 2.69 GB, then `Length of string too large`.
- Live, sub 64 (64 MiB result): 1,314,442,794 instructions (entry 9884), then the size error.
- Live, sub 2048 (2 GiB result), script
  `let s = ""; s.pad(1048576, "a"); let t = ""; t.pad(2048, "b"); s.replace("a", t); s.len()`:
  HTTP 503 `Canister called ic0.trap with message: 'Panicked at 'capacity overflow',
  .../alloc/src/raw_vec/mod.rs:29:5'`, no entry. On wasm32 a `String` over `isize::MAX`
  (2 GiB) panics, so the trap does not depend on the memory limit.
- Python for comparison: `('a'*1048576).replace('a', 'b'*4096)` -> `MemoryError: memory limit
  exceeded: 4294967295 bytes > 67108864 bytes` in 0.2 s (Monty prechecks). Holds.

### Checked and holding

| Site | Result |
|---|---|
| Boa `JsValue::from_json` of replies (`js.rs:403`) | linear: 10k/20k/40k/80k-key object 1.9/3.8/7.5/18.5 ms native |
| Monty compile of the 100 KiB top-level script (`python.rs:229`) | linear for 8 shapes (globals, locals, list/str/int literals, nested ifs, lambdas with free names, defs): 5-15 ms at 100 KB native |
| `decode_reply` + `idl_to_json` of a big `nat` (`candid_bridge.rs:96-110, 357`) | candid decodes LEB128 in one linear pass (`types/number.rs:232-275`); `to_str_radix(10)` subquadratic: 12.5/25/50/100/200 KB -> 1.9/3.9/10/25/63 ms native; 400 KB refused by `DECODING_QUOTA` |
| Python `from_json` (`python.rs:105-117`) | one pass over the value; reply size bounded by the decoding quota |
| Rhai `to_dynamic` of cached answers per replay (`rhai_rt.rs:73`) | one conversion per native call, each bounded by the decoded reply; checked by `on_progress` before the next operation |
| Rhai `pad` (string, string-with-string, array, blob) | size prechecked before allocating (`string_more.rs:1298`, `array_basic.rs:265-276`) |
| Rhai `+` on strings/arrays/maps | built-in checks the combined size before building (`func/builtin.rs:943`) or after a bounded (2x) result |

## Grid

| Row | Cell | Reason |
|---|---|---|
| C1 icp_code_mode | finding SW37-1, SW37-2 | Rhai engine paths below |
| C1a feature subsets | finding (rhai subsets) | Both findings ship in every build with `rhai`; `js`/`python`-only builds n/a |
| C2 demo_store | n/a | No budget of its own; its unbounded work is known (W15) |
| C3 web-time shim | holds | It is the Monty budget clock; Monty native ops are known (L3r3-1/-2) |
| C4 vendored chrono | n/a | No script-reachable long native op (datetime only from the fixed clock) |
| C5 demo page, C6, C7-C10, C12-C15 | n/a | No per-run budget on those surfaces (page regex is known) |
| C11 .did files | n/a | Interface text, no runtime |
| A1-A2, A5, A7-A12 | n/a | Not a source of script or reply input to an engine (A5 override parse is known) |
| A3 token holder | finding SW37-1, SW37-2 | Reaches both with one Rhai script |
| A4 AI model | n/a | Writes the scripts; same input as A3 |
| A6 allowlisted target | holds | Reply conversions checked above are linear; decode-side hostility is known |
| E1-E3, E6-E15 | n/a | No interpreter budget on these entry points (E3 decode, E11 parse known) |
| E4 / E4a / E4b query | n/a | No engine runs in the query; search rendering is known |
| E5, E5a | finding SW37-1, SW37-2 | `run_script` -> `rhai_rt::run` |
| E5b search | n/a | No interpreter; known items cover its native work |
| E5c / E5d callbacks | finding (SW37-2 after an await, code path) | A `replace` in a replay after a call traps in a callback: `Trapped` entry, reply lost. Not run live |
| E16 JS host functions | holds | Value budget before conversion; `from_json` linear (measured) |
| E17 console | n/a | Known (formatting before cap) |
| E18 Python host functions | holds | `from_json` linear; `to_json` known |
| E19 Rhai host functions | holds | `call_canister` converts args under `ValueBudget`; `request` compares bounded JSON |
| E19 Rhai standard package (script-callable natives) | **finding SW37-1** (`parse_json`), **SW37-2** (`replace`) | Swept `string_more`, `array_basic`, `blob_basic`, `map_basic`, `lang_core`; `pad`, `+`, `split`, `to_chars`, `sub_string`, `crop`, `to_upper` hold (prechecked or bounded at about 3x) |
| E20-E25 demo_store | n/a | No interpreter |
| E26-E28 | n/a | Off-chain tooling |
| U1-U4 | holds | `parse_body` serde_json is linear in the 2 MiB body, depth 128 |
| U5 / U6 script source | **finding SW37-1** | Rhai compile quadratic in source shape, no budget until the first operation |
| U7 script values | finding SW37-2 | One `replace` produces up to 1 TiB before the 1 MiB check |
| U8 reply bytes | holds | Large `nat`/`int` decode linear, `to_str_radix` subquadratic up to the decoding quota |
| U9 reject text | n/a | Copied once, truncated |
| U10 / U11 interface and admin text | n/a | Known (candid_parser) |
| U12 | n/a | Known (inspect_message decode) |
| U13, U14, U16 | n/a | No interpreter budget |
| U15 platform values | n/a | Trusted |
| O1 / O2 outbound | finding (indirect) | A compile or `replace` trap after a send closes the run as `Trapped` and loses the reply |
| O3 audit writes | finding (indirect) | Traps before any send leave no entry (register #11), so these runs are invisible in the log |
| O4-O10 | n/a | Not script-driven |
| D1-D10 | n/a | No durable state written by the native ops; the trap rolls back |
| V1 run slots | holds | A trap in the first execution rolls back the slot; in a callback cleanup returns it |
| V2-V5 caches | n/a | Not touched by the native ops |
| V6 Monty memory atomics | n/a | Python only; replace prechecked (above) |
| V7 getrandom | n/a | |
| V8 Boa GC, V9 executor | n/a | |
| V10 engine statics | holds | Rhai's engine is built per run; nothing persists |
| V11 | n/a | |
| K1-K5 commit points | finding (indirect) | SW37-2 inside a replay after an await traps in a callback (code path; known `Trapped` mechanism) |
| K6-K7 | holds | Cleanup unaffected by these traps |
| K8 trap before first send | **finding SW37-1/2** | Both trap with no entry and no outbound effect; cost borne by the canister |
| K9-K12 | n/a | |
| LIM1 call cap | n/a | No calls needed |
| LIM2 `max_instructions` | **finding SW37-1** | Checked on Rhai operations only; compile and `parse_json` are not operations: 24.3e9 and 27.9e9 reported against 20e9, larger inputs trap |
| LIM3-LIM14 | n/a | Not budgets on engine work |
| LIM15 log cap | n/a | Known (Rhai `print` formats a bounded value) |
| LIM16 value limits | holds | Applied before conversion in Rhai (`ValueBudget`) |
| LIM17-LIM22 | n/a | Not engine budgets (LIM20 decode quota: holds for big `nat`, above) |
| LIM23 `MAX_CODE_BYTES` | **finding SW37-1** | 100 KiB is ~2.5x above the size at which a Rhai source traps (60 KB); it was sized for linear parse cost. `parse_json` bypasses it entirely |
| LIM24 / LIM25 JS | n/a | Known (#23 and reported Boa items) |
| LIM26 Python | holds for compile (linear) and `str.replace` (precheck); other Monty ops known |
| LIM27 Rhai 1M operations, 1 MiB string | **finding SW37-1, SW37-2** | Operation count does not see compile or `parse_json`; string limit is applied after `replace` returns |
| LIM28 wasm stack | n/a | Parser recursion bounded by Rhai expr depth (64/32) |
| LIM29 memory limit | finding SW37-2 | Not reached: the trap is `capacity overflow` at 2 GiB (wasm32 `isize::MAX`) |
| LIM30 40e9 | **finding SW37-1, SW37-2** | Reached by compile, `parse_json`; `replace` panics first |
| LIM31, LIM32 | n/a | |
| W1, W2 | holds | Linear |
| W3, W4 | n/a | Known / admin-sized |
| W5 | n/a | Known |
| W6 | holds for big integers (above); other decode cost known |
| W7 | n/a | Known |
| W8 JS parse/compile, conversions | holds for `from_json`; compile known |
| W9 | n/a | |
| W10 Python parse/compile | holds | Linear (above); `eval`/`exec` known |
| W11 Rhai replays | **finding SW37-1** (compile), holds for replays | Compile is once per run but unbudgeted; each replay's conversions are bounded and checked between operations |
| W12-W16 | n/a | Not engine work, or known |
| F1-F11 | n/a | Not reachable from these ops |
| F12 stack overflow | holds for Rhai | expr-depth limits apply to `parse_json` too |
| F13 out of memory | **finding SW37-2** | Rhai string growth inside `replace` is the uncapped path (panics at 2 GiB) |
| F14 40e9 | **finding SW37-1** | New trigger: Rhai compile and `parse_json` |
| F15, F16 | n/a | |
| B1-B4 | holds | Linear |
| B5 | n/a | Known |
| B6 | holds for `Nat`/`Int` -> decimal (measured) |
| B7 | n/a | Known |
| B8 | holds (`from_json`) |
| B9 | holds (`from_json`); `to_json` known |
| B10 | holds | `to_dynamic`/`from_dynamic` per value, bounded |
| B11-B17 | n/a | Not engine work |
| VF1 cycles | finding | Each trap burns 40e9 instructions at the canister's cost with no audit entry |
| VF5 work per run | **finding SW37-1** | Policy budget overrun up to the 40e9 limit |
| VF2-VF4, VF6-VF8 | n/a | |
| AD1-AD9 | n/a | No setting bounds compile or `replace`; raising `max_instructions` does not help |
| S2 rhai 1.27.0 | **finding SW37-1, SW37-2** | Both are dependency behaviour the app's limits do not reach |
| S1, S3-S10 | n/a | |
| CL8 "Checked ... on every Rhai operation" | finding SW37-1 | True as written, but compile and `parse_json` are not operations, and the budget is overrun |
| CL13 "Rhai: 1M operations per run ... eval and modules disabled" | finding SW37-1 | `parse_json` is a second parser entry point at run time that the `eval` removal does not cover |
| CL10 value limits | holds | Not about engine-internal values |
| CL24 known limitations | finding | Lists one Boa built-in hitting 40e9 (#23); says nothing about Rhai |
| CL34 stack 8 MiB | n/a | |
| Other CL rows | n/a | Not about engine budgets |
| T7 / T10 tests | finding (gap) | `limits_and_sandbox` covers loops, recursion, string size by `+`; nothing covers compile cost, `parse_json`, or `replace` growth. Native tests cannot count wasm instructions (register #58 class) |
| History #23 | n/a | JS only |
| History #13, #15 (Rhai) | holds | Replay cap and nested-array cost still bounded |

## Candidates

### SW37-1 (High) Rhai compile and `parse_json` cost is quadratic and outside every budget

- **Where.** `src/runtimes/rhai_rt.rs:187` (`engine.compile(code)` before any `on_progress`),
  `build_engine` registers `StandardPackage`, which includes `parse_json`
  (`rhai-1.27.0/src/packages/lang_core.rs:180`). Quadratic loops: optimizer
  `optimizer.rs:245-268` (`statements.remove(index)` per pure statement), parser
  `parser.rs:548-549` (linear `external_vars` scan per captured name), parser variable stack.
- **Repro (live).** Rhai script `"let a = 1;" + "{}" * 13495` (37,000 B): `policy: instruction
  budget exceeded (24267157933 > 20000000000)`. 60,000 B: HTTP 503, 40e9 limit, no audit entry.
  `let f = || { b0;b1;...; }; 1` at 100,028 B: HTTP 503. At run time, 9 lines that build
  `[|| [[b0,...,b11999]]]` and call `parse_json`: HTTP 503 after ~72k operations.
- **Fix.** Compile with `OptimizationLevel::None` (102 KB `{}` script: 7.45 s -> 6.8 ms
  native), replace `parse_json` with a host function that parses with `serde_json` under the
  value budget, and bound the parser's remaining quadratic cases (a lower Rhai source limit or
  a pre-scan of distinct identifiers). Regression: a wasm-level test (PocketIC) that runs the
  three scripts and asserts a clean error under `max_instructions`.

### SW37-2 (High) Rhai string `replace` builds its result before the 1 MiB size check; one call traps

- **Where.** `rhai-1.27.0/src/packages/string_more.rs:1172-1239` (four `replace` overloads,
  no size check); the engine's `check_data_size` runs only after the call returns.
- **Repro (live).** `let s = ""; s.pad(1048576, "a"); let t = ""; t.pad(2048, "b");
  s.replace("a", t); s.len()` -> HTTP 503 `Panicked at 'capacity overflow'`, no entry. With a
  64-byte `t`: 1.31e9 instructions and a 64 MiB string before `Length of string too large`.
  Native: 2.69 GB resident for a 1 KiB `t` before the same error. Python's `str.replace` with
  the same sizes raises `MemoryError` (precheck).
- **Not tested.** The same call in a replay after an outbound call (a callback, where the Wasm
  memory limit is not applied to responses per the known JS heap finding): code path only;
  not run, because it could leave the shared canister above its memory limit.
- **Fix.** Register replacement `replace` overloads (they take precedence over the package)
  that compute `matches x (sub_len - find_len) + len` with checked arithmetic and refuse above
  `max_string_size` before allocating. Regression: a native Rhai test asserting the size error
  for 1 MiB x 2 KiB, plus a counting-allocator assertion that peak allocation stays near 2 MiB.

## Not tested

- Every Rhai parser shape. Four were measured; the fix above (optimizer off) removes the
  worst, the parser cases remain.
- JS `String.prototype.replaceAll` blow-up: Boa capacity-overflow family, already reported.
- Exact native-to-wasm instruction ratio (about 25e9 instructions per native second for the
  Rhai optimizer, from 12.5 KB: 0.11 s vs 2.81e9).
- Mainnet cycle cost of each 40e9 trap.
