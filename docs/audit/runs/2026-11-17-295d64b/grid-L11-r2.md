# Coverage grid: L11 Sandbox and determinism, round 2 (std unsupported-platform panics)

Run `2026-11-17-295d64b`. Task: replace round 1's sampled probes (`grid-L11-r1.md`, cell C3:
"none found reachable") with a trace of every std unsupported-platform panic linked into the
release Wasm, back to the exported entry points.

Evidence key: **R** reproduced, **C** code path only.

## Method

1. **Deployed module.** `software.wasm` of `icp_code_mode` in the local network state, sha256
   `ad3ee221...2411` (equals the module hash `icp canister status` reports). Data segment 0 is
   at memory `0x800000`. Panic `Location` structs (`ptr, len, line, col`) were located by
   searching the data for pointers to the std source paths, then `wasm2wat` was scanned for
   `i32.const <Location address>` and a direct-call graph was built (34,612 functions,
   6,037 table entries). The deployed module is stripped (`strip = true`), so:
2. **Named twin.** Private copy (`rsync` without `target/`, `docs/`, `design/`, `video/`), a
   profile `audit-names` that only inherits `release` and sets `strip = false`, shared
   `CARGO_TARGET_DIR`. Same panic stubs, same caller-set sizes per stub as the deployed module
   (sleep 56/56, `SystemTime::now` 4/4, `Condvar::wait_timeout` 2/2, `Instant::now`/`elapsed`
   2+2), so callers were named there (v0 names demangled with `rustc-demangle` 0.1.28 from the
   local registry) and walked to exports.
3. Line numbers were checked against std source (`sys/time/unsupported.rs:13` =
   `Instant::now`, `:35` = `SystemTime::now`, `sys/thread/unsupported.rs:45` = `sleep`).
4. Native probe test in the private copy, then three live scripts on the local canister (no
   deploy, no setting changed). Copy and build outputs of this round deleted afterwards.

## Linked std unsupported-platform panics and their callers

| Panic (deployed func / named func) | Direct callers (named build) | Reachable from script input? |
|---|---|---|
| `std::thread::sleep` "can't sleep" (`32157` / `34178`) | 56 callers, all Rhai: `lang_core::sleep` and `sleep_float` (the `sleep(n)` built-in), and every inlined `rhai::func::native::locked_read` / `locked_write` (`Dynamic::read_lock`/`write_lock`/`as_int`/`is_map`/..., `rhai_rt::to_json_value`, `Engine::eval_stmt`, ...) | **Yes (R).** `sleep(1)`, `sleep(0.000001)`, and a Rhai data race (`let x = 20; let f = \|a\| this += x + a; x.call(f, 2)`) trap: `Panicked at 'can't sleep', .../library/std/src/sys/thread/unsupported.rs:45:5`, HTTP 503 from the gateway |
| `Condvar::wait_timeout` (same "can't sleep" Location) (`32154` / `34139`) | `Condvar<FutexWaiters>::wait_timeout` <- `Atomics.wait`; `Condvar<()>::wait_timeout` <- `parking::Parker::park` <- `futures_lite::block_on` <- `boa SimpleJobExecutor` | No (C). `Atomics` deleted from the global (`js.rs:314`) and no `SharedArrayBuffer`; `SimpleJobExecutor` is only the fallback when no executor is given (`boa context/mod.rs:1223-1225`), and `js.rs:281` always gives `BoundedJobs` |
| `Condvar::wait` "condvar wait not supported" (`32155` / `34140`) | same two chains as above | No (C), same reasons |
| `Instant::now` "time not implemented" `:13` (`32152` / `34134`) | `Atomics.wait`; `StdClock::new` <- `ContextBuilder::build` <- `js::new_context` | No (C). `build` calls `StdClock::new` only in `self.clock.unwrap_or_else(...)` (`context/mod.rs:1211`); `js.rs:278` always sets `FixedClock` |
| `Instant::elapsed` (`32153` / `34135`) | `Atomics.wait`; `StdClock::now` (vtable only) | No (C): no `StdClock` instance exists |
| `SystemTime::now` "time not implemented" `:35` (`32156` / `34156`) | `<js::Hooks as HostHooks>::utc_now` and `DefaultHooks::utc_now` (vtable slots; `time::OffsetDateTime::now_utc`); `StdClock::system_time_millis` (vtable); `chrono::Utc::now` (vendored, `utc.rs:97-98`) <- `monty_types::DateTimeSource::read` <- `sandbox_instant` (datetime/time clock calls) and `StandardLib::create` | No (C). `HostHooks::utc_now` is deprecated and has no call site in boa 0.22.0 (Date uses `context.clock()`); no `StdClock`; `DateTimeSource::read` calls `Utc::now` only for `System`, and every Monty VM takes its policy from `program.vm_env()` (`vm/mod.rs:935, 1007`), which `python.rs:212-233` sets to `Fixed`; `VmEnv::default` (the only `DEFAULT_OS_POLICY` user) has no non-test caller |
| `parking_lot_core` wasm parker "Parking not supported" (3 funcs) | `dashmap::RawRwLock` slow paths <- Boa `Symbol.for` / `Symbol.keyFor` global registry | No (C): slow path needs contention; single-threaded and no guard is held across script code |
| `std::env::vars` "not supported on this platform", `sys::random::fill_bytes` | not linked (strings absent) | n/a |
| `std::fs`, `std::env::var`, `thread::spawn` on unsupported | return `Err`, no panic | n/a |

