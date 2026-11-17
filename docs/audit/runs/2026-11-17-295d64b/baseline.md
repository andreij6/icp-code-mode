# Baseline (Phase 0)

Run `2026-11-17-295d64b`. First full run of `docs/audit/PLAN.md`; there is no earlier run
directory to diff against.

## Commit and environment

| Item | Value |
|---|---|
| Commit | `295d64b715c67ed5ea2c6311d086b35245b8b2cb` ("Add a reusable audit: generic plan, findings register, runnable workflow") |
| Last code commit | `cdfa3d9` (2026-11-17 04:34 -0500). `git diff cdfa3d9 HEAD` touches only `.claude/workflows/audit.js`, `docs/audit/PLAN.md`, `docs/audit/REGISTER.md`: no application code changed since the commit that closed register round R6 |
| Date | 2026-11-17 |
| Run by | Claude (Opus 5.5) as a subagent of the `audit` workflow, Phase 0/1 step |
| Host | macOS 15.7.4, arm64 |
| rustc / cargo | 1.100.0 (0ef1bc386 2026-11-09) / 1.100.0 (pinned by `rust-toolchain.toml`, `channel = "1.100.0"`) |
| icp-cli | 1.7.0 |
| ic-wasm | 0.11.1 |
| wasm-objdump | from `/opt/homebrew/bin` (wabt) |
| node | v26.11.1 (video tooling only) |
| Uncommitted work excluded | `docs/per-customer-canisters-plan.md` (modified) and `design/` (new) belong to someone else and were not read or audited. All builds used `git archive HEAD`, so they contain only committed files |

Build method: `git archive HEAD` into a private scratch copy (no `target/`, no `.env`, no
`design/`), `CARGO_TARGET_DIR=/private/tmp/claude-501/audit-target` (the run's shared build
directory). Nothing was built in the working tree. The private copy is deleted at the end
of this step.

## Builds: every supported target and feature combination

Supported combinations come from `Cargo.toml` (`[features] default = ["js","python","rhai"]`,
each engine optional) and the README ("per-language worker" builds with
`--no-default-features --features <engine>`).

| Command (all `--locked`) | Result | Warnings |
|---|---|---|
| `cargo check -p icp_code_mode --target wasm32-unknown-unknown --no-default-features` | ok | 0 |
| same, `--features js` | ok | 0 |
| same, `--features python` | ok | 0 |
| same, `--features rhai` | ok | 0 |
| same, `--features js,python` | ok | 0 |
| same, `--features js,rhai` | ok | 0 |
| same, `--features python,rhai` | ok | 0 |
| same, `--features js,python,rhai` (= default) | ok | 0 |
| `cargo check -p demo_store --target wasm32-unknown-unknown` | ok | 0 |
| `cargo clippy --workspace --all-targets` | ok | 5 distinct lints (collapsible `if` x3 at `src/bridge.rs:142`, `src/bridge.rs:429`, `src/search.rs:62`; complex type `src/bridge.rs:195`; manual `is_multiple_of` `src/runtimes/js.rs:114`). Style only |

Limit of this check: `cargo check` does not link, so it cannot show an unresolved import
(for example a wasm-bindgen import pulled in by a feature subset). Only the default build
was inspected for imports (below). Release wasm builds were deliberately not run in this
step (run constraint: only the Live phase builds wasm). **Not verified here:** that every
non-default feature subset links to a Wasm that imports only `ic0.*`.

## Test suite

| Command | Result | Time |
|---|---|---|
| `cargo test --workspace --locked` (default features) | 59 passed, 0 failed (57 in `icp_code_mode`, 2 in `demo_store`) | 37.1 s wall including compilation; test execution 8.03 s |
| `cargo test -p icp_code_mode --lib --no-default-features` | 32 passed | 0.19 s execution |
| same, `--features js` | 41 passed | 7.98 s execution |
| same, `--features python` | 37 passed | 0.29 s execution |
| same, `--features rhai` | 38 passed | 0.20 s execution |

All tests are native (host target). None runs on a replica, and `IcBridge` cannot be driven
off-chain (its `ic0` stubs panic), so every on-chain code path is untested by the suite
(register #54, #58). There is no CI configuration in the repository (no `.github/`, no other
pipeline file), so nothing runs these tests automatically.

## Deployable artifacts

No release Wasm was built in this step. The figures below are for the artifacts icp-cli
cached at `.icp/cache/artifacts/` (mtime 2026-11-17 04:32, two minutes before code commit
`cdfa3d9`). Read-only `icp canister status` on the running local network shows the deployed
module hash equals the cached artifact's hash, so these are what is deployed. That they were
built from exactly the committed source is **not proven**; the Live phase rebuilds and
compares.

