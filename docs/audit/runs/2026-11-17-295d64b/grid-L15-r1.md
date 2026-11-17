# Coverage grid: L15 Supply chain and build, round 1

Run `2026-11-17-295d64b`. Lens question: is what ships what was reviewed?
Evidence: **R** = reproduced (command given), **C** = code path. Candidate findings are
numbered L15-1 to L15-4 (details in the lens report returned to the workflow).

Method: `git archive HEAD` into a private scratch copy; `CARGO_TARGET_DIR=/private/tmp/claude-501/audit-target`;
nothing built in the working tree; copy deleted afterwards. No release build of `icp_code_mode`
was run (run constraint); demo_store release builds only (small). The running local network was
not touched.

## Dynamic checks run

| Check | Result |
|---|---|
| Vendored chrono vs crates.io 0.4.45 | `src/` byte-identical to the `.crate` (sha256 `1aa79e62...`, equals the index `cksum`); `Cargo.toml` differs only by the header comment, `wasmbind` removed from `default`, three `[[test]]` removed |
| Lockfile vs local index cache | 387 registry packages: 0 yanked, 0 checksum mismatches, 0 missing (index cache last refreshed between 2026-07-30 and 2026-11-16 per crate; yanks after that are not seen) |
| Dependency graph per feature subset (wasm32, normal edges), all 8 subsets | no `wasm-bindgen`, `js-sys`, `web-sys`, `iana-time-zone`, `instant` in any subset |
| Debug wasm link of all 8 subsets + `wasm-objdump -j Import` | none: 34 imports, every other subset: 36, all `ic0.*`, 0 non-`ic0`. Default debug set == deployed release set (36 names) |
| `RUSTFLAGS` overriding `.cargo/config.toml` | build fails loudly at getrandom (`compile_error`), so the custom backend and stack flag cannot be silently dropped together |
| Deployed artifact layout | `__stack_pointer` init 8,388,608; first data segment at 8,388,608: stack-first, overflow traps |
| Strings in deployed artifacts | `icp_code_mode`: 769 absolute `/Users/<builder>/.cargo/registry/src/...` paths + 1 absolute build-dir path `<checkout>/target/wasm32-unknown-unknown/release/build/candid_parser/59cfcb93f05a2646/out/grammar.rs`; `demo_store`: 43 registry paths, no build-dir path |
| demo_store rebuilt from HEAD in another checkout path and target dir | sha256 `5ed9b423...` == deployed: same-machine reproducible when only CARGO_HOME paths are embedded |
| `icp build demo_store` with `CARGO_TARGET_DIR` set and an old wasm in `target/` | **L15-1**: reports success, artifact byte-identical to the old wasm (`5ed9b423...`), source marker absent; fresh cargo output (`2d10c79e...`, marker present) ignored |
| demo_store `reserved += quantity` in debug vs release tests | **L15-3**: debug panics `attempt to add with overflow`; release wraps `reserved` to 4,294,967,293 |
| gzip of deployed `icp_code_mode` | 3,641,028 B (`-9`): README "3.6 MB gzipped" holds |

## Grid

