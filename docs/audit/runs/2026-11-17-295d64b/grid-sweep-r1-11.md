# Variant sweep r1-11: dependency recursion the app's limits do not bound

Run `2026-11-17-295d64b`, Phase 6. Pattern under test:

> A recursion inside a dependency (parser or built-in) that no app limit bounds, where the
> app bounds only bytes, element counts or interpreter-level depth.

Already known and not re-reported here: Boa parser on deep parens/brackets, Boa built-ins on
deep data, candid_parser on nested interfaces, candid's decoder on deep replies (depth guard
is a no-op on wasm), the tmp_hack decode without quota, json_to_idl cost, Rhai deep arrays
(not reachable under default limits).

Result: **no new candidate.** Every other dependency recursion reachable from an untrusted
input either has its own depth limit or is stopped by an app limit or quota first.

## How it was tested

- Private copy of the tree (no `target/`, `design/`, `docs/per-customer-canisters-plan.md`,
  `video/`, `.env`) in `/private/tmp/claude-501/sweep11.*`, shared
  `CARGO_TARGET_DIR=/private/tmp/claude-501/audit-target`. A probe `cdylib` (workspace member
  in the copy only) depends on `icp_code_mode` with default features and exports
  `run_js`, `run_py` (the app's own `runtimes::js::run` / `runtimes::python::run`, polled once
  with a no-op waker and a bridge that refuses calls) and `cand` (the app's
  `candid_bridge::Service::parse`, `encode_args`, `decode_reply`).
- Built `--release --target wasm32-unknown-unknown` with the repo's `.cargo/config.toml`
  (verified in the artifact: `__stack_pointer` init 8,388,608, first data segment at
  8,388,608, so stack-first and an overflow traps). Run in Node 26 inside a Worker with a
  1,000 MB V8 stack and all 36 `ic0` imports stubbed, a fresh instance per case. A trap
  would show as `TRAP`; every case below ended in `OK` or a clean `ERR`.
- No live canister was touched. Evidence for each cell: R = reproduced in the wasm probe,
  CP = the code path settles it.

## Cells

