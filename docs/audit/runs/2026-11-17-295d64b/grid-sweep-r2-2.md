# Variant sweep r2-2: limit applied after a dependency's recursive work; dependency guard absent on wasm

Run `2026-11-17-295d64b`, Phase 6, round 2. Pattern under test:

> A depth limit applied to the output of a recursive dependency instead of its input; a
> dependency's safety guard compiled out on the deployment target.

Searched in its general form: (a) any app limit (depth, node count, text bytes) that is
checked on what a dependency produced, after the dependency already did work sized by the
input; (b) any guard inside a dependency that is absent or different in the shipped build
(wasm32, release: `cfg(target_family = "wasm")`, `cfg(debug_assertions)`, 32-bit `usize`).

Not re-reported (known): candid decoder depth guard no-op on wasm, candid_parser on nested
interfaces, Boa parser/built-ins on deep data, Boa `capacity overflow` built-ins, tmp_hack
decode without quota, json_to_idl cost, Rhai deep data, inspect_message decode quota, Python
VM-side repr/bigint past the budget, Python print formatting in full.

## How it was tested

- Private copy of the tree (no `target/`, `design/`, `docs/per-customer-canisters-plan.md`,
  `video/`, `.env`) in `/private/tmp/claude-501/sweep-r2-2.*` (deleted afterwards), shared
  `CARGO_TARGET_DIR=/private/tmp/claude-501/audit-target`. A probe `cdylib` (workspace member
  in the copy only) calls the app's own `runtimes::{python,js,rhai_rt}::run` with a bridge
  that refuses calls, polled once.
- Built `--release --target wasm32-unknown-unknown` with the repo's `.cargo/config.toml`
  (artifact checked: `__stack_pointer` init 8,388,608, stack-first). Run in Node 26 Workers
  (1,000 MB V8 stack, all `ic0` imports stubbed, `performance_counter` = 0, so the instruction
  budget never fires in the probe). `TRAP` = wasm trap; `ERR` = clean error.
- `cargo tree --target wasm32-unknown-unknown -e normal` listed the 299 crates in the canister;
  each was grepped for wasm/debug-gated code.
- Evidence: R = reproduced in the wasm probe, CP = code path. No live canister was touched.

## Cells

