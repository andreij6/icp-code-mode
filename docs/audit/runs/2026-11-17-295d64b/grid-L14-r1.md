# Coverage grid: L14 Configuration, round 1

Run `2026-11-17-295d64b`. Lens question: what can an operator set that breaks a guarantee?
Evidence: **R** = reproduced (command or test given), **C** = code path. Candidate findings are
numbered L14-1 to L14-9 (details in the lens report returned to the workflow).

## Dynamic work done

- Native probe tests in a private copy (`tests/l14.rs`, not in the repo; shared
  `CARGO_TARGET_DIR`), all passing as probes:
  - `empty_token_digest_is_accepted_and_authenticates`: `add_token_hash(sha256(""))` returns
    `Ok(())`; `mcp::authenticate("POST", "/mcp", [("authorization", v)])` returns Ok for
    `v` = `"Bearer "`, `"Bearer    "`, `"bearer \u{a0}"`, `"Bearer \u{3000}"`. `printf %s "" | shasum -a 256`
    gives the same digest `e3b0c442...b855` (the README command with `MCP_TOKEN` unset).
  - `partial_record_resets_omitted_options`: Candid text
    `(record { max_calls_per_script = 25; max_instructions = 20_000_000_000; certified_reads = opt true })`,
    annotated with `set_limits`'s type from `icp_code_mode.did` and encoded the way icp-cli does,
    decodes to `call_timeout_seconds: None`.
  - `out_of_range_values_are_accepted_by_the_interface`: `max_calls 0 / max_instructions 0`,
    `u32::MAX / u64::MAX / call_timeout opt 0`, `max_instructions 100e9 / call_timeout u32::MAX` all
    decode into `Limits` (nothing in `set_limits`/`apply` rejects them).
  - `authorize_boundaries`: calls=0 refuses the first call; instructions=0 refuses once 1
    instruction is used; instructions=u64::MAX never refuses; calls=u32::MAX at slot MAX-1 is Ok
    and `n + 1` cannot wrap (refused at n = MAX); empty method list refuses every method.
  - `monty_accepts_a_huge_feed_budget`: Monty 1.6.0 runs with `max_feed_duration` of
    `u64::MAX - 5e6` ns and with `Duration::MAX` for both budgets without panicking.
- Live, read-only (local network, port 8010): `icp canister status` and `get_policy` (query).
  Limits are the defaults (`call_timeout_seconds = null`, `certified_reads = null`, 25, 20e9);
  settings: freezing threshold 2,592,000 s, Wasm memory limit 3,221,225,472, Wasm memory
  threshold 0, reserved cycles limit 5T, log visibility controllers; memory size 1,246,514,630 B
  with 836 audit entries (so about 1.2 GB is heap high-water mark, see D7 below). No setting
  was changed.

## Grid

