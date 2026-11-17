# Variant sweep r3-11: build-time configuration honoured by the executor, not by the interface description

Run `2026-11-17-295d64b`, Phase 6. Pattern under test:

> A build-time configuration honoured by the executor but not by the interface description.

Already known and not re-reported: per-language builds advertise all three languages and
`default: javascript` in `tools/list` (a Rhai-only worker fails every call that omits
`language`); `EXECUTION_BUDGET` / README:367 Python 10B (wasm32 vs native cfg); release
profile without overflow checks (demo_store `reserved`); the getrandom custom backend vs
"no randomness"; `icp.yaml` copying from a hard-coded `./target` path (breaks the README
worker recipe); no CI for feature subsets; .did/artifact drift has no guard.

Method: listed every build-time switch (`Cargo.toml [features]`, every `cfg(...)`/`cfg!` outside
tests via `grep -rn 'cfg\|feature' src demo_store/src shims`, `.cargo/config.toml` rustflags,
`[profile.release]`, dependency feature flags, `icp.yaml` build steps, `env!`), then every
surface that describes the canister to a client, an AI model or an operator (`tools/list`,
`initialize`, the input validator `parse_tool_call`/`Language::from_str`, error texts, the
demo page compiled in with `include_str!`, the `.did` and `candid:service` metadata,
`get_policy`, README), and asked for each pair whether the description follows the switch.

Evidence: R = reproduced; CP = code path. Reproduction: private copy (no `target/`), one
added `#[cfg(all(test, not(feature = "js")))]` test in `src/mcp.rs`,
`CARGO_TARGET_DIR=/private/tmp/claude-501/audit-target cargo test --lib --no-default-features --features rhai sweep_r3_11 -- --nocapture`.
Output:

```
validator accepts uncompiled javascript: true
unknown language error: unsupported language `cobol` (expected javascript, python or rhai)
executor: Err("language `javascript` is not compiled into this canister")
demo offers js/python select: true true
demo comparison hard-codes javascript: 4
```

Copy deleted afterwards.

## Cells