| Row | Item | Cell | Reason |
|---|---|---|---|
| B9 / U7 / W10 / LIM16 / CL10 | Python `to_json` (`src/runtimes/python.rs:62-99`): `_` branch `Value::String(o.py_repr())` (`:98`), bigint `o.py_repr()` (`:71`), non-str dict key `k.py_repr()` then `budget.bytes` (`:92-93`) | **finding** (R) | `ObjectRef::py_repr` (`monty-types-1.6.0/src/object.rs:580-640`) renders a shared sub-object once per reference with no size limit, and BigInt via `write!("{v}")` (`:717`) with no digit limit (the VM refuses `str()` past 4,300 digits). The app's `ValueBudget` is never charged for `:71`/`:98` and is charged only after the full string exists for `:92`. See candidate 1 |
| B9 (Python export) | Monty export of results/args into `MontyGraph` before `to_json` | holds (CP, R) | Export is memoised (`monty-1.6.0/src/object_bridge.rs:212-240`), so its size follows the heap (64 MiB cap), and it takes the VM recursion guard per level (`:183-186`). A plain DAG list (`a=[a,a]` x20) is refused by the 200k-node budget: `ERR value has more than 200000 elements` |
| W10 / LIM26 / F12 | ruff `stacker::maybe_grow` (no-op on wasm) for sources under Monty's 4 KiB scan threshold | holds (R) | Beyond r1-11's balanced shapes: unclosed `(`, `[`, `{` x4,095; unclosed `f(`, `a[`, `-(` x2,047; nested f-strings x1,365; `lambda:` x585; ternary x340; `{1:` x1,365: all clean `SyntaxError`/`too deeply nested`, no trap |
| W10 (eval/exec) | Monty `eval`/`exec` parse of runtime strings | holds (R) | `compile_snippet` passes the same scan threshold (`monty-1.6.0/src/builtins/eval_exec.rs:299-315`). `eval("("*4095)`, `exec("["*4095)`, `eval('f"{'*1365)` clean; `eval` of a 2,047-paren / 1,365-dict snippet under 0, 5, 11 and 12 native re-entries (`sorted(key=...)`, cap `MAX_RUN_REENTRY_DEPTH = 12`, `bytecode/vm/recursion.rs:126`) clean |
| W10 / C10 | Monty `MAX_NESTING_DEPTH` is 200 in release, 30 in debug (`monty-1.6.0/src/parse.rs:41-47`): native tests never reach the shipped bound | holds (R) | On wasm release: nested `def`, `async def`, `if`, `for`, `try`, `with`, lambda, list, call, listcomp, genexp, ternary, `not`, unary, f-string, dict, subscript, `match`, pattern at depth 150 run cleanly; at 199 every shape is refused (`Source is too deeply nested`) or runs. No trap |
| W10 (sleep) | Monty `block_for` busy-spins on wasm32 with the execution clock stopped (`monty-types-1.6.0/src/resource.rs:770-846`) | holds (CP) | Reached only via `SleepMode::System`; the app sets `SleepMode::Zero` (`src/runtimes/python.rs:215`), so `time.sleep` never waits |
| B10 / W11 | Rhai `to_json_value` clones each subtree (`d.clone().into_array()`, `src/runtimes/rhai_rt.rs:99`) before checking the next level's depth | holds (R) | Rhai's data-size check counts expanded totals: nesting 1,000-9,990 is refused at 128 levels (`ERR value nested deeper than 128 levels`), 20,000 by `Size of array/BLOB too large` |
| B10 / U7 (Rhai stringify) | Rhai `to_string`, `print`, `debug`, `throw`, interpolation of shared large strings or closure-shared DAGs (would `max_string_size` only see the formatted output?) | holds (R) | Rhai charges string bytes inside arrays at `push` (`Length of string too large` for 2 x 1 MiB) and charges closure-shared DAGs by expanded size (`Size of array/BLOB too large` at 2^14). Nothing large reaches the formatter |
| B10 | Rhai default limits differ debug/release (`rhai-1.27.0/src/api/limits.rs:9-38`) | holds (CP) | `build_engine` sets call levels, expr depths, sizes explicitly (`src/runtimes/rhai_rt.rs:122-131`); `unchecked` not enabled |
| B10 | Rhai wasm-gated code (`rhai-1.27.0`, 15 files) | n/a (CP) | Removes i128/u128 functions, file module resolver and stdout print on wasm; no limit is gated |
| B8 / U7 (JS) | Snapshot, `to_json_safe`, `error_string`, console | holds (CP) | Depth/node/byte limits are enforced inside `SNAPSHOT_JS` while walking the input (`src/runtimes/js.rs:41-94, 208-225, 325-340`); thrown values go through the same snapshot. JS BigInt `toString` cost is register #23 (accepted) |
| W8 / S2 (Boa) | Typed-array bounds asserts compiled out in release (`boa_engine-0.22.0/src/builtins/typed_array/builtin.rs:605, 1872, 2156, 2177`; `array_buffer/utils.rs:141, 337`) guarding `unsafe` memmove | holds (R) | Resizable `ArrayBuffer` shrunk inside coercion callbacks of `copyWithin` (start and end), `fill`, `set` (offset, same buffer, different type), `slice` with species, `sort` comparator, `lastIndexOf`, `includes`, `with`, `subarray`, `reverse`, `DataView`: every case re-checks bounds (clean RangeError/TypeError or clamped copy); after growing back no byte beyond the shrunk length was written |
| W8 (Boa) | Other Boa wasm/debug gates (`module/loader`, `sys`, builder slot asserts, intl) | n/a (CP) | Module loading is not reachable (no loader); remaining asserts are construction-time invariants |
| W6 / B6 / LIM20-21 | candid `RecursionDepth` (no-op on wasm, `candid-0.10.40/src/utils.rs:16-31`) in `de.rs`, `types/subtype.rs`, `types/type_env.rs`, `types/value.rs` | not re-reported | All reached through reply decoding (known finding) or through types from a parsed interface, where `candid_parser` overflows first (known); r1-11 measured encode/decode of `opt^n` to 40,000 cleanly |
| W7 / B7 | `candid_parser` / `pest_stack_guard` | not re-reported / n/a | candid_parser known. `pest` (with `pest_stack_guard`, which uses `psm`) is pulled in only by `handlebars`, which no app path calls at runtime |
| U3 / W1 / B1 | serde_json recursion limit (128) on request bodies and JS args | holds (CP) | Checked during parsing (input side). `cargo tree -e features -i serde_json` on wasm: only `default`, `std`; `unbounded_depth` (which removes the guard) is not enabled by any crate |
| E3 / E4 / E5 / U12 / U13 | ic-cdk argument decoding (both canisters) | holds / not re-reported (CP) | r1-11: `skipping_quota(10000)` with the 50x skip charge; inspect_message decode quota is a known finding |
| U8 / U9 / U10 | Reply bytes, reject text, interface text | not re-reported | Known findings (decoder guard, tmp_hack decode, 512 KiB checked after receipt). Management `canister_metadata` reply is a system-built flat `record { value: blob }` |
| U11 / AD7 | Override text parse | not re-reported | candid_parser (known) |
| B11 / W12 / LIM6 | MCP result `serde_json::to_string` then `truncate_escaped` to 1 MB | finding (part of candidate 1) | Escaping and truncation run on the full string after it exists; normally the input is held to 4 MiB by `ValueBudget`, but the Python repr path above hands it 469 MB |
| B16 / W3 | Search pretty-printer and `collect_vars` | not re-reported | Render cost checked after render is a known finding; deep types measured clean by L3 |
| B5 / W5 | `json_to_idl` depth | holds (CP) | `depth > MAX_DEPTH` is checked on entry, before recursion (`src/candid_bridge.rs:195-200`), i.e. on the input |
| B12 / D1-D5 | Stable record decode | n/a | Written only by the app or a controller; flat types |
| W15 / U13 / E21-E23 | demo_store argument handling | n/a | No recursive dependency on its inputs; arithmetic wrap is a known finding |
| F9 / C3 / B14 | web-time shim `Instant + Duration` `expect` | n/a for this pattern | App code, not a compiled-out guard; budget-size issues are known findings |
| S4 / C15 | getrandom custom backend | n/a | Not a guard |
| C5 / C6 / C12 / C13 / C14 | Demo page, scripts, video, audit tooling, idea doc | n/a | Not in the canister; no recursive dependency on untrusted input in the canister path |

