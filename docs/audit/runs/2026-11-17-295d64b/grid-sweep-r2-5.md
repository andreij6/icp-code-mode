# Variant sweep r2-5

Pattern: "A limit on count only, where the size of each item is unbounded or configurable."

Question asked of every inventory row: does a limit here count items (nodes, elements,
entries, arguments, requests, methods, pages) while the size of each item is not counted,
or is counted on a path the item can avoid?

Method: code reading at `295d64b` plus native tests in a private copy
(`/private/tmp/claude-501/sweep-r2-5.*`, deleted afterwards; `CARGO_TARGET_DIR` the shared
audit target, debug test profile, the binary copied out right after each build because the
shared target directory is rebuilt by other agents). The tests called
`runtimes::{python,rhai_rt,js}::run` and `search::render` paths with the existing
`bridge::mock::MockBridge`; peak memory measured with `/usr/bin/time -l` (macOS, max RSS).
Nothing was deployed and no running instance was contacted.

Evidence key: **R** = reproduced natively; **C** = code path; **known** = already in the
list of known findings (not re-reported).

## New candidates

| Id | Severity | Title | Evidence |
|---|---|---|---|
| SW2-5a | High | Python values that are not `str` (big ints, exceptions, Decimal, any repr fallback) count as one node with no text bytes, so one shared value referenced N times leaves the script as N copies: register #21 regressed for Python | R |
| SW2-5b | High | Rhai's size limits count array elements, map entries and value-string bytes only: map keys and curried `FnPtr` contents are not counted, so a Rhai run's memory and nesting depth are unbounded (2 GB held, 18M elements under the 10,000-element limit, native stack overflow from a 3-line script) | R |
| SW2-5c | Medium | JS `console.log` / `print` applies the value budget per argument and formats every argument before the log cap: one call with N references to one 1 MB string makes the host allocate about 2N MB | R |
| SW2-5d | Low | `search::MAX_METHODS` caps the number of methods in a search result, not its size; the code calls searches "size-capped" | C |

### SW2-5a: Python repr fallbacks bypass `MAX_VALUE_BYTES`

`src/runtimes/python.rs:61-100` `to_json` calls `budget.node()` for every value but
`budget.bytes()` only for `str`, `bytes` and dict keys. Ints beyond i64 become
`Value::String(o.py_repr())` (`:71`), and every other type falls to
`_ => Value::String(o.py_repr())` (`:98`), with no byte charge. `monty_types` `py_repr` of a
`BigInt` is `write!(f, "{v}")` (`monty-types-1.6.0/src/object.rs:717`), with no 4300-digit
cap (that cap lives in the VM, not in the export); an `Exception` node carries its full
`arg: Option<String>` (`graph.rs:95-100`) and its repr includes it. The same `to_json` is
used for results and for `icp.call` arguments (`python.rs:244`).

| Script (Python, MockBridge) | Outcome |
|---|---|
| `x = str(10**4000)` then `[x] * 2000` (control) | refused: `value holds more than 4194304 bytes of text` |
| `x = 10**4000` then `[x] * 2000` | Ok, 8,008,001 bytes of JSON |
| `x = 10**200000` then `[x] * 50` | Ok, 10,000,201 bytes |
| `icp.call(L, 'icrc1_balance_of', {'owner': [10**4000] * 2000})` | call made with 8,008,011 bytes of arguments |
| `e = ValueError('x' * 1000000)` then `[e] * 20` | Ok, 20,000,341 bytes |
| same, `[e] * 500` | Ok, 500,008,501 bytes, 963 MB peak RSS, 258 s (debug); the VM held about 1 MB, inside its 64 MiB cap |

