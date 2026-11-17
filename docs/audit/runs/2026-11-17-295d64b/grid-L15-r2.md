# Coverage grid: L15 Supply chain and build, round 2

Run `2026-11-17-295d64b`. Lens question: is what ships what was reviewed?
Scope this round: rows S1, S2, S7, C1a, CL33 (plus S9, I23, A11, which the same checks settle).
Evidence: **R** = reproduced (command given), **C** = code path. Candidate findings: L15-r2-1.

Method: `git archive HEAD` (295d64b) into a private copy `/private/tmp/claude-501/l15r2-*`,
`CARGO_TARGET_DIR=/private/tmp/claude-501/audit-target` (shared), copy deleted afterwards.
Toolchain in the copy: rustc/cargo 1.100.0, ic-wasm 0.11.1, icp 1.7.0. The running network was
not touched. Nothing in the working tree was built or changed apart from this file.

## Dynamic checks run

| Check | How | Result |
|---|---|---|
| RustSec advisories vs `Cargo.lock` | `git clone --depth 1 rustsec/advisory-db` (head `233155b4`, 2026-11-17); a matcher applying each advisory's `patched`/`unaffected` ranges to all 391 locked packages (vendored `chrono` matched by name) | 0 vulnerability or unsoundness advisories apply. 4 `informational = "unmaintained"` advisories apply: `smartstring 1.0.1` (RUSTSEC-2026-0249), `chrono 0.4.45` vendored (RUSTSEC-2026-0543), `paste 1.0.15` (RUSTSEC-2024-0436), `atomic-polyfill 1.0.3` (RUSTSEC-2023-0089). 68 other advisories exist for locked crate names; every locked version is in a patched range |
| OSV cross-check | `POST https://api.osv.dev/v1/querybatch` with the 387 registry packages plus `chrono 0.4.45` (RustSec and GHSA for crates.io) | Same 4 ids, nothing else. No GHSA for `boa_*` 0.22.0, `monty`/`monty-types` 1.6.0, `ruff_python_*` 0.0.14, `candid` 0.10.40, `candid_parser` 0.4.1, `rhai` 1.27.0, `ic-cdk` 0.20.3 |
| Live crates.io sparse index | fetch each registry crate's index file; compare `yanked`, `cksum` | 387 packages: 0 yanked, 0 checksum mismatches, 0 missing (live, not the local cache r1 used) |
| Newest versions of the priority crates | same index data | `boa_*` 0.22.0, `monty`/`monty-types` 1.6.0, `candid` 0.10.40, `candid_parser` 0.4.1, `rhai` 1.27.0, `ic-cdk` 0.20.3, `ic-stable-structures` 0.7.2: all newest. `ruff_python_parser`/`_ast`/`ruff_text_size` locked 0.0.14, newest 0.0.22; pinned by monty 1.6.0's `version = "0.0.14"` (caret on 0.0.z is exact) |
| Where the flagged crates sit (`cargo tree --target wasm32-unknown-unknown -e normal -i ...`) | | `smartstring` <- `rhai` (in every subset with `rhai`); `chrono` (vendored) <- `monty`, `monty-types` (every subset with `python`); `paste` is a proc-macro under `candid` (build time only); `atomic-polyfill` <- `heapless 0.7` <- `postcard` <- `monty` only on targets without native atomics: absent from the wasm32 graph |
| Rhai string type | rhai-1.27.0 `src/types/immutable_string.rs:50`, `src/lib.rs:280, 328, 491` | `ImmutableString(Shared<SmartString>)`; `Identifier = SmartString`; `Map = BTreeMap<Identifier, Dynamic>`: every script string and map key is a `smartstring` value. rhai offers a `compact_str` feature (`lib.rs:493`) that swaps it out |
| Release (LTO, `opt-level="z"`) Wasm of all 8 subsets, then `ic-wasm metadata ... shrink` as `icp.yaml` does | `cargo build -p icp_code_mode --target wasm32-unknown-unknown --release --locked [--no-default-features --features <set>]` | see table below |
| Imports of every release subset | `wasm-objdump -x -j Import` | all `ic0.*`; 36 imports in every subset with an engine (identical set, same sorted-list hash as the deployed default), 34 for `none` (no `canister_self_copy/size`). 0 non-`ic0` imports |
| Exports of every release subset | `wasm-objdump -x -j Export`; `ic-wasm check-endpoints --candid icp_code_mode.did` | 17 exports, identical set in all 8; `check-endpoints` reports only `canister_post_upgrade` and `get_candid_pointer` as "unexpected" in every subset (lifecycle hook and non-method export, expected) |
| Rebuilt default vs deployed | sha256 / `cmp` / `strings` | rebuilt `8d91ef12...` vs deployed `ad3ee221...`, both 11,402,905 B; the string-level difference is the embedded `candid_parser` build-dir path (`<checkout>/target/...` vs `/private/tmp/claude-501/audit-target/...`). Already reported as L15-2 (r1) |
| Python `strftime` / f-string format with hostile directives (vendored chrono on the untrusted path) | probe test in the private copy: 26 format strings (`%`, `%Q`, `%::z`, `%.3f`, `%-%`, emoji, ...) x 5 values (`date`, `time`, `datetime` incl. year 1 and 9999) | 130 cases, 0 panics; invalid ones return `ValueError` (monty uses `StrftimeItems::new_lenient` and `render_strftime`, monty-1.6.0 `src/types/time.rs:457`, `date.rs:373-380`) |
| Rhai strings across smartstring's inline/heap boundary | probe test: ASCII and 2-byte UTF-8 strings of length 0-69 used as value, map key, `+=` target | all round-trip (native 64-bit layout only; the wasm32 inline capacity differs) |

