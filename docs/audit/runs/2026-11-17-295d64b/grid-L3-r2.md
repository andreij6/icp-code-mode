# Coverage grid: L3 Resource bounds, round 2 (Boa global Symbol registry)

Run `2026-11-17-295d64b`. Row ids refer to `inventory.md`. Cells: **finding** (candidate id
below), **holds** (why), **n/a** (why). Candidate L3r2-1 is described in the lens's returned
findings.

## How it was tested

- **Code path.** `boa_engine-0.22.0/src/builtins/symbol/mod.rs:41-80`: `static
  GLOBAL_SYMBOL_REGISTRY: LazyLock<GlobalSymbolRegistry>` holds two `FxDashMap`s
  (`keys: Box<[u16]> -> JsSymbol`, `symbols: JsSymbol -> Box<[u16]>`); `JsSymbol::new`
  (`src/symbol.rs:175-186`) stores a third `Box<[u16]>` copy of the description in an `Arc`
  owned by the map. No code removes entries. `Symbol.for` (`mod.rs:320-336`) is the only
  writer. `src/runtimes/js.rs:308-315` deletes `Math.random`, `Atomics`,
  `SharedArrayBuffer`, `WeakRef`, `FinalizationRegistry`, not `Symbol.for`;
  `boa_gc::force_collect` (`js.rs:361`) only sweeps the thread-local GC heap, and the registry
  is outside it (`JsSymbol` is `unsafe_empty_trace`, `Arc`-counted).
- **Other engine statics checked (V10).** Boa: `CANNOT_BLOCK_COUNTER`,
  `ASYNC_EVAL_QUEUE_INDEX`, `CODEBLOCK_ID_COUNTER`, `SYMBOL_HASH_COUNT` are counters;
  `RecursionLimiter::SEEN` is a Debug-fmt scratch map that empties itself; boa_string
  `RAW_STATICS_CACHE` is a fixed table; `futex::CRITICAL_SECTION` is unreachable (Atomics
  deleted). Rhai 1.27.0: only fixed tables and two `OnceCell` hash seeds. Monty: fixed
  LazyLocks (per task brief). `GLOBAL_SYMBOL_REGISTRY` is the only growable engine static.
- **Native test** (private copy, `CARGO_TARGET_DIR=/private/tmp/claude-501/audit-target`,
  `cargo test --lib audit_l3_symbol -- --nocapture --test-threads=1`, debug profile). The
  test turns on the project's own counting allocator for its thread
  (`counting_alloc::COUNT_THIS_THREAD`) and reads `monty_types::LIVE_MEMORY` before and after
  each `js()` run (`MockBridge`). Keys are a 131,072-char string built by doubling
  (`let S='k'; for(j<17) S+=S`) plus a suffix; `'k'.repeat(1e5)` cannot be used because
  `String.prototype.repeat` charges the frame's 1M loop counter (LIM24).

  | Run | Script (200 iterations unless noted) | Live heap retained after the run |
  |---|---|---|
  | A | `Symbol.for(S+i)` | +150.03 MiB (total 150.08) |
  | B | `Symbol.for(S+'b'+i)` | +150.03 MiB (total 300.10) |
  | A again | same keys as A | +0.00 MiB |
  | C control | `Symbol(S+i)` (not registered) | +0.00 MiB |
  | D control | `a.push(S+i)` (200 strings in an array) | +0.00 MiB |
  | E | `Symbol.for(S+'e'+i); throw new Error('x')` (run ends with an error) | +150.03 MiB (total 450.13) |
  | F | 100,000 x `Symbol.for('s'+i)` (short keys) | +13.39 MiB (total 463.52) |

  150.03 MiB = 200 x 131,072 x 6 bytes: three UTF-16 copies per key. Short keys cost about
  140 bytes each. The controls show that `force_collect` reclaims everything else a run
  allocates, so the growth is the registry alone.
- **Live test** (local network, port 8010, `icp_code_mode`, module hash `0xad3e...2411`),
  token from the `MCP_TOKEN` line of `.env`, never printed. Instruction counts from the
  closing audit entries, matched by `script_sha256`:

  | Audit id | Script (20 iterations, 131,072-char keys) | Instructions |
  |---|---|---|
  | 911 / 917 | baseline: build `S`, `return` | 19,875,185 / 19,926,026 |
  | 912 / 915 | `Symbol(S+'L3r2a'+i)` | 322,072,784 / 326,806,093 |
  | 913 | `Symbol.for(S+'L3r2a'+i)`, first time (inserts) | 678,637,783 |
  | 914 | same script and keys in a new request (lookups only) | 479,803,848 |

  If each run had a fresh registry, 913 and 914 would cost the same. The second run is ~199M
  instructions (29%) cheaper, so the 20 keys from run 913 were still registered in a separate
  later request on the deployed canister. Cost of a retained byte: about
  (678.6M - 19.9M) / (20 x 786,432 B) = 42 instructions. Under the default `max_instructions`
  of 20e9 one run that ends cleanly keeps about 20e9 / 42 = 476 MB (about 454 MiB). With the
  default 3 GiB `wasm_memory_limit` (LIM29), about 7 such runs fill an empty heap. This was not
  driven to the limit.