README.md:369 says every value leaving a script is limited to 4 MiB of text "in all three
languages". It is not for Python, and the amplification is the one register #21 was fixed
for: small in the VM, gigabytes once copied out. Not run on-chain. By the code path, about
3,000 references to a 1 MB exception take more than 3 GiB of host memory, which ends the
message in an out-of-memory trap (no audit entry if no message was sent yet; register #11).
Fix: charge `py_repr` text to `budget.bytes()` before building the string. For big ints,
check the digit count before formatting: `bits * log10(2)` gives an upper bound. Regression
test: add `[10**4000] * 2000` and `[ValueError('x' * 1000000)] * 20` to
`python::values_are_bounded`.

### SW2-5b: Rhai's size limits leave out map keys and `FnPtr` curry payloads

`rhai-1.27.0/src/eval/data_check.rs:19-101`: `calc_array_sizes` / `calc_map_sizes` count one
per element or entry and add `Str` value lengths. They never look at map keys
(`map.values()` only) and give 0 for `Union::FnPtr`, including its curried `Dynamic`s. The
engine is configured with only these limits (`rhai_rt.rs:122-131`): string 1 MiB, array
10,000, map 10,000, variables 1,000. README.md:451 says "Rhai [is limited] by its size
limits", and register #24 accepts the missing memory cap for JS only.

| Script (Rhai, MockBridge) | Outcome |
|---|---|
| `let m = #{}; let s = "k"; s.pad(1000000, 'k'); for i in 0..N { m[s + i] = 1; } m.len()`, N = 1 / 500 / 2000 | Ok; peak RSS 15 MB / 529 MB / 2,055 MB (about 6k operations for N = 2000) |
| `let big = []; for i in 0..9000 { big.push(i); } let f = Fn("x").curry(big); let a = []; for i in 0..2000 { a.push(f); } a.len()` | Ok, 2000: one array holding 18M elements under the 10,000-element limit; 446 MB RSS |
| `let f = Fn("x"); for i in 0..N { f = Fn("x").curry(take(f)); } 1`, thread stack 8 MiB | N = 1000 Ok; N = 20,000, 30,000 and 100,000: **`thread has overflowed its stack`, process abort** (recursive drop/clone of the nested `FnPtr`). N up to about 100,000 fits in the 1M-operation limit (125,000 hits `Too many operations`) |

The same values cannot leave the script: `to_json_value` counts map-key bytes, and
`rhai::serde::from_dynamic` refuses `FnPtr` ("Output type incorrect: Fn"). So the harm is
inside the run: memory beyond the 3 GiB Wasm limit (about 3,500 one-MB keys) and native
recursion. On-chain, both end in a trap, which leaves no audit entry before the first send
and a `Trapped` entry after it. Not run on-chain: wasm frame sizes differ from native debug,
so the overflow depth on wasm is unverified; the 100k depth reachable here is about 5x the
native-debug threshold. This also contradicts the "Rhai variant not reachable under default
limits" part of the known native-stack-overflow finding: the known analysis bounded Rhai
nesting through the array-size limit, which `FnPtr` nesting escapes. Fix: make
`to_json_value`, `print` and `debug` refuse `FnPtr`. For memory, give Rhai the same
counting-allocator cap as Python (the allocator already exists, `lib.rs:357-425`), or
disallow `curry` and cap total map-key bytes through an `on_var`/`on_def_var` check.
Regression test: the three scripts above must fail cleanly.

### SW2-5c: JS console formats all arguments before the cap

`src/runtimes/js.rs:227-248` `console_fn` checks `budget(st)` and `log_bytes >= MAX_LOG_BYTES`
once, then runs `display` on **every** argument and joins them before truncating to
the 64 KiB cap. `display` (`:217-225`) copies strings in full with `to_std_string_escaped`
(no snapshot, no 4 MiB check), and gives every other argument its own snapshot with a fresh
4 MiB budget. The argument count is bounded only by Boa's stack-size limit (10,240 values).

| Script (JS, MockBridge, debug) | Peak RSS | Log kept |
|---|---|---|
| `const s = 'x'.repeat(1e6); const a = Array(1).fill(s); console.log(...a)` plus the same with `[s,s,s,s]` elements | 57 MB | 21 bytes |
| same, 500 arguments | 4,918 MB (71 s) | 21 bytes |
| same, 2000 arguments | 15,946 MB (287 s) | 21 bytes |

The engine holds one 1 MB string and an array of references. The host makes the copies, so
this is the register #21 amplification on the print path, not the JS-heap hoarding that
register #24 accepts. README.md:369 lists "printed values" under the 4 MiB limit. On-chain
(not run), about 2,000 one-MB arguments pass 4 GiB and trap the message. If the call
happens in a response callback and stays under 4 GiB, it reaches the known "JS heap past
wasm_memory_limit after an await" state by a different route. That is not run: it would
be destructive and needs the Live phase. The 21-byte log kept is the known LogBuf marker
issue. Fix: format arguments one at a time against the remaining log bytes, stop at the cap,
and run strings through the same byte budget. Regression test: extend
`js::console_output_is_capped_while_running` with `console.log(...Array(500).fill(s))` and
assert the bytes allocated (counting allocator) stay near the cap.

### SW2-5d: search result size is count-capped only

`src/search.rs:17` `MAX_METHODS = 40` limits shown methods. Each shown method's `signature`
is the full one-line pretty-print of its (possibly inline) types, and every named type it
references is printed in `types` (`search.rs:118-139`). An interface is limited only to
512 KiB (`MAX_DID_BYTES`) and the number of allowlisted canisters is unlimited, so one
search can exceed `MAX_RESULT_BYTES` once two or more large interfaces are in scope. It is
then cut to invalid JSON (known L7-1), and five such searches overflow the batch reply
(known). `mcp.rs:390-391` exempts searches from the one-tool rule because they are
"size-capped", and the inventory lists LIM32 as protecting result size. Only the claim and
the count-only cap are new; the downstream effects are known. Fix: cap the rendered bytes
(stop adding methods and types at a byte budget, then report `omitted_methods`).

## Limits (LIM)

| Row | Count limit | Item size | Cell |
|---|---|---|---|
| LIM1 calls per script | 25 (configurable) | args 4 MiB each (value budget), reply <= 2 MiB | holds for the pattern: both dimensions bounded; prepayment held per call is known |
| LIM5 audit page 100 | entries | grows with `max_calls_per_script` | known |
| LIM8 batch 50 | messages | whole body bounded by ingress 2 MiB; replies by LIM7 | holds |
| LIM9 5 searches | searches | keywords unbounded | known; result size: **SW2-5d** |
| LIM10 one script | 1 | | n/a |
| LIM12 8 runs | runs | per-run memory: JS uncapped (#24 accepted), Python 64 MiB plus replies (known), Rhai uncapped (**SW2-5b**) | **SW2-5b** |
| LIM13 audit error 4096 B | | bytes | holds |
| LIM14 16 violations x 512 B | both | | holds (marker overrun known) |
| LIM15 log 64 KiB | bytes | formatting before cap: JS **SW2-5c**, Python known | **SW2-5c** |
| LIM16 200k nodes / 4 MiB | nodes and bytes | Python repr fallbacks counted as nodes only: **SW2-5a**. JS counts strings and keys; JS UTF-16 counting known. Rhai counts strings, blobs and keys; `FnPtr` refused by `from_dynamic` | **SW2-5a** |
| LIM17 method 256 B | | bytes | holds |
| LIM19 512 KiB interface | bytes per interface; count of interfaces not limited (allowlist) | | reverse pattern (size per item, no count); count is admin-sized, known |
| LIM20 decoding quota | cost | | known (400 KB effective) |
| LIM24 JS loop 1M per frame, recursion 256, stack 10,240 | counts | cost per iteration unbounded | known (synchronous JS budget findings) |
| LIM25 JS 64 rounds x 64 pending, 100k jobs | counts | each request <= 4 MiB args; budget checked every 256 jobs | 64 x 4 MiB = 256 MB of host copies per round from shared strings: holds for the per-request claim; inside the accepted JS-memory class (#24). Jobs: known |
| LIM26 Python 256 suspensions, recursion 200 | counts | batch per suspension unbounded | known (no per-batch cap) |
| LIM27 Rhai: strings 1 MiB, array 10,000, map 10,000, variables 1,000, functions 256, 64 requests, 1M ops | counts | keys and `FnPtr` uncounted (**SW2-5b**); variables x per-value size (1,000 x about 1 MiB) is the same class; 64 answers held as `serde_json::Value` for the whole run (W11) | **SW2-5b**; W11 memory unverified (see Not tested) |
| LIM31 truncate 80 B | | | known (echo finding) |
| LIM32 search 40 methods | methods | signature and type text unbounded | **SW2-5d** |
| LIM2-LIM4, LIM6, LIM7, LIM11, LIM18, LIM21-LIM23, LIM28-LIM30 | | | n/a (byte, time or depth limits, not counts) or known |

## Other inventories

| Rows | Cell | Reason |
|---|---|---|
| C1, C1a | covered above | Python-only builds have SW2-5a, Rhai-only SW2-5b, JS-only SW2-5c |
| C2 demo_store, E20-E25, W15 | known | Order count, zero-quantity lines and `list_orders` size are known; `OrderItem` is fixed-size |
| C3-C15 | n/a | No count limit over items of variable size |
| A1-A12 | covered through E rows | A3 is the actor of SW2-5a/b/c |
| E1, E2, E6-E11, AD1-AD9 | known | Admin inputs unbounded in count and size (known) |
| E3, E4, E4a, E4b | known / holds | Query-path 50 searches known; headers scanned in full, bounded by ingress |
| E5, E5a | **SW2-5a/b/c** | |
| E5b | **SW2-5d** | |
| E5c, E5d, E12-E15 | n/a or known | E13 page known; E12 `pending_run_ids` and allowlist are unbounded lists (known #31, admin) |
| E16 JS `icp.call` | holds | One snapshot per call, 4 MiB |
| E17 JS console | **SW2-5c** | |
| E18 Python host functions | **SW2-5a** | |
| E19 Rhai host functions | **SW2-5b** (in-engine); holds at the exit | `to_json_value` counts keys; `FnPtr` refused |
| E26-E28 | n/a | Client tooling |
| U1-U4, U12-U16 | holds / n/a | Ingress-bounded or trusted |
| U5 query, canister_id | known | |
| U6 | holds | 100 KiB |
| U7 | **SW2-5a**, **SW2-5c** | |
| U8, U9, U10 | known | Quota, echo and 512 KiB findings |
| U11 | known | |
| O1 | **SW2-5a** (8 MB of arguments sent to encoding) | The encoded call is then refused by the 2 MiB inter-canister limit, after the conversion work |
| O2-O10 | n/a | |
| D1-D3, D5-D10 | known / n/a | Admin-sized |
| D4 audit | known | `methods` grows with the configurable call cap (known); `canisters` likewise; `error` and `violations` byte-capped |
| V1, V3, V6-V11 | n/a or known | V3 unbounded count known (#31) |
| V2, V4, V5 | reverse pattern | Size per entry bounded (512 KiB, error text), count admin-bounded |
| K1-K12 | n/a | No count limits |
| W1, W2, W4-W10, W12-W16 | known or n/a | W3 known; W11 see Not tested |
| W11 | unverified | Count 64, each answer is a decoded reply held as `serde_json::Value` for the whole run |
| F12, F13 | **SW2-5b** | New route to native recursion and out-of-memory in Rhai |
| F1-F11, F14-F16 | n/a | |
| B9 | **SW2-5a** | |
| B10 | **SW2-5b** (in-engine only) | |
| B8 | **SW2-5c** (console path) | |
| B1-B7, B11-B17 | n/a or known | B16 search rendering: **SW2-5d** |
| VF1, VF5 | **SW2-5a/b/c** | Host work and memory paid by the canister |
| VF2-VF4, VF6-VF8 | n/a | |
| S1-S10 | n/a | |
| CL10 values claim | **SW2-5a**, **SW2-5c** | |
| CL24 / README.md:451 Rhai memory claim | **SW2-5b** | |
| CL36 "size-capped" searches | **SW2-5d** | |
| Other CL rows | n/a or known | |
| T1-T12 | gap | `values_are_bounded` and `stringify_replacer_and_size_limits` test only `str` repetition; no test for Rhai `FnPtr` or map keys; the console test uses one argument per call |
| History | family re-checked | Register #21 (fixed): still holds for JS values and Rhai strings; regressed for Python non-`str` values (SW2-5a) and the JS console path (SW2-5c) |

## Refuted while sweeping

- Rhai `FnPtr` with a 1 MB name, referenced 10 times, as a result or call argument:
  `rhai::serde::from_dynamic::<Value>` refuses `FnPtr` ("Output type incorrect: Fn"), so it
  cannot carry the bypass out of the script. Only the in-engine symptoms in SW2-5b remain.
- JS object keys: `SNAPSHOT_JS` adds `k.length` to `bytes`, so repeated long keys are charged.

## Not tested

- Every on-chain consequence: the out-of-memory trap for SW2-5a/b/c, the wasm stack depth
  for SW2-5b, and the heap-past-`wasm_memory_limit` route for SW2-5c. They are destructive
  and belong to the Live phase.
- W11 Rhai `answers`: the actual `serde_json::Value` size of a reply near the decoding quota
  was not measured, so whether 8 runs x 25 answers can approach 3 GiB at default limits is
  unverified.
- SW2-5d was not run: it needs two allowlisted interfaces near 512 KiB.
