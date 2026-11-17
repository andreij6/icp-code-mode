# Coverage grid: Testability (register class), round 1

Run `2026-11-17-295d64b`. Lens question (register class "Testability", an extra lens per
PLAN.md section 6): can the behaviour of this row be checked by the project's tooling, and if
not, which harness is missing? Rows are the "no automated test" rows of `inventory.md`
section 17 plus register #54-#58.

Method: code reading; native probe tests `p1`-`p10` in `src/audit_testability.rs` of a
private copy (`git archive HEAD`, package version suffixed so its artifacts do not collide
with other agents' copies in the shared `CARGO_TARGET_DIR`; copy deleted afterwards);
dev-profile wasm32 builds of every feature subset from that copy, imports listed with
`wasm-objdump`; `diff -r` of `vendor/chrono` against the registry's chrono 0.4.45. No
request was sent to the local network by this lens (inspect_message, admin guards and the
budget were already exercised live by L1, L5 and L6; not repeated).

Evidence key: **R** reproduced natively (probe named), **C** code path only, **X** reproduced
by another lens this run (id given).

Candidate findings of this lens: **T-1** .. **T-4** (end of file).

| Row | Cell | Reason / evidence |
|---|---|---|
| K1-K7 `IcBridge` commit points (register #58) | finding (#58 still open; scope corrected) + **T-1** | R p1-p4, p6: the production bridge *can* be driven natively up to the send: method-length check, principal parse, `reserve` (allowlist, cap), `deny`, override lookup, encoding and the composite_query refusal all run. It stops at the first platform read: the send (`cost_call should only be called inside canisters`), any cache/TTL access (`time should only be called inside canisters`), and script hosting for JS and Rhai (`canister_self_size ...` at engine setup; Python runs). So `record_message`, `on_first_pending` with a real call, `Trapped` in `Drop`, `PENDING_RUNS` and every audit entry stay untestable. Register #58 wording "cannot drive the production bridge" is too broad; the gap is the missing seam for clock, counter, self id and the call primitive |
| E3 `inspect_message` | gap (covered by #54) | Calls `msg_method_name`/`msg_arg_data`/`accept_message` directly: no native seam. Behaviour holds live (X: L1 six refusals, L5). No automated test (I18 gap stands) |
| E6-E14 admin guards | gap (covered by #54) | `guard = "controller"` exists only in the macro-generated export, so a native call of the function bypasses it: a native test cannot exercise the guard at all. Holds live (X: L1). The bodies are natively testable (R p4: `set_candid_override` size/parse) |
| E5b `run_search` update path | gap (#58 + T-1) | Natively testable only when every scoped canister has an override (no fetch, no `time()`); any fetch stops at `time()`/`cost_call` (R p4). Behaviour covered live by L4/L6 |
| LIM13 `MAX_AUDIT_ERROR_BYTES` | **finding T-4** (+ T-3) | R p3: `truncate(s, 4096)` keeps 4096 bytes and appends `…[truncated]` (14 B): 4,109-4,110 B stored. README "error (≤ 4 KiB)". Char-boundary cut holds (no split UTF-8). No test |
| LIM14 `MAX_VIOLATIONS` / `MAX_VIOLATION_BYTES` | **finding T-4** (+ T-3) | R p2: 17 denials keep 16 and count 1 (holds); a 513 B violation is stored as 526 B; `run_script`/`run_search` then append a 17th entry `…and n more` (`src/lib.rs:156-158, 233-235`, C). README "violations (≤ 16, each ≤ 512 B)". No test |
| LIM17 `MAX_METHOD_LEN` | holds (+ T-3) | R p1: 256 B passes to policy, 257 B refused, 129 x `é` (258 B) refused: bytes, not chars, before any send. No test in the suite |
| LIM18 `DID_CACHE_TTL_NS` / `DID_FAIL_TTL_NS` | **finding T-1** | `cache_get`/`failed_get` read `ic_cdk::api::time()` directly; R p4 shows a native test dies there. Expiry semantics are L9-5/L9-7 (X); register #45/#46 guards "none" cannot become tests without a clock seam |
| LIM19 `MAX_DID_BYTES` | holds at the boundary (+ T-3) | R p4: override of exactly 524,288 B accepted, 524,289 B refused with the documented message, unparsable text refused. Depth inside the limit is L9-2 (X). The fetched-interface size check (`bridge.rs:490-493`) sits after the fetch: untestable natively (T-1/#58) |
| composite_query refusal | holds (+ T-3) | R p4: `IcBridge::call` refuses a `composite_query` method before any message, through an override. Untested in the suite although reachable natively today |
| LIM15 Python print cap | finding (behaviour X: L6-4, L7-8) + **T-3** | Python raises `MemoryError` at the cap where JS/Rhai truncate. Testable natively with `MockBridge` today (L6/L7 did); no test in the suite (I16 gap) |
| I13 instruction budget | **finding T-1** | R p5: with `instruction_deadline() == 0` (budget already spent) a 200,000-iteration loop plus one host call succeeds in JS, Python and Rhai (`19999900000`, 3 calls), and `check_call` admits a call with `max_instructions = 0`, because `bridge::instruction_counter()` is 0 off-chain. Python also takes a different branch natively (`vm_budget` = 2 s wall time, `python.rs:24-43`; std `Instant` instead of the shim). Every budget path is dead code in tests; this is how L6-1 and L6-2 (High) and register #32 (guard manual) stayed untested |
| C3 web-time shim | holds (C; X: L9, L11) / gap (#54) | The wasm32 `Instant` is `cfg(target_arch = "wasm32")` and is never compiled by `cargo test`; natively the crate re-exports `std::time`. No wasm runtime in the toolchain (`wasmtime`, PocketIC absent). F9 unreachable per L9/L11 |
| C4 vendored chrono (tests removed) | holds | R: `diff -r` of `vendor/chrono/src` against registry `chrono-0.4.45/src` is empty; manifest differs only in the header comment, `wasmbind` dropped from `default`, and three `[[test]]` targets (`dateutils`: shells out to `date`; `wasm`: needs `wasmbind`; `win_bindings`: Windows). No code delta to test. Its unit tests cannot run here (`cargo test -p chrono`: "requires dev-dependencies and is not a member of the workspace"), which matters only if the vendored source is ever edited |
| C2 demo_store writes (E21-E23) | gap (covered by #54) | `create_order`'s success path calls `ic_cdk::api::time()` (`demo_store/src/lib.rs:305`), so it panics natively; error paths, `cancel_order`, `restock` are natively testable but untested. Behaviour findings L1-5, L5-5/6 (X) |
| C1a / S9 feature-subset links and Wasm imports | holds today (R) / **finding T-2** | R: dev-profile `wasm32-unknown-unknown` builds of `--no-default-features` with none, `js`, `python`, `rhai`, `js,rhai`, `python,rhai`, `js,python` all link; imports are only `ic0.*` (34 for none, 36 otherwise; 0 others). Release/LTO artifacts of subsets not built (run constraint), but they link a subset of the same code. Nothing in the repository checks this (T-2) |
| S5 / I24 reproducibility | not tested here | Needs two clean release builds of one commit (`ahash/compile-time-rng` predicts a mismatch); left to L15 / Live. No check exists in the repository (T-2) |
| T2 `.did` match tests | holds (R baseline) / T-2 | The only guard against interface drift (register #51 class) runs only when someone runs `cargo test` by hand |
| B5 `json_to_idl` | holds (R p7, p8) | 200,000 random JSON values against 5 methods (records, opt blob, tuples, recursive variant, float32, func/service, null/reserved): 0 panics. Type-directed property: 30,000 values, `encode(decode(encode(x))) == encode(x)` byte-for-byte, 0 failures. Value-level fidelity findings are L7's (X: L7-3, L7-5, L7-11..L7-15) |
| B6 `decode_reply` / `idl_to_json` | holds (R p7) | 200,000 mutated replies (byte flips, inserts, deletes, truncation): 2,268 decoded, the rest refused, 0 panics |
| B7 `Service::parse` | holds for shallow input (R p7) | 50,000 mutated interfaces: 3,506 parsed, encoding random values against every method of each mutated service: 0 panics. Deep nesting excluded (stack overflow is an abort, L9-2) |
| B8 / B9 / B10 runtime converters | holds for the reply domain (R p9, p10) | Differential + property: 600 random replies in the shape `idl_to_json` emits (safe ints, finite floats, unicode/escape strings, `__proto__`/`constructor`/numeric keys, empty containers, nesting) returned unchanged by JS, Python and Rhai: 0 mismatches, 0 errors; harness detects a change (p10: JS turns `u64::MAX` into `1.8446744073709552e19`). Script-produced values: L6-3, L6-9, L7-2, L7-4, L7-6, L7-7 (X) |
| #54 no local-network integration tests | still open | R: `git ls-files` has no integration test, harness script or PocketIC dependency; `scripts/` holds only `demo-setup.sh` |
| #55 no misbehaving target | still open | C: `demo_store` is the only target and always replies synchronously; nothing never replies, replies late, oversizes, traps or lies |
| #56 no fuzz/property tests | still open | C: no proptest/quickcheck/fuzz target in the tree. This run's probes (p7-p9 here, L7's P1-P15) were written in private copies and are not kept; L7 found its converter findings with exactly this kind of test |
| #57 nothing on mainnet | still open, not testable here | No mainnet access in this run |
| #58 bridge off-chain | still open (scope corrected, see K1-K7) | R p1-p6 |

## Candidate findings

**T-1 (Medium, testability). Platform reads have no seam, so every instruction-budget and
TTL path is dead code in native tests.** `bridge::instruction_counter()` and `now_ms()` are
hard-wired to 0 off-chain (`src/bridge.rs:77-82, 128-133`), `cache_get`/`failed_get`/
`record_message` call `ic_cdk::api::time()` (panics off-chain), Python picks a wall-clock
budget natively (`python.rs:28, 38-43`), and the web-time shim is wasm-only. R p5: with the
budget already spent (deadline 0), all three languages run a 200,000-iteration loop and send
a call; `check_call` admits a call under `max_instructions = 0`. R p4: an interface fetch dies
at `time`. Consequence: the budget (I13, register #32 manual), the TTLs (#45/#46 none), and
the High findings L6-1/L6-2 could not have been caught by any test in this suite. Fix: put
`instruction_counter()` and `time()` behind `HostBridge` (or a thread-local test override),
make `MockBridge` advance a fake counter per host call/engine step, and add per-language
"budget exhausted mid-loop" and TTL-expiry tests.

**T-2 (Low, testability). No CI or build gate.** Nothing runs the 59 tests, the `.did` match
tests, per-subset builds, the `ic0`-only import check (I23, CL25) or a reproducibility check
(I24). Today every subset links with `ic0`-only imports (R, dev profile), but the two
`[patch]` entries exist only to keep it that way, and a dependency bump re-enabling
`wasmbind`/`wasm-bindgen` would surface only at install time. Fix: a CI job running
`cargo test --workspace`, the subset matrix for wasm32, and `wasm-objdump -j Import` asserting
`ic0.*` only.

**T-3 (Low, testability). Rows testable with the existing native harness have no test.**
LIM13, LIM14, LIM17, LIM19 (override), the composite_query refusal (R p1-p4, each a few lines
on the real `IcBridge`) and the Python print cap (X: L6-4/L7-8 with `MockBridge`). I16 and I17
have no test. Fix: keep p1-p4 as suite tests.

**T-4 (Low, claims/limits). Audit size caps exceed the documented bounds.** README
"violations (≤ 16, each ≤ 512 B), error (≤ 4 KiB)": `truncate` appends a 14-byte marker after
cutting to the limit (R p2: 526 B violation; R p3: 4,109 B error, max 4,110 B), and the closing
entry adds a 17th violation `…and n more` (C `src/lib.rs:156-158, 233-235`). Bounded, harmless
overshoot; graded Low because the demonstrated outcome is 14 bytes and one entry (the severity
table's "differs from the documentation" would read Medium). Fix: truncate to
`max - marker.len()` and document 16 + one count entry, or state the bounds as written.

## Not tested, and why

- Release (LTO, `opt-level=z`) Wasm of the non-default subsets: run constraint (only Live
  builds release Wasm); dev builds of the same code import only `ic0.*`.
- Reproducibility (two clean release builds): cost; L15/Live.
- Anything on-chain (inspect_message, guards, budget, TTL expiry, `Trapped`): not repeated
  here; done live by L1, L5, L6, L9 where noted. No automated harness exists (#54, #58, T-1).
- Vendored chrono unit tests: cannot run from this workspace (dev-dependencies of a
  non-member); the source is identical to upstream.
- Mainnet (#57): no access.
