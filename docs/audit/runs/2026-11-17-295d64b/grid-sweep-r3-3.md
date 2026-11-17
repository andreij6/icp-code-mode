# Variant sweep r3-3: a filter on one entry path, claimed for every path

Run `2026-11-17-295d64b`, Phase 6, round 3. Pattern (seed, known, not re-reported: L5-3,
the canister-caller variant of register #30, "Canisters without a token make icp_code_mode
pay for every call; the unpaid-call filter covers ingress only"):

> A filter enforced on one entry path and claimed for every path.

Question asked of every inventory row: is there a filter here (authentication, cost filter,
allowlist, cap, budget, size check, sandbox removal, canonicalisation, switch-off) that is
installed on one way in, while a sentence in the README, the `.did`, a code comment or a text
shown to the AI says it holds for all of them? Ways in considered: ingress through the
gateway (query, then upgraded update), direct ingress, inter-canister callers (update and
replicated query), self-calls, the cleanup callback, `init` / `post_upgrade` vs the admin
endpoints, the three runtimes and each SDK function, `icp.candid` vs `icp.call` vs search,
the two interface-fetch transports, the two demo-page request helpers, and the build paths
(default vs per-language worker).

Method: code reading at `295d64b` (`src/*.rs`, `src/runtimes/*.rs`, `src/demo.html`,
`demo_store/src/lib.rs`, both `.did` files, `icp.yaml`, `README.md:98-125, 360-500`), every
absolute comment in `src/` (`grep -i "every|always|never|cannot|only|all "`), and the Boa
0.22 / Monty 1.6 sources for the sandbox removals (`boa_engine-0.22.0/src/context/hooks.rs`,
`builtins/date/mod.rs`, `monty-types-1.6.0/src/os_policy.rs`, `monty-1.6.0/src/modules/time.rs`).
Nothing compiled, nothing run against the live network. Round-1 sweep r1-4 ("guardrail on one
path, resource open on another") and round-2 sweep r2-9 ("per-request limit in one of two
sibling handlers") already covered most rows from the guardrail side; this sweep starts from
the claims and re-checks the rows they did not name.

Evidence key: **CP** = the code path settles it; **known** = a listed finding already covers
it (named); **holds** = every path applies the filter, or no claim extends it.

## Result

**No new candidate.** Every claimed-universal filter either runs on every way in, or its gap
is already a known finding.

## Cells

### Components (C)

| Row | Filter and its claim | Cell |
|---|---|---|
| C1 icp_code_mode | `inspect_message` "rejects update calls that carry no registered token ... before the canister is charged" (README:375, `lib.rs:265-272`) | **known** (seed L5-3: canister callers, including replicated `http_request` and admin methods) |
| C1a feature subsets | Language filter: tools/list advertises three languages | **known** (per-language builds advertise all three) |
| C2 demo_store | C1's allowlist presented as blocking `cancel_order` / `restock` | **known** (demo_store has no access control; README presents them as blocked) |
| C3 web-time shim | "deterministic instruction budgets" (README:484) applies wherever Monty reads `Instant` | holds (CP): Monty's clocks other than the VM tracker read `OsPolicy` (`modules/time.rs:3-6, 276-287`); `ProcessTime` defaults to `Zero` (`os_policy.rs:340-346`), so no script-visible reading of the shim exists |
| C4 vendored chrono | n/a: library, no entry path |
| C5 demo page | token "sent only as the Authorization header" (README:239-240, `demo.html:81`) | holds (CP): both helpers, `rpc()` (`demo.html:211-223`) and `post()` (`:363-368`), put the token only in `Authorization`; `$("raw")` shows the body, not headers. Demo page off under `certified_reads`: the update route never serves it (`authenticate` gives 404/405 first, `mcp.rs:271-275`) |
| C6 demo-setup.sh | "stay blocked" | **known** (as C2) |
| C7 icp.yaml / C10 release profile | `--locked` on both canister builds (`icp.yaml:14, 28`) | holds: the README worker recipe (`README.md:116`, `icp.yaml:6`) omits `--locked`, but nothing claims every build is locked, and `cargo build` without `--locked` still uses the committed `Cargo.lock` unless the manifest changed. No effect shown. The worker recipe's stale-wasm problem is **known** (hard-coded `./target` copy) |
| C8 / C9 | n/a: flags and toolchain, no filter with a claim |
| C11 `.did` | `Limits.max_instructions` "Checked ... before every call and after the run" | **known** (budget checked in `reserve` before the fetch and encode, not again before the send) |
| C12 video / C13 audit tooling / C14 idea doc | n/a: no entry path into the canister beyond the demo page (C5); video token handling **known** |
| C15 non-method exports | n/a: not callable as messages |

### Actors (A)

| Row | Cell |
|---|---|
| A1 anonymous web | holds (CP): only `GET /` and `/demo` are served without a token, on the query route only |
| A2 direct ingress | holds (CP): `inspect_message` covers `http_request_update` (token) and every other method name, including `http_request` called as an update (`_ =>` branch, controller only, `lib.rs:274-282`) |
| A3 token holder | holds: one `authenticate` for query, update and `inspect_message`. Revoked tokens in flight: **known** |
| A4 AI model | claims shown to the AI ("every run is audited", search "free ... no audit entry ... does not count toward the call cap", "no randomness", "identical semantics in every language") are all **known** findings |
| A5 controller | holds (CP): admin methods carry `guard = "controller"` on every route plus `inspect_message` for ingress. `InitArgs` reuse the same validators (`apply`, `lib.rs:37-48`); InitArgs has no override field, so `set_candid_override`'s size and parse checks have no unchecked sibling |
| A6 target / A7 `aaaaa-aa` | Allowlist "checked before any message is sent, including the interface fetch" (README:365): the fetch checks only that the target is allowlisted (`policy.rs:200-203`), not its method list, and sends `aaaaa-aa.canister_metadata` and `<target>.__get_candid_interface_tmp_hack`. holds as documented: README:493-496 names both transports as the fetch, and the README claims a principal check for fetches, which is what runs. Re-check after the first await and audit labels: **known** |
| A8 other canisters | **known** (seed L5-3). Unverified side note, not a candidate: the request size bound in the inventory (U1-U3, "ingress message limit") is an ingress property; same-subnet inter-canister requests may be larger. Every work item it would enlarge (JSON parse, search keywords) is already a known finding and needs a token |
| A9 infrastructure | `certified_reads` "every request goes through an update call" | **known** (unauthenticated / wrong-path requests still answered by the uncertified query; skip-certification data on every path) |
| A10 demo_store callers | **known** |
| A11 / A12 | n/a |

### Entry points (E)

| Row | Cell |
|---|---|
| E1 / E2 | holds: same `apply`; additive re-apply on upgrade is **known** |
| E3 `inspect_message` | **known** (seed; decode without skipping quota also known) |
| E4 / E4a / E4b query route | `MAX_SEARCHES`, `DID_FAILED`, instruction budget missing on this route: **known**. `MAX_BATCH`, reply cap, one-script rule: holds (shared `parse_body`, `finish`, upgrade on any `Execute`, `mcp.rs:293-366`) |
| E5 / E5a / E5b update route | `RunSlot` only on scripts, searches unaudited when cached, no-tool updates unaudited: **known** |
| E5c callbacks | holds: continuations of the same `IcBridge`, no separate filter |
| E5d cleanup | `Trapped` entry drops the violation count: **known**; error text is fixed, `methods` comes from the same bridge |
| E6-E11 admin updates | holds (A5). `remove_allowlist` is the only path that evicts caches and the only way to leave the allowlist short of reinstall; overrides kept: **known** |
| E12-E14 admin queries | holds: guard on every route. "Read the audit log with an update call" (README:439) is possible: an ingress update to `get_audit_logs` passes `inspect_message`'s controller branch |
| E15 | n/a |
| E16 JS SDK | value limits on `args` only, not on `canister` / `method`: **known** (r2-9 candidate). `MAX_PENDING` counts `icp.call` and `icp.candid` alike (`js.rs:250-268`); getter overrun **known**. `eval` / `Function` blocked on every compile path through `HostHooks::ensure_can_compile_strings` (r1-4) |
| E17 console/print | log cap and formatting: **known** |
| E18 Python SDK | kwargs refused for every `icp.*` function before the eager/queued split (`python.rs:245-248`); sync and async share `host()` (`:123-138`). `eval`/`exec` bypassing `MAX_CODE_BYTES`, `random`, clock granularity: **known** |
| E19 Rhai SDK | both `call_canister` arities and `candid` go through `request()` and `MAX_REQUESTS` (`rhai_rt.rs:154-176, 211`); 2-arg form sends `null`, which needs no value check. `eval` disabled engine-wide (r1-4). The 1M-operation cap applies per replay, which bounds the script's logical execution (grid-L16 CL13) |
| E20-E25 demo_store | **known** |
| E26 demo page / E27 setup script / E28 video | see C5, C6, C12 |

### Inputs, effects, state

| Row | Cell |
|---|---|
| U1-U5 | holds: one parser for both routes. `query` / `canister_id` unbounded and the padded `canister_id`: **known** |
| U6 / U7 | `MAX_CODE_BYTES` in `LanguageRouter::execute`, the only way into the engines except Python `eval` (**known**). Value limits per runtime and per argument: **known** |
| U8-U10 | reply / interface quotas per transport: **known** (tmp_hack decode without quota) |
| U11 | see A5 |
| U12 | **known** |
| U13 | **known** |
| U14 | see C5 |
| U15 | holds: JS time only through `context.clock()` (`boa_engine-0.22.0/src/builtins/date/mod.rs:58, 246, 364`; the deprecated `utc_now` hook has no caller), `local_timezone_offset_seconds` overridden to 0 (`js.rs:162-170`); Boa is built without `temporal` / `intl` (`default-features = false`). Python clocks fixed by `OsPolicy` |
| U16 | n/a |
| O1 script calls | holds: `reserve` runs on every runtime's call path; composite_query and oneway refused before send on every path (search lists them with their mode, grid-L12 B16) |
| O2 interface fetch | see A6 |
| O3 audit writes | coverage gaps are **known** |
| O4-O8 | holds: single guarded writers |
| O9 | **known** |
| O10 | **known** |
| D1-D3 | holds: written only by the admin endpoints or `apply`, which share validators |
| D4 | writers `run_script`, `run_search`, `record_message`, cleanup: error and violation caps differ only where **known** |
| D5 | **known** (any principal, kept after removal) |
| D6 / D7 | holds: platform-guarded |
| D8 | **known** |
| D9 / D10 | **known** |
| V1 | **known** (scripts only) |
| V2 | TTL filter: `cache_get` is the only reader for both routes (`bridge.rs:283-288`), so query and update apply the same 1 h expiry. holds |
| V3 | **known** (searches enter it, slot-free) |
| V4 | **known** (query route ignores it) |
| V5 | **known** (query re-parse under the query limit) |
| V6-V11 | n/a: no filter installed per path |

### Commit points, limits, work, failures, conversions

| Row | Cell |
|---|---|
| K1-K12 | n/a: commit behaviour, not a filter per path. Policy not re-checked after awaits: **known** |
| LIM1 call cap | per slot, not per message; searches have their own cap: **known** |
| LIM2 budget | query search, Python soft budget, check-before-fetch: **known** |
| LIM3 timeout | not applied to fetches: **known** |
| LIM4 certified_reads | **known** |
| LIM5 | holds (one reader) |
| LIM6 / LIM7 | holds: `tool_result` and `finish` shared by every tool and route; truncation-as-success and `id:null` collapse: **known** |
| LIM8 / LIM10 / LIM11 | holds (shared code, r2-9) |
| LIM9 | **known** |
| LIM12 | **known** |
| LIM13 / LIM14 | **known** (marker overflow, `Trapped`) |
| LIM15 | **known** |
| LIM16 | **known** |
| LIM17 | holds: every runtime reaches `IcBridge::call` |
| LIM18 | holds (V2) for the cache TTL; failure TTL **known** |
| LIM19 | holds: fetched and override text both checked (r2-9) |
| LIM20 / LIM21 | **known** per site |
| LIM22 | **known** (JS numbers above 2^53) |
| LIM23 | see U6 |
| LIM24-LIM27 | per-runtime limits; differences **known** |
| LIM28-LIM30 | n/a: platform |
| LIM31 | **known** |
| LIM32 | holds: same `render` on both routes |
| W1-W16 | n/a for this pattern beyond the rows above: none has a guarded route next to an unguarded one that is not already known (W3 query/update, W15 demo_store) |
| F1-F16 | n/a: aborts, not entry paths |
| B1-B3 | holds: one `authenticate`, one hash |
| B4 | canonicalisation "before the allowlist check, the interface cache and the override lookup" (README:446-447): `parse_principal` in `candid` / `call`, `Principal`-typed admin args, `to_text()` keys throughout (`policy.rs:233-241`, `bridge.rs:207-212, 471-476`). Padded search id: **known** |
| B5-B10 | **known** per conversion |
| B11 | **known** |
| B12-B16 | holds or n/a: no second path |
| B17 | holds: comparison rows render constants (`demo.html:478-480`); script splicing and regex cost **known** |

### Value flows, admin, supply chain, claims, tests

| Row | Cell |
|---|---|
| VF1 / VF3 / VF5 | **known** (seed, unaudited updates, searches) |
| VF2 | **known** (replicated `http_request` paid by C1 is part of the seed) |
| VF4 | holds: assets move only through `IcBridge::call`; self / `aaaaa-aa` bypasses **known** |
| VF6 | **known** |
| VF7 / VF8 | n/a |
| AD1-AD9 | see A5; unvalidated ranges **known** |
| S1-S10 | see C7; reproducibility, unpinned `ic-wasm`, advisories **known** |
| CL1 / CL3 / CL19 / CL20 | **known** |
| CL2 | **known** |
| CL6 | see A6 |
| CL7-CL10 | **known** |
| CL11 | holds for the removals and clock (U15); `JSON.stringify` replacer covered by test T8 |
| CL12 | Python "no OS access": every `OsCall` other than `import icp` gets `no_handler_result` (`python.rs:270-276`), every unknown name `Undefined`. holds |
| CL13 | holds (E19) |
| CL14 / CL15 | **known** |
| CL16 | **known** (seed) |
| CL17 / CL18 | **known** |
| CL21 | **known** (`certified_reads` routes; timeout scope) |
| CL22 | holds for both size checks; TTL failure behaviour **known** |
| CL23 | see B4 |
| CL24-CL27 | **known** or holds (C3) |
| CL28 | **known** (skip-certification on every path) |
| CL29 | **known** (Python random) |
| CL30 | holds (C5) |
| CL31 / CL32 | holds / **known** |
| CL33-CL37 | n/a: no per-path filter |
| T1-T12 | gap, not a finding: no test drives a non-ingress caller or compares routes (**known**: "No test compares the query route with the update route") |

## Candidates

None.

## Not tested, and why

- No inter-canister caller was used. L5-3 already reproduced the cost side through the local
  proxy canister; a token-carrying caller would need the bearer token in call arguments on the
  command line, which this run must not expose.
- Whether same-subnet inter-canister requests to `http_request_update` may exceed the 2 MiB
  ingress bound was not checked against the platform. It would only enlarge work items that
  are already known findings (unbounded search keywords, body parse), for token holders.
