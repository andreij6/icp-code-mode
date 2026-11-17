# Coverage grid: L3 Resource bounds, round 1

Run `2026-11-17-295d64b`. Row ids refer to `inventory.md`. Cells: **finding** (candidate id
below), **holds** (why), **n/a** (why). Candidates are numbered L3-1 ... L3-8 and are
described in the lens's returned findings.

## How it was tested

- Private copy of the tree (no `target/`), shared `CARGO_TARGET_DIR`. Native probes as
  `#[cfg(test)]` tests built with `cargo test --release` (profile `opt-level="z"`, LTO), each
  heavy case run on a thread with an 8 MiB stack; instruction counts are macOS
  `/usr/bin/time -l` "instructions retired" for the whole test process (native aarch64, a
  proxy for wasm instructions, not an on-chain count).
- A small wasm probe crate (`cdylib` depending on `icp_code_mode`, same `.cargo/config.toml`
  rustflags including `-zstack-size=8388608`, same release profile) run in Node with stubbed
  `ic0` imports and a large V8/OS stack, so a trap is the 8 MiB shadow stack running out
  ("memory access out of bounds"). This matters because candid's recursion guard is a
  no-op on wasm (`candid-0.10.40/src/utils.rs:12-32`) and native-only tests cannot see it.
- No live canister was touched.

## Limits