| Artifact | Size | SHA-256 | Imports | Exports |
|---|---|---|---|---|
| `icp_code_mode` (default features, after `ic-wasm metadata` + `shrink`) | 11,402,905 B | `ad3ee221eb9f591db50565ca907e56396c38e9b54e287a25ce906542ce5e2411` | 36, all `ic0.*`: accept_message, call_cycles_add128, call_data_append, call_new, call_on_cleanup, call_perform, call_with_best_effort_response, canister_liquid_cycle_balance128, canister_self_copy, canister_self_size, certified_data_set, cost_call, data_certificate_copy, data_certificate_present, data_certificate_size, debug_print, is_controller, msg_arg_data_copy, msg_arg_data_size, msg_caller_copy, msg_caller_size, msg_method_name_copy, msg_method_name_size, msg_reject, msg_reject_code, msg_reject_msg_copy, msg_reject_msg_size, msg_reply, msg_reply_data_append, performance_counter, stable64_grow, stable64_read, stable64_size, stable64_write, time, trap | `canister_init`, `canister_post_upgrade`, `canister_inspect_message`, `canister_query http_request`, `canister_update http_request_update`, 6 admin `canister_update`s, 3 admin `canister_query`s, plus non-method exports `memory`, `get_candid_pointer`, `__getrandom_v03_custom`. Custom section `icp:public candid:service` (4,442 B) |
| `demo_store` | 501,773 B | `5ed9b423e1f130c01374ed3ee6920702d3a02ef14963585aa852c1fde1c0a029` | 7, all `ic0.*`: debug_print, msg_arg_data_copy, msg_arg_data_size, msg_reply, msg_reply_data_append, time, trap | 7 queries, 3 updates, hidden query `__get_candid_interface_tmp_hack`, `get_candid_pointer`, `memory`; `icp:public candid:service` section |

Nothing outside `ic0` is imported by either artifact, so both are deployable on that count.
README claim "about 12 MB" (README.md:120) matches (11.4 MB). The deployed `icp_code_mode`
has the default 3 GiB Wasm memory limit, freezing threshold 2,592,000 s, one controller.

Reproducibility note for the supply-chain lens: `rhai`'s default features enable
`ahash/compile-time-rng` (`cargo tree -e features -i ahash`), which embeds random constants
generated by `const-random-macro` (getrandom 0.2) at compile time. Two builds of the same
commit are therefore expected to differ in hash. Not yet demonstrated (needs two release
builds; Live phase or L15).

## Fresh local deployment

Not done in this step: deploying is reserved for the Live phase, and the run constraints say
to reuse the project's running local network on port 8010 instead of starting a second one.
Read-only observation only: `icp canister status` shows both canisters Running on that network
(`icp_code_mode` = `4caro-hl777-77775-aaaba-cai`, `demo_store` = `4fbx2-kt777-77775-aaabq-cai`).

Static review of the instructions, for the Live phase to confirm by following them:

| Step (README) | Observation |
|---|---|
| Requirements (README.md:98-102) list Rust 1.100, wasm32 target, icp-cli >= 1.7, ic-wasm | The `.did` regeneration step (README.md:122-126) uses `candid-extractor`, which is not in the requirements list. `icp.yaml` itself checks only for `ic-wasm` |
| Quickstart `icp network start -d && icp deploy` | `icp.yaml` builds with `cargo build ... --release --locked` into `target/` of the project; `init_args: "(null)"`. To confirm live |
| Token registration (README.md:51-55, 141-149) | Uses `openssl`, `shasum`, `cut`: not listed as requirements (standard on macOS/Linux) |
| `scripts/demo-setup.sh` | Parses `icp canister status demo_store` output with `sed -n 's/^Canister Id: *//p'`; the current icp-cli 1.7.0 prints `Canister Id: ...` as the first line (seen above), so the pattern matches today. Depends on human-readable CLI output |
| Upgrades with Candid-breaking `.did` changes | README.md:471-472 says `icp deploy --yes` was needed; the Quickstart does not mention it |

## Baseline verdict

- Builds: every feature combination type-checks for wasm32; default build artifacts import only `ic0`.
- Tests: all 59 pass (default) and every single-engine subset passes its own tests.
- Not established in this step: release links of non-default subsets, artifact reproducibility,
  and a fresh deployment from the instructions (all deferred to Live / L15).