| Row | Entry | Reason / evidence |
|---|---|---|
| C1 | holds (imports), **finding L15-1** (build step), **finding L15-2** (reproducibility) | Default artifact imports only `ic0` (baseline). Build steps and embedded paths, see C7/I24 |
| C1a | holds (imports); not tested (sizes) | All 8 subsets link with only `ic0` imports (debug link, R). Release links of subsets not run (constraint); debug imports are a superset of LTO release imports except for `cfg(not(debug_assertions))` code, none of which adds `extern` blocks in app code. "JS + Rhai about 6 MB" not measured |
| C2 | holds | Deployed demo_store rebuilt bit-for-bit from HEAD (R) |
| C3 `shims/web-time` | holds | Only `monty-types` (wasm32 `target_os=unknown`) and `rhai` (behind `no_time`, unused) depend on it. monty-types 1.6.0 uses `Instant::now`, `elapsed`, comparisons; never `Add`, so the panicking `Add<Duration>` (`shims/web-time/src/lib.rs:43`) is unreachable (C). Missing upstream impls (`AddAssign`, `Sub<Duration>`, `SubAssign`) would fail to compile, not misbehave. `SystemTime` re-export (panics on wasm32) has no caller in the graph (C) |
| C4 `vendor/chrono` | holds | Byte-identical to upstream except the documented edits (R). Drift effect: without `wasmbind`, `Utc::now()` uses `SystemTime::now()`, which panics on wasm32; the only caller in the graph is `monty_types::DateTimeSource::System::read` (`os_policy.rs:58`), and `src/runtimes/python.rs:214` always sets `DateTimeSource::Fixed` (C). No later chrono patch release can arrive while the patch pins 0.4.45 (security fixes need a manual re-vendor; none known offline) |
| C5 demo page | holds | No external scripts, styles or fonts in `src/demo.html`; nothing third-party loads on the canister origin |
| C6 `scripts/demo-setup.sh` | holds | Parses human-readable `icp` output, but `set -euo pipefail` and the empty-id check make a format change fail loudly |
| C7 `icp.yaml` build steps | **finding L15-1**; **finding L15-2** (unpinned ic-wasm) | `cp target/wasm32-unknown-unknown/release/<name>.wasm` is hard-coded while `cargo build` writes to `$CARGO_TARGET_DIR`/`build.target-dir`; the README worker recipe (`--target-dir target/rhai`) hits the same path mismatch (R with demo_store). `ic-wasm` only checked for presence (icp_code_mode entry only) |
| C8 `.cargo/config.toml` | holds | Stack 8 MiB with stack-first layout verified in the artifact; overriding rustflags fails loudly (R). Config is found from cwd, so `--manifest-path` builds from elsewhere also fail loudly at getrandom |
| C9 toolchain pin | holds; MSRV not tested | `channel = "1.100.0"` exact. `rust-version = "1.98"` equals the highest declared MSRV in the wasm graph (monty, monty-types, monty-macros 1.98); 1.98 is not installed, so not compiled |
| C10 release profile | **finding L15-3** (overflow checks); holds (rest) | `opt-level="z"`, `lto`, `codegen-units=1`, `strip` change size and instruction counts only. `panic="abort"`: wasm32 aborts anyway; the `Trapped` entry relies on ic-cdk's cleanup callback, not unwinding. Profile leaves `overflow-checks` at its release default (off) while tests run with it on (R) |
| C11 `.did` files | holds (when tests run) | `ic-wasm metadata -f <file>.did` embeds the committed file, not one extracted from the Wasm; `did_file_matches_exported_interface` catches drift, but nothing runs tests automatically (L15-4) |
| C12 video tooling | holds / n/a | Node built-ins only, no npm dependencies or lockfile needed; `composition.html` loads Google Fonts (dev only, fonts not scripts). Secrets handling is L10 |
| C13 audit tooling | n/a | Not built or deployed |
| C14 idea doc | n/a | Not built |
| C15 / E15 extra exports | holds | `get_candid_pointer` and `__getrandom_v03_custom` lack the `canister_` prefix, so they are not callable messages |
| A11 build/supply chain actor | holds (lockfile, checksums), **finding L15-2/L15-4** | `--locked` in `icp.yaml`, no git sources, all checksums match the index. No advisory scan possible offline and none configured |
| S1 lockfile | holds | 391 packages; 387 registry, verified against the local index (no yanks seen, checksums equal) |
| S2 direct deps, young / pre-1.0 on the critical path | holds (with recorded risk) | monty/monty-types `=1.6.0` and boa 0.22.0 are the newest versions in the local index (refreshed 2026-11-16); rhai 1.27.0, candid_parser 0.4.1, ic-cdk 0.20.3 likewise newest. Exact pin on monty is deliberate (the app uses `LIVE_MEMORY`/`BASELINE_MEMORY` internals). monty pulls `ruff_python_*` 0.0.14 (pre-1.0, "internal component crate of Ruff"; 0.0.22 exists); publisher not verifiable offline. Engine-level defects already in the register (#18-#21, #23, #24) show these engines carry risk, but no new defect demonstrated here |
| S3 patches | holds | See C3, C4 |
| S4 getrandom | holds | 0.3.4 and 0.4.3 both declare `__getrandom_v03_custom` with layout-identical `Error(NonZeroRawOsError)`; 0.2.17 is only in the proc-macro `const-random-macro`, which is never expanded (S5) |
| S5 compile-time randomness | holds: prediction **refuted** | `ahash` 0.8.12 has `runtime-rng` enabled in every subset that has `compile-time-rng`; `get_fixed_seeds` then takes the `runtime-rng` branch and the `const_random!` branches are `cfg`'d out (`random_state.rs:47-104`). No compile-time random constant reaches the Wasm. Confirmed indirectly: demo_store rebuild is bit-identical; `icp_code_mode` same-path rebuild left to Live |
| S6 duplicate versions | holds | Size cost only; no duplicated type crosses an API in app code (builds and tests pass) |
| S7 toolchain / icp-cli / ic-wasm | **finding L15-2** | rustc pinned; icp-cli ("≥ 1.7" in README) and ic-wasm unpinned; `ic-wasm shrink` output depends on the ic-wasm version |
| S8 build flags | see C8, C10 | |
| S9 imports, non-default subsets | holds | All 8 subsets: only `ic0` (R, debug link) |
| S10 dev/test only | holds / n/a | Not shipped |
| CL25 "release Wasm imports only ic0.*" | holds; **finding L15-4** (no guard) | True for default (baseline) and every subset (R), but only checked by hand |
| CL26 getrandom stream only for hash seeding | n/a for L15 | Determinism is L11 |
| CL27 shim gives deterministic budgets | holds | `performance_counter(1)` is replicated state; see C3 |
| CL33 sizes | holds (12 MB, 3.6 MB gz); not tested (JS+Rhai 6 MB) | |
| CL34 stack 8 MiB | holds | Verified in the artifact |
| I23 | holds; **gap = L15-4** | |
| I24 | **violated: finding L15-2** | Hash depends on CARGO_HOME, the username and the build directory; ic-wasm unpinned |
| LIM28 Wasm stack | holds | See CL34 |
| F9 shim `Instant + Duration` overflow | **refuted** | No `+` on `Instant` in monty-types 1.6.0 (C) |
| B14 instruction count -> Duration | holds | `Duration::from_nanos(u64)`, saturating subtraction in the shim |
| AD9 build-time choices | **finding L15-1** (worker recipe); `--yes` is L13 | |
| T (C4 test targets removed) | holds | Upstream chrono tests are for chrono itself; the app's Python datetime path is covered by `python::determinism_and_sandbox` |
| Register classes | n/a | No past L15 findings |
| All other rows (A1-A10, A12, E1-E14, E16-E28, U, O, D, V, K, LIM1-LIM27, LIM29-LIM32, W, F1-F8, F10-F16, B1-B13, B15-B17, VF, AD1-AD8, CL1-CL24, CL28-CL32, CL35-CL37) | n/a | Runtime behaviour of the shipped code, not how it is built or sourced; owned by other lenses. Exception noted under L15-3: F16 (demo_store wrap) is where the release flag shows |