| Row | Cell | Reason |
|---|---|---|
| LIM6 MAX_RESULT_BYTES | holds | `truncate_escaped` counts escaped bytes before the cut; guard test `result_size_is_bounded_after_escaping` passes |
| LIM7 MAX_REPLY_BYTES | holds | `finish` and `http_update` re-measure the serialized reply; `replies_always_fit` passes |
| LIM8 MAX_BATCH | holds | Checked after `serde_json` parses the whole body, but the body is bounded by the ingress limit (~2 MiB) and serde_json's recursion limit (128); cost linear |
| LIM9 MAX_SEARCHES | holds | Counted before `exec` in `http_update`; total keyword bytes still bounded only by the body (see L3-7) |
| LIM10 one script / request | holds | `ran_script` checked before `exec` |
| LIM11 rpc error 300 B | holds | `rpc_error` truncates every message |
| LIM12 MAX_CONCURRENT_RUNS | holds for scripts | `RunSlot` before any work; searches take no slot (L4/L16 question, not a resource-bound defect here: a search holds no VM heap) |
| LIM13 MAX_AUDIT_ERROR_BYTES | holds | `bridge::truncate` before every `append_audit` of `error` (run_script, run_search); the `Trapped` error is a constant |
| LIM14 MAX_VIOLATIONS/BYTES | holds | `IcBridge::deny` truncates and counts overflow |
| LIM15 MAX_LOG_BYTES | holds | JS checks the cap before formatting, Rhai truncates in `RunState::log`, Monty `CollectString(_, Some(64 KiB))`. One JS `console.log` of a huge string copies it before truncation: JS memory, register #24 |
| LIM16 value limits | holds (results/args) / finding L3-3 | Depth/nodes/bytes enforced in all three runtimes (guard tests pass), but per-node conversion cost is not bounded: `json_to_idl` cost is nodes x variant tags / record fields |
| LIM17 MAX_METHOD_LEN | holds | Checked first in `IcBridge::call` |
| LIM18 TTLs | holds | DID_CACHE / DID_FAILED keyed by allowlisted principal, so bounded by allowlist size |
| LIM19 MAX_DID_BYTES | finding L3-2 | Bounds bytes only; 512 KiB of Candid still makes `candid_parser` overflow the stack (nested types) or spend >40B instructions (alias chains, nested records) |
| LIM20 DECODING_QUOTA | finding L3-5 | IDLValue decoding is "untyped", so candid multiplies every cost by 50: effective quota ~400k elements, so a valid blob reply over ~350-400 KB is refused (2 MiB allowed on the wire) |
| LIM21 MAX_DEPTH | finding L3-4 | Applied by `idl_to_json` only after candid has fully decoded the reply; candid's own depth guard is a no-op on wasm |
| LIM22 MAX_SAFE_INT | n/a | Precision, not a resource bound (L7) |
| LIM23 MAX_CODE_BYTES | finding L3-1 | 100 KiB bounds bytes, not parser nesting: 800 bytes of JS traps |
| LIM24 JS limits | finding L3-1 | Loop/recursion/stack limits are interpreter limits; Boa's parser and several built-ins recurse natively outside them |
| LIM25 JS rounds/pending/jobs | holds | Guard tests pass; `request_fn` checks budget before snapshotting args |
| LIM26 Python limits | holds | Wasm probe: deep source (50k parens/brackets, 100k unary minus, 25k-term sum) and deep runtime values (`str`, `==`, `repr`, `hash` on 100k-deep lists/dicts/tuples) all end in clean Python exceptions |
| LIM27 Rhai limits | holds for parse / finding L3-1 (runtime) | Expr-depth limit refuses 50k-deep source cleanly; arrays nested 9,000 deep (under the 10,000 size limit) trap in `to_string()` and `==` |
| LIM28 8 MiB stack | finding L3-1, L3-2, L3-4 | Not enough for the inputs above |
| LIM29 3 GiB memory | holds/accepted | JS unbounded (register #24); Rhai/Python bounded (see W11, LIM26) |
| LIM30 platform limits | n/a | Platform; relied on by LIM7/F14 |
| LIM31 80 B id truncation | holds | `parse_principal` truncates the echo |
| LIM32 MAX_METHODS | finding L3-7 (Low) | Caps output only; every allowed method is pretty-printed and matched against every keyword |
| (missing kind) MAX_AUDIT_PAGE by count only | finding L3-8 (Low) | Page bounded to 100 entries, not bytes; entry size grows with `max_calls_per_script` |

## Unbounded work

| Row | Cell | Reason |
|---|---|---|
| W1 parse_body | holds | Ingress-bounded, linear, depth 128 |
| W2 inspect_message decode + SHA-256 | holds | Ingress-bounded; a trap there only rejects the message (free) |
| W3 search render | finding L3-7 (Low) | 1M keywords x demo_store (10 methods): 2.4B native instructions per search, no limit on keyword count; scales with allowed methods |
| W4 list_allowed copy | holds | Admin-sized |
| W5 json_to_idl | finding L3-3 | Variant tag lookup allocates two strings per tag per value; record key checks are keys x fields |
| W6 decode_reply + idl_to_json | finding L3-4, L3-5 | Deep reply traps (20,000 levels, ~20 KB); quota refuses large valid replies |
| W7 Service::parse | finding L3-2 | Stack overflow and super-linear cost within 512 KiB |
| W8 JS parse / snapshot / console / from_json | finding L3-1 (parse, built-ins); holds (snapshot, from_json: depth <= 128) | |
| W9 JS rounds x pending | holds | Budget checked in `request_fn` and `reserve` |
| W10 Python | holds | See LIM26; host-side `queued` args are counted by the allocator against 64 MiB |
| W11 Rhai answers retention (note 5) | holds | Measured: a reply that does not fit Rhai's size limits (2M-element array) ends the run at once with an uncatchable "Size of array/BLOB too large", so retained answers are each within Rhai's limits and at most 64 (25 calls by default) |
| W12 truncate_escaped / re-serialize | holds | Linear in bounded results |
| W13 audit_page | finding L3-8 (Low) | See above |
| W14 PENDING_RUNS / audit growth | n/a here | Register #31 (L5); PENDING_RUNS bounded by in-flight requests |
| W15 demo_store orders | finding (Low, demo only) | Anyone can grow `orders`; `list_orders(null)` returns all of them; demo canister by design |
| W16 demo page | n/a | Browser-side, input bounded by the 1.9 MB reply |

## Untrusted inputs

| Row | Cell | Reason |
|---|---|---|
| U3 body | holds | See W1 |
| U5 tool args | finding L3-7 (query keywords unbounded); holds for `code` (100 KiB), `canister_id` (linear parse) | |
| U6 script source | finding L3-1 | |
| U7 script values | holds for size; finding L3-3 for conversion cost | |
| U8 reply bytes | finding L3-4, L3-5 | |
| U10 interface text | finding L3-2 | |

## Failure sites

| Row | Cell | Reason |
|---|---|---|
| F12 native stack overflow | finding L3-1, L3-2, L3-4 | Reproduced in wasm: Boa parser (400 nested parens), Boa built-ins (String/flat/Proxy/prototype chains/bind/compare on 100k-deep data), Rhai `to_string`/`==` (9,000-deep arrays), candid_parser (50,000 nested `opt`), candid decoder (20,000-deep reply). Holds: Monty, Rhai parser, `search::render`/`collect_vars` (30,000-deep type renders fine) |
| F13 out of memory | holds/accepted | JS register #24; Rhai/Python bounded (W11, LIM26) |
| F14 40B instruction limit | finding L3-2, L3-3 | Besides register #23: `encode_args` (100 tags x 200k values: 50B native instr) and `Service::parse` (16k aliases: 390B; 10k nested records: 126B) are uninterruptible |

## Boundary conversions

| Row | Cell | Reason |
|---|---|---|
| B5 json_to_idl | finding L3-3 | |
| B6 idl_to_json | finding L3-4 (depth checked after decode) | |
| B7 .did -> TypeEnv | finding L3-2 | |
| B8 JS <-> JSON | holds | Snapshot bounds depth/nodes/bytes; `from_json` input depth <= 128 |
| B9 Python <-> JSON | holds | Bounded `to_json`; `from_json` input depth <= 128 |
| B10 Rhai <-> JSON | holds | Bounded `to_json_value`; `to_dynamic` results size-checked by Rhai |

## Inventory notes and register

| Item | Cell | Reason |
|---|---|---|
| Note 5 (Rhai answers) | holds | See W11 |
| Note 6 (search keywords, free query path) | finding L3-7 (Low) | Bounded per request by the query instruction limit (query) and 40B (update; reachable directly by any token holder); no failure demonstrated with the demo allowlist |
| Parser recursion: Boa | finding L3-1 | |
| Parser recursion: Monty | holds | |
| Parser recursion: candid_parser | finding L3-2 | |
| #12 | holds | Guard test passes; but see L3-1 for other native recursion in Boa |
| #13 | holds | Guard test passes |
| #14 | holds | Guard tests pass |
| #15 | variant found | Deep Rhai arrays now also overflow the stack (L3-1) |
| #16 | holds | Truncation before append (no test) |
| #17 | holds | Skipping quota and `set_full_error_message(false)` in `decode_reply` |
| #18-#21 | holds | Guard tests pass |
| #22 | holds for size, but L3-2 | Size check does not bound parse cost |
| #23 | still accepted; README claim "Every other path stops with a clean error" is false (L3-1, L3-2, L3-3, L3-4) | |
| #24 | still accepted | 30 replies of 2M elements: 381 MB live peak in one JS run (native) |
