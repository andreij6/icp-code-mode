# Coverage grid: L11 Sandbox and determinism, round 3 (host requests from engine-native callbacks)

Run `2026-11-17-295d64b`. Question: when `icp.*` is issued from a frame that a native builtin
re-entered (sort comparators and keys, map/filter closures, dict probes, Proxy traps, `toJSON`,
string interpolation, `switch`), is the suspension refused cleanly or resumed correctly, is
every sent request audited, and was every audited request sent?

Evidence key: **R** reproduced, **C** code path only.

## Method

1. Private copy (`rsync` without `target/`, `docs/`, `design/`, `video/`, `.env`), shared
   `CARGO_TARGET_DIR`. Probe modules `l11r3_probe*` appended to `src/runtimes/mod.rs` with an
   echo bridge (`call` answers its own args, optional refusal after N calls to model the cap),
   run through `LanguageRouter::execute`. Copy deleted afterwards.
2. Rust 1.100 `slice::sort_by` checked standalone with inconsistent comparators (random
   orderings; NaN treated as Equal): it panics with "user-provided comparison function does
   not correctly implement a total order" for many inputs of length >= 21.
3. Live, local `icp_code_mode` (`4caro-...`), no deploy, no setting changed. Scripts sent
   through `/mcp`; calls only to `demo_store.get_product` (read-only). Audit entries read with
   `get_audit_logs(0, n)`.

## Results per runtime

| Probe | JS (Boa) | Python (Monty) | Rhai |
|---|---|---|---|
| comparator / key calls host | `sort((x,y)=>icp.call(..))`: comparator gets a Promise, NaN -> 0, requests queued; sent and audited only if awaited (R) | `sorted/min/max(key=)`, `map`, `filter`, `__hash__`/`__eq__` in a dict probe, `__str__` in `print`: `NotImplementedError: external function 'icp.call' cannot suspend in this synchronous context`, nothing sent, catchable (R) | `sort`, `dedup`: suspension swallowed by the native (`array_basic.rs:1574-1591`, `:1319-1324`), re-terminated by `on_progress`; replays resume correctly while the comparator stays a total order (R) |
| map / filter / reduce / for_each / some / index_of / drain | n/a (JS callbacks just queue) | refused (R) | correct result, one send per request, no duplicate sends (R) |
| string interpolation, `switch` | n/a | n/a | correct (`"a1b2"`, `22`), 2 sends each (R) |
| Proxy `ownKeys`/`get` trap and getter during the argument snapshot | nested requests queued before the outer one; all sent and audited, no borrow panic (R) | `__repr__` of an argument object is not called by the converter (`py_repr` gives `A()`), nothing sent (R) | n/a (converter is pure Rust) |
| `toJSON` in the final result / thrown value / `console.log` after the last round | request queued, never sent, never audited; result reports success (R). Variant of the already-reported "requests only sent when the run loop finds the top-level promise" (`js.rs:374-383`) | n/a | n/a |
| comparator that is not a total order (NaN keys, shuffle comparator) | **trap** (L11-r3-1), Array and TypedArray (R live) | clean: own timsort, Rust `sort_by` only for NaN-free numbers/short strings (`monty sorting/mod.rs:84-90, 160-200`) (R) | **trap** (L11-r3-1): `sort(|..|)` relies on `catch_unwind`, inert under `panic = "abort"`; builtin float `sort()` with NaN has no guard at all (R live) |
| policy refusal / call error inside the comparator | propagates (comparator throws) | refused before send | **swallowed**: sort continues, result `isError: false` with wrong order (L11-r3-2, R live) |

## Grid

