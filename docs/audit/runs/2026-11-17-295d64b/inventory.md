# Inventory (Phase 1)

Run `2026-11-17-295d64b`. Derived from the committed tree at `295d64b` with `git ls-files`,
`grep`, `cargo tree`, `wasm-objdump` and reading every file under `src/`, `demo_store/`,
`shims/`, `scripts/`, `icp.yaml`, `.cargo/`, the `.did` files, `README.md` and
`docs/per-customer-canisters.md`. `vendor/chrono` was inventoried from its manifest header
only; `video/` from its scripts' imports and network calls. Excluded (someone else's
uncommitted work): `docs/per-customer-canisters-plan.md`, `design/`.

Row ids (C, A, E, U, O, D, V, K, LIM, W, F, B, VF, AD, S, CL, T) are the rows of the
coverage grid. `file:line` refers to commit `295d64b`.

## Diff against the previous run

There is no previous run directory: this is the first full run of the plan, so every row is
new. For orientation only: no application code changed since `cdfa3d9`, the commit that
closed register round R6 (`git diff cdfa3d9 HEAD` lists only the audit plan, register and
workflow). The code that changed most over rounds R4-R6 (`git diff --numstat d508ece HEAD`):
`src/bridge.rs` +232/-43, `src/runtimes/js.rs` +140/-19, `src/runtimes/python.rs` +107/-18,
`src/lib.rs` +64/-5, `src/runtimes/rhai_rt.rs` +19/-6, `src/policy.rs` +13/-7,
`src/mcp.rs` +12/-0, `icp_code_mode.did` +15/-5. These are the youngest rows and are reviewed
first.

---

## 1. Components

