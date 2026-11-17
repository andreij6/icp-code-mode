# Coverage grid: L11 Sandbox and determinism, round 4 (memory-corruption oracle)

Run `2026-11-17-295d64b`. Question: rounds 1-3 and the Testability-r3 campaign (220k cases)
looked only for traps. Wasm has no guard pages, so corruption of the single shared heap
(other runs' suspended VMs, `IcBridge` state, `DID_CACHE`, `ACTIVE_RUNS`) would not trap.
Does any input corrupt memory silently, and can one run influence another?

Evidence key: **R** reproduced, **E** reproduced on the deployed bytes in the Node `ic0`
emulator, **L** reproduced on the local replica (port 8010), **C** code path only.

## Method

1. **Sanitizer (native).** Private copy (`rsync` without `target/`, `docs/`, `design/`,
   `video/`, `.env`, `.icp`) in `/private/tmp/claude-501/l11r4.*`, shared `CARGO_TARGET_DIR`,
   probe module `l11r4_probe` appended to the copy's `src/runtimes/mod.rs` only. Built with
   `cargo +nightly test --lib --target aarch64-apple-darwin` and
   `RUSTFLAGS=-Zsanitizer=address` (all crates instrumented: monty, monty-types, boa_engine,
   boa_gc, rhai, the app; std not rebuilt). Miri was not used: the component is not
   installed and installing it is a download.
   - Corpus 1 (821 hand-written cases, `corpus.mjs`): dicts and sets whose keys' `__eq__` /
     `__hash__` clear, grow, pop or replace the container during lookup, insert, `|`, `==`,
     `fromkeys`, `{**d}`, `update` (Monty); `sort`/`sorted`/`min`/`max` with keys and `__lt__`
     that clear, extend, reverse, re-sort or slice-delete the list being sorted; `index`,
     `count`, `remove`, `in`, `==`, `<` with mutating `__eq__`; 45 mutation-during-iteration
     shapes (`for` over list/dict/set/items/values with append/remove/del/clear,
     comprehensions that `clear()`/`pop()` their iterable, `map`/`filter`/`zip`/`reversed`
     over a list cleared inside, stale iterators after clear+refill, `__index__`/`__len__`/
     `__iter__`/`__repr__`/`__str__` that mutate the container being sliced, extended,
     printed or formatted); 28 generator/finally/cycle shapes (yield in finally, close/throw,
     self-referencing and self-running generators, 200 suspended generators whose finally
     clears the shared list, reference cycles through lists/dicts/instances/exceptions,
     recursive `__hash__`/`__eq__`, dataclasses with `order`/`frozen`/`slots`/self-reference).
     JS (Boa): 13 array mutations x 32 callback-taking or coercing built-ins (sort, toSorted,
     map, filter, reduce, find*, flatMap, Array.from, join, indexOf/lastIndexOf/includes,
     fill, copyWithin, splice, slice, with, at, toSpliced, flat, concat, iterators,
     JSON.stringify replacer) plus 38 shapes: Map/Set iteration with delete/clear/refill and
     100 live iterators, for-in with deletion, Proxy `ownKeys`/`get` traps mutating the
     target, generator return/finally re-entry, 500 suspended generators, async generators,
     WeakMap/WeakSet ephemeron chains under GC pressure, resizable/transferred ArrayBuffers
     inside typed-array sort/set/fill/map/copyWithin/from/DataView, species constructors,
     `toJSON`/reviver mutation, promise chains and `icp.call` results mutated in `then`.
     Rhai: 7 closure/array/map mutation shapes.
   - Corpus 2: 5,800 cases from the Testability-r3 generator (`t3/gen.mjs`, new seed 4242;
     2,500 Python, 2,500 JS, 800 Rhai), filtered to drop sizes that allocate gigabytes
     natively.
   - Each corpus ran sequentially and interleaved (two runs joined, each suspending at every
     host call through `YieldBridge`), then the whole native suite (58 tests).
2. **Differential oracle (emulator).** The deployed module (`software.wasm` of
   `icp_code_mode`, sha256 `224cfa7a…c39eec769`, the same bytes the replica runs now) in the
   Testability-r3 `ic0` emulator, with a fixed `ic0.time`. The same 6,621 cases were run
   (a) each in a fresh instance, (b) in three long-lived instances in shuffled order (case k
   after up to 2,206 unrelated cases), (c) with a host call prepended, each alone, and
   (d) the same, interleaved: up to six runs suspended at once, replies delivered in random
   order. Compared: the MCP result text and every audit entry the run wrote (status,
   methods, calls, violations, error, language, canisters, script hash), decoded from
   `get_audit_logs`.
3. **4 GiB edge (emulator, then replica for the trap type).** Harness variants with memory
   maximum 49,152 pages (3 GiB, as in r3) and 65,536 pages (the wasm32 maximum), plus an
   export of `__stack_pointer` to read SP after a trap. Linear memory was brought to exactly
   65,536 pages from inside a response callback by script allocations (binary search over
   chunk counts; the emulator is deterministic), or, for screening, by `memory.grow` from the
   host. Then a deep native recursion was run, inside a callback or as a new ingress script.
   Trap type on the real replica checked with three ordinary token-holder requests that trap
   before any message is sent (no audit entry, no setting changed).

Scratch files: session scratchpad `l11r4/`. Private copy deleted at the end.

## Results

### Sanitizer

| Run | Cases | Result |
|---|---|---|
| Corpus 1 + 2, sequential (ASan) | 6,621 | 0 ASan reports (160 s) |
| Corpus 1 + 2, interleaved with suspension (ASan) | 6,621 | 0 ASan reports (294 s) |
| Native suite + probe (ASan, `RUST_MIN_STACK` raised) | 58 tests | all pass, 0 reports. With the default 2 MiB test-thread stack ASan reports `stack-overflow` in a deep-recursion test: ASan frame growth, not a memory error |

Behaviour of note (no memory error): Monty reports `RuntimeError`/`ValueError` or completes
for every mutate-during-lookup/sort/iterate shape; dict/set lookups whose `__eq__` clears the
table return `False` cleanly.

### Differential oracle

| Comparison | Same | Different | Cause of every difference |
|---|---|---|---|
| fresh vs sequential (after up to 2,206 cases) | 6,615 | 6 | instruction budget: the emulator scales `performance_counter` from wall time, so cases near the 2e9 budget flip |
| fresh vs interleaved-suspended (results) | 6,611 | 6 | same 6 cases, same cause |
| fresh vs interleaved-suspended (audit entries, 4,592 script groups) | 4,591 | 1 | instruction count in the error text |

Traps in both arms: only the known Rhai `sleep` (L11-r2-1). No result, log line or audit
field depended on what ran before or alongside.

### 4 GiB edge

Layout: shadow stack at [0, 8 MiB), SP starts at 8,388,608 and grows down (L15-r1). An
overflow makes SP wrap to just below 2^32. Below 65,536 pages, the first store there is out of
bounds and traps. At 65,536 pages every address below 2^32 is valid.

| Probe | Memory | Result |
|---|---|---|
| `String(a)` on a 100,000-deep array, in a callback, 3 GiB harness | 46,768 pages | trap `memory access out of bounds` (E) |
| same, 4 GiB harness, heap below the edge | 64,448 pages | trap, SP `0xffffffe0` (E) |
| same, heap exactly 65,536 pages | 65,536 | trap in most stack alignments: the wrapping frame straddles 2^32 (func 3335, 80-byte frame, SP `0xffffffd0`: stores at SP+24 in bounds, SP+49 out). Silent in 3 of 288 alignments: SP of the wrapping frame starts exactly at 0 (E) |
| silent alignment, e.g. `({toString: () => ({toString: () => a.join()}) + ""}) + ""` | 65,536 | **no trap**, `isError: false`; 134,026,152 bytes written downward from 2^32 (E) |
| cross-run: run A (silent alignment) in its callback; run B suspended holding `new Uint8Array(198000000)` (zeros) at the top of memory | 65,536 | A returns `isError: false`. B resumes and returns `nonzero_in_last_MiB: 523237`, `last16: [0,0,252,127,192,148,173,0,…]`, `isError: false` (E). Control without A's recursion: 0 non-zero bytes. Control at 3 GiB: A traps |
| same, with live JS strings at the top instead of a buffer | 65,536 | trap `function signature mismatch` / `table index is out of bounds`: `call_indirect` through overwritten string headers (E) |
| parser route: ingress `return [[[` + 3,500 `(` … `)` + `]]];` with memory already at 65,536 pages | 65,536 | **no trap**, returns `[[[1]]]`, 57,635,552 bytes written below 2^32 (E) |
| plain 3,500 parens, same memory | 65,536 | wraps silently, writes 57.6 MB, then a later frame straddles 2^32 and traps (E): a trap rolls the message back |
| replica trap type, `String(a)` 100,000 deep | live | `stack overflow` (L): wasmtime's native stack runs out before the 8 MiB shadow stack, so this path cannot wrap on the replica |
| replica trap type, `Object.keys` on a 100,000-deep Proxy chain | live | `heap out of bounds` (L): shadow stack first; runs at script run time, so reachable inside a callback |
| replica trap type, 3,000 nested parens | live (L18/L2 rounds) | `heap out of bounds` (L): shadow stack first |
| Proxy-chain recursion in a callback, 4 GiB: 904 wrapper combinations + 192 chains with 0-47 JS-trap levels on top | 65,536 | 0 silent: every one traps with SP in {`-16`, `-32`, `-48`, `-96`, `-144`} (E). The callback route on the replica (Proxy is the shadow-first path found) is **not demonstrated** |
| parser route, 218 nesting prefixes before 3,500 parens | 65,536 | 27 silent (e.g. `[[[`, `-(-(`, `[{a:`, `` `${`${ ``, `typeof typeof`), each writing 57.6 MB below 2^32 and returning `isError: false`. The rest trap (E) |
| parser route, cross-run: run B suspended with a zero-filled 213,384,000-byte buffer at the top (filled in its callback, memory 65,536 pages); an unrelated ingress script `return [[[` + 3,500 `(` `1` `)` + `]]];` | 65,536 | ingress script: `[[[1]]]`, `isError: false`. B resumes: `nonzero_in_last_MiB: 136735`, `isError: false` (E). Same with `-(-(` (136,643) and `[{a:` (136,735). Control with 300 parens: B reads 0 non-zero bytes |

## Grid

| Row | Cell | Reason / evidence |
|---|---|---|
| V8 Boa GC heap | **finding (L11-r4-1)**; holds for the corpus | ASan: no memory error in any Boa path (callbacks mutating arrays, typed-array buffer resize/transfer in callbacks, ephemerons under GC pressure, suspended generators, Proxy traps) (R). Differential: no cross-run effect (E). But at 65,536 pages a shadow-stack overflow writes stack frames over the top of the GC heap without trapping (E), and the top of the heap held run B's live data |
| V10 engine statics | holds | Differential oracle over 6,621 cases: no case's result, logs or audit entry changed after up to 2,206 earlier cases or with six runs suspended alongside (E). Seed stream (V7) and Boa symbol registry carry state across runs, but nothing observable differed |
| V6 `LIVE_MEMORY` / counting allocator | holds | The allocator's only `unsafe` delegates to `System` and saturates the counter (`src/lib.rs:385-418`) (C). ASan: no report over Python cases interleaved with suspension (R). Python memory-limit results identical fresh vs interleaved (E) |
| I6 Isolation | **finding (L11-r4-1)** | At 65,536 pages another run's script overwrote run B's suspended buffer, and B returned it as success. Two routes did this: an ingress parser recursion (136,735 non-zero bytes in B's last MiB) and a callback recursion (523,237) (E). No other cross-run channel in the differential (E) |
| I20 Sandbox | **finding (L11-r4-1)** | A script writes outside its own memory, into any other run's or the canister's heap, at 65,536 pages (E) |
| S2 direct deps (Monty unsafe) | holds (tested in debug) | Real `unsafe` in monty 1.6.0 is in `heap/mod.rs`, `heap/stable_heap.rs`, `heap/free_list.rs`, `heap_traits.rs`. `sorting/timsort.rs` and `intern.rs` contain none (timsort permutes `usize` ids; the hits are the words `unsafe` and `unsafe_hash` in text and identifiers), and the `dataclasses` hits are the `unsafe_hash` option. `StableHeap` hands out `&T` only for `Some` slots, frees need `&mut`, pages are boxed and never moved (C). ASan over 360 + 2,500 Python cases and interleaving: 0 reports (R). Not covered: release-only paths (the ASan build is debug: Monty `MAX_NESTING_DEPTH` 30, debug asserts on) |
| S2 (boa_gc, rhai unsafe) | holds for the corpus | 178 `unsafe` sites in boa_gc, 6 in rhai; ASan clean over 2,954 JS and 807 Rhai cases (R) |
| F12 native stack overflow | **finding (L11-r4-1)** (variant of the reported Boa parser/built-in recursion) | The reported recursion traps only while memory is below 4 GiB. At 65,536 pages it can complete silently after writing 57-134 MB over the heap (E). Replica: the parser and Proxy paths overflow the shadow stack first (L); the `String(a)` path overflows the native stack first (L) |
| LIM28 8 MiB stack, stack-first layout | **finding (L11-r4-1)** | Overflow detection relies on the wrapped address being out of bounds. That holds only below 65,536 pages, and even at 65,536 pages it holds only when the wrapping frame straddles 2^32 |
| LIM29 3 GiB `wasm_memory_limit` | enabler (known L8-6) | Not enforced in response callbacks (L8-6, L: 3.42 GB reached). Reaching exactly 65,536 pages from a callback is deterministic (E: 4,066 x 1 MiB + 790 x 4 KiB strings, or a 198 MB buffer). After the limit is crossed, ingress is refused (L8-6), so the parser route also needs the limit raised (the setting change that L8-6 names as the way out) |
| CL34 ".cargo/config.toml: a stack overflow is a trap" | **finding (L11-r4-1)** | False at 65,536 pages (E) |
| I5 Clean failure | partly (L11-r4-1) | At 65,536 pages, the misaligned cases still trap. Live strings at the top produce `call_indirect` traps through overwritten headers (E) |

## Candidate findings

**L11-r4-1. At the wasm32 memory maximum, a shadow-stack overflow does not trap: a script
overwrites the top of the shared heap, including other runs' suspended data, and both runs
report success.**
Severity proposed: High (another user's run returns wrong data as success; the shared heap is
corrupted silently). Not Critical: no durable-state or authorization effect was demonstrated.
Precondition: linear memory at exactly 65,536 pages. L8-6's callback path reaches that
size, and the exact count is reachable deterministically (E). The overflow then has two routes:
- **Ingress route (demonstrated, E; trap type on the replica L):** the Boa parser's
  recursion runs out of the shadow stack first on the replica (`heap out of bounds`, L). An
  ingress script with 3,500 nested parens behind one of 27 of the 218 prefixes tried
  completes silently and overwrites another run's suspended data. It needs ingress to be
  accepted at that memory size. After L8-6 the canister refuses updates until
  `wasm_memory_limit` is raised to 4 GiB or more, which is the setting change L8-6 names as
  the way out, or until the limit is set at 4 GiB from the start.
- **Callback route (not demonstrated):** it needs a runtime recursion that exhausts the
  shadow stack first on the replica and that completes in a silent alignment. `String(a)`
  wraps silently in the emulator (3/288 alignments, cross-run overwrite E), but the replica
  traps it on the native stack first (L). The Proxy chain is shadow-first on the replica (L),
  but no silent alignment was found in 1,096 (E).

Grade: High. The evidence is on the deployed bytes in the emulator. The replica-specific
inputs (which stack overflows first) were checked live. The 4 GiB state itself was not
created on the replica: it would push the shared local network to 4 GiB, which belongs to the
Live phase. A trap anywhere later in the same message rolls the corruption back, so only
complete, silent alignments matter. Those exist for the parser route.

Root cause: the stack-first layout (`-zstack-size=8388608`, `.cargo/config.toml`) has no
stack-limit check, so overflow detection is delegated to an out-of-bounds store. That store
stops being out of bounds when linear memory is at the wasm32 maximum. The 3 GiB platform
limit that would keep memory below the maximum is not enforced in callbacks (L8-6) and is
operator-adjustable.

Fix:
- Never let linear memory reach 65,536 pages. Have the allocator refuse to grow past
  4 GiB minus a margin (a `GlobalAlloc` wrapper over the existing counting allocator), and
  document that `wasm_memory_limit` must stay below 4 GiB.
- Also bound engine recursion by depth: the Boa parser nesting (the reported finding) and
  the Boa built-ins on deep data. Then the stack is never exhausted at all.

Moving the stack above the data is not a fix: overflow would then run silently into static
data at any memory size.

Regression test: the emulator harness with a 65,536-page memory and the parser-route script
(`return [[[` + 3,500 `(` `1` `)` + `]]];` after the host grows memory to the maximum). It
must end in a clean error or a trap.

## Not tested, and why

- Miri: not installed (would need a component download). ASan covers out-of-bounds and
  use-after-free on system-allocator memory only; Monty's slot reuse (a stale `HeapId`
  reading a reused slot) is a logical aliasing bug ASan cannot see. Differential testing
  covered that effect indirectly (E: no result differed).
- ASan build is debug; release-only code paths (Monty nesting 200, Boa asserts compiled out)
  were exercised only in the emulator, without a sanitizer.
- The 4 GiB scenarios on the real replica (memory pushed to 65,536 pages on the shared local
  network: Live phase). Whether wasmtime on the replica permits exactly 65,536 pages is spec
  behaviour but was not observed.
- Which recursions overflow the shadow stack first on the replica was checked for three
  paths only (String: native first; Proxy and parser: shadow first).
- Concurrent edits: the working tree's `src/` changed during this round (tests added to
  `src/lib.rs`, `src/runtimes/js.rs`, `src/runtimes/mod.rs`, `src/bridge.rs` around 10:02-10:10
  by another party). This round's copy predates them; the emulator runs used the deployed
  module, which these edits did not change.
</content>
</invoke>