Release subset sizes (bytes; "shipped" = after `ic-wasm metadata` + `shrink`, as `icp.yaml` deploys):

| Subset | cargo output | shipped | gzip -9 of shipped | code section | functions | imports |
|---|---|---|---|---|---|---|
| default (js, python, rhai) | 12,117,911 | 11,402,905 | 3,641,019 | 8,432,319 | 34,612 | 36 |
| js, python | 10,823,180 | 10,210,496 | 3,320,673 | 7,350,502 | 29,149 | 36 |
| python, rhai | 8,747,844 | 8,229,479 | 2,702,626 | 6,046,347 | 24,080 | 36 |
| python | 7,442,106 | 7,021,485 | 2,380,361 | 4,950,089 | 18,591 | 36 |
| **js, rhai** | **6,054,315** | **5,593,299** | 1,645,875 | 4,468,724 | 21,318 | 36 |
| js | 4,759,921 | 4,399,972 | 1,322,277 | 3,387,492 | 15,845 | 36 |
| **rhai** | **2,667,255** | **2,430,878** | 704,905 | 2,098,008 | 10,704 | 36 |
| none | 1,254,785 | 1,158,555 | 361,617 | 943,602 | 4,896 | 34 |

## Grid

| Row | Entry | Reason / evidence |
|---|---|---|
| S1 lockfile | holds (integrity, vulnerabilities); **finding L15-r2-1** (unmaintained crates, no advisory gate) | R: 0 yanked, 0 checksum mismatches against the live index; 0 vulnerability/unsound advisories apply (RustSec + OSV). 4 unmaintained advisories apply, 2 of them compiled into the shipped Wasm on the untrusted-input path |
| S2 direct deps, young / pre-1.0 | holds for vulnerabilities; **finding L15-r2-1** for `rhai -> smartstring`; risk recorded for `ruff_python_*` | R: no advisory (RustSec or GHSA) for any priority crate at its locked version; all are the newest release except `ruff_python_*` 0.0.14 (8 releases behind 0.0.22), which the app cannot move without a monty release because monty 1.6.0 pins it exactly. The untrusted Python parser is therefore a pre-1.0 "internal component" crate the project cannot patch independently; no defect demonstrated, so not a finding |
| S7 toolchain | holds (rustc), already reported (ic-wasm/icp-cli unpinned = r1 L15-2) | rustc/cargo 1.100.0 from `rust-toolchain.toml` were used (R). No advisory database or `cargo-audit`/`cargo-deny` config in the repo (`deny.toml`, `audit.toml`, `.cargo/audit.toml` absent) — part of L15-r2-1 / r1 L15-4 |
| C1a feature subsets: imports | holds | R: release LTO links of all 8 subsets import only `ic0.*` (36, or 34 with no engine) and export the same 17 symbols as the deployed default. Closes the r1 gap (debug links only) and I23's "subsets unverified" |
| C1a feature subsets: deployability beyond imports | holds by inference; not installed | Every subset is smaller than the default in code size, function count and module size, uses the same target features, and the default installs on the local replica, so no platform size or count limit is closer for a subset. Installing each subset is a Live-phase step |
| CL33 "all-in-one about 12 MB (3.6 MB gzipped). JS + Rhai without Python is about 6 MB" | holds | R: cargo output 12.1 MB / 6.05 MB; shipped 11.4 MB / 5.59 MB; shipped default gzips to 3.64 MB. The README does not say which artifact it measures, but both readings round to the stated figures. Rhai-only (not claimed) is 2.4-2.7 MB |
| S9 imports, non-default subsets | holds | As C1a (R, release) |
| I23 | holds (all subsets, R); gap remains: no automated check (r1 L15-4) | |
| A11 build/supply-chain actor | **finding L15-r2-1** | The advisory scan that r1 could not run offline now shows 2 unmaintained crates in the shipped artifact; nothing in the repo would have reported them |
| CL25 "release Wasm imports only ic0.*" | holds | R for every subset |
| Other rows | n/a this round | Owned by r1 grid (`grid-L15-r1.md`) and other lenses |