| Row | Dependency recursion | Cell | Reason |
|---|---|---|---|
| W8 / LIM24 (JS) | Boa re-entering `Context::run` from native code (callbacks, accessors, traps, coercions) | holds (R) | Boa counts `host_call_depth` together with frames against the recursion limit of 256 (`boa_engine-0.22.0/src/vm/mod.rs:1016-1030`). At depths 100, 200 and 255 every re-entry path ended in a clean `RuntimeLimitError` or `OK`, never a trap: `Array.prototype.map` callback, getter, Proxy `get` trap, `toString` and `valueOf` coercion, `sort` comparator, `String.prototype.replace` callback, `Function.prototype.apply`, `Reflect.construct`, generator through `Array.from`, `toJSON` through `JSON.stringify`, Promise executor |
| W8 (JS) | regress, the RegExp engine (parser, IR passes, backtracker) | holds (R) | `regress-0.12.0/src/types.rs:23` `MAX_NESTING_DEPTH = 256`. 255 nested groups, nested `(?:..)*`, nested lookaheads, `v`-mode nested classes and 255 named groups compile and run; 256 groups give `SyntaxError: ... too deeply nested` |
| W8 / B8 (JS) | `JSON.parse` | holds (R, CP) | Boa validates with `serde_json::from_str` first (`boa_engine-0.22.0/src/builtins/json/mod.rs:250`), so serde_json's 128-level limit refuses deep text before Boa's parser sees it; the reviver tree walk uses an explicit stack (`:75-164`) |
| W8 (JS) | Dynamic code (`Function` constructor, `eval`) built from runtime strings | holds (R) | `Function` constructor: `EvalError: dynamic code evaluation is disabled`; `eval` is undefined. No route to the Boa parser other than the script source (known finding) |
| U7 / B8 (JS host side) | `error_string` -> `JsError::try_native` and `JsNativeError` Display on a thrown error | holds (R, CP) | `try_native` reads one level (`cause` stays opaque, `boa_engine-0.22.0/src/error/mod.rs:557-650`), Display does not follow `cause`. A thrown `Error` with a 100,000-deep `cause` chain and an `AggregateError` of 100,000 errors both end in a clean `ERR` |
| V8 (JS) | `boa_gc` marking of deep object graphs (`force_collect` after every run) | holds (R, CP) | Tracing uses a queue (`boa_gc-0.22.0/src/lib.rs:234, 280-318`), not recursion; the 100,000-deep `cause` chain above was collected after the run without a trap |
| W10 / LIM26 (Python) | ruff_python_parser on sources under Monty's 4 KiB scan threshold | holds (R) | Monty skips its pre-parse nesting scan for sources of at most 4 KiB (`monty-types-1.6.0/src/run_options.rs:6-13`) and relies on ruff's `stacker::maybe_grow`, which cannot grow the stack on wasm (`ruff_python_parser-0.0.14/src/parser/mod.rs:176`). Measured anyway at the threshold: 4,095 unary minus, 4,095 `~`, 4,095 `+`, 2,048 parens, 2,047 brackets, 1,023 `not`, 1,365 `**`: all clean `SyntaxError`-style tracebacks, no trap. Over the threshold (5,000 unary minus) the scan refuses cleanly. L3's 50k/100k cases cover the scanned path |
| W10 (Python) | Monty heap -> host object export, `json`, `re`, `tomllib`, `copy` | holds (CP) | Export takes a VM recursion guard per level (`monty-1.6.0/src/object_bridge.rs:183-187`); `json.loads` limit 200 (`modules/json/load.rs:57, 440`), `json.dumps` takes recursion tokens (`dump.rs:551, 580`); `re` translator `MAX_NESTING = 64` (`types/re_syntax.rs:39`) and fancy-regex `MAX_RECURSION = 64` (`fancy-regex-0.20.2/src/lib.rs:147`, `parse.rs:1027`); `tomllib` builds with an explicit stack (`modules/tomllib/value.rs:5-6`). All are tied to the interpreter recursion limit or a fixed depth, not to bytes |
| W11 / B10 (Rhai) | Rhai data recursion (`to_string`, `==`, clone, drop) | not re-reported | Known finding (Rhai variant not reachable under default limits). `to_dynamic` input is a reply at most 128 deep; `to_json_value` is the app's bounded walk |
| W5 / B5 / W6 (candid) | candid encoder (`IDLArgs::to_bytes_with_types`, type table build) and decoder on deep interface types that `candid_parser` accepts | holds (R) | Method `f : (opt^n nat) -> (opt^n nat)`: parse, `encode_args` and `decode_reply` succeed for n = 1,000, 5,000 and 10,000; for n = 20,000 to 40,000 parse and encode succeed and decode fails cleanly (`candid decoding failed: Cannot parse header`). 1,000 to 20,000 nested records parse and fail encoding cleanly. Nothing traps below the depth where `candid_parser` itself overflows (known finding) |
| B7 / W3 (search) | candid pretty-printer in `search::render` | holds | L3 measured 30,000-deep types rendering without a trap; not repeated |
| E3 / U12 / B2 | `candid::decode_one::<HttpUpdateRequest>` in `inspect_message` (default `DecoderConfig`: no decoding or skipping quota; candid's depth guard is a no-op on wasm) | holds: no impact (CP) | An extra record field holding a deeply nested `opt` would be skipped recursively and can overflow the stack, but a trap in `inspect_message` only rejects the ingress message, which is the same outcome as refusing it; `inspect_message` is not charged (`src/lib.rs:265-272`), no state changes, and a well-formed gateway request cannot carry the extra field. Not a finding |
| E4 / E5 / E6-E14 / E20-E23 / U13 | ic-cdk argument decoding of every method in both canisters | holds (CP) | ic-cdk 0.20.3's generated entry points set `skipping_quota(10000)` (`ic-cdk-macros-0.20.3/src/export.rs:258-260`), and skipped values are charged 50x (`candid-0.10.40/src/de.rs:562-585`), so a deep skipped field is refused after about 200 levels; the expected types (`HttpRequest`, `HttpUpdateRequest`, admin args, demo_store args) have no deep typed path |
| W1 / U3 | serde_json on the request body and on JS call arguments | holds (CP) | serde_json's default recursion limit of 128 applies to `parse_body` and `js.rs:395`; nothing deeper reaches a recursive `Value` walk |
| W12 / B11 | `serde_json::to_string` of results and replies | holds (CP) | Values are built by bounded walks (snapshot, `to_json`, `idl_to_json` at most 128 deep) |
| B12 / D1-D5 | Candid `Decode!` of stable records | n/a | Written only by the app or a controller, flat record types |
| B17 / W16 / C5 | Demo page (browser JSON parsing and regex) | n/a | Runs in the user's browser on replies the canister built at most 128 deep; the regex cost issue is a known finding |
| C12 / C6 / C13 | Video tooling, setup script, audit tooling | n/a | Not deployed; no untrusted recursive input reaches a dependency |
| App-level `is_array_like` (`src/candid_bridge.rs:117-124`) | recursion over nested `opt` with no depth bound | holds (R) | App code, not a dependency. `encode_args("f", [null])` (the only path that calls it) on `f : (opt^n nat) -> (nat)` for n = 10,000, 20,000 and 40,000, which `candid_parser` parses: all encode, no trap |

## Candidates

None.
