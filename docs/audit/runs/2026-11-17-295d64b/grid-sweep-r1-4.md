# Variant sweep r1-4: guardrail on one path, resource open on another

Run `2026-11-17-295d64b`, Phase 6. Source finding: L1-5. icp_code_mode's allowlist
blocks demo_store `cancel_order`/`restock`, and README.md:172-173 and
`scripts/demo-setup.sh:3` present them as blocked, but demo_store takes direct ingress
from anyone.

Pattern, stated generally: **a guardrail is shown working on one path, while the resource
it protects can be reached by another path with no guardrail** (direct ingress, a
canister-to-canister call, the query path instead of the update path, the admin path
instead of the init path, another engine entry point, another identity).

For each row the question is: what does this row protect, and is there a second path to
that resource that skips the guardrail? Rows where the second path is a finding that is
already known are marked **known** and not reported again.

Evidence key: **R** = reproduced here; **R-prior** = reproduced earlier in this run (cited
grid); **CP** = the code path makes it unambiguous.

## Result

No new candidate. Each second path either does not exist, re-checks the guardrail, or is
already a known finding.

## Cells

### Components (C)

| Row | Cell |
|---|---|
| C1 icp_code_mode | holds: every way in to the protected resources (outbound calls made with C1's principal, policy, audit, cycles) goes through `mcp::authenticate` (E4/E5) or the controller guard (E6-E14). Second paths are known: canister callers skip `inspect_message`; a self-call or `aaaaa-aa` on the allowlist; `icp.candid` override reads; searches on the query path |
| C1a feature subsets / per-language workers | holds: a worker is a separate canister with its own principal, tokens and allowlist. With `init_args "(null)"` (icp.yaml:20) it starts deny-by-default with no tokens. There is no router that forwards to workers (`LanguageRouter` dispatches inside one canister, `src/runtimes/mod.rs:46-60`), so no front canister applies guardrails that a worker reached directly would skip |
| C2 demo_store | **known** (L1-5, L5-5/6): writes are open to anyone. The other demo_store guardrails, the stock check and `EmptyOrder`/`UnknownProduct`/`UnknownCustomer` (`demo_store/src/lib.rs:278-310`), sit inside the method, so ingress and MCP get the same checks. Oversell and zero-quantity lines are known |
| C3 web-time shim, C4 vendored chrono | n/a: these are libraries with no entry points |
| C5 demo page | holds: the only network calls are `fetch("/mcp")` (`src/demo.html:219, 367`), and its "Blocked by policy" example (`src/demo.html:195-199`) tries `icrc1_transfer` on the ledger. The resource is C1's own ledger balance, which only C1's principal can move, so direct ingress cannot reach it. The comparison runner's "classic" calls also go through `execute_icp_script` (`src/demo.html:385-389`) |
| C6 demo-setup.sh | **known** (part of L1-5: "stay blocked" comment, line 3) |
| C7 icp.yaml local network | holds (R): the gateway listens only on `127.0.0.1:8010` (`lsof`, pocket-ic pid listening on 127.0.0.1:8010). The dev network's open demo_store is not reachable from other hosts |
| C8-C10 build flags/toolchain | n/a: no path to a protected resource |
| C11 `.did` files | holds: `candid:service` is public on purpose (icp.yaml:16, 28). It names the admin methods, but those are guarded on every path (E6-E14 below) |
| C12 video | the token is kept out of git (`.gitignore`) and out of images: the demo page's token field is `type="password"` (`src/demo.html:76`), so screenshots (`video/assets/*.png`) do not show it. Profile and DevTools leftovers are **known** |
| C13 audit tooling, C14 idea doc | n/a: C14's free-trial "read-only" instance (line 75) is an idea, not built. Line 38 already says direct top-ups bypass the factory |
| C15 non-method exports | holds: no `canister_` prefix, so they cannot be called as messages |

### Actors (A)

| Row | Cell |
|---|---|
| A1 anonymous web | holds: only `GET /` and `/demo` are served without a token. `http_update` serves no demo page (an unauthenticated GET gets 405 from `authenticate`, `src/mcp.rs:271-275, 363-366`), so the `certified_reads` switch-off of the page (`src/mcp.rs:337-341`) has no second path through the update route |
| A2 anonymous direct ingress | holds (R-prior, grid-L1-r1 A2): direct calls to `http_request_update`, `http_request`-as-update and every admin method are refused |
| A3 token holder | holds: `/mcp` runs the same `authenticate` on the query and update paths. The second paths (query-path search cap, unaudited no-tool updates, revocation in flight) are **known** |
| A4 AI model | n/a: it only reads content |
| A5 controller | holds: admin methods carry both the guard and `inspect_message`. `InitArgs` (E1/E2) apply the same validation as the admin methods: `apply` calls `policy::add_token_hash`, `set_allowed` and `set_limits` (`src/lib.rs:37-48`), and InitArgs has no override field, so the 512 KiB and parse checks of `set_candid_override` have no unchecked path. The re-application on upgrade is **known** |
| A6 allowlisted target | holds for resources tied to C1's identity. For resources not tied to it (anything the target lets any caller do), the allowlist is not a guardrail. The only such target in the repo is demo_store: **known** L1-5 |
| A7 management canister | **known** (`aaaaa-aa` on the allowlist) |
| A8 other canisters | **known** (unpaid-call filter covers ingress only). They can also call the query `http_request` as an inter-canister call, which C1 pays for (same known class). State changes there are discarded because it is a query method, and `/mcp` still needs a token |
| A9 infrastructure | **known** (#48, the certified_reads skip-certification finding). Admin reads: `icp canister call` sends an update unless `--query` is given (R: `icp canister call --help`), so the README's audit-log command (README.md:325) is certified by default |
| A10 anyone on demo_store | **known** L1-5 |
| A11 supply chain, A12 factory | n/a |

### Entry points (E)

| Row | Cell |
|---|---|
| E1/E2 init/post_upgrade | holds: same validators as the admin methods (see A5); post_upgrade undoing revocations is **known** |
| E3 inspect_message | **known** (ingress only, by design) |
| E4/E4a/E4b http_request | holds: token checked before the body is parsed. Demo page: see A1. Query-path search caps are **known** |
| E5/E5a/E5b http_request_update | holds: same `authenticate`; per-request caps apply to all callers of the function. Searches taking no run slot is **known** |
| E5c/E5d callbacks | n/a: system only |
| E6-E11 admin updates | holds: guard plus `inspect_message`, no second writer except InitArgs (A5) |
| E12-E14 admin queries | holds: guard on query, and on update via `inspect_message` plus guard (R-prior). Integrity: see A9 |
| E15 | holds (C15) |
| E16 JS SDK | holds: `eval`/`new Function` are blocked through Boa's `HostHooks::ensure_can_compile_strings` (`src/runtimes/js.rs:162-167`). That hook blocks every dynamic-compile path, including `(()=>{}).constructor`, generator and async function constructors, not only the global name. Boa 0.22 has no `ShadowRealm` (R: no match in `boa_engine-0.22.0/src`), so a script cannot get a fresh realm with `Math.random`/`Atomics`/`WeakRef` restored |
| E17 console/print | holds: both go through the snapshot (value limits) |
| E18 Python SDK | `import os` refused (test). `eval`/`exec` are **known** |
| E19 Rhai SDK | holds (R): with `disable_symbol("eval")` and `set_max_modules(0)` on rhai 1.27.0 (same settings as `src/runtimes/rhai_rt.rs:119-132`), `eval("1")`, `Fn("eval")`, `call(Fn("eval"), ..)` and `Fn("eval").call(..)` are all refused at parse time ("'eval' is a reserved keyword"), and `import` gets "Too many modules imported". The guarded terminate marker cannot be wrapped through a function pointer |
| E20-E25 demo_store | **known** L1-5 (E21-E23). Reads are public demo data, and C1 allowlists all 7 reads anyway, so MCP holds back nothing that ingress exposes |
| E26-E28 | see C5, C6, C12 |

### Inputs, effects, state

| Row | Cell |
|---|---|
| U1-U5 | holds: one parser and classifier for both paths (`parse_body`, `classify`). `MAX_BATCH` is in `parse_body`, and `MAX_REPLY_BYTES` is in `finish`, which both paths call |
| U6/U7 | holds: `MAX_CODE_BYTES` is checked in `LanguageRouter::execute`, the only way into the engines |
| U8-U10 | n/a: replies from targets, not a guarded resource |
| U11 | see A5 |
| U12 | **known** |
| U13 | **known** L1-5 / L5 |
| U14-U16 | see C5, C12 |
| O1 script calls | holds: `reserve` runs on every call path (JS, Python, Rhai all go through `IcBridge::call`). Second paths are **known**: in-flight revocation, self-call, `aaaaa-aa` |
| O2 interface fetch | **known** (fallback not re-checked; overrides served without an allowlist check) |
| O3 audit writes | **known**: unaudited paths (direct demo_store writes, authenticated requests that run no tool, query-path searches) |
| O4-O8 | holds: guarded writes, constant certified data |
| O9 demo_store writes | **known** L1-5 |
| O10 video | **known** |
| D1-D3, D5 | holds: written only by guarded admin methods or init/upgrade with the same validators |
| D4 audit | holds for reads (controller only, update by default). Wiping by reinstall is **known** |
| D6-D7 | holds: platform controllers only. Log visibility is not set in icp.yaml, so it defaults to controllers |
| D8/V11 | **known** |
| D9/D10 | see C12. `.env` is gitignored |
| V1-V10 | n/a for this pattern: heap state with no second way in from outside. Query-method state changes are discarded, so an inter-canister `http_request` cannot poison the caches |

### Limits, work, failures, conversions, flows, admin, supply chain, claims, tests

| Row | Cell |
|---|---|
| LIM1, LIM2, LIM12 | **known**: searches, query path, unpaid canister calls |
| LIM3, LIM4 | **known**: timeout not applied to fetches; certified_reads skip-certification |
| LIM5-LIM8, LIM10, LIM11 | holds: shared code for both routes (see U1-U5) |
| LIM9 | **known** (query path) |
| LIM13-LIM32 | n/a or **known**: per-run limits inside the one execution path. The differences between runtimes are L6 findings that are already known |
| W1-W16 | n/a for this pattern: none of them has a guarded route next to an unguarded one, beyond the known W3 query/update search split and W15 demo_store |
| F1-F16 | n/a: aborts, not access paths |
| B1-B17 | holds: B3 hashes and B4 canonicalises on every path (grid-L1-r1 B3/B4) |
| VF1, VF3, VF5 | **known** (unpaid canister calls, unaudited updates, searches) |
| VF4 | holds: assets held by C1's principal can only move through C1, and C1 checks the allowlist. Bypasses through self or `aaaaa-aa` are **known** |
| VF6 | **known** |
| VF2, VF7, VF8 | n/a |
| AD1-AD9 | see A5. AD3 remove_allowlist leaving overrides is **known** |
| S1-S10 | n/a for this pattern |
| CL3, CL6, CL16, CL19 | **known** |
| CL29 (sandbox claims) | JS and Rhai dynamic-code bans hold on every path (E16, E19). Python `random` and eval are **known** |
| CL30 | holds (C5, C12) |
| Other claims | n/a for this pattern |
| T1-T12 | n/a. The Rhai `Fn("eval")` path and the JS constructor path have no test. The probes above are the evidence. Neither is a finding |

## Not tested, and why

- Calls to `http_request` / `http_request_update` from another canister: no caller
  canister is deployed, and creating one changes the shared network. The cells rely on the
  code path (A8).
- JS `(()=>{}).constructor("...")` was not run. The cell relies on Boa routing every
  dynamic compile through `ensure_can_compile_strings`, which the existing test
  `js::tests` covers for `eval` and `new Function` only.