## Candidate finding

**L15-r2-1 — Shipped Wasm contains two crates RustSec marks unmaintained, both on the
untrusted-script path; nothing in the build reports advisories** (Low; class supply chain;
evidence: R).
- `smartstring 1.0.1` (RUSTSEC-2026-0249, repository archived 2026-05-03) backs every Rhai
  string and map key (`rhai-1.27.0 src/types/immutable_string.rs:50`, `src/lib.rs:280,328,491`);
  in every subset with `rhai`, including the default. rhai's own `compact_str` feature
  replaces it.
- `chrono 0.4.45` (RUSTSEC-2026-0543, unmaintained, all versions) is the `vendor/chrono` copy
  pinned by `Cargo.toml [patch.crates-io]`; used by monty/monty-types for Python `datetime`.
  No upstream fixes will come, and the patch already blocks automatic ones.
- `paste` (proc-macro, build time) and `atomic-polyfill` (not in the wasm32 graph) are flagged
  but not shipped.
- No vulnerability or unsoundness advisory applies; no misbehaviour demonstrated (strftime and
  smartstring probes passed). Root cause shared with r1 L15-4: no advisory check
  (`cargo audit` / `cargo deny check advisories`) runs anywhere.
- Reproduce: clone `rustsec/advisory-db`, run `cargo audit -f Cargo.lock` (or the OSV
  `querybatch` above) at 295d64b.

## Not tested

- Installing the release subset Wasms on a replica (Live phase).
- smartstring on the wasm32 layout (inline capacity differs from the native 64-bit test).
- A from-scratch rebuild of `ahash`/`const-random` to re-test compile-time randomness: the
  shared target dir already held those rlibs, and `cargo clean -p` there would disturb other
  agents' builds. r1 refuted S5 by code path.
- Advisories newer than advisory-db `233155b4` (2026-11-17) and the OSV state at query time.
