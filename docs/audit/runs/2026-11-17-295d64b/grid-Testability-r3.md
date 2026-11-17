# Coverage grid: Testability (register class), round 3

Run `2026-11-17-295d64b`, third round for this lens. Question: were parsers, converters and
dependency panic sites ever exercised, and what does a fuzz/property campaign against the
shipped artifact find? Rows: F1-F16, W6-W10, B5, B6, U6, U8, S2, register #56.

## Method

1. **Panic sites from the artifact.** The deployed module was taken from the local replica's
   state (`.icp/cache/.../7fffffffffa000020101/software.wasm`, SHA-256
   `224cfa7a…c39eec769`, equal to `icp canister status` module hash). Its data segments were
   scanned for `core::panic::Location` records (wasm32 layout: file ptr, file len, line, col,
   16 bytes) whose pointer/length land on a `.rs` path. Each record is a panic site that
   survived LTO (unwrap, expect, `panic!`/`unreachable!`, bounds check, RefCell borrow,
   `async fn` resumed after completion, capacity overflow). Reachability is not implied.
2. **Native-bytes harness.** No compile was needed. The deployed bytes run in Node 26 under a
   small `ic0` emulator (all 36 imports: stable memory, msg/reply, call_new/perform with
   reply callbacks, trap, performance counter scaled from wall time). The copy differs from the
   deployed module only by one added export (the function table, to invoke reply callbacks)
   and a memory maximum of 49,152 pages (= the canister's 3 GiB `wasm_memory_limit`); code and
   data sections are byte-identical (checked). Setup per instance: `canister_init(null)`,
   `add_api_token_hash`, `set_limits(25, 2e9)`, `set_allowlist(ryjl3…, *)`,
   `set_candid_override` with an 8-method interface (records, variants, recursion, opt opt,
   float32, func/service, vec null, all int widths). A fresh instance follows every trap (no
   rollback in the emulator). The harness reproduces every known trap with the replica's exact
   panic text (Boa repeat/padStart/join `capacity overflow`, Rhai `can't sleep`).
   Scratch files only (session scratchpad `t3/`); nothing in the repository was changed.
3. **Campaign (step budget: about 220k cases).**

   | Target | Cases | Traps (unique roots) |
   |---|---|---|
   | Register seeds (#12-#40 shapes, known traps) | 24 | 4 known + 1 #23 |
   | JS built-in API fuzz (300+ templates x 50 interesting values, 1-4 per script, 3 rounds) | 19,000 | known join/repeat/pad; **at()**; new capacity-overflow sites; #23 timeouts |
   | Python API + scoping/compiler statements (290+ templates, 3 rounds) | 43,000 | **json.dumps indent**; **re.compile heap**; known bigint pow |
   | Rhai API fuzz (3 rounds) | 43,000 | known sleep (+ float variant) |
   | Source-level mutation of all three parsers | 24,000 | known Boa parser recursion only |
   | Candid replies: 30,000 mutated valid encodings + 27 hand-built hostile replies x 8 methods x 3 languages, decoded by `decode_reply`/`idl_to_json`/engine `from_json`; args through `json_to_idl`; echo round trip | 31,596 | 0 |
   | JSON-RPC bodies (mutated ids, batches, nesting, bad UTF-8, BOM, huge queries) on `http_request_update` and `http_request` | 15,000 x 2 | 0 |
   | Interface text through `set_candid_override` -> search render -> `icp.call` | 14,000 (262 parsed) | 0 |
   | Nesting depth probes per parser | 60 | known (JS) |

   Live confirmation (ordinary token-holder requests, one each, no setting changed) on the
   local network: `"abc".at(-1e19)`, `json.dumps(1, indent=2**31)` and
   `Array.prototype.toSorted.call({length: 2**29})` all return HTTP 503 with
   `Canister called ic0.trap` and the same panic text as the emulator.

Evidence key: **E** reproduced on the deployed bytes in the emulator; **L** reproduced on the
local replica; **C** code path; **X** another lens/round.

## Panic sites in the deployed module (per crate)

7,433 `Location` records; 119 in the app's own `src/`, 74 in `vendor/chrono`, 7,240 in
dependencies and std.

| Crate | Sites | | Crate | Sites |
|---|---|---|---|---|
| monty 1.6.0 | 1,810 | | candid 0.10.40 | 78 |
| boa_engine 0.22.0 | 1,483 | | candid_parser 0.4.1 (+ lalrpop `grammar.rs`) | 6 + 145 |
| rhai 1.27.0 | 948 | | regress 0.12.0 (Boa RegExp) | 79 |
| regex-automata / regex-syntax / aho-corasick / fancy-regex (Monty `re`) | 405 / 229 / 245 / 178 | | monty-types 1.6.0 | 38 |
| std / core / alloc | 265 | | boa_ast / boa_gc / boa_string / boa_interner | 30 / 14 / 10 / 6 |
| boa_parser 0.22.0 | 248 | | serde_json 1.0.154 | 19 |
| ic-stable-structures 0.7.2 | 127 | | ruff_python_ast / codegen / literal 0.0.14 | 15 / 5 / 4 |
| num-bigint 0.4.8 / 0.5.1 | 117 / 92 | | ic-cdk / ic-cdk-executor | 10 / 12 |
| ruff_python_parser 0.0.14 | 86 | | smartstring 1.0.1 | 14 |

App sites beyond inventory F: RefCell borrows (F8, about 50 sites), `async fn` resume guards
(12), `json!` macro unwraps (infallible), `truncate`/`truncate_escaped` (cut on a char boundary,
`bridge.rs:153-161`, `mcp.rs:174-187`: hold), `js.rs:331` slice at an ASCII `rfind` (holds),
`candid_bridge.rs:70-71, 222` indexing guarded by length (hold).

## Grid

| Row | Cell | Reason / evidence |
|---|---|---|
| F1 `trap` on bad token hash | not applicable | Admin input at install/upgrade only; rolled back |
| F2 `Encode!().expect` | holds | E: no trap in any admin call or 20k+ audit writes in the emulator; encoding of these records cannot fail for in-memory values |
| F3 `Decode!().expect` | not applicable here | Schema-change risk (L13 / register #51); not reachable by fuzzed input |
| F4 audit `append().expect` | not tested | Needs stable memory exhaustion (emulator caps stable at 256 GiB); L5/L18 own it |
| F5 `candid_bridge.rs:108, 285` unwraps | holds | E: 31,596 replies / argument encodings, 0 traps; guards by length |
| F6 `mcp.rs:256` | holds | E: 30,000 JSON-RPC requests, 0 traps |
| F7 Rhai `Mutex::lock().unwrap()` | holds | E: 43,000 Rhai scripts + 8,000 source mutations; no poisoning (panic aborts first) |
| F8 RefCell borrows | holds | E: no `already borrowed` panic in 220k cases, including reply callbacks, replays and interleaved host calls |
| F9 web-time `Instant + Duration` | not tested | Needs `max_instructions` near `u64::MAX` (admin); X: L9/L11 found it unreachable |
| F10 `policy.rs:222` | holds | Keys validated on insert (C) |
| F11 casts / `f64 as f32` | holds (no trap) | E: float32 inf/NaN/3.4e38 encode and round-trip without trap; fidelity is L7's |
| F12 parser / converter recursion | known (X) + holds for Python and Rhai | E: JS `(`x3000 and `1+`x30000 and `-`x10000 end in a wasm `memory access out of bounds` (8 MiB shadow stack): the reported Boa-parser finding. Python: `(`, `[`, `not`, `-`, `lambda`, `.real`, `[0]` up to 30,000 deep all end in a clean SyntaxError when the host stack is large; Rhai stops at `max_expr_depths`. Whether the replica's native stack (not the 8 MiB wasm stack) is deep enough for Monty's parser at 10,000-30,000 levels is **unverified** (V8's 1 MB default stack overflowed first; a 64 MB stack did not) |
| F13 out of memory | **finding T3-3** (Python) / known (JS #24) | E: Python `re.compile` escapes the 64 MiB Monty limit and exhausts the 3 GiB heap |
| F14 instruction limit | known (#23) | E: 60+ JS cases (`Array.prototype.*.call({length: 2**53-1})`, `BigInt.asUintN(2**26, -1n)`, `BigInt ** BigInt`, `new Uint8Array(2**28)` stringified) run one uninterruptible built-in past the scaled limit; Python bigint `**` (reported) |
| F15 reply size | holds | E: no reply over the limit in 30,000 bodies (X: register #2/#4 guards) |
| F16 demo_store | not applicable | Not the fuzzed component |
| (new) dependency panic sites | **findings T3-1, T3-2, T3-4, T3-6** | E/L, see below |
| W6 `decode_reply` + `idl_to_json` | holds on wasm32 | E: 30,000 mutated + 648 hostile replies (vec null x 2^31, lengths beyond u32, 20k-byte LEB nat/int, self-recursive opt/vec types, record of 2^31 fields, type table of 2^31 entries, variant index > u32, principal > 29 bytes, unknown opcodes, duplicate/unsorted fields) in all three languages: 0 traps, all end in `candid decoding failed` or a value. 32-bit length truncation does not occur (lengths over u32 are refused) |
| W7 `Service::parse` | holds (shallow) | E: 14,000 mutated interfaces (262 parsed, then search render and a call): 0 traps. X: round 1 50,000 natively. Deep nesting: reported (L9-2) |
| W8 JS parse/compile, snapshot, from_json | **finding T3-1, T3-4**; parser recursion known | E/L |
| W9 JS rounds x pending | holds | No trap in any multi-call case; bound is X (SW5-1, T2) |
| W10 Python parse/compile, `to_json`, `from_json` | **finding T3-2, T3-3**; parser holds | E/L. 43,000 statements incl. Monty scope analysis (`nonlocal`, comprehensions in class bodies, walrus, closures over deleted names), `match`, `try/finally` with `break`/`return`, decorators, dunder overrides: no `prepare.rs`/compiler panic; Monty reports these as Python errors or as `RuntimeError: Internal error in monty` (no trap) |
| B5 `json_to_idl` | holds on wasm32 | E: seed encodings from JS/Python/Rhai values for every method; X: round 1 property test natively |
| B6 `idl_to_json` | holds on wasm32 | E: as W6 |
| U6 script source | **findings T3-1..T3-4, T3-7** | E/L |
| U8 reply bytes | holds | E: as W6 |
| S2 direct dependencies | **finding T3-6** | Panic sites listed above were never inventoried; ruff_python_parser 0.0.14 (pinned by monty) has 86 sites and produced no trap in 51,000 Python sources (43,000 generated, 8,000 source mutations) |
| #56 no fuzz/property tests | still open; **finding T3-5** | The campaign above lives in scratch files; the repository has no harness that runs the Wasm |

## Candidate findings

**T3-1 (High). `String.prototype.at` with an index at or below -2^63 traps the canister.**
`"abc".at(-1e19)` (also `-9.3e18`, `-(2**63)`). Boa `builtins/string/mod.rs:541`:
`Integer(i) if i < 0 && (-i) <= len` negates `i64::MIN`, which wraps in release builds, so the
guard passes and `(len + i) as usize` is out of range; `s.get_expect(k..=k)` panics
`Unexpected get()` (`boa_string-0.22.0/src/lib.rs:466`). E and L (HTTP 503, `ic0.trap`).
`Array.prototype.at` and `%TypedArray%.at` use a different guard and return `undefined` (E).

**T3-2 (High). Python `json.dumps(x, indent=n)` with n >= 2^31 traps.**
`import json\njson.dumps(1, indent=2**31)`. Monty `modules/json/dump.rs:365-372` builds
`" ".repeat(count)` outside its resource tracker; on wasm32 the byte count exceeds `isize::MAX`
and `alloc::raw_vec` panics `capacity overflow`. `indent=2**28` ends in a clean
`MemoryError`; every other width/precision path probed (`format`, f-strings, `%`, `center`,
`zfill`, `bytes`, `itertools`, `decimal`) is guarded. E and L.

**T3-3 (High). Python `re.compile` bypasses the 64 MiB memory limit and exhausts the canister
heap.** `import re\nre.compile("12345678901234567890" * 2**20)` (a 20 MiB string, inside the
64 MiB limit) ends in `unreachable` after wasm memory reaches the 3 GiB cap (allocation failure
abort), in 0.24 s. `"a" * 2**24` does the same. A 5 MiB pattern returns a clean `PatternError`
but grows wasm memory from 10 MiB to 1.18 GiB first, and a non-trapping message keeps that
memory (wasm memory does not shrink). README:371 and :451 state Python is limited to 64 MiB.
E only; not sent to the shared replica to avoid pushing its heap to 3 GiB.

**T3-4 (High, variant of the reported Boa capacity-overflow finding; merge).** Sites the report
(`repeat`, `padStart/padEnd`, `join`) does not name, each panicking `capacity overflow` from a
length the script chooses: `Array.prototype.toSorted.call({length: 2**29})` (4 ms; E and L),
`String.raw({raw: {length: 2**28}})` (E, 11 s), `%TypedArray%.prototype.join` via
`String(new Uint8Array(2**29))` (E, 28 s), `toReversed`/`toSpliced`/`with`/`Array.from` on a
2^31 array-like (`thin-vec` capacity overflow, E, 37 s; on the replica the 40e9 instruction
limit may trap first, register #23). A fix that wraps only the three named built-ins leaves
these.

**T3-5 (Medium, testability). The test suite cannot observe wasm32-only failures, and nothing
runs the built Wasm.** Native `cargo test` runs on a 64-bit host with no 3 GiB heap: T3-2,
T3-3, T3-4 and the reported repeat/pad/join traps cannot fail there (2 GiB allocations succeed).
This round shows a harness of about 200 lines (Node + `ic0` stub over the release Wasm,
table export and memory maximum added) runs about 10,000 scripts a minute and reproduces every
known trap with the replica's panic text. Fix: keep that harness and a corpus (register seeds,
this run's traps) in the repository and run it on the release artifact in CI; it also closes
most of #54/#58 for the no-network paths.

**T3-6 (Low, method). The failure-site inventory is derived from the app's own `unwrap`/`expect`
and misses about 98% of the shipped panic sites.** The artifact carries 7,433 panic locations;
inventory F lists about 20, all in `src/`. T3-1..T3-4 sit in dependency code that no lens
listed. Fix (a PLAN method change, per section 13): derive Failure sites from the artifact
(Location records per crate) and give each high-count crate a fuzz target.

**T3-7 (High, variant of the reported Rhai `sleep` finding; merge).** `sleep(1e308)` panics
`cannot convert float seconds to Duration` at `core/src/time.rs:1050` before reaching the
reported `can't sleep` site; `sleep(0.5)` reaches the reported one; `sleep(NaN)`, negative and
zero values return. E. A fix that guards only the thread-sleep call leaves this path.

## Not tested, and why

- Exact instruction counts: the emulator scales wall time (2 instructions/ns), so "limit" and
  timeout outcomes are #23-class indications, not measurements.
- Replica native stack depth for Monty's parser at 10k-30k nesting (see F12).
- `re.compile` and the slow T3-4 sites on the replica (heap and time cost on a shared network).
- F4 (stable memory exhaustion), F9 (admin-set limits), F16 (demo_store).
- Monty `re` matching (catastrophic backtracking in `fancy-regex`) was not fuzzed beyond
  compile; Boa `regress` backtracking is #23-class.
- Concurrency of traps with in-flight runs (no network calls are interleaved in the emulator
  beyond sequential reply delivery).