## Candidates

1. **Python result and call-argument conversion stringifies values with an unbounded
   `py_repr` and charges the 4 MiB text limit only afterwards, or not at all** (R).
   `src/runtimes/python.rs:71, 84, 92-93, 98`. Probe results (wasm32 release build of the app's
   `python::run`):
   - `class P:\n def __init__(s,x): s.x=x\na=[1]\nfor i in range(N):\n a=[a,a]\nP(a)` (88 bytes):
     N=18 -> OK, 1,835,011-byte result; N=24 -> OK, 117,440,515 bytes (3.5 s); N=26 -> OK,
     469,762,051 bytes (13.8 s); N=29 -> `TRAP RuntimeError: unreachable` (27.8 s, out of
     memory abort). Same with `@dataclass` and inside a dict/list.
   - `1 << 64000000` (13 bytes): OK, 19,265,922-byte decimal string after 58.5 s of wasm;
     `1 << 16000000` 4,816,482 bytes in 7.4 s. The VM refuses `len(str(1 << 1000000))` with
     `Exceeds the limit (4300 digits)`; the host-side conversion has no such limit.
   - Control: the plain list `a` at N=20 is refused (`value has more than 200000 elements`);
     a tuple dict key at N=26 is refused (`more than 4194304 bytes of text`) only after 11.5 s
     building the key's repr.
   Contradicts README:369 / CL10 (4 MiB of text in all three languages). With a 64 MiB Python
   heap, a short script makes the host do work and allocation far past `max_instructions` and
   the heap cap; on the IC this ends in the 40B-instruction or out-of-memory trap (the probe
   reached the OOM trap; the 40B figure was not measured). A run with no host call traps in
   its first message, so it leaves no audit entry. Related but distinct from L7-6 (same
   branch, fidelity) and the known VM-side repr/bigint budget finding (this runs outside the
   VM, after it finished).

No other cell produced a candidate.