- **memory_size is not a usable signal here.** `icp canister status` showed 1,246,514,630
  before and 3,259,715,014 after the window, but other agents' runs (for example audit ids
  916 and 918, 2.07e9 and 5.15e9 instructions, not this lens's scripts) ran in the same
  window, and Wasm memory never shrinks. This lens's runs keep about 15.7 MiB (20 keys x
  786 KB). **No upgrade was done** (only the Live phase may upgrade). Those 15.7 MiB stay
  allocated until the next upgrade. The canister answered a baseline run normally afterwards.
- **Cross-run observability (I6/I20).** Native: run 1 `Symbol.for('probe')` then run 2
  `[Symbol.keyFor(Symbol.for('probe')), Symbol.keyFor(Symbol('probe')),
  Symbol.for('probe')===Symbol.for('probe'), Object.getOwnPropertySymbols({[Symbol.for('zz')]:1,
  [Symbol.for('probe')]:2}).map(String)]` returns `["probe", null, true, ["Symbol(zz)",
  "Symbol(probe)"]]`. These are the same as with a fresh registry. No symbol value outlives a
  run, so identity cannot be compared across runs. Property order is insertion order, not
  the global hash order. A script has no clock or instruction counter. The only signal is
  cost: a lookup is cheaper than an insert. The caller can see that only in audit entries
  (controllers) or in the number in a budget-exceeded error, after the run.

## Grid

| Row | Cell | Reason |
|---|---|---|
| V10 engine statics | **finding L3r2-1** | `GLOBAL_SYMBOL_REGISTRY` is a process-global, insert-only map that a script fills through `Symbol.for`. Measured: +150 MiB kept per run, the same after a run that throws, and still present in a later request on the live canister. Every other Boa, Rhai and Monty static is a counter or a fixed table (see above) |
| V8 Boa GC heap | holds (for retention) | Controls C and D keep 0.00 MiB after the run: `force_collect` reclaims the run's GC heap, strings and unregistered symbols. Charging one run for another's GC work is the separate L8-1. The registry is outside the GC heap, so nothing in V8 frees it |
| LIM29 3 GiB Wasm memory | **finding L3r2-1** | Round 1 graded this row "holds/accepted" because each JS run frees what it allocates (#24). That is false for the registry. The limit is cumulative across runs, and about 7 clean runs at the default budget reach it (extrapolated from the measured 42 instructions per byte). Nothing in the app tracks or caps it |
| F13 out of memory | **finding L3r2-1** | Round 1 graded this row "accepted (#24)". #24 covers one run's trap, which rolls back that message. Here the memory is committed by runs that succeed, so after the limit is reached the out-of-memory trap is persistent, not per-run. Not reproduced to the trap: that would make the shared dev canister unusable until an upgrade, and upgrades belong to the Live phase |
| LIM24 JS loop/recursion limits | holds | `String.prototype.repeat` charges the frame's loop counter (`'k'.repeat(1e5)` x ~10 calls hits it). Building by doubling uses 17 iterations. The limit bounds iterations, not memory, as documented. Neither path is a new bypass |
| LIM26 Python 64 MiB / LIM27 Rhai sizes | n/a | No process-global engine registry in Monty or Rhai (V10 check). Their per-run memory is bounded and freed |
| W8 JS unbounded work | **finding L3r2-1** (memory dimension) | The cost per run is bounded by `max_instructions`. The memory a run keeps after it ends is not bounded |
| E5 `http_request_update` | **finding L3r2-1** (reach) | Any token holder: `execute_icp_script` with `language: "javascript"`. No allowlist entry and no outbound call are needed, so no policy check or call cap applies |
| E4 `http_request` query | n/a for writing (a query's heap changes are discarded) | Whether queries trap once the registry fills the heap is unverified: the limit is not driven, and the platform's enforcement of `wasm_memory_limit` for queries was not checked |
| AD4 `set_limits.max_instructions` | holds | A larger budget raises the memory one run can keep (42 instructions per byte), but the total across runs is unbounded at any setting. The budget does not affect this finding |
| AD8 upgrade (recovery) | holds (code path) | No `pre_upgrade` (`src/lib.rs:60-69`, all state in stable structures), so an upgrade starts a fresh heap and clears the registry. The recovery path exists but is undocumented and was not exercised (no upgrade in this phase) |
| I4 Bounded cost | **violated (L3r2-1)** | One token holder makes the canister hold unbounded memory across requests |
| I6 Isolation / I20 sandbox | holds (natively tested), with a cost side channel | No value, identity, `keyFor` result or ordering differs across runs. A lookup costs less than an insert, but a script cannot see that. Callers see it only after the fact, and all tokens are one identity (#49) |
| I10 Honest interface; README.md:451-453 ("a script that hoards memory ends in a trap ... The cap of 8 concurrent scripts limits how far this can spread"); README.md:70 ("fresh VM per call") | **finding L3r2-1** (claim part) | The README treats JS memory as per-run. Registry memory accumulates across sequential runs, so the 8-run cap does not limit it, and the engine is not fully fresh per call. README.md:451-453 is not in the Claims inventory (section 16): a missing row |
