# Variant sweep r4-6: serialised, re-parsed under different limits, parse error defaulted

Run `2026-11-17-295d64b`, Phase 6, round 4. Pattern (seed, known, not re-reported: L7-2, JS
call arguments are written with `Value::to_string` and re-read with
`serde_json::from_str(..).unwrap_or(Value::Null)` at `src/runtimes/js.rs:395`, so arguments
128 containers deep become `null` and the call is still sent):

> A value serialised and re-parsed between components with different limits, with the parse
> error defaulted instead of reported.

Question asked of every inventory row: does a value leave one component in a serialised
form (text, Candid bytes, JSON, an agent's structured output) and get read back by another
component whose rules differ (depth, type, schema, version), and if the reader fails, is the
failure turned into a default (`null`, `None`, empty, "nothing found") instead of an error?

Method: grep of every `unwrap_or*`, `.ok()`, `if let Ok`, `from_str`, `from_slice`,
`Decode!`, `decode_one`, `to_dynamic`/`from_dynamic`, `from_json`/`to_json` site in
`src/`, `demo_store/src/`, `shims/`; reading `src/demo.html`, `scripts/`, `video/*.mjs`,
`.claude/workflows/audit.js`; dependency source for the readers (Boa `from_json`, Monty
`object_bridge`/`MontyGraph::type_name`, ic-stable-structures `StableCell::init`, Monty
execution clock). One native probe in a private copy (no `target/`, `.icp/`, `.git`,
`design/`, `video/`, `.env`, `docs/per-customer-canisters-plan.md`), shared
`CARGO_TARGET_DIR`, `cargo test --lib sw_probe`; copy deleted afterwards. Two read-only
queries to the local `demo_store` (port 8010) with icp-cli. Nothing deployed, upgraded or
reconfigured; no `icp_code_mode` method was called. **R** = reproduced, **C** = code path.

## Candidates

### SW4-6-1 Install, upgrade and admin arguments under `opt` that do not fit their type are decoded as `null`, and the setting is silently not applied (Medium)

- **Where:** `src/lib.rs:62-72` (`init(args: Option<InitArgs>)`, `post_upgrade(..)`),
  `src/lib.rs:38-49` (`apply`: `None` returns, `limits: None` keeps the old limits),
  `src/lib.rs:19-24` (`InitArgs.limits: Option<Limits>`), `src/policy.rs:33, 38`
  (`Limits.call_timeout_seconds: opt nat32`, `certified_reads: opt bool`),
  `src/lib.rs:324` (`set_candid_override(.., did: Option<String>)`, `None` = clear).
- **Mechanism:** the operator's tool serialises the arguments as Candid with its own types;
  the canister re-reads them with ic-cdk's typed decoder. Candid's `opt` rule turns any
  value under `opt` that does not fit the declared type into `None` instead of a decoding
  error. The app then treats `None` as "not given". Outside `opt` the same mistakes are a
  loud error (control P2c).
- **Evidence (R, native, candid 0.10.40 as locked; untyped encoding = `candid_parser::parse_idl_args(..).to_bytes()`):**
  - P1 `(opt record { token_sha256 = vec {"a"x64}; allowlist = vec {}; limits = opt record { max_calls_per_script = 5; max_instructions = 1_000_000_000 } })`
    -> `token_sha256` applied, `limits = None` (plain integer literals are `int`, not
    `nat32`/`nat64`). Install/upgrade succeeds with the default 25 / 20e9 limits.
  - P2 same with `allow_list` misspelled -> the whole `InitArgs` is `None`: no token, no
    allowlist, no limits; `apply(None)` returns; install/upgrade succeeds.
  - P2c control: `set_limits(record { ..; max_instruction = 1 : nat64 })` (not under `opt`)
    -> decode error.
  - P3 `set_limits(record { max_calls_per_script = 25 : nat32; max_instructions = 20_000_000_000 : nat64; call_timeout_seconds = opt 60; certified_reads = opt true })`
    -> `call_timeout_seconds: None` (unbounded wait, register #44's protection off) while
    the operator asked for 60 s; `certified_reads: Some(true)`.
  - P3b `certified_reads = opt 1` -> `None` (uncertified queries).
  - P4 `set_candid_override(principal "aaaaa-aa", opt blob "service : {}")` -> `did = None`,
    i.e. "clear the override"; same with ic-cdk-style quotas
    (`decode_args_with_config`).
  - Live, read-only (R): the same rule in a deployed ic-cdk canister. `demo_store
    list_products` with hex `4449444c016e7c01000105` (`opt int 5`, untyped) returns all 12
    products, exactly like `(null)`; typed `(opt "tea")` returns 4.
- **Who triggers it:** any encoder that does not type the arguments against the canister's
  `.did`: `--args-format hex`/`bin`, another canister (the factory idea in C14, an
  orchestrator, a governance proposal payload), agents with a hand-written or stale IDL.
  icp-cli 1.7.0 `canister call` types arguments against the fetched interface and refuses
  P1-style mistakes client-side (R: `list_products '(opt 5)'` -> "type mismatch: 5 can not
  be of type text"), so the README's documented admin calls are protected. For `init_args`
  / `upgrade_args` / `icp canister install --args`, the icp-cli binary's error strings for
  that path are only "failed to parse Candid args" / "failed to encode Candid args to
  bytes" (no "with specific types" variant, unlike `call`), which suggests inferred types;
  **not executed** (installing is a deploy). If install args are untyped, P1 is the
  ordinary way to write limits in `icp.yaml`, and they are dropped on every deploy.
- **Distinct from known findings:** "A terse or old-shape set_limits (or InitArgs.limits)
  silently turns certified_reads and call_timeout_seconds off" is about *omitted* fields
  under whole-record replacement; here the operator *supplied* the value and it is
  discarded by the decoder. "post_upgrade re-applies InitArgs additively" is about what
  `apply` does with arguments it received; here `apply` never receives them.
- **Invariant:** Honest interface / Fidelity (data crossing a boundary arrives intact or
  fails loudly); Configuration (an operator is misled: success reply, setting not applied).
- **Fix (proposed, not applied):** make the admin and install arguments non-`opt` where
  absence is not meaningful (`InitArgs.limits` as a record with defaults supplied by the
  operator, or a separate `set_limits` call), or decode them untyped first and compare the
  wire type with the expected type (`candid::types::subtype::subtype_with_config` with
  `OptReport::Error`) and trap on mismatch; at minimum, echo the applied `Limits` and
  override state in the reply / a log line so the operator can see what was stored.
- **Regression test:** P1, P2, P3 and P4 as unit tests on the decoder: each must be an
  error, never `None`.

### SW4-6-2 The audit workflow turns an agent output it could not obtain into "nothing found", and that counts toward the stopping rule (Medium)

- **Where:** `.claude/workflows/audit.js:192-194` (`verify`: `if (!found) return null`),
  `:208-221` (`absorb` filters out `null` results), `:230-247` (review and sweep results go
  through `verify`/`absorb`; `fresh` 0 -> `dry + 1`), `:268-269` (`work = critic ? .. : []`
  then logs "the critic found no further gaps"), `:287-291` (live verdicts matched by exact
  `title`; an unmatched refutation is ignored).
- **Mechanism:** each agent's answer is serialised by the agent, schema-checked by the
  harness and read back by the script; the script already knows the read can fail (it
  handles `null` for the inventory, live and report agents). For reviewers, verifiers,
  sweepers and the critic the failure is defaulted instead:
  1. A reviewer or sweeper that does not finish: `verify(null)` returns `null`, `absorb`
     drops it, nothing is added to `notTested`, nothing is logged; the round can count as
     dry.
  2. A verifier that does not finish for a non-empty candidate list: the candidates are
     neither confirmed nor refuted nor listed anywhere; they vanish from the report.
  3. A critic that does not finish: `work = []` and the log says "the critic found no
     further gaps"; the loop ends.
  4. A live agent that rewrites a title: its refutation (`real: false`) matches nothing
     and is dropped, so the finding stays confirmed without its live refutation.
- **Evidence (C):** unambiguous from the script; not executed (running the workflow is out
  of scope for a sweep agent).
- **Effect:** the run report, the register and `complete_by_stopping_rule` are records the
  plan relies on. Agent failures can make a round look dry (two of them satisfy the
  plan's stopping rule), drop candidate findings without trace, and print a false
  "critic found no further gaps".
- **Invariant:** Records match reality (the audit report is the record); plan section 1
  ("an empty cell is unfinished work, not an assumption of safety").
- **Fix (proposed, not applied):** treat a `null` agent result as an error everywhere:
  log it, add the lens/pattern to `notTested` and back to `work`, never count that round
  as dry, carry unverified candidates into the report as "unverified"; log "the critic did
  not finish" separately; match live verdicts by a stable id passed in the prompt, not by
  title.
- **Regression test:** a dry-run harness for the workflow with an `agent()` stub that
  returns `null` for one reviewer, one verifier and the critic; assert the round is not
  dry, the candidates appear as unverified and the log names each failure.

## Grid

| Row | Serialise -> re-parse / convert site | Verdict |
|---|---|---|
| B8 JS -> JSON -> text -> JSON (call args) | `js.rs:262-263` `to_string`, `:395` `from_str(..).unwrap_or(Null)`; only failure mode is serde_json's 128-level limit (numbers are finite and re-read exactly by ryu/serde_json; strings round-trip except lone surrogates, known L7-10) | known (seed L7-2) |
| B8 JS result / printed values | `to_json_safe` -> `Value` directly (no text round trip); print failure -> `<unprintable value>` | holds / known (print claim) |
| B8 JS thrown non-Error value | `error_string` (`js.rs:335-337`): snapshot failure (BigInt, depth) -> "Uncaught <unserializable value>"; the run is still an error, only the detail is lost | holds (failure reported as failure) |
| B8 JSON -> JS replies | Boa `from_json` errors are mapped to a script error (`js.rs:403`); no depth or range default (numbers outside i32 become f64, exact <= 2^53, and replies carry larger ints as strings) | holds |
| B8 top-level `undefined` | `to_json(..)?.unwrap_or(Null)` (`js.rs:214`): a script returning nothing yields `null` | holds (JSON.stringify semantics) |
| B9 Python -> JSON | `to_json`; `unwrap_or_default` on `as_bool/as_str/items/pairs` unreachable: `MontyGraph::type_name` (`monty-types graph.rs:546`) names exactly the node kinds those accessors read; export-guard markers known | holds / known |
| B9 JSON -> Python | `from_json`; Monty import is arena based (`object_bridge.rs:59-100, 878+`), no depth cut on the way in; `as_f64().unwrap_or(NaN)` unreachable (no `arbitrary_precision`) | holds |
| B10 Rhai <-> JSON | `to_json_value` errors propagate; `to_dynamic` errors propagate (Rhai size-limit refusal known); `from_dynamic` NaN -> null known L7-4 | holds / known |
| B5/B6 JSON <-> Candid (`json_to_idl`, `idl_to_json`, `big`) | errors reported; `big()`'s `unwrap_or_else` unreachable (`mag` <= 2^53 and `s` has at most one `-`) | holds |
| U8 / O1 Candid bytes to and from targets | target's decoder applies the opt rule to our arguments and ours to its replies under a stale/overridden interface | known (L7r3-1, both directions, P7) |
| B7 / D5 / V5 override text re-parsed | `cached_service` (`bridge.rs:207-213`) maps an override parse error to `None` ("must fetch"); `run_search` then renders "interface not fetched" for it. Unreachable at this commit: `set_candid_override` stores only text that `Service::parse` accepts, `Service::parse` is deterministic and `candid_parser` has been 0.4.1 since the override was added (`git log`); would surface after a parser upgrade. Query-path re-parse cost known | holds (latent) / known |
| B7 fetched interface | parsed once in the update, `Rc<Service>` cached (no re-parse); `canister_metadata` decode failure -> fallback known; non-UTF-8 metadata reported | holds / known |
| B2 / E3 `HttpUpdateRequest` decoded twice | `inspect_message` vs method decoder quotas known; a failed decode in `inspect_message` defaults to "reject", the safe side | known / holds |
| B1 / U3 request body | one `from_slice`, error answered with -32700; query and update paths parse the same bytes with the same parser | holds |
| U4 JSON-RPC ids | serde_json f64 re-read of large ids | known (L7-9) |
| U5 search `canister_id` | `Principal::from_text(id.trim()).ok()` (`search.rs:39`): an invalid id is reported as "not allowlisted" instead of "invalid canister id"; the bridge reports it. Same line and root cause as the known whitespace finding | known (merge as symptom of "Search accepts a whitespace-padded canister_id ...") |
| U5 tool arguments | name/arguments defaults, `language: null` | known |
| B11 tool result text | `serde_json::to_string(..).unwrap_or_default()` (`mcp.rs:159`), `to_vec(..).map_or(0, ..)` (`mcp.rs:405`), `to_vec(..).unwrap_or_default()` (`mcp.rs:62`): serialising a `serde_json::Value` cannot fail (string keys, finite numbers, Vec writer) | holds |
| B11 truncation to invalid JSON | client re-parse fails | known (L7-1) |
| B12 stable Candid (`Limits`, `AllowedMethods`, `AuditEntry`) | `Decode!(..).expect` traps rather than defaults; old-shape `Limits` -> opt fields `None` is the known terse/old-shape finding; `StableCell::init` silently re-initialises on a magic mismatch (`ic-stable-structures-0.7.2 cell.rs`), unreachable: memory ids 0-5 never changed (`git log -p src/policy.rs`) | holds / known |
| E1 / E2 / AD1 `opt InitArgs`, `InitArgs.limits` | mistyped value under `opt` -> `None` | **SW4-6-1** |
| E8 / AD4 `Limits` opt fields | mistyped `call_timeout_seconds` / `certified_reads` -> `None` | **SW4-6-1** |
| E11 / AD7 `set_candid_override(.., opt text)` | mistyped text -> `None` = clear | **SW4-6-1** |
| E6 / E7 / E9 / E10 / E12-E14 | no `opt` argument; mistypes are loud decode errors | holds |
| E4 / E5 `HttpRequest.certificate_version: opt nat16`, `HttpResponse.upgrade: opt bool` | gateway-defined types; a mismatch would only drop an optional hint | holds |
| E20-E25 demo_store `opt` filters | `list_products(opt text)` / `list_orders(opt principal)`: mistyped filter -> all rows (R live, read-only: 12 products for `opt int 5`). Reached through icp_code_mode only with a stale interface (known); direct callers: demo_store has no access control and `list_orders(null)` is known | known / n/a |
| B13 language name | `FromStr` error reported | holds |
| B14 time / web-time shim | `checked_add` -> `None` is std-compatible; Monty's execution clock uses only `elapsed`/`saturating` (`monty-types resource/execution_clock.rs`), no `Instant + Duration` | holds |
| B15 certificate header | constant tree | n/a |
| B16 search rendering | `as_service(..).unwrap_or_default()`, `as_func(..).ok()`, `find_type(..).ok()` (`search.rs:104-137`): the same `TypeEnv` that `check_prog` validated; both search and `Service::method` resolve through the same `TypeEnv` calls, so no method is dropped by one and accepted by the other | holds |
| B17 demo page | `pretty()` falls back to raw text (display only); `post`/`tool`/`classicCall` throw on parse failure; interface regexes match all 10 demo_store methods and all 13 types of `demo_store.did` (R, node) (estimate only; known regex cost) | holds / known |
| C6 `scripts/demo-setup.sh` | sed over CLI text, empty result is a loud error | known (L18-7) |
| C12 video tooling | `tts.mjs:68` sample rate defaults to 24000 when the mime type has none (Gemini's documented PCM rate); dev only | n/a |
| C13 audit workflow | agent output not obtained -> "nothing found" / "no further gaps" | **SW4-6-2** |
| C13 audit workflow, `keyOf` | `(location + '|' + root_cause).slice(0, 160)` dedupe: two different root causes at one long location collapse and the second confirmed finding is dropped silently. Lossy key, not a parse default: outside this pattern, noted for the critic | note |
| C1-C5, C7-C11, C14, C15, A1-A12 | no further serialise/re-parse boundary | n/a |
| O2-O10, D1-D10, V1-V11, K1-K12 | covered above (O1/O2 via U8/B7, D1-D5 via B12/D5, V2/V5 via B7) | as above |
| LIM1-LIM32 | the only limit pairs that sit on a re-parse are serde_json 128 vs the 128-level snapshot (seed), Candid opt typing (SW4-6-1, L7r3-1) and Monty's export guard (known) | known / SW4-6-1 |
| W1-W16, F1-F16, VF1-VF8, S1-S10 | no additional re-parse; F3 is a trap, not a default | n/a |
| CL1-CL37 | README:379 "`post_upgrade` accepts the same optional `InitArgs` as `init`" says nothing about mistyped arguments being dropped (SW4-6-1); CL21 `.did` Limits comments describe `null` as "default", which is also what a mistyped value becomes | SW4-6-1 |
| T1-T12 | no test decodes a mistyped admin/install argument; no test for the workflow script | SW4-6-1 / SW4-6-2 |

## Not tested

- Whether icp-cli 1.7.0 encodes `init_args` / `upgrade_args` / `install --args` with the
  wasm's interface types: needs an install or upgrade, which is the Live phase. A Live
  repro: deploy a throwaway copy with `init_args` P1 and read `get_policy().limits`.
- No admin method of the running `icp_code_mode` was called with a mistyped argument
  (setting changes are outside this phase); the canister-side rule was reproduced natively
  and on `demo_store` (same ic-cdk decoder) with a read-only query.
- SW4-6-2 was not executed: running the audit workflow with a failing agent stub needs a
  workflow harness this repository does not have.