| Row | Cell | Reason / evidence |
|---|---|---|
| E16 JS `icp.*` | **finding** (L11-r3-1) | `Array.prototype.sort`/`toSorted`/TypedArray `sort` call Rust `slice::sort_by` with the user comparator (`boa_engine-0.22.0/src/builtins/array/mod.rs:2671-2681`). Live: `a.sort((x, y) => x.price - y.price)` on 40 objects, 1 in 5 without `price`, traps: `Panicked at 'user-provided comparison function does not correctly implement a total order'`, HTTP 503, no audit entry (R). Host requests from callbacks: holds (each queued request is sent once and audited once when the script awaits; R native). `toJSON` at final conversion: duplicate of the reported pending-drop pattern |
| E18 Python `icp.*` | holds | Every native re-entry (`evaluate_function`, `monty bytecode/vm/call.rs:434-457`) converts an external, OS, method or future suspension into a catchable `NotImplementedError` before anything reaches the host; R native for sorted/min/max key, map, filter, dict `__hash__`/`__eq__`, `print` `__str__`, `call_async` in a key. Generators consumed by `sum` and comprehensions suspend normally (3 sent, 3 audited). NaN and inconsistent orders sort cleanly |
| E19 Rhai `icp::*` | **finding** (L11-r3-1, L11-r3-2) | Closures, interpolation and `switch` replay correctly (R). `sort` and `dedup` convert every comparator error into a fallback order: ErrorTerminated (suspension), `ErrorRuntime` from a cached refusal or reject. The suspension case is safe for request identity but makes a consistent comparator inconsistent across replays -> trap after messages were sent (R live: 16 `get_product` sent, then `Trapped`, result discarded). The refusal case returns wrong data as success (R live) |
| K3 Python awaits | holds | No await is reachable from a re-entered frame (refused above), so every K3 await is at top-level VM suspension; sent == audited (R native counts) |
| K4 Rhai replay await | **finding** (L11-r3-1 via L11-r3-2) | The await itself is correct, but the replay re-runs native sorts with a mix of cached answers and fallback `Equal`, so a replay can panic. On wasm the panic is a trap in the continuation after the K1 commits: `Started`/`Call` entries x16, then `Trapped` (R live, run_id 5855) |
| F8 RefCell re-entrancy | holds (now R) | Previously decided by reading. Dynamic: JS host function re-entered from a Proxy trap, a getter and a `toJSON` inside the argument snapshot and inside `console.log`; resolver `then` lookup runs with no `State` borrow held (`js.rs:383` takes the batch first). No `BorrowMutError` panic in any probe. Rhai: `request` holds the `Mutex` only around `to_dynamic`; `on_progress`/`on_print` never run under it. `IcBridge` is only entered from the drive loops, never from engine callbacks (C). New: a different panic class (std sort) is reachable from the same callbacks, L11-r3-1 |
| I27 / CL35 Rhai replay | holds for request identity; **finding** for the "uncatchable stop" (L11-r3-2) | No probe served an answer to a different request or sent one twice: `request` refuses while a request is pending and compares the cached `Request` by value (`rhai_rt.rs:64-83`); repeated identical sends seen in `sort`/`dedup` are the script's own repeated requests. But README.md:489-491 ("the run is stopped (uncatchably)") is false inside `sort`/`dedup`: the native catches the stop, keeps running the sort on fallback values, and only the next script operation re-terminates. The replay path is deterministic, yet the sort's internal consistency is not, which is what traps |
| I5 Clean failure | **finding** (L11-r3-1) | see E16/E19 |
| I3 Records match reality | holds for this lens | Every probe: requests sent == bridge calls == audit `calls`; requests created after the last round are neither sent nor audited (consistent, already reported) |
| Tool description "identical semantics in every language" (`src/mcp.rs:80`) | **finding** (part of L11-r3-2) | A cap refusal inside a comparator: JS throws, Python refuses before the send, Rhai returns success with wrong data |

## Not tested

- `dedup` with a swallowed refusal: code path only (`rhai array_basic.rs:1319-1324`, `unwrap_or(FALSE)`), same mechanism as the reproduced `sort` case.
- Rhai `sort_desc` with NaN floats and Boa `toSorted`: code path only, same `sort_by` call.
- Whether Boa or Rhai call `sort_by` elsewhere with user input (e.g. `Intl`-free collation): not swept; this round only covered the sort entry points named.
