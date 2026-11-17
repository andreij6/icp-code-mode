# boa_engine 0.22.0, patched for icp-code-mode

Copied from crates.io `boa_engine-0.22.0` (Unlicense OR MIT) without `tests/`,
`benches/` and `examples/`, and used through `[patch.crates-io]` in the root
`Cargo.toml`. Every change is marked `// icp-code-mode patch` in the source. All of
them stop work that would otherwise run until the IC traps the message (instruction
limit, heap ceiling, stack overflow or a panic under `panic = "abort"`).

| File | Change | Why |
|---|---|---|
| `src/context/mod.rs` | `ContextBuilder::interrupt(Rc<dyn Fn() -> bool>)` and the `Context` field behind it (`fuzz` feature) | The host checks its instruction deadline and per-run memory cap from inside the VM. |
| `src/vm/mod.rs` (`execute_one`) | Every 64 steps, call the interrupt; `true` throws the uncatchable `NoInstructionsRemain`, as running out of `instructions_remaining` does | One script turn could run to the 40B-instruction trap between host calls. |
| `src/context/mod.rs` | `Context::set_stack_floor` and `Context::host_check`: below the floor address a `RangeError` ("native stack limit exceeded"); every 4096 checks, poll the interrupt | Built-ins recurse natively on deep data (prototype, Proxy and array nesting), and native loops (`Array.prototype.join.call({length: 2**31})`) run without VM steps. |
| `src/object/internal_methods/mod.rs` | `host_check` in the `[[GetOwnProperty]]`, `[[HasProperty]]`, `[[Get]]` (and its `try_get` variant), `[[Set]]`, `[[Delete]]` and `[[OwnPropertyKeys]]` dispatchers | Every native loop and recursion over objects goes through them. `[[DefineOwnProperty]]` and the prototype/extensibility methods are left out: Boa calls them with `expect` in many places. |
| `src/object/operations.rs` (`JsObject::call`) | `host_check` | Native-to-JS call recursion (`toString` inside `join`, comparisons). |
| `src/builtins/iterable/async_from_sync_iterator.rs` | `Get(next)` no longer `expect`s success | `host_check` can fail any `[[Get]]`. |
| `src/builtins/proxy/mod.rs` (`Proxy::create`) | Refuse a target chain longer than 1000 proxies | Traps that skip `host_check` (`defineProperty`, `isExtensible`, ...) recurse through the chain. |
| `src/builtins/array/mod.rs` (`sort_indexed_properties`) | Stable bottom-up merge sort instead of `slice::sort_by` | `sort_by` panics when the comparator is not a total order. Covers `Array.prototype.sort`/`toSorted` and the `%TypedArray%` ones. |
| `src/builtins/array/mod.rs`, `src/builtins/typed_array/builtin.rs` | `Vec::with_capacity` from an array-like length is capped at 2^16 (join, toLocaleString, SortIndexedProperties) | The length is script-chosen (up to 2^53); pre-sizing panicked with "capacity overflow". |

To upgrade Boa, re-apply these on the new version (search for `icp-code-mode patch`)
or drop the ones upstream has made unnecessary.