| Row | Build-time switch / surface | Cell | Reason |
|---|---|---|---|
| C1a / U5 / B13 / E5a | Features `js`/`python`/`rhai` vs the input validator `parse_tool_call` -> `Language::from_str` (`src/mcp.rs:210-224`, `src/runtimes/mod.rs:34-44`) | **finding (R), candidate SW3-11a** | `Language` and `from_str` are not feature-gated, so a worker accepts every language name at parse time. Unknown names (`cobol`) get a free `-32602` (query path, `mcp.rs:354`), but an uncompiled one passes validation, upgrades to an update (`mcp.rs:359`), takes a run slot, sets `ran_script` (so the batch's next script is refused), and is refused only inside `LanguageRouter::execute` (`mod.rs:63-64`) with an `Error` audit entry written by `run_script` (`lib.rs:218-262`). The validator's error text also lists "javascript, python or rhai" on a build that has one of them. Distinct from the known tools/list finding: there the advertised schema is wrong; here the server's own validation contract is wrong, which moves the refusal from a free parse error to a paid, slot-holding, audited update |
| C5 / E26 / B17 | Features vs the demo page (`DEMO_HTML = include_str!("demo.html")`, `mcp.rs:318`, served on every build) | **finding (R), candidate SW3-11b** | The page compiled into every build offers JavaScript, Python and Rhai in `#lang` (`demo.html:144`) and example chips for each, and the comparison runner hard-codes `language: "javascript"` (`demo.html:388, 403, 457`). On a Rhai- or Python-only worker "Run comparison" fails on its first `classicCall`, and each failed call is an audited `Error` run; on a JS-less worker two of three language options and four of five examples fail. README:112-120 documents workers but not that their demo page is broken |
| C1a / initialize | Features vs `initialize` (`mcp.rs:135-142`) | holds | `instructions` names no language; `serverInfo.version` is `CARGO_PKG_VERSION`, the same for every subset, so a client cannot tell builds apart, but nothing it says is false |
| C1a / E12 | Features vs `get_policy` / `PolicyView` (`lib.rs:27-36`) | holds (omission only) | Reports limits, allowlist, tokens, runs; says nothing about languages, so it does not contradict the build. An operator cannot read the build's language set from the canister; noted for the critic, not a finding |
| C11 / C7 / E1-E15 | Features vs `.did` and `candid:service` metadata (`icp.yaml` `ic-wasm metadata -f icp_code_mode.did`) | holds | No endpoint, type or comment depends on a feature: `#[ic_cdk::*]` methods in `lib.rs` are not `cfg`-gated, so the same `.did` is correct for every subset. `AuditEntry.language` is free text |
| C1a / E5b / B16 | Features vs search output (`src/search.rs`) | holds | Search output names no language (grep finds none) |
| C1a / CL9 / CL10 / CL11-13 | Features vs README guardrail rows (README:362-376) | holds | Rows are per-language descriptions of engines; README:41 and 112-120 state engines are optional features. The "in all three languages" wording (CL10) describes the all-in-one build and is not false for a subset that has fewer |
| V6 / LIM26 | `#[cfg(feature = "python")]` counting allocator (`lib.rs:359`) | holds | Only the Python memory cap depends on it; README states the 64 MiB limit only for Python and lists "no JS memory cap" as known |
| C1a / T7 / T5 | Feature-gated tests (`mod.rs:70`, `mcp.rs:533, 662`) | not re-reported | Tests for subsets do not exist; covered by the known "no CI / feature-subset builds" testability findings |
| LIM2 / LIM26 / B14 | `cfg(target_arch = "wasm32")` in `bridge.rs:78-82, 129-131`, `python.rs:28, 38`, `shims/web-time` | not re-reported / holds | `EXECUTION_BUDGET` doc and README:367 are known. Time and instruction counter cfgs: no interface text claims native behaviour; tool description says "Date/time are fixed to the block time", true on wasm32 |
| S4 / V7 / CL26 / CL29 | `getrandom_backend="custom"` rustflag | not re-reported | Known ("no randomness" description; stream used beyond hash seeding) |
| C10 / F16 | `[profile.release]` overflow checks off, `panic = "abort"` | not re-reported / holds | Overflow: known (demo_store `reserved`). `panic = "abort"`: README describes panics as traps with a `Trapped` entry from cleanup, which is what the IC does with an aborting panic |
| C8 / LIM28 / CL34 | `-zstack-size=8388608` | holds | README:376 "8 MiB wasm stack" matches the rustflag for `wasm32-unknown-unknown`, the only target that ships |
| S2 / LIM24 / CL11 | `boa_engine` `default-features = false` | holds | No interface text promises `Intl`, Annex B or other optional Boa features; README:370 lists only removals the code performs at runtime |
| S2 / LIM27 / CL13 / CL29 | rhai features `sync`, `no_time`, `serde` | holds | `no_time` removes `timestamp()`; description says "No ... clock", consistent. `sync`/`serde` are not described |
| S5 / S7 / C7 / C9 | `ahash/compile-time-rng`, unpinned `ic-wasm`, toolchain pin | not re-reported | Reproducibility is known; none of these feed an interface description |
| C7 / AD1 | `icp.yaml` `init_args: "(null)"`, hard-coded `./target` copy | not re-reported | Known (post_upgrade re-apply; stale wasm for the worker recipe) |
| C7 / AD9 / CL33 | README worker recipe and sizes (README:112-120) | holds | Sizes are approximate claims about artifacts, not interface behaviour; build path issue is known |
| C2 / C11 | demo_store build (no features, no cfg outside tests) | not applicable | No build-time switch besides the release profile (known) |
| C3 / C4 / S3 | `[patch.crates-io]` web-time shim, vendored chrono | holds | They change dependency behaviour on wasm32 (instruction-count clock, no `wasmbind`); the only interface claim (CL27, README:478-486) describes exactly that |
| C12 / C6 / C13 / C14 | video tooling, demo-setup script, audit tooling, idea doc | not applicable | Not built with Cargo features; no interface description that varies with a build switch |
| D1-D10, V1-V5, V8-V11, K1-K12, LIM1, LIM3-LIM23, LIM25, LIM29-32, W1-W16, F1-F15, B1-B12, B15, VF1-VF8, AD2-AD8, A1-A12, U1-U4, U6-U16, O1-O10, E16-E25 | runtime state, limits, conversions, entry points | not applicable | None is selected or altered by a build-time switch (grep: the only feature `cfg`s are the engine modules, router arms, the Python allocator and feature-gated tests; the only target `cfg`s are listed above). Engine-specific rows (E16-E19, B8-B10, W8-W11) exist or not as a whole with their feature, and are described only by the surfaces covered above |
| T1-T12 / History | tests, register | covered | Register has no prior feature-subset finding besides the known tools/list one |

## Candidates

- **SW3-11a** (R): the input validator accepts languages the build does not contain, so a
  worker refuses them only after a paid update, a run slot and an audit entry, and its
  error text lists languages it cannot run. Medium/Low (cost and misleading contract;
  likely merges with the known tools/list finding under one root cause: the `Language` set
  is not derived from the compiled features).
- **SW3-11b** (R): the demo page compiled into every build offers all three languages and
  its comparison runner is JavaScript-only, so on a Rhai- or Python-only worker it fails
  and leaves `Error` audit entries. Low/Medium (same root cause).