| Row | Cell | Entry | Reason / evidence |
|---|---|---|---|
| AD1 | `InitArgs` token hashes | **finding L14-1** (symptom) | `apply` -> `add_token_hash` accepts the empty-string digest like any other 64-hex value (R) |
| AD1 | `InitArgs` limits | **finding L14-2** (symptom) | `apply` stores `a.limits` unvalidated, same as `set_limits` (C: `src/lib.rs:46-48`) |
| AD1 | `InitArgs` at upgrade | **finding L14-9** | `post_upgrade` runs the same additive `apply`: tokens and allowlist entries passed again come back even if revoked/removed since; nothing in InitArgs can remove (C: `src/lib.rs:38-49, 68-72`) |
| AD1 | malformed hash | holds | `ic_cdk::trap` in `apply` rolls back the install/upgrade (F1); the old module and state stay |
| AD2 | allowlist `aaaaa-aa` | **finding L14-5** | `set_allowed` accepts the management canister; with `"*"` and an override for `aaaaa-aa` every token holder can call management methods as this canister (C: `src/policy.rs:155-157`, no check) |
| AD2 | allowlist own principal | **finding L14-5** | accepted; a script can then call `http_request_update` on itself with a token and start nested runs, each with a fresh call cap and instruction budget (C: `check_call` has no self check; interface comes from public `candid:service` metadata) |
| AD2 | empty method list | holds | every method refused (R: `authorize_boundaries`); interface fetches still allowed, consistent with "allowlisted canister" (`check_interface_fetch`) and with README "checked before any message is sent, including the interface fetch" |
| AD2 | unknown / misspelled method names | holds | fail closed: a name that matches no method allows nothing |
| AD2 | unbounded method lists, entry count | **finding L14-7** | no bound; whole allowlist decoded and copied on every search (query and update) and returned whole by `get_policy` (C: `src/lib.rs:112, 129, 339`) |
| AD3 | `remove_allowlist` leaves the override | **finding L14-6** | stable override D5 survives; heap caches are evicted, so re-allowlisting silently reuses the old `.did` (C: `src/lib.rs:301-304`) |
| AD4 | `max_calls_per_script = 0` | holds (kill switch) | refuses every call and fetch (R); query-path cached searches still answer; clean errors |
| AD4 | `max_calls_per_script` large | holds | effective ceiling stays the runtime limits (JS 64x64 rounds, Python 256 suspensions, Rhai 64 requests); `slots.set(n + 1)` cannot wrap because `authorize` refuses at `n >= max` (R) |
| AD4 | `max_instructions = 0` / tiny | holds (kill switch) | first `reserve` and the final check refuse; Python budget 0 gives a clean `TimeoutError` |
| AD4 | `max_instructions` >= ~40e9 | **finding L14-2** | deadline `base + max` lies beyond the IC's per-message limit: Rhai's `on_progress` stop (the fix for register #15) and JS round/job checks no longer fire before the 40B trap. Python unaffected (turn capped at 30e9) (C + R that the value is accepted) |
| AD4 | `max_instructions` near `u64::MAX` (F9) | holds (refuted) | `instruction_deadline` saturates; `vm_budget` is a finite `Duration`; Monty only compares durations and never does `Instant + Duration` (read: `monty-types-1.6.0/src/resource.rs`, `resource/execution_clock.rs`); native run with `Duration::MAX` budgets completes (R). The shim's `Add` is unreachable from Monty |
| AD4 | `call_timeout_seconds = 0` | **finding L14-2** (symptom) | accepted (R); ic-cdk documents that a 0 s timeout "would most likely time out (SysUnknown)", which the bridge reports as "outcome unknown: the call may have executed" for every call (C: `src/bridge.rs:524-530`, ic-cdk `call.rs:288-301`) |
| AD4 | `call_timeout_seconds` > 300 | holds | silently capped at 300 by the IC; README and `.did` say so |
| AD4 | timeout honoured everywhere | **finding L14-4** | only script calls use it; interface fetches always use `bounded_wait` with ic-cdk's default 300 s, two in sequence (C: `src/bridge.rs:426, 434`) |
| AD4 | partial record | **finding L14-3** | `set_limits` replaces the whole record; omitted optional settings decode as `null` and silently revert to unbounded wait / uncertified reads (R) |
| AD4 | change during in-flight runs | **finding L14-8** | `IcBridge` snapshots `max_instructions` and `call_timeout` at start, `check_call` and the final check read the live record (C: `src/bridge.rs:319, 323`, `src/policy.rs:197`, `src/lib.rs:227`) |
| AD5 | token hash validation | **finding L14-1** | digest of `""` accepted; `authenticate` accepts an empty (or whitespace-only, incl. Unicode whitespace) bearer, so that registration lets anyone in (R) |
| AD5 | label length | **finding L14-7** | unbounded; returned by `get_policy` |
| AD6 | revoke | holds | trims/lowercases like add; returns false if absent; takes effect on next request (no cache of tokens) |
| AD7 | override for non-allowlisted canister | **finding L14-6** | accepted; `IcBridge::candid` returns the override text before any allowlist check (C: `src/bridge.rs:471-477`); no message is sent |
| AD7 | size/parse validation | holds | 512 KiB and `Service::parse` checked before storing (`src/lib.rs:324-333`) |
| AD8 | Wasm memory limit 3 GiB (LIM29, D7) | holds | platform default, observed live; exceeding it traps the message (rollback). Observation for L3/L18: memory size 1.25 GB with an 836-entry audit log, i.e. a heap high-water mark from earlier runs that Wasm memory never gives back, leaving about 1.8 GB headroom for JS (register #24) |
| AD8 | Wasm memory threshold 0 | not applicable | the code has no `on_low_wasm_memory` hook, so the setting has nothing to drive |
| AD8 | freezing threshold 30 d, reserved cycles limit | not applicable here (L18) | platform defaults, not set by the project; observed idle burn 32.7e9 cycles/day at 1.25 GB. Running out of cycles without warning is an L18 question |
| AD8 | controllers, log visibility | holds | defaults (deployer only, controllers-only logs); no app code depends on them beyond `is_controller` |
| AD8 | reinstall wipes D1-D5 | holds as platform behaviour | contradicts "immutable" wording (CL4) only for L16 |
| AD9 | feature subsets | holds for this lens | a subset removes engines; limits and policy code are shared. Linking of subsets is L15 (I23) |
| AD9 | `icp deploy --yes` | holds (register #51) | needed for Candid-breaking upgrades; already open in the register |
| LIM1 | default 25 | holds | consistent across runtimes at the default; boundaries tested above |
| LIM2 | default 20e9 | holds | leaves 20e9 headroom under the 40e9 per-message limit for post-run work (result conversion, audit write, reply) |
| LIM3 | default `null` (unbounded wait) | holds as registered (#44, Fixed opt-in) | still the default, observed live; README "Options" explains. Unsafe-for-production default already in the register; not repeated |
| LIM4 | default `null` (uncertified reads) | holds as registered (#47, Fixed opt-in) | still the default, observed live. Whether `true` protects anything is L10-1, not repeated |
| D1 | stored `Limits` record evolution | holds | two optional fields decode as null from old bytes (`policy::old_audit_entries_still_decode`) |
| D7 | canister settings in `icp.yaml` | holds | no `settings:` block, so platform defaults apply; nothing in the code assumes other values |
| C7 | `init_args: "(null)"` | holds | `apply(None)` is a no-op at install and upgrade; tokens and allowlist are added afterwards per README |
| C7 | local network port 8010 | not applicable | dev only |
| E8 | `set_limits` guard | holds | `guard = "controller"` and `inspect_message`; validation gaps are L14-2/L14-3 |
| E6, E7, E9-E11 | admin updates | see AD2-AD7 | guard holds (controller); content gaps listed above |
| E12 | `get_policy` | **finding L14-7** (symptom) | returns every label and method list in one query reply, no paging |
| F14 | `max_instructions` above 40e9 | **finding L14-2** | as AD4 |
| U11 | admin inputs | see AD1-AD7 | |
| A5 | operator | holds | all settings require a controller; the lens findings are about values a controller can set |
| LIM5-LIM28, LIM30-LIM32 | compile-time constants | not applicable | not operator-settable; covered by L3/L6 boundary tests |
| C8-C10 | build flags, toolchain pin | not applicable here | build configuration belongs to L15 |