Not a std panic but checked on the same path: Monty's own `block_for` on wasm32 spins on the
web-time shim (`monty-types resource.rs:837-845`), only for `SleepMode::System`; the runtime
uses `SleepMode::Zero`.

## Live evidence (local `icp_code_mode`, rhai)

| Script | Result | Audit |
|---|---|---|
| `sleep(0); sleep(-1); sleep(0.0); 41 + 1` (control) | `42` | normal entry |
| `print("before"); sleep(1); 7` | HTTP 503 `canister_error`, trap message above; no JSON-RPC body | no entry (log length 1149 before and after) |
| data race `x.call(f, 2)` | HTTP 503, same trap | no entry |
| `let p = icp::call_canister(demo_store, "get_product", [1]); print("got reply"); sleep(1); p` | HTTP 503 after the call was sent and answered; result and logs lost | `Started` (1161) then `Trapped` (1162) with the call recorded |

Native (private copy, `cargo test --lib l11r2_probe`): `sleep(1)` returns after 1.01 s;
`sleep(1e-6)` returns; the data race returns `Err("Data race detected ...")` after ~64 ms
(five 10 ms `std::thread::sleep`s), and inside `try` it is caught. So the native suite sees
working behaviour where the canister traps.

## Grid

| Row | Cell | Reason / evidence |
|---|---|---|
| I7 Determinism | holds | No linked std clock is reachable (table above). The reachable unsupported panic (sleep) is deterministic: it traps on every replica |
| I5 Clean failure | **finding** (L11-r2-1) | Any token holder's Rhai `sleep(n)` (n > 0) or a Rhai data race ends in a canister trap, not a JSON-RPC error (R) |
| I20 Sandbox / CL29 | **finding** (L11-r2-1) | Rhai keeps a blocking `sleep` built-in; Python's sleep is zero (`SleepMode::Zero`), JS has none; Rhai's traps. `no_time` removes only `timestamp`/`elapsed` |
| C3 web-time shim | holds (now traced) | The shim's `Instant` is used by Monty only; std `Instant::now` stubs are linked but reachable only through `StdClock::new` (never built) and `Atomics.wait` (deleted). `SystemTime` re-export: every linked `SystemTime::now` caller is unreachable (table). Round 1's "none found reachable" is confirmed for the clock; it was wrong in spirit for the panic family, since `can't sleep` is reachable |
| C4 vendored chrono | holds | `Utc::now` is linked (Locations `utc.rs:97:58`, `:98:76` in the deployed data) but reached only via `DateTimeSource::System`, which no Monty VM in this build uses. Holds as long as every `MontyRun` sets a non-`System` `datetime` (one construction site, `python.rs:212-233`; no test asserts it on wasm) |
| F12 native stack overflow | not applicable to this round | No std unsupported-platform panic sits on a recursion path; stack overflow is a separate trap class (L3/L9) |
| F14 40e9 instruction limit | holds for this question | Known script-triggered traps (register #23, round 1 L11-1) cost up to 40e9 instructions; the sleep trap is a cheaper, unconditional member of the same "script can trap the message" family, recorded as L11-r2-1 |
| F (Failure sites inventory) | **gap** (part of L11-r2-1) | The inventory derived failure sites from `expect`/`unwrap`/indexing in the app; panics inside std's wasm32 stubs reached through dependencies (`thread::sleep`, `Instant::now`, `SystemTime::now`, `Condvar::wait`) were not a category. Derivation that finds them: Locations pointing at `library/std/src/sys/*/unsupported.rs` (and `no_threads.rs`) in the release Wasm, plus a caller walk |
| E19 Rhai `icp::*` / engine built-ins | **finding** (L11-r2-1) | `StandardPackage` (`rhai_rt.rs:121`) includes `LanguageCorePackage::sleep`; the `sync` feature (`Cargo.toml`) turns shared-variable locks into a `try_read`/`try_write` loop with `std::thread::sleep(10 ms)` (`rhai-1.27.0/src/func/native.rs:650-700`) |
| E16/E17 JS host functions | holds | No std sleep/clock caller in Boa is reachable (table) |
| E18 Python | holds | No Monty path to `Utc::now`, std sleep or std `Instant` with the configured policy |
| T (tests) | gap (part of L11-r2-1) | Every engine test runs natively, where std sleep and clocks work, so this class cannot fail a test. No wasm-target execution test exists (register #58 harness gap) |

## Not tested

- A rhai-only feature build was not built; the same Rhai code is linked, so the same trap is
  expected (C).
- The named twin's code was matched to the deployed module by stub signatures and caller-set
  sizes, not byte for byte (the twin has 34,580 functions vs 34,612 deployed; builds are not
  reproducible, see S5/I24). All reachability conclusions were also checked in source.
- Indirect calls were resolved by reading the source of each vtable owner, not by type-based
  call-graph analysis of `call_indirect`.
</content>
</invoke>
