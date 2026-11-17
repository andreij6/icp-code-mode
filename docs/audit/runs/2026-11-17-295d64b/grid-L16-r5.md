# Coverage grid: L16 Claims, round 5 (README fresh deployment)

Run `2026-11-17-295d64b`. Scope: the Phase 0 "fresh local deployment from the project's own
instructions" that baseline.md deferred and no earlier grid carried out. Rows: C6, C7, AD1, AD9,
S7, CL33 and every setup sentence of README.md (Quickstart, Build and test, Deploy locally,
token, allowlist, demo store, curl examples, MCP client config, upgrade notes).
Each cell: **finding**, **holds** (why), or **n/a** (why).

Evidence key: **R** reproduced (command and output below), **C** code path only.

## What was run, and the deviations from "word for word"

Clean copy: `git archive 295d64b` into a private scratch directory (no `target/`, no `.env`,
no `.icp/`). `target` in the copy was a symlink to the run's shared build directory
(`/private/tmp/claude-501/audit-target`), so the icp.yaml build steps ran exactly as written
(no `CARGO_TARGET_DIR`), while honouring the disk constraint. Tools: icp-cli 1.7.0,
ic-wasm 0.11.1, rustc 1.100.0 (picked up from the copy's `rust-toolchain.toml`).

Deviations, and why:

1. **`icp network start -d` was not run.** The project's network already holds port 8010;
   a second one would collide (run constraint). The copy was attached to the running network
   by copying only `.icp/cache/networks/local/descriptor.json`. Unverified: that the command
   works in a clean clone on a free port.
2. **Fresh canisters in a separate environment, not the developer's canisters.** The running
   `icp_code_mode` (`4caro-…`) is **not** a 295d64b build: module hash `0x89b4c117…` (baseline saw
   `ad3ee221…`), its `get_policy` returns `limits.tool_calls_per_minute` and `candid_overrides`,
   which 295d64b's `.did` does not have, and the working tree's HEAD is now `91409d1` with
   uncommitted changes in `src/`. Installing 295d64b over it would be a downgrade of someone
   else's newer deployment, with stored data written by newer code. So the copy's `icp.yaml`
   got one extra block (`environments: - name: audit, network: local`) and every command ran
   with `ICP_ENVIRONMENT=audit`, which lets the README's commands run unchanged. The gateway
   name therefore is `icp_code_mode.audit.localhost:8010` instead of `.local.`. The gateway's
   domain file gained three `.audit.localhost` lines; the two `.local.localhost` lines are
   unchanged (diffed).
3. Token kept in a mode-600 scratch file instead of a shell variable (each tool call is a new
   shell); never printed; revoked and deleted at the end.

Canisters created (disposable, left in place): `icp_code_mode` `52jen-jl777-77775-aaafa-cai`,
`demo_store` `55icz-et777-77775-aaafq-cai`, `icp_code_mode_rhai` `5bmyi-tt777-77775-aaahq-cai`
(environment `audit`). The test token was revoked (`tokens = vec {}`, then 401).

## Step log

| # | README step | Result |
|---|---|---|
| 1 | Requirements (README.md:98-102) | All present on this host. The documented flow also invokes `openssl`, `shasum`, `cut` (token, :53-54, :144-148), `curl` (:243-328), `bash`/`sed` (`scripts/demo-setup.sh`), `candid-extractor` (:125): none listed. **finding L16-r5-3** |
| 2 | `icp deploy` (first) | exit 0 in 6.7 s; created both canisters; printed `Frontends (serving http_request): icp_code_mode: http://icp_code_mode.audit.localhost:8010/` (README:138 holds, with `.local.` in the default environment). Cargo reported `Fresh icp_code_mode` for a copy never compiled in that directory: see L16-r5-4 (audit method) |
| 3 | Token (README.md:53-54, exactly as written, in bash) | `(variant { Ok })`, 64-hex token; later authenticated requests 200 |
| 4 | Ledger allowlist (README.md:58 / :156) | `()`; "The local network ships an ICP ledger" holds: `ryjl3-…` answered every call |
| 5 | `scripts/demo-setup.sh` (as written) | Printed `Allowlisted demo_store (4fbx2-…) in icp_code_mode [local]`: it wrote to the **developer's** canister although `ICP_ENVIRONMENT=audit` was set. Same 8 methods were already there, so no state change (get_policy diffed). **finding L16-r5-2**. `scripts/demo-setup.sh audit` then allowlisted `55icz-…` correctly |
| 6 | curl examples (README.md:246-326) | initialize, `notifications/initialized` (202), tools/list, search (first one 0.63 s via update, repeat 0.007 s via query, output identical to README:279), JS (`did bytes 17368`, `methods 14`, `sym ICP`, `dec 8`, `bal 0`; supply differs only by fees burnt on this network), Python (identical to README:309), Rhai, policy-blocked call (`isError: true`, PolicyViolation entry, `calls = 0`), `get_audit_logs '(0, 5)'`: all as documented |
| 7 | MCP client config (README.md:330-335) | JSON valid. Streamable-HTTP handshake by curl: initialize with protocolVersion 2025-06-18 -> negotiated 2025-03-26, `content-type: application/json`; GET /mcp 405; DELETE 405; `MCP-Protocol-Version` header accepted. **But** Node (`fetch`, `dns.lookup`) and Python (`urllib`) get `ENOTFOUND` / `gaierror 8` for `icp_code_mode.local.localhost` and `icp_code_mode.audit.localhost` on this macOS host; only curl works, because curl maps `*.localhost` to loopback itself. `http://localhost:8010/mcp?canisterId=<id>` works from Node (200). **finding L16-r5-1** |
| 8 | `icp deploy` (second, README.md:176; upgrade) | Sources touched first so cargo really recompiled (`Dirty … src/lib.rs has changed`, `Compiling icp_code_mode`). exit 0; "Checking compatibility" passed with no prompt (same `.did`); `init_args "(null)"` re-applied = no-op. Token, allowlist and audit log survived (log 16 -> 19 after one search that re-fetched the interface; the heap cache is gone after upgrade as README:468 says) |
| 9 | Module hash vs build output | Deployed `icp_code_mode` `0x224cfa7a…c769` = `.icp/cache/artifacts/icp_code_mode` = `ic-wasm metadata`+`shrink` of cargo output `9b3d5f5f…` rerun by hand. `demo_store` `0x5ed9b423…` = same recomputation (and = baseline). In the README flow (no `CARGO_TARGET_DIR`) what ships is what cargo built |
| 10 | Worker recipe (README.md:112-119: copy the entry, use the `--target-dir target/rhai` command) | Deployed `icp_code_mode_rhai`: module hash `0x224cfa7a…` = the **all-in-one** artifact; the 2,667,383 B Rhai-only wasm cargo built in `target/rhai/` was ignored. Confirms the reported icp.yaml `./target` copy finding (L15-1) in the documented flow; not repeated as new |
| 11 | Candid-breaking upgrade (README.md:471-472, AD9) | `.did` in the copy edited to drop `get_audit_log_len`, `icp deploy icp_code_mode </dev/null`: "BREAKING change … Method get_audit_log_len is only in the expected type … Use --yes", exit 1, module hash unchanged. Holds: refusal is loud and names `--yes`. `.did` restored, nothing installed |
| 12 | `.did` regeneration (README.md:125) | `candid-extractor` on the deployed artifact = committed `icp_code_mode.did` byte for byte (after trailing-space normalisation). Holds; tool unlisted (step 1) |

## Grid

| Row | Cell | Evidence and reason |
|---|---|---|
| C6 `scripts/demo-setup.sh` | **finding L16-r5-2** (Low) | R step 5. Default `ENV="${1:-local}"` and an explicit `-e "$ENV"` override icp-cli's own `ICP_ENVIRONMENT`. Parsing of `Canister Id:` works with icp-cli 1.7.0 (already reported as fragile). Executable bit is in git (100755) |
| C7 `icp.yaml` build steps | holds in the README flow; known finding confirmed for the worker recipe | R steps 2, 8, 9, 10. Default flow: deployed hash = ic-wasm(cargo output). Worker recipe ships the wrong artifact (L15-1, already reported). `ic-wasm` presence check only on the first canister: n/a here (installed) |
| AD1 `InitArgs` / `init_args: "(null)"` | holds for the documented flow | R steps 2 and 8: install and upgrade with `(null)` succeed and change nothing. Additive re-application of non-null args on upgrade is already reported |
| AD9 feature subset, `--yes` | holds (`--yes`); feature subset = C7 finding | R step 11: a breaking `.did` stops the deploy with a message naming `--yes`; a non-breaking upgrade needs nothing |
| S7 toolchain / icp-cli / ic-wasm | holds for the pinned rustc; **finding L16-r5-3** for unlisted tools | `rust-toolchain.toml` was honoured in the copy (`rustup show active-toolchain` -> 1.100.0 overridden by the copy's file). icp-cli 1.7.0 matches "≥ 1.7". Unpinned ic-wasm already reported (L15-2) |
| CL33 benchmarks and sizes | holds | Fresh `demo_store` (`5ed9b423…`) returns byte-identical `sales_summary`, `list_products`, `list_orders` to the developer's instance (sha-256 of the CLI output), so the R-live benchmark of L16-r4 applies to a fresh deployment. Sizes settled in L15-r2 |
| README:3-5 video links | holds | `video/poster.jpg`, `video/icp-code-mode-explainer.mp4` are in the archive |
| README:47-50 Quickstart deploy | holds except network start not run (deviation 1) | R step 2 |
| README:51-55, 141-149 token | holds | R step 3 (bash, exactly as written) |
| README:56-59, 151-157 ledger allowlist | holds | R step 4 |
| README:60, 330-335 client config / gateway URL | **finding L16-r5-1** (Medium) | R step 7 |
| README:98-102 requirements | **finding L16-r5-3** (Low) | R step 1 |
| README:105-110 `cargo test`, `cargo build` | holds | build R steps 8-9; tests per baseline (59 pass) |
| README:112-119 worker recipe | known finding (L15-1) confirmed | R step 10 |
| README:120 sizes | holds | L15-r2 |
| README:122-126 `.did` regeneration | holds | R step 12 |
| README:131-139 deploy, port 8010, URL printed | holds | R step 2 output |
| README:159-165 admin endpoints | holds | `set_allowlist`, `add_api_token_hash`, `revoke_api_token_hash`, `get_policy`, `get_audit_logs`, `get_audit_log_len` used as controller, all as documented |
| README:167-181 demo store "two commands" | holds with C6 caveat | R steps 5 and 8 |
| README:243-328 curl examples | holds | R step 6 |
| README:378-379 upgrades keep state, `post_upgrade` takes InitArgs | holds | R step 8 |
| README:441-444 / 468-470 interface cache re-fetched after upgrade | holds | R step 8 (first search after upgrade went through the update path, 3 new entries) |
| README:471-472 `icp deploy --yes` | holds | R step 11 |

## Candidate findings

- **L16-r5-1 (Medium).** The documented MCP endpoint `http://icp_code_mode.local.localhost:8010/mcp`
  (Quickstart step 4, curl section, client config) does not resolve for Node- or Python-based
  MCP clients on macOS (`ENOTFOUND`); only curl and browsers special-case `*.localhost`.
- **L16-r5-2 (Low).** `scripts/demo-setup.sh` ignores `ICP_ENVIRONMENT` and silently targets
  `local`; with that variable set to another environment it allowlists the store in the wrong
  `icp_code_mode`.
- **L16-r5-3 (Low).** The Requirements list omits tools the documented flow runs:
  `openssl`, `shasum`, `cut`, `curl`, `bash`/`sed`, `candid-extractor`.
- **L16-r5-4 (audit method, not product).** In the shared build directory, a `git archive` copy
  is reported `Fresh` by cargo without being compiled (archive mtimes are the commit time,
  older than other agents' outputs), so a deploy or "rebuild" from such a copy ships whatever
  was last built there. Here the result happened to equal a real rebuild (`9b3d5f5f…` both
  ways), so no wrong artifact was shown.

## Not tested

- `icp network start -d` in a clean clone (port 8010 taken by the project's network).
- Install over the developer's canisters with `.local.localhost` names (they run newer code
  than 295d64b; see deviation 2).
- Linux name resolution of `*.localhost` (systemd-resolved and nss-myhostname usually resolve
  it, so L16-r5-1 is probably macOS-specific); a real MCP client binary (Node `fetch` and
  `dns.lookup` were used, which is what Node clients call).
- Mainnet URLs (`https://<id>.icp0.io/mcp`).