| Id | Component | Kind | Source of truth |
|---|---|---|---|
| C1 | `icp_code_mode` canister: MCP server, policy, audit, three script engines | deployable Wasm (`cdylib`) + `rlib` for tests | `Cargo.toml` `[lib]`, `icp.yaml` canister 1 |
| C1a | Feature subsets of C1: `js`, `python`, `rhai`, any combination, none | build variants ("per-language workers") | `Cargo.toml [features]`, README.md:112-120, `icp.yaml` comment |
| C2 | `demo_store` canister: demo target with 7 queries and 3 updates, heap-only seed data | deployable Wasm | `demo_store/Cargo.toml`, `icp.yaml` canister 2 |
| C3 | `shims/web-time`: replacement crate, `Instant` = call-context instruction counter on wasm32 | `[patch.crates-io]` | `Cargo.toml:41`, `shims/web-time/` |
| C4 | `vendor/chrono` 0.4.45 with `wasmbind` removed from default features and `[[test]]`s removed | vendored `[patch.crates-io]` | `Cargo.toml:42`, `vendor/chrono/Cargo.toml` header |
| C5 | Demo page `src/demo.html`, compiled in with `include_str!`, served at `GET /` and `GET /demo` | page | `src/mcp.rs:321-330` |
| C6 | `scripts/demo-setup.sh`: allowlists demo_store's 7 reads + `create_order` | operator script | file |
| C7 | `icp.yaml` build steps: `cargo build --release --locked`, `cp`, `ic-wasm metadata candid:service -v public`, `ic-wasm shrink`; local network on gateway port 8010; `init_args: "(null)"` | build/deploy config | file |
| C8 | `.cargo/config.toml`: wasm32 rustflags `getrandom_backend="custom"`, `-zstack-size=8388608` | build flags | file |
| C9 | `rust-toolchain.toml`: channel 1.100.0, wasm32 target, rustfmt, clippy | toolchain pin | file |
| C10 | `[profile.release]`: `opt-level="z"`, `lto`, `strip`, `codegen-units=1`, `panic="abort"` | build flags | `Cargo.toml:44-49` |
| C11 | Interface files `icp_code_mode.did`, `demo_store/demo_store.did` (tests assert they match `export_candid!`) | interface | files, `src/lib.rs:463`, `demo_store/src/lib.rs:353` |
| C12 | `video/` pipeline: `build.sh`, `tts.mjs` (Gemini API, key from env/.env/~/.env), `render.mjs` (ffmpeg), `cdp.mjs` (spawns Chrome, DevTools on 127.0.0.1), `stills.mjs`, `capture-demo.mjs` (reads `MCP_TOKEN` from `.env`, writes it into the demo page's localStorage), `composition.html`; outputs MP4/poster | dev tooling, not deployed | files |
| C13 | `.claude/workflows/audit.js` + `docs/audit/` | audit tooling | files |
| C14 | `docs/per-customer-canisters.md`: idea for a factory canister (not built) | doc | file |
| C15 | Exported non-method Wasm functions `get_candid_pointer` (ic-cdk `export_candid!`) and `__getrandom_v03_custom` (`src/lib.rs:433-449`) | Wasm exports | `wasm-objdump -j Export` |

Searched for and not found: CI pipelines, Dockerfiles, other workspace members, build
scripts (`build.rs`), timers, heartbeat, `pre_upgrade`.

## 2. Actors

| Id | Actor | How they interact | Checks that apply |
|---|---|---|---|
| A1 | Anonymous web user | `GET /`, `GET /demo` (demo page, no auth); any other path/method gets 404/405 JSON; `POST /mcp` without a valid token gets 401 | none for the page; `mcp::authenticate` |
| A2 | Anonymous direct ingress caller | Calls canister methods without the gateway: `http_request` (query, never inspected), `http_request_update` (refused by `inspect_message` unless the decoded request carries a registered token), admin methods (refused by `inspect_message` unless controller; also `guard = "controller"`) | `src/lib.rs:273-283`, guards |
| A3 | Bearer-token holder (MCP client, AI agent, demo page user) | `POST /mcp`: initialize, ping, tools/list, search, execute scripts. All holders act as one on-chain principal (register #49) | `policy::verify_token` (SHA-256 lookup) |
| A4 | The AI model behind A3 | Reads `tools/list` descriptions, `initialize.instructions`, search results and script results, then writes scripts | none (content consumer) |
| A5 | Controller (operator) | Admin updates/queries, `init`/`post_upgrade` args, canister settings, upgrades/reinstall | `ic_cdk::api::is_controller` guard |
| A6 | Allowlisted target canister | Receives calls from C1; returns replies, rejects, interface text; may be slow, never reply, trap, lie, return huge data | allowlist + call cap before send; decoding quotas after |
| A7 | Management canister `aaaaa-aa` | Target of `canister_metadata` interface fetches; a script target only if a controller allowlists it | `check_interface_fetch` / `check_call` |
| A8 | Other canisters (non-ingress callers) | Can call `http_request_update` directly (no `inspect_message`; must still carry a token), `http_request`, and demo_store's updates | `mcp::authenticate` |
| A9 | Infrastructure: HTTP gateway / boundary nodes, subnet node operators | See request contents (tokens, register #48); a single node can alter uncertified query responses (register #47) | `certified_reads` option |
| A10 | Anyone, on demo_store | `create_order`, `cancel_order`, `restock` and all queries; demo_store has no access control and no `inspect_message` | none |
| A11 | Build/supply chain: crates.io, toolchain, icp-cli, ic-wasm; video tooling: Gemini API, Chrome, ffmpeg | Provide code and binaries | lockfile (`--locked`), toolchain pin |
| A12 | Future customer / factory (doc only, C14) | Not implemented | n/a |

## 3. Entry points

C1 (`icp_code_mode`), from `#[ic_cdk::...]` attributes in `src/lib.rs` and the exported
names in the Wasm:

| Id | Entry point | Kind | Who may call | Changes state |
|---|---|---|---|---|
| E1 | `init(opt InitArgs)` `src/lib.rs:62` | lifecycle | installer | yes: certified data, tokens, allowlist, limits; traps on a malformed token hash |
| E2 | `post_upgrade(opt InitArgs)` `src/lib.rs:68` | lifecycle | controller (upgrade) | yes, same as E1; heap caches, `PENDING_RUNS`, `ACTIVE_RUNS` reset |
| E3 | `inspect_message` `src/lib.rs:273` | ingress filter | replica, every ingress update | no; decodes `HttpUpdateRequest` and hashes the token |
| E4 | `http_request` (query) `src/lib.rs:76` | HTTP gateway | anyone | no (query); may answer from heap caches; sets `IC-Certificate` headers |
| E4a | `GET /`, `GET /demo` (and any query string/fragment) | route in E4 | anyone | no; 404 when `certified_reads` |
| E4b | `POST /mcp` JSON-RPC: `initialize`, `ping`, `tools/list`, notifications (no id), client responses (ignored), `tools/call` `search_icp_canisters` answered from cache; anything needing work returns `upgrade: true` | route in E4 | token holder | no |
| E5 | `http_request_update` `src/lib.rs:98` | HTTP gateway (upgraded) or direct call | token holder (ingress via E3; canisters directly) | yes: audit log, interface caches, `PENDING_RUNS`, `ACTIVE_RUNS`, outbound calls |
| E5a | `tools/call execute_icp_script` -> `run_script` `src/lib.rs:218` | tool | token holder; at most 1 per HTTP request, 8 concurrently | yes |
| E5b | `tools/call search_icp_canisters` -> `run_search` `src/lib.rs:121` | tool | token holder; at most 5 per HTTP request; not counted in the 8-run cap | yes (fetches, audit) |
| E5c | Reply/reject callbacks of every outbound call (continuations of E5a/E5b) | callback | system | yes |
| E5d | Cleanup callback after a trap in a continuation: drops `IcBridge` (writes `Trapped` entry, `src/bridge.rs:243-267`) and `RunSlot` (`src/lib.rs:212`) | callback | system | yes |
| E6 | `set_allowlist(principal, vec text)` | admin update | controller | yes |
| E7 | `remove_allowlist(principal) -> bool` | admin update | controller | yes (also evicts heap caches) |
| E8 | `set_limits(Limits)` | admin update | controller | yes |
| E9 | `add_api_token_hash(text, text) -> Result` | admin update | controller | yes |
| E10 | `revoke_api_token_hash(text) -> bool` | admin update | controller | yes |
| E11 | `set_candid_override(principal, opt text) -> Result` | admin update | controller | yes |
| E12 | `get_policy() -> PolicyView` | admin query | controller | no |
| E13 | `get_audit_logs(nat64, nat64)` | admin query | controller | no |
| E14 | `get_audit_log_len()` | admin query | controller | no |
| E15 | Non-method exports `get_candid_pointer`, `__getrandom_v03_custom` | Wasm exports | not callable as messages (no `canister_` prefix) | no |

Script SDK (host functions reachable from untrusted script code inside E5a):

| Id | Host function | Runtime | Notes |
|---|---|---|---|
| E16 | `icp.call(id, method, args)`, `icp.candid(id)` (pending Promises), `icp.principal()` | JS `src/runtimes/js.rs:250-298` | queued, dispatched per round with `join_all` |
| E17 | `console.log/info/warn/error/debug`, `print` | JS `src/runtimes/js.rs:227-248, 300-306` | |
| E18 | `icp.call`, `icp.call_async`, `icp.candid`, `icp.principal`, `import icp`, `print` | Python `src/runtimes/python.rs:45-57, 124-140, 266-273` | name lookups answered `Undefined`; other OS calls `no_handler_result` |
| E19 | `icp::call_canister` (2 and 3 args), `icp::candid`, `icp::principal`, `print`, `debug` | Rhai `src/runtimes/rhai_rt.rs:135-180` | replay model |

C2 (`demo_store`, `demo_store/src/lib.rs`):

| Id | Entry point | Kind | Who | Changes state |
|---|---|---|---|---|
| E20 | `list_products`, `get_product`, `search_products`, `get_inventory`, `get_customer`, `list_orders`, `sales_summary` | query | anyone | no |
| E21 | `create_order(principal, vec OrderItem)` | update | anyone | yes (heap): reserves stock, appends an order |
| E22 | `cancel_order(nat64)` | update | anyone | yes |
| E23 | `restock(nat64, nat32)` | update | anyone | yes |
| E24 | `__get_candid_interface_tmp_hack` (hidden query) | query | anyone | no |
| E25 | No init/post_upgrade: heap state re-seeded on every install/upgrade | lifecycle | | |

Other surfaces: E26 demo page buttons and the comparison runner (`src/demo.html`, they only
call `POST /mcp`); E27 `scripts/demo-setup.sh`; E28 video scripts (local CLI).

Missing-kind check: searched for timers, heartbeat, `pre_upgrade`, `#[update(manual_reply)]`,
HTTP outcalls, `composite_query`: none.

## 4. Untrusted inputs

| Id | Input | Origin | First handled at | Size bound before use |
|---|---|---|---|---|
| U1 | HTTP `method`, `url` | E4/E5 caller | `mcp::authenticate`, `demo_page` | ingress message limit (~2 MiB) only |
| U2 | HTTP headers (`Authorization: Bearer ...`) | E4/E5 | `mcp::authenticate` `src/mcp.rs:276-288` | none (whole header list scanned) |
| U3 | HTTP body (JSON-RPC message or batch) | E4/E5 | `parse_body` `src/mcp.rs:293` | serde_json (recursion limit 128), batch <= 50 after parsing |
| U4 | JSON-RPC `id`, `method`, `params` | U3 | `classify` `src/mcp.rs:236` | echoed text truncated to 300 B in errors; reply <= 1.9 MB |
| U5 | `params.name`, `arguments.code`, `arguments.language`, `arguments.query`, `arguments.canister_id` | U4 | `parse_tool_call` `src/mcp.rs:210` | code <= 100 KiB (checked in `LanguageRouter::execute`, after the body is parsed); query and canister_id unbounded |
| U6 | Script source code (a program) | U5 | engine compile/eval | 100 KiB |
| U7 | Values produced by scripts: canister ids, method names, call args, results, printed values, thrown values | U6 | snapshot/`to_json*` per engine, `bridge::parse_principal`, `MAX_METHOD_LEN` | 128 levels, 200k nodes, 4 MiB text (`bridge.rs:47-48`); method <= 256 B; principal parse |
| U8 | Candid reply bytes from targets | A6 | `Service::decode_reply` `src/candid_bridge.rs:96` | IC reply limit 2 MiB; decoding and skipping quota 20M |
| U9 | Reject codes/messages and call errors from targets/system | A6 | `bridge.rs:528-531`, `fetch_did` | none of our own (embedded in error string; client text <= 1 MB; audit <= 4 KiB) |
| U10 | Interface text from targets (`canister_metadata` value, `__get_candid_interface_tmp_hack` string) | A6/A7 | `IcBridge::candid` `src/bridge.rs:487-494` | 512 KiB checked after the reply is received |
| U11 | Admin inputs: InitArgs, allowlist method lists, Limits, token hash + label, candid override text | A5 | E1/E2, E6-E11 | override <= 512 KiB; token hash 64 hex; labels, method lists and counts unbounded |
| U12 | `inspect_message` argument bytes | A2 | `candid::decode_one::<HttpUpdateRequest>` `src/lib.rs:276` | ingress limit |
| U13 | demo_store arguments (`items` vector, ids, quantities, text) | A10 | E20-E23 | ingress limit |
| U14 | Demo page: token typed by the user; JSON responses rendered into the DOM | user / C1 | `src/demo.html` | n/a |
| U15 | Platform values: `time()`, instruction counters, `data_certificate()`, `msg_caller()` | replica | various | trusted platform (consensus) |
| U16 | Video tooling: Gemini API responses, `.env` contents | external | `video/tts.mjs`, `video/capture-demo.mjs` | n/a (dev only) |

## 5. Outbound effects

| Id | Effect | Where | Pre-check |
|---|---|---|---|
| O1 | Inter-canister call from a script, `Call::unbounded_wait` (default) or `Call::bounded_wait(...).change_timeout(secs)`, raw Candid args, no cycles attached | `src/bridge.rs:510-533` | `reserve` -> `policy::check_call` (allowlist, method, call cap, instruction budget) before interface fetch and encoding |
| O2 | Interface fetch: `Call::bounded_wait(aaaaa-aa, "canister_metadata")`, then fallback `Call::bounded_wait(target, "__get_candid_interface_tmp_hack")`: up to 2 messages per fetch, one cap slot | `src/bridge.rs:424-437` | `reserve(p, None)` -> `check_interface_fetch` (target allowlisted) |
| O3 | Stable memory writes: audit entries (per message `Started`/`Call`, per run closing entry, `Trapped` in cleanup) | `policy::append_audit` `src/policy.rs:247` | token holder only (via E5) |
| O4 | Stable memory writes by admin: limits, allowlist, tokens, overrides | `src/policy.rs` | controller guard |
| O5 | `certified_data_set(SKIP_CERT_DATA)` | E1/E2 | n/a |
| O6 | HTTP responses (JSON-RPC replies, demo HTML, IC-Certificate headers) | E4/E5 | reply size cap 1.9 MB |
| O7 | `accept_message` | E3 | token/controller check |
| O8 | `trap` (aborts message): init token hash, `expect`s, IC limits | see Failure sites | |
| O9 | demo_store heap writes | E21-E23 | none |
| O10 | Video tooling: HTTPS POST to Gemini API with API key; local Chrome DevTools; writes token into browser localStorage | `video/*.mjs` | dev only |

Searched for and not found: cycles attached to calls (`with_cycles`), HTTP outcalls,
`ic0.call_cycles_add128` use in app code (imported by ic-cdk), timers, logging via
`debug_print` in app code (imported by ic-cdk for panics).

## 6. Durable state

C1 state lives in `ic-stable-structures` 0.7 via one `MemoryManager` (`src/policy.rs:118-133`);
all values are Candid-encoded `Storable`s with `Bound::Unbounded` and `expect` on encode/decode.

| Id | State | Memory id | Format | Writers | Survives upgrade |
|---|---|---|---|---|---|
| D1 | `LIMITS: StableCell<Limits>` | 0 | Candid record (4 fields, 2 optional added later) | controller (E8, E1/E2) | yes |
| D2 | `ALLOWLIST: StableBTreeMap<Principal, AllowedMethods(Vec<String>)>` | 1 | Candid | controller (E6, E7, E1/E2) | yes |
| D3 | `TOKENS: StableBTreeMap<String /*sha256 hex lowercase*/, String /*label*/>` | 2 | String | controller (E9, E10, E1/E2) | yes |
| D4 | `AUDIT: StableLog<AuditEntry>` (index + data) | 3, 4 | Candid record; `AuditStatus` variant gained `Started`, `Call`, `Trapped`; `run_id`, `methods` optional | any token holder through E5 (and cleanup E5d) | yes; never pruned |
| D5 | `CANDID_OVERRIDES: StableBTreeMap<String /*principal text*/, String /*.did*/>` | 5 | String | controller (E11) | yes; not removed by `remove_allowlist` |
| D6 | Certified data (32-byte constant) | system | bytes | E1/E2 | reset on upgrade, re-set in E2 |
| D7 | Canister settings (controllers, freezing threshold, Wasm memory limit 3 GiB, log visibility) | system | | controller | yes |
| D8 | demo_store `STORE` | heap | Rust structs | anyone (E21-E23) | **no**: re-seeded on upgrade (by design, demo) |
| D9 | Demo page token in browser `localStorage` key `icp-code-mode-token` | browser | text | page user | browser-local |
| D10 | `.env` with `MCP_TOKEN` (and possibly `GEMINI_API_KEY`) in the working tree, gitignored | disk | text | developer | n/a |

## 7. Volatile shared state (outlives one request, lost on upgrade)

| Id | State | Where | Written by | Read by |
|---|---|---|---|---|
| V1 | `ACTIVE_RUNS: Cell<u32>` run-slot counter | `src/lib.rs:200` | `RunSlot::take`/`drop` (scripts only) | E5a, E12 |
| V2 | `DID_CACHE: BTreeMap<String, (did, Rc<Service>, fetched_at)>`, TTL 1 h | `src/bridge.rs:195` | `IcBridge::candid` after a fetch | E4 (search), E5 |
| V3 | `PENDING_RUNS: BTreeSet<u64>` | `src/bridge.rs:198` | `record_message` insert, `IcBridge::drop` remove | E12 |
| V4 | `DID_FAILED: BTreeMap<String, (error, at)>`, TTL 5 min | `src/bridge.rs:200` | `IcBridge::candid` | E5 |
| V5 | `OVERRIDES: BTreeMap<String, (did, Rc<Service>)>` parsed-override cache | `src/bridge.rs:202` | `override_service` (also from query E4, discarded there) | E4, E5 |
| V6 | `monty_types::LIVE_MEMORY` / `BASELINE_MEMORY` process-global atomics, fed by the counting allocator (python feature only) | `src/lib.rs:357-425`, `src/runtimes/python.rs:151-157, 229` | every allocation; `rebased` around awaits | Monty memory limit |
| V7 | getrandom custom backend `STATE: Cell<u64>` (splitmix64 stream, shared by all runs) | `src/lib.rs:433-449` | ahash / boa `rand` seeding | hash seeds in all engines |
| V8 | Boa GC heap (thread-local, `boa_gc::force_collect` after each run) | `src/runtimes/js.rs:361` | JS runs | JS runs |
| V9 | ic-cdk executor state: suspended futures of in-flight runs (each keeps its VM heap) | ic-cdk | E5 | callbacks |
| V10 | Engine-internal statics (Boa intrinsics/realms, Rhai, Monty interned data), if any | dependencies | | not inspected |
| V11 | demo_store `STORE` (also D8) | demo_store | | |

Per-run state that is *not* shared (by design): `IcBridge` counters/messages/logs, JS
`State`, Rhai `RunState` (`Arc<Mutex<_>>` per run), Python `out` buffer and `queued`.

## 8. Commit points

ICP commits state at the end of each message execution; every `await` on an inter-canister
call ends one execution and starts another.

| Id | Commit point | What is committed there | What a trap after it loses |
|---|---|---|---|
| K1 | First poll of an outbound call future returns Pending (`on_first_pending` in `IcBridge::sent`, `src/bridge.rs:387-419`) | the `Started`/`Call` audit entry (O3) together with the message (O1/O2); `PENDING_RUNS` insert | nothing earlier |
| K2 | Each `await` in JS `drive` (`join_all` per round, `src/runtimes/js.rs:391-400`) | all K1 entries of that round; JS heap stays in memory | |
| K3 | Each `await` in Python (`rebased(host(...))`, `rebased(join_all)`, `src/runtimes/python.rs:252, 262`) | same | |
| K4 | Each `await` in the Rhai replay loop (`src/runtimes/rhai_rt.rs:214-218`) | same; `answers` cache stays on heap | |
| K5 | Each `await` in `run_search` (`src/lib.rs:137`) and the two sequential awaits inside `fetch_did` | interface-fetch entries | |
| K6 | End of `http_request_update`: final audit entry (`Ok`/`Error`/`PolicyViolation`), `bridge.finish()`, reply | closing entry, DID caches | trap here (e.g. reply too large, out of stable memory) rolls back the closing entry; cleanup K7 writes `Trapped` |
| K7 | Cleanup callback after a trap in a continuation (`IcBridge::drop` with `is_recovering_from_trap()`) | `Trapped` entry, `PENDING_RUNS` removal, `RunSlot` return | if cleanup itself traps: no closing entry ("unknown") |
| K8 | Trap before the first send | nothing: the whole request is rolled back, no entry (register #11, accepted) | |
| K9 | Policy refusal before any send (`reserve` error) | no message; violation is recorded only in the closing entry (K6) | if K6 traps and nothing was sent, no entry at all |
| K10 | Admin updates E6-E11, E1/E2 | single execution each | |
| K11 | Upgrade (E2) while runs are suspended | stable state; heap (`PENDING_RUNS`, caches, VM heaps) lost; in-flight calls' callbacks dropped | runs with only `Started`/`Call` entries and no closing entry |
| K12 | demo_store updates | single execution each | |

## 9. Limits

| Id | Limit | Value | Where | Protects |
|---|---|---|---|---|
| LIM1 | `max_calls_per_script` (default) | 25, settable | `src/policy.rs:18`, D1 | outbound messages per run (O1, O2) |
| LIM2 | `max_instructions` (default) | 20e9, settable, any u64 | `src/policy.rs:20`, D1 | per-run instructions; checked in `reserve`, after run, JS rounds/jobs/console, Rhai `on_progress`, Python VM budget |
| LIM3 | `call_timeout_seconds` | None (unbounded wait), settable any u32 | D1, `src/bridge.rs:524-527` | stopping/upgrading when a target never replies |
| LIM4 | `certified_reads` | None/false | D1, `src/lib.rs:79` | query response integrity |
| LIM5 | `MAX_AUDIT_PAGE` | 100 | `src/policy.rs:22` | admin query size |
| LIM6 | `MAX_RESULT_BYTES` | 1,000,000 escaped bytes (logs: 250,000) | `src/mcp.rs:20` | reply size |
| LIM7 | `MAX_REPLY_BYTES` | 1,900,000 | `src/mcp.rs:23` | IC 2 MiB reply limit (trap) |
| LIM8 | `MAX_BATCH` | 50 messages per JSON-RPC batch | `src/mcp.rs:26` | reply growth |
| LIM9 | `MAX_SEARCHES` | 5 per HTTP request | `src/mcp.rs:29` | fan-out of fetches |
| LIM10 | one `execute_icp_script` per HTTP request | 1 | `src/mcp.rs:392` | per-request budgets |
| LIM11 | rpc error message | 300 B | `src/mcp.rs:147` | echoing input |
| LIM12 | `MAX_CONCURRENT_RUNS` | 8 scripts (searches not counted) | `src/lib.rs:198` | heap held by suspended VMs |
| LIM13 | `MAX_AUDIT_ERROR_BYTES` | 4096 | `src/lib.rs:193` | audit entry size |
| LIM14 | `MAX_VIOLATIONS` / `MAX_VIOLATION_BYTES` | 16 / 512 | `src/bridge.rs:188-189` | audit entry size |
| LIM15 | `MAX_LOG_BYTES` | 64 KiB per run (JS/Rhai truncate; Python raises when the cap is hit, `monty-types io.rs:133`) | `src/bridge.rs:11` | log memory |
| LIM16 | `MAX_VALUE_NODES` / `MAX_VALUE_BYTES` / depth | 200,000 / 4 MiB / 128 | `src/bridge.rs:47-48`, `candid_bridge::MAX_DEPTH` | values crossing script <-> host |
| LIM17 | `MAX_METHOD_LEN` | 256 B | `src/bridge.rs:178` | policy message size |
| LIM18 | `DID_CACHE_TTL_NS` / `DID_FAIL_TTL_NS` | 1 h / 5 min | `src/bridge.rs:180-183` | staleness / repeated fetches |
| LIM19 | `MAX_DID_BYTES` | 512 KiB (fetched and overrides) | `src/bridge.rs:186` | parse cost |
| LIM20 | `DECODING_QUOTA` (decoding and skipping) | 20,000,000 | `src/candid_bridge.rs:16` | hostile replies |
| LIM21 | `MAX_DEPTH` | 128 | `src/candid_bridge.rs:19` | recursion both directions |
| LIM22 | `MAX_SAFE_INT` | 2^53-1 | `src/candid_bridge.rs:14` | JS number precision |
| LIM23 | `MAX_CODE_BYTES` | 100 KiB | `src/runtimes/mod.rs:15` | parse cost |
| LIM24 | JS: loop iterations 1M, recursion 256, stack size 10,240, backtrace 8 | | `src/runtimes/js.rs:22-24, 283-287` | runaway JS |
| LIM25 | JS: `MAX_ROUNDS` 64, `MAX_PENDING` 64, `MAX_PROMISE_JOBS` 100,000 (budget check every 256 jobs) | | `src/runtimes/js.rs:26-37, 108-118` | host round-trips, microtask spin |
| LIM26 | Python: recursion 200, suspensions 256, memory 64 MiB, VM budget = remaining policy budget on wasm (2 s wall time natively), turn <= 30e9 instructions | | `src/runtimes/python.rs:20-43, 220-225` | runaway Python |
| LIM27 | Rhai: 1M operations per replay, 32 call levels, expr depth 64/32, string 1 MiB, array 10,000, map 10,000, variables 1,000, functions 256, modules 0, `MAX_REQUESTS` 64 | | `src/runtimes/rhai_rt.rs:20-23, 122-131` | runaway Rhai, replay count |
| LIM28 | Wasm stack | 8 MiB | `.cargo/config.toml:9` | native recursion in engines/parsers |
| LIM29 | Canister Wasm memory limit | 3 GiB (default setting, D7) | canister settings | heap |
| LIM30 | IC platform limits relied on: 40e9 instructions per update message, 2 MiB ingress/reply, 300 s best-effort timeout cap, query instruction limit | | platform | |
| LIM31 | `truncate` of invalid canister id in errors | 80 B | `src/bridge.rs:466` | error size |
| LIM32 | Search `MAX_METHODS` | 40 methods per search | `src/search.rs:17` | search result size |

Missing-kind check: no limit on number of allowlist entries, method names per entry,
token count, token label length, override count, `query` keywords count/length,
`canister_id` length in search, audit log size (register #31), number of concurrent
searches, PENDING_RUNS size, demo_store order count.

## 10. Unbounded work (sized by untrusted input)

| Id | Work | Size driver | Bound |
|---|---|---|---|
| W1 | `parse_body` JSON parse of the request body | U3 | ingress size; serde_json depth 128; batch checked only after parsing |
| W2 | `inspect_message` decode + SHA-256 per ingress | U12 | ingress size |
| W3 | `search::render`: for every allowed method, pretty-print signature and `contains` for every keyword (both query and update paths); `collect_vars` over the type env | U5 `query` (number/length of keywords, unbounded), interface size | 40 methods shown, but matching runs over all methods; no keyword limit |
| W4 | `search::scope` + `policy::list_allowed()` copies the whole allowlist per request | admin-sized | none |
| W5 | `json_to_idl`: recursion (<= 128), records: unknown-key scan `keys x fields`, `field_value` linear search per field; variant tag search formats a string per field | U7 + target interface | value budget 200k nodes / 4 MiB |
| W6 | `decode_reply` + `idl_to_json` | U8 | 2 MiB reply, 20M quota, depth 128 |
| W7 | `Service::parse` (candid_parser) of fetched/override interfaces | U10/U11 | 512 KiB; parser recursion on nested types bounded only by stack (8 MiB) |
| W8 | JS: script parse/compile (Boa parser recursion on nested source), snapshot walk, console formatting, `JsValue::from_json` of replies | U6/U7/U8 | 100 KiB source; value limits; stack 8 MiB |
| W9 | JS: rounds x pending requests (64 x 64), each refused one costs a policy check + violation slot | U6 | call cap stops sends; refusals continue until 64 rounds |
| W10 | Python: parse/compile, `to_json` of call args per suspension (<= 256), `from_json` of replies | U6/U8 | value limits, suspensions |
| W11 | Rhai: up to 64 full replays of the script, each up to 1M operations; `answers` keeps every reply (as `serde_json::Value`) for the whole run; `rhai::serde::to_dynamic` per replay per answer | U6/U8 | 64 requests x reply size (each up to 2 MiB on the wire, larger as `Value`) held per run, x 8 concurrent runs |
| W12 | `truncate_escaped` char scan and `serde_json::to_string` of results; `http_update` re-serializes every reply to measure size | result size | value limits |
| W13 | `audit_page` | admin args | 100 |
| W14 | `PENDING_RUNS`, audit log growth | number of runs/searches | none (register #31) |
| W15 | demo_store: `orders` vector grows per `create_order` by anyone; `list_orders(null)` returns all orders | A10 | none (2 MiB reply limit eventually traps the query) |
| W16 | Demo page: `classicDefsChars` regex scan of interface text; rendering | C1 replies | browser |

## 11. Failure sites (aborting constructs outside tests)

`panic = "abort"` in release: every panic is a trap. Overflow checks are off in release
(default), so integer arithmetic wraps instead of trapping.

| Id | Site | Trigger | Consequence |
|---|---|---|---|
| F1 | `src/lib.rs:41` `ic_cdk::trap` on invalid token hash in `apply` | bad InitArgs | install/upgrade fails (rolled back) |
| F2 | `src/policy.rs:104, 107` `Encode!(..).expect("candid encode")` | encoding failure | trap on any write |
| F3 | `src/policy.rs:110` `Decode!(..).expect("candid decode")` | stored bytes not decodable as the current type (schema change) | trap on every read of that record: `limits()` is read on every request |
| F4 | `src/policy.rs:252` `append(..).expect("audit log out of stable memory")` | stable memory exhausted / grow fails | trap in K6 (closing entry lost; K7 tries again and traps too) |
| F5 | `src/candid_bridge.rs:108` `out.pop().unwrap()`, `:285` `o.iter().next().unwrap()` | guarded by length checks | none |
| F6 | `src/mcp.rs:256` `method.unwrap()` | guarded by `valid` | none |
| F7 | `src/runtimes/rhai_rt.rs:65, 136, 138, 144, 190, 199, 211, 218, 220` `Mutex::lock().unwrap()` | poisoning (needs a panic, which aborts anyway) | none expected |
| F8 | `RefCell` borrows: `IcBridge` (`messages`, `touched`, `violations`), JS `State` (`borrow_mut` in host functions), policy/bridge thread-locals | re-entrant borrow | panic -> trap |
| F9 | `shims/web-time/src/lib.rs:43` `Instant + Duration` `.expect("overflow ...")` | Monty adding a huge budget `Duration` to `Instant` (e.g. `max_instructions` near `u64::MAX` makes `vm_budget` huge), if Monty uses `+` | trap on every Python run (to verify) |
| F10 | `policy.rs:222` `e.key()[..16]` | keys validated to 64 hex on insert | none |
| F11 | Casts: `bridge.rs:333` len as u32; `candid_bridge.rs:216` `f64 as f32` (saturates to inf, no trap); `bridge.rs:89` f64 as i64 (in range) | | silent value change for float32 (fidelity) |
| F12 | Native stack overflow: engine parsers on deeply nested source (Boa, Monty; Rhai has expr-depth limits), `candid_parser` on nested types, `collect_vars` recursion | U6, U10/U11 | trap; 8 MiB stack |
| F13 | Out of memory: JS has no cap (register #24), Rhai `answers` (W11), 8 concurrent VMs, 3 GiB limit | U6 | trap |
| F14 | IC instruction limit 40e9 per message: one uninterruptible Boa built-in (register #23); Python turn limit 30e9 + work before the turn; `max_instructions` set above 40e9 | U6, AD | trap |
| F15 | IC reply size 2 MiB | guarded by LIM7 | |
| F16 | demo_store: `s.inventory[(product_id - 1)]` / `s.products[..]` indexing; `reserved += quantity` (wraps in release) | validated product ids; duplicate items in one order | wrong state rather than trap (to check) |

## 12. Boundary conversions

| Id | Conversion | Where |
|---|---|---|
| B1 | HTTP body bytes -> `serde_json::Value`; JSON-RPC classification | `src/mcp.rs:236-304` |
| B2 | Candid ingress arg -> `HttpUpdateRequest` (in `inspect_message` and the method) | `src/lib.rs:276`, ic-cdk |
| B3 | Token text -> SHA-256 hex; caller id = first 16 hex chars | `src/policy.rs:226`, `src/lib.rs:122, 222` |
| B4 | Principal text <-> `Principal` (canonicalisation) in search scope, bridge, overrides | `src/search.rs:39`, `src/bridge.rs:464-468`, `src/policy.rs:233-242` |
| B5 | JSON -> Candid (`json_to_idl`): numbers from floats/strings (underscores stripped, `-0`), hex blobs (`0x` optional), records by name/id/`_n_`, tuples from arrays, variants from `{"Tag":v}`/`"Tag"`, opt null, func/service from text, float32 from f64 | `src/candid_bridge.rs:58-92, 195-301` |
| B6 | Candid -> JSON (`idl_to_json`): ints as numbers when <= 2^53-1 else decimal strings, nat64/int64 via `big`, floats with `"NaN"`/`"inf"` strings, opt flattened (opt opt ambiguity, register #42), blob -> array, tuple records -> arrays, variant -> single-key object | `src/candid_bridge.rs:303-366` |
| B7 | `.did` text -> `TypeEnv`/actor (class unwrapped) | `src/candid_bridge.rs:34-46` |
| B8 | JS value -> JSON: `SNAPSHOT_JS` + Boa `to_json` + `normalize_numbers` (integral f64 <= 2^53 -> i64); `JSON.stringify` wrapper; JSON -> JS via `JsValue::from_json`; errors -> strings | `src/runtimes/js.rs:45-94, 207-225, 325-340, 403` |
| B9 | Python `MontyObject` -> JSON: big ints -> `py_repr` string, non-finite floats -> `null`, set/tuple -> array, dict keys `py_repr` when not str, bytes -> array, other types -> `py_repr` string; JSON -> MontyObject (u64 above i64 -> bigint, other numbers -> float) | `src/runtimes/python.rs:59-117` |
| B10 | Rhai `Dynamic` -> JSON (blobs -> arrays, maps, strings, else `rhai::serde::from_dynamic`); JSON -> Dynamic (`rhai::serde::to_dynamic`) | `src/runtimes/rhai_rt.rs:76, 85-117` |
| B11 | Script result -> MCP `CallToolResult` text (`serde_json::to_string`, string results passed raw), escaped-size truncation, logs joined | `src/mcp.rs:156-188` |
| B12 | `AuditEntry`/`Limits`/`AllowedMethods` <-> Candid bytes in stable memory | `src/policy.rs:100-116` |
| B13 | Language name -> `Language` (case-insensitive, aliases `js`, `py`) | `src/runtimes/mod.rs:34-44` |
| B14 | Time: ns -> ms (Boa `FixedClock`, timezone 0) -> seconds (Monty `DateTimeSource::Fixed`); instruction count -> `Duration` (web-time shim, 1 ns = 1 instruction) | `src/bridge.rs:77-82`, `src/runtimes/python.rs:214`, `shims/web-time` |
| B15 | `IC-Certificate` header (base64 certificate + fixed tree) | `src/lib.rs:82-94` |
| B16 | Interface -> search JSON (pretty-printed signatures and type texts) | `src/search.rs:49-159` |
| B17 | Demo page: JSON -> DOM (`innerHTML` with `esc` for `&<>` only; `textContent` elsewhere); `.did` text regex parsing | `src/demo.html:433-508` |

## 13. Value flows

| Id | Flow | Who pays / who gains |
|---|---|---|
| VF1 | Cycles for every `http_request_update` execution (after E3 lets it in), every outbound message (O1, O2 incl. failed `canister_metadata` attempts), every stable write (O3) and stored byte (D4 growth) | the canister (its controller), caused by token holders |
| VF2 | Query calls (E4) | free on mainnet; replica capacity |
| VF3 | Calls from other canisters to E5 (no `inspect_message`) | the caller pays sending; C1 pays execution if the token check passes |
| VF4 | Assets held by C1's principal on target canisters (e.g. ledger balances) moved by allowlisted methods (e.g. `icrc1_transfer` if a controller allowlists it) | every token holder acts as this one principal (register #49) |
| VF5 | Rhai replays and JS/Python work up to the instruction budget per run, x 8 concurrent, x unlimited requests over time | canister cycles; register #31 (no rate limit) |
| VF6 | demo_store stock reservation (model value: `reserved`, `on_hand`) | anyone |
| VF7 | Factory fees / customer cycles (C14, not built) | n/a |
| VF8 | Gemini API usage for the video (dev key) | developer |

## 14. Admin surface

| Id | Setting | Valid range in code | Outside the range |
|---|---|---|---|
| AD1 | `InitArgs` at install/upgrade (`init_args: "(null)"` in `icp.yaml`) | token hashes must be 64 hex | trap (F1) |
| AD2 | `set_allowlist(p, methods)` | any principal (incl. `aaaaa-aa`, this canister itself, demo_store), any strings, `"*"` wildcard, empty list (canister allowlisted for interface fetches but no methods) | accepted |
| AD3 | `remove_allowlist(p)` | | evicts heap caches; override D5 stays |
| AD4 | `set_limits`: `max_calls_per_script` any u32 (0 disables calls), `max_instructions` any u64 (0 refuses everything; > 40e9 lets runs reach the IC trap; near `u64::MAX` see F9), `call_timeout_seconds` any u32 (0? > 300 capped by the IC per the comment), `certified_reads` | no validation | accepted |
| AD5 | `add_api_token_hash(hash, label)` | hash 64 hex (case-folded, trimmed); label any length | error for bad hash |
| AD6 | `revoke_api_token_hash` | | returns false if absent |
| AD7 | `set_candid_override(p, opt did)` | <= 512 KiB and parses; any principal (not required to be allowlisted) | error |
| AD8 | Canister settings (controllers, memory limit, freezing threshold, log visibility), upgrades/reinstall (reinstall wipes D1-D5) | platform | |
| AD9 | Build-time choices: feature subset, `icp deploy --yes` for Candid-breaking upgrades | | |

## 15. Supply chain

| Id | Item | Detail |
|---|---|---|
| S1 | Lockfile | `Cargo.lock`: 391 packages; 387 from crates.io, 4 local (`icp_code_mode`, `demo_store`, `web-time` shim, vendored `chrono`); no git sources; all builds `--locked` |
| S2 | Direct deps (resolved) | ic-cdk 0.20.3, candid 0.10.40, candid_parser 0.4.1, ic-stable-structures 0.7.2, serde_json 1.0.154, serde_bytes 0.11.19, sha2 0.10.9, hex 0.4.3, base64 0.22.1, getrandom 0.3.4, futures 0.3.34, boa_engine/boa_gc 0.22.0 (pre-1.0), monty/monty-types =1.6.0 (exact pin, young), rhai 1.27.0 (features sync, no_time, serde + defaults) |
| S3 | Patches | `web-time` -> `shims/web-time` (own code, 53 lines); `chrono` -> `vendor/chrono` 0.4.45 (upstream minus `wasmbind` default and test targets; drift from upstream not checked) |
| S4 | getrandom | three versions: 0.2.17 (only via `const-random-macro`, a proc-macro run at build time), 0.3.4 (ahash, monty, direct) and 0.4.3 (boa's `rand` 0.10) resolve to the custom backend `__getrandom_v03_custom` (deterministic splitmix64) |
| S5 | Build-time randomness | rhai's default features enable `ahash/compile-time-rng`: random constants baked in at compile time, so builds are not bit-reproducible (to demonstrate) |
| S6 | Duplicate crate versions | 33 crates in two or three versions (e.g. `hashbrown` x3, `syn` 2 and 3, `rand` 0.8 and 0.10, `sha2` 0.10 and 0.11) |
| S7 | Toolchain | rustc 1.100.0 pinned; `rust-version = "1.98"`; icp-cli and ic-wasm versions not pinned in the repo (README: icp-cli >= 1.7) |
| S8 | Build flags | release `opt-level="z"`, `lto`, `strip`, `codegen-units=1`, `panic="abort"`; wasm32 rustflags `getrandom_backend="custom"`, stack 8 MiB; `ic-wasm shrink`; candid metadata public |
| S9 | Wasm imports (default build, cached artifact) | 36, all `ic0.*`; non-default subsets not linked in this step |
| S10 | Dev/test only | `futures` executor (tests); video: Chrome, ffmpeg, Node, Gemini API; `candid-extractor` (docs) |

## 16. Claims

Promises in the README, the interface comments, the tool descriptions shown to the AI model,
and code comments. "Shown to AI" marks text in `tools/list` or `initialize`.

| Id | Claim | Source |
|---|---|---|
| CL1 | "every run is audited" | README.md:25; TOOL_DESCRIPTION `src/mcp.rs:77` (shown to AI) |
| CL2 | Search is "answered by the query call, so it is fast and free" | README.md:14 |
| CL3 | Search is "Read-only, fast and free (no consensus round, no audit entry, does not count toward the call cap)" | SEARCH_DESCRIPTION `src/mcp.rs:69` (shown to AI). Code: a search that needs a fetch runs as an update, writes audit entries and takes call-cap slots (`src/lib.rs:121-190`) |
| CL4 | "Deny-by-default safety ... append-only on-chain StableLog"; "immutable audit log" | README.md:32, 42 |
| CL5 | Policy, tokens, audit log "survive upgrades" | README.md:42, 378; `src/policy.rs:4-5` |
| CL6 | Allowlist "deny by default. Checked before any message is sent, including the interface fetch" | README.md:365 |
| CL7 | Call cap 25 "Counts every icp.call attempt that passes policy and every interface fetch that misses the cache. Reserved before the await, so concurrent batches count" | README.md:366 |
| CL8 | Instruction budget "20B per script, measured from the script's own start. Checked before each host request, between JS rounds, and on every Rhai operation; Python has its own 10B VM budget" | README.md:367. Contradicted by README.md:371 and `src/runtimes/python.rs:34-43` (Python uses the remaining policy budget); `EXECUTION_BUDGET` doc comment (`python.rs:24-28`) says on-chain 10B but is only used off-chain |
| CL9 | "At most 50 messages per JSON-RPC batch. One execute_icp_script and five searches per HTTP request ... At most 8 scripts run at once ... JS: at most 64 concurrent requests per round and 64 rounds. Rhai: at most 64 requests" | README.md:368 |
| CL10 | "Any value leaving a script ... is limited to 128 levels, 200k elements and 4 MiB of text, in all three languages" | README.md:369 |
| CL11 | JS limits and removals; "Fixed clock = block time, UTC"; "JSON.stringify is held to the value limits too, replacer included" | README.md:370 |
| CL12 | Python "Recursion 200, 64 MiB memory, 256 suspensions ... no OS access" | README.md:371 |
| CL13 | Rhai "1M operations per run, 32 call levels ... eval and modules disabled" | README.md:372 |
| CL14 | "Captured logs <= 64 KiB, enforced while running. Result <= 1 MB of escaped JSON. Any reply that would exceed 1.9 MB ... is replaced by a small error, so replying can never trap" | README.md:373; `src/mcp.rs:312-313` |
| CL15 | Audit fields and caps; "One entry per outbound message, written as the system accepts it, plus one closing entry per run" | README.md:374 |
| CL16 | `inspect_message` "rejects update calls that carry no registered token ... before the canister is charged" | README.md:375; `src/lib.rs:265-272` |
| CL17 | "the log never shows a message that was not sent, and never misses one that was" | README.md:394-396; `src/bridge.rs:382-394` |
| CL18 | "A script run ends with exactly one of these" closing entries | README.md:400-407 |
| CL19 | "Two kinds of request write no entries ... a search answered from cached interfaces, and a script refused as busy" | README.md:411-412 |
| CL20 | Missing closing entry is "pending or unknown, never proof of a trap. A trap is recorded as Trapped" | README.md:414-426; `.did` comments |
| CL21 | `call_timeout_seconds` / `certified_reads` semantics; "Read the audit log with an update call" | README.md:428-439; `.did` Limits comments |
| CL22 | Interfaces cached 1 h, failures 5 min, "Interfaces over 512 KiB are refused" | README.md:441-444 |
| CL23 | Canister ids canonicalised "before the allowlist check, the interface cache and the override lookup" | README.md:446-447 |
| CL24 | Known limitations: no JS memory cap; one built-in can hit 40B trap; tokens visible to nodes; one identity; Rhai O(n^2); opt opt; Candid-breaking statuses | README.md:449-472 (register #23, #24, #42, #48, #49, #51) |
| CL25 | "The release Wasm imports only ic0.*" | README.md:487 |
| CL26 | getrandom stream "only used for hash seeding"; "never use it for secrets" | README.md:478-481; `src/lib.rs:429-432` |
| CL27 | Shim makes Monty's limits "deterministic instruction budgets" | README.md:484-485; `shims/web-time/Cargo.toml` |
| CL28 | Skip-certification: query responses work on the certified domain; "Script results always come from the update call, which consensus certifies" | README.md:497-499; `src/lib.rs:51-54` |
| CL29 | Tool description: "identical semantics in every language"; "No network, filesystem, clock, or randomness (Date/time are fixed to the block time; Math.random is removed)" | `src/mcp.rs:79, 101` (shown to AI). Python exposes a seeded `random` |
| CL30 | Demo page: "contains no secrets"; token "sent only as the Authorization header"; "Only its SHA-256 hash is stored on-chain" | README.md:239-240; `src/demo.html:81` |
| CL31 | `PolicyView.active_runs`: "at most MAX_CONCURRENT_RUNS"; `pending_run_ids`: runs "still waiting on a reply" | `src/lib.rs:32-35`, `.did` |
| CL32 | `get_audit_logs` "Newest first; limit is capped at 100" | `.did`, `src/lib.rs:346` |
| CL33 | Benchmarks table (2.6x / 1.2x / 1.4x fewer input tokens) and "12 MB (3.6 MB gzipped) ... JS + Rhai about 6 MB" | README.md:120, 194-203 |
| CL34 | `.cargo/config.toml`: a stack overflow is a trap that discards the audit entry, so the stack is 8 MiB | `.cargo/config.toml:5-7` |
| CL35 | Rhai: "Scripts are deterministic, so every replay takes the same path; a divergence is reported as an error" | `src/runtimes/rhai_rt.rs:3-8` |
| CL36 | `MAX_SEARCHES` comment: "Searches are cheap and size-capped, so they are exempt" from the one-script rule | `src/mcp.rs:390-391` |
| CL37 | Idea doc cost figures (create 0.5T cycles, idle ~8T/year, 15-20M cycles per run) | `docs/per-customer-canisters.md` |

## 17. Tests

59 tests (57 in `icp_code_mode`, 2 in `demo_store`), all native.

| Id | Test | Asserts |
|---|---|---|
| T1 | `tests::run_slots_are_capped_and_returned` | `RunSlot` cap of 8 and return on drop |
| T2 | `tests::did_file_matches_exported_interface` (both crates) | `.did` files match `export_candid!` |
| T3 | `bridge::mock::on_first_pending_fires_once_and_only_if_pending` | hook fires once only if pending (mechanism of K1) |
| T4 | `candid_bridge::tests::*` (7) | round trips of records/opt/blob/variants/positional args, big nat/int, recursive types, descriptive errors incl. unknown record field, JS floats/-0/NaN/inf, deep replies error |
| T5 | `mcp::tests::*` (13) | auth/routing codes, initialize/list in query, upgrade on tools/call, execution in update via mock, bad params -32602, batches, cached search in query, demo page, `replies_always_fit`, certified mode, id validation, escaped result bound, one script + 5 searches per request |
| T6 | `policy::tests::*` (7) | authorize allow/deny, caps, stable check_call, token hashing, audit paging, run_id linking, old entries/limits decode |
| T7 | `runtimes::cross_language_tests::*` (5) | same script in 3 languages, catchable policy errors, syntax/loop/recursion containment, interleaved runs isolated, unknown language/oversized code |
| T8 | `runtimes::js::tests::*` (8) | determinism hardening, Promise.all batching, values/errors, deep/huge values, promise-chain bound, stringify replacer and sizes, big ints, console cap |
| T9 | `runtimes::python::tests::*` (5) | gather batching, determinism/sandbox, value bounds, memory limit under interleaving, memory limit |
| T10 | `runtimes::rhai_rt::tests::*` (5) | replay issues each call once, terminate marker not swallowed, limits/sandbox, request cap, print cap |
| T11 | `search::tests::*` (4) | allowed methods + referenced types, keyword filter, scope canonicalisation, unfetched/failed/cap |
| T12 | `demo_store::tests::seed_is_consistent` | seed shape and query helpers |

Rows with **no automated test** (each is also a register "manual"/"none" guard or new):
E1/E2 `apply` and post-upgrade behaviour; E3 `inspect_message`; E5 through the real
`IcBridge` (all of K1-K7, the `Trapped` entry, `PENDING_RUNS`); `run_search` update path
(only a mock in T5); E6-E14 admin endpoints (`set_candid_override` size/parse check,
`remove_allowlist` eviction, `get_policy`); LIM3 timeout path; LIM12 cap on a live network;
LIM13/LIM14 audit truncation and violation caps; LIM17 method length; LIM18 TTLs; LIM19 size
check; composite_query refusal; `canister_metadata` fallback; Python print cap (LIM15);
counting allocator `realloc` path; B15 certificate header; F3 schema evolution beyond the
two cases in T6; F9 web-time shim; C4 vendored chrono (its test targets were removed); C2
writes (E21-E23); C5 page behaviour; C6 script; C1a non-default feature links and S9 imports
(no CI); S5 reproducibility.

## 18. History

From `docs/audit/REGISTER.md` (rounds R1-R6, all 2026-11-17): 53 findings and 5 testability
gaps.

| Status | Ids |
|---|---|
| Open | 31 (L5, audit log growth / no rate limit), 51 (L13, Candid-breaking status additions) |
| Accepted | 11 (L2), 23 (L3), 24 (L3), 42 (L7), 48 (L10), 49 (L10) |
| Mitigated | 10 (L2) |
| Fixed (opt-in) | 44 (L9), 47 (L10) |
| Fixed | 1-9, 12-22, 25-30, 32-41, 43, 45, 46, 50, 52, 53 |
| Testability gaps (open) | 54-58 |

Classes in the register: L1, L2, L3, L4, L5, L6, L7, L8, L9, L10, L12, L13, L16 and
**Testability** (not a plan lens, so it is an extra lens). Every guard named in the register
exists as a test in the suite (checked by name against `cargo test` output). Guards marked
manual or none: 5, 6, 7, 9, 10, 11, 15, 16, 17, 22-25, 27, 30, 31, 32, 42, 44 (failure path),
45, 46, 48, 49, 51, 52, 53; bridge path of 1; 8 (mechanism only); 29 (never triggered live).

## 19. Notes for the lenses (observed while inventorying; not findings)

Each needs a reviewer to prove or refute it under the plan's evidence rule.

1. CL3 / CL8 / CL29: documentation and tool descriptions shown to the AI model disagree with the code (search audit/cap behaviour; Python budget; "no randomness").
2. LIM12 / CL31: searches (E5b) take no run slot, so concurrent searches are not capped by `MAX_CONCURRENT_RUNS`, and their `Started` entries enter `PENDING_RUNS`.
3. LIM15: on hitting the 64 KiB output cap Python raises an error (Monty `check_print_collect_limit`), while JS and Rhai truncate silently: a cross-runtime difference.
4. B9: Python non-finite floats become `null` and unknown object types become `py_repr` strings without an error, where B6/B8 behave differently.
5. W11: Rhai keeps every reply for the whole run (up to 64), multiplied by 8 concurrent runs.
6. W3: search keyword count is unbounded and runs in the free query path.
7. F9 / AD4: `set_limits` accepts any value; very large `max_instructions` may overflow `Instant + Duration` in the web-time shim on the Python path; values above 40e9 defeat the clean-error design.
8. AD2 / A7: allowlisting `aaaaa-aa` or this canister's own principal exposes management or self calls (self-call to `http_request_update` with a token runs nested scripts).
9. D5 / AD3: overrides survive `remove_allowlist`, and `icp.candid` returns cached or override text without an allowlist check (no message is sent, so CL6 may still hold literally).
10. S5: non-reproducible build via `ahash/compile-time-rng`.
11. W15 / E21: demo_store accepts writes from anyone and grows without bound; duplicate product lines in one order are each checked against the same availability.
12. B17: demo page `esc` does not escape quotes and `slice(0, 40)` is applied after escaping (can cut an entity).
