# ICP Code Mode — an MCP server inside a canister

[![Watch the 52-second explainer (recorded when the comparison counted all 10 store methods; the 8-method figure is 2.3x)](video/poster.jpg)](video/icp-code-mode-explainer.mp4)

▶ **[Watch the 52-second explainer](video/icp-code-mode-explainer.mp4)** (1080p, with voiceover and captions).
It shows the same task run with classic MCP and with Code Mode, both against a demo store canister.

An Internet Computer canister that speaks the **Model Context Protocol** (2025-03-26,
Streamable HTTP via the ICP HTTP gateway) and follows the search-and-execute pattern
with two tools:

- **`search_icp_canisters`** — read-only discovery. It lists the allowlisted canisters, their
  allowed methods with Candid signatures, and the types those signatures use, filtered by
  keywords. Once interfaces are cached it is answered by a query call, fast and free. A search
  that must first fetch an interface runs as an update call: audited, limited like a
  script's calls, and holding one of the run slots. A search over a canister whose last fetch
  failed (remembered for five minutes) also goes through an update, so the failure is audited.
- **`execute_icp_script`** — instead of dozens of granular tools, the LLM sends one short
  script in **JavaScript**, **Python** or **Rhai**.

The script:

- discovers target canisters' Candid interfaces,
- batches inter-canister calls,
- filters and transforms the data inside the Wasm sandbox,
- returns one distilled JSON result.

On-chain policy (allowlist, call cap, instruction budget) is enforced, and every run is audited.

## Benefits

- **Fewer round-trips:** Batch multiple inter-canister calls (`Promise.all` / `asyncio.gather`) inside a single `tools/call` instead of sequential LLM tool turns.
- **Smaller context window:** Filter, join, and aggregate raw Candid responses inside the Wasm sandbox so only the final JSON summary reaches the model.
- **Zero per-canister tool wiring:** `search_icp_canisters` and `icp.candid` discover `.did` interfaces dynamically—no custom MCP server or schema needed per canister.
- **Deny-by-default safety & auditability:** Controllers restrict access per `(Principal, method)`, cap calls/instructions, and inspect an on-chain `StableLog` that the canister code only appends to.
- **No local MCP runtime:** Any HTTP-capable MCP client (Claude, Gemini, etc.) connects directly to the canister URL with a Bearer token.

## Comparison with Cloudflare Code Mode

Cloudflare's Code Mode introduced the pattern of letting an LLM write code against a typed SDK inside a sandbox rather than calling many fine-grained MCP tools. **ICP Code Mode** brings that same search-and-execute pattern natively onto the Internet Computer, with several advantages for ICP workflows:

- **On-chain, consensus-certified execution:** Scripts run inside the canister's deterministic Wasm sandbox (`http_request_update`) and make native inter-canister calls, rather than running in an off-chain isolate talking to external RPCs.
- **Native Candid bridge:** Built-in `.did` parsing and type-directed JSON ⇄ Candid conversion—no off-chain IDL tooling or generated client bindings required.
- **Multi-language runtime:** Supports **JavaScript** (Boa), **Python** (Monty), and **Rhai** in one canister (or per-language feature builds). Boa and Monty are vendored with sandboxing patches (see "Why the patches").
- **On-chain policy & append-only audit log:** Allowlists, SHA-256 token hashes, instruction budgets, and `StableLog` audit entries (which the canister code never edits or deletes) live in canister stable memory and survive upgrades.
- **Zero external infra:** The canister *is* the MCP server via the ICP HTTP gateway—no separate Worker, proxy, or API server to host.

## Quickstart (How to use)

1. **Deploy locally** ([build & deploy details](#deploy-locally-with-icp-cli)):
   ```bash
   icp network start -d && icp deploy
   ```
2. **Create and register a Bearer token** ([token details](#create-a-bearer-token-only-its-sha-256-goes-on-chain)):
   ```bash
   export MCP_TOKEN=$(openssl rand -hex 32)
   icp canister call icp_code_mode add_api_token_hash "(\"$(printf %s "$MCP_TOKEN" | shasum -a 256 | cut -d' ' -f1)\", \"my-laptop\")"
   ```
3. **Allowlist target canisters/methods** ([policy & admin endpoints](#allowlist-what-scripts-may-call-deny-by-default)):
   ```bash
   icp canister call icp_code_mode set_allowlist '(principal "ryjl3-tyaaa-aaaaa-aaaba-cai", vec {"icrc1_balance_of"; "icrc1_symbol"; "icrc1_decimals"; "icrc1_metadata"; "icrc1_total_supply"})'
   ```
4. **Connect your MCP client** ([client config](#mcp-client-config)) or **test with `curl`** ([curl examples](#curl-examples-local)) against `http://localhost:8010/mcp?canisterId=<canister-id>` (the id comes from `icp canister status icp_code_mode --id-only`), or `https://<canister-id>.icp0.io/mcp` on mainnet. The friendly `icp_code_mode.local.localhost` name works in curl and browsers, but not in Node or Python on macOS unless `/etc/hosts` maps it to 127.0.0.1.

For SDK signatures, value encoding, and execution limits, see [The `icp` SDK](#the-icp-sdk) and [Guardrails](#guardrails).

```
MCP client ──POST /mcp──▶ HTTP gateway ──▶ http_request (query)
                                             ├─ initialize / tools/list / ping → answered here
                                             ├─ search_icp_canisters → answered here from cached interfaces
                                             └─ execute_icp_script (or search needing a fetch) → upgrade: true
                                  ──▶ http_request_update (update)
                                        LanguageRouter ─▶ Boa | Monty | Rhai   (fresh VM per call)
                                              │ icp.candid / icp.call / icp.principal
                                        HostBridge ─▶ policy::check_call ─▶ candid_bridge (JSON⇄Candid)
                                              └─▶ ic_cdk::call::Call::bounded_wait ─▶ target canister
                                        audit log (StableLog) ◀─ one entry per run
```

## Layout

| Path | What |
|---|---|
| `src/lib.rs` | Lifecycle (`init`, `post_upgrade`), `http_request` / `http_request_update`, admin endpoints, skip-certification, counting allocator, deterministic `getrandom` backend |
| `src/mcp.rs` | JSON-RPC 2.0 router (`initialize`, `notifications/initialized`, `ping`, `tools/list`, `tools/call`, batches), bearer auth |
| `src/bridge.rs` | `HostBridge` trait (shared by all runtimes) and `IcBridge` (policy → Candid fetch → call → decode) |
| `src/candid_bridge.rs` | `.did` parsing (`candid_parser`) and type-directed `serde_json::Value` ⇄ `IDLValue` conversion |
| `src/demo.html` | Demo UI served at `GET /`, including the Classic MCP vs Code Mode comparison |
| `demo_store/` | Demo target canister: 10 store methods with seed data |
| `scripts/` | `demo-setup.sh` (allowlists `demo_store` in `icp_code_mode`), plus the checks CI runs (Wasm imports, README URLs, demo page, demo setup, audit workflow) |
| `video/` | Explainer video: script, HTML composition, Gemini TTS and render pipeline, final MP4 |
| `src/search.rs` | `search_icp_canisters`: allowlist × interface → filtered signatures and referenced type definitions |
| `src/policy.rs` | Stable-memory guardrails: allowlist, limits, SHA-256 token hashes, audit log (appended to only) |
| `src/runtimes/` | `LanguageRouter` + `js.rs` (Boa), `python.rs` (Monty), `rhai_rt.rs` (Rhai) |
| `icp_code_mode.did` | Canister interface. A test checks that it matches `export_candid!` |
| `icp.yaml` | icp-cli project (build steps, canister settings, local network on port 8010) |
| `shims/web-time`, `vendor/chrono` | `[patch]`es that keep wasm-bindgen imports out of the Wasm (see "Why the patches") |
| `vendor/boa_engine`, `vendor/monty`, `vendor/candid` | `[patch]`ed copies with small sandboxing changes, each listed in its `PATCHES.md` (see "Why the patches") |
| `harness/` | PocketIC test that installs the release Wasm and sends a corpus of wasm32-only traps (its own crate and lock file) |
| `.github/workflows/ci.yml` | CI: native tests and clippy per feature set, release Wasm, ic0-only import check, reproducible-build hash check, the PocketIC harness, `cargo deny check advisories`, tooling tests |
| `deny.toml` | cargo-deny advisory policy (the accepted unmaintained crates) |

## Build and test

Requirements:
- Rust 1.100, pinned in `rust-toolchain.toml` (monty 1.6 needs ≥ 1.98)
- the `wasm32-unknown-unknown` target
- [icp-cli](https://cli.internetcomputer.org) ≥ 1.7
- `ic-wasm` 0.11.1 exactly (icp.yaml checks the version, so the same source gives the same Wasm)
- `candid-extractor` (regenerating the `.did`), `openssl`, `shasum` and `curl` (token setup and examples)

```bash
cargo test --lib
```

```bash
cargo build --target wasm32-unknown-unknown --release --locked --target-dir target
```

Release builds keep overflow checks (`overflow-checks = true` in `Cargo.toml`), so overflow
traps instead of wrapping. Before a release, also run the tests in that profile:
`cargo test --release --workspace --lib`.

The PocketIC harness installs that release Wasm and checks that a corpus of scripts that
used to trap on wasm32 (memory exhaustion, deep recursion, huge allocations) gets a JSON-RPC
reply or a 503:

```bash
cd harness && cargo test --locked
```

The pocket-ic crate downloads its server unless `POCKET_IC_BIN` points to a 16.x server binary.

Engines are Cargo features (`default = ["js", "python", "rhai"]`). To build a dedicated
per-language worker, for example a Rhai-only one:

```bash
cargo build --target wasm32-unknown-unknown --release --no-default-features --features rhai --target-dir target/rhai
```

To deploy a worker, copy the canister entry in `icp.yaml` under a new name, use that build command,
and change its `cp` source to `target/rhai/wasm32-unknown-unknown/release/icp_code_mode.wasm`.
The all-in-one build is about 12 MB (3.8 MB gzipped). JS + Rhai without Python is about 6 MB.

To regenerate the `.did` after changing endpoints:

```bash
candid-extractor target/wasm32-unknown-unknown/release/icp_code_mode.wasm > icp_code_mode.did
```

## Deploy locally with icp-cli

```bash
icp network start -d
```

```bash
icp deploy
```

The gateway URL is printed on deploy: `http://icp_code_mode.local.localhost:8010/`. That
`*.localhost` name resolves in curl and browsers only; programmatic clients (Node, Python)
should use `http://localhost:8010/?canisterId=<canister-id>`, or add an `/etc/hosts` entry.
The network uses port 8010 (set in `icp.yaml`) so it can run next to other projects on the default 8000.

`icp.yaml` also sets the canister's `freezing_threshold` (30 days), `wasm_memory_limit`
(3 GiB, well above the 1 GiB heap ceiling), `wasm_memory_threshold` (1 GiB) and
`log_memory_limit` (2 MiB); see [Operations](#operations). Install arguments (`init_args`)
must match their Candid types exactly: icp-cli does not type them, so annotate numbers
(`5 : nat32`); `icp.yaml` has an example. Upgrades ignore them.

### Create a bearer token (only its SHA-256 goes on-chain)

```bash
export MCP_TOKEN=$(openssl rand -hex 32)
```

```bash
icp canister call icp_code_mode add_api_token_hash "(\"$(printf %s "$MCP_TOKEN" | shasum -a 256 | cut -d' ' -f1)\", \"my-laptop\")"
```

### Allowlist what scripts may call (deny by default)

The local network ships an ICP ledger:

```bash
icp canister call icp_code_mode set_allowlist '(principal "ryjl3-tyaaa-aaaaa-aaaba-cai", vec {"icrc1_balance_of"; "icrc1_symbol"; "icrc1_decimals"; "icrc1_metadata"; "icrc1_total_supply"})'
```

Use `vec {"*"}` to allow every method of a canister except the ones that delegate authority
(`icrc2_approve`, `icrc37_approve_tokens`, `icrc37_approve_collection`, `approve`). An
approval outlives `revoke_api_token_hash` and `remove_allowlist`, so these must be listed by
name; for ledgers it is better to name every method. `"*"` is always the wildcard: a method
literally named `*` can only be reached through it. `vec {}` allowlists the interface only:
scripts can read it with `icp.candid` (those fetches are sent and logged) and search lists the
canister with no methods, but every call is refused.

`set_allowlist` refuses the management canister (`aaaaa-aa`) and this canister, and caps its
input at 256 methods per canister, 256-byte method names, 1,000 canisters and 512 KiB of
method names in total. Token labels are at most 128 bytes, at most 1,000 tokens can be
registered, and the hash of the empty token is refused.

The other admin endpoints are controller-only:
- `set_limits(record { max_calls_per_script; max_instructions; call_timeout_seconds; certified_reads; tool_calls_per_minute })`
  sets them and returns the previous values. An omitted or `null` optional field keeps its
  current value; to change it, give it explicitly (`opt 300`, `opt false` and `opt 120` are the
  defaults). See "Options" below.
- `remove_allowlist`
- `revoke_api_token_hash` (the full hash, or the 16-character id shown by `get_policy` and in
  the audit `caller`; an id that matches more than one token is refused)
- `set_candid_override(principal, opt text)` (at most 1,000 overrides; replacing or clearing
  one still works at the cap)
- `get_policy`, `get_metrics` (see [Operations](#operations)), `get_audit_log_len`
- `get_audit_logs(offset, limit)` (newest first), `get_audit_logs_before(opt id, limit)` (pass
  the last id of a page to get the next one; unlike offsets, it does not shift as entries are
  appended) and `get_audit_run(run_id, opt from)` (one run's entries, oldest first; each call
  looks at no more than 5,000 ids, so continue from the last id returned plus one, or from
  `from` + 5000 after an empty page; the run is complete once an entry with a status other
  than `Started`/`Call` is returned)

Arguments to `set_limits`, `set_candid_override` and the install must match their Candid types
exactly. A value that Candid would otherwise turn into `null` without saying so is refused: an
untyped number such as `opt 60`, a misspelled or unknown field, or `opt blob` passed to
`set_candid_override`.

## Demo store and side-by-side comparison

`demo_store/` is a second canister: a small shop with 10 methods (7 reads, 3 writes) and
deterministic seed data (12 products, 8 customers, 60 orders). It exposes its interface
through `__get_candid_interface_tmp_hack`. Deploying it and allowlisting its reads plus
`create_order` takes two commands; `cancel_order` and `restock` stay blocked for scripts
(icp_code_mode's allowlist), so the guardrail is visible. The store itself has no access
control and accepts writes from anyone, so deploy it on a local network only. Orders are
capped at 20 lines, and the store at 1,000 orders, cancelled ones included; nothing frees a
slot, so once anyone fills it, `create_order` returns `StoreFull` for everyone until the next
install or upgrade reseeds it.

```bash
icp deploy
```

```bash
scripts/demo-setup.sh
```

It targets `$ICP_ENVIRONMENT` (default `local`); pass an environment name to override it, for
example `scripts/demo-setup.sh ic`.

The demo page's **Classic MCP vs Code Mode** section runs each task both ways against the
store and checks that both sides return the same answer:
- **Classic MCP:** one tool per store method. Each call is a real tool call, and its full result
  goes through the model.
- **Code Mode:** one search, then one script.

It reports LLM round trips, tool calls, and tool-definition / tool-result / generated / total
input tokens (estimated as characters ÷ 4 over a modelled agent loop), plus measured canister
time. "How the numbers are counted" on the page spells out what is measured and what is
modelled.

Measured on the local network:

| Task | Classic MCP | Code Mode | Total input tokens |
|---|---|---|---|
| Top 3 customers by spend | 4 tool calls, ≈3.2k tok of results | 2 tool calls, ≈0.5k tok | 2.3× fewer |
| What needs restocking? | 13 tool calls, ≈0.4k tok of results | 2 tool calls, ≈0.5k tok | 1.1× *more* |
| Tea sales report | 2 tool calls, ≈3.1k tok of results | 2 tool calls, ≈0.5k tok | 1.2× fewer |

Both sides see the same 8 allowlisted methods (Classic MCP as 8 tool definitions). Code
Mode's cost here is mostly its fixed overhead: two tool definitions plus one search. On the
restocking task the store's results are small, so Classic MCP comes out slightly ahead; the
gap favours Code Mode as results grow.
Classic MCP grows with every row and call, so the gap widens with real data volumes.

## Explainer video

`video/` builds the explainer from source:
- `script.md` has the script and the tweet copy.
- `composition.html` holds the scenes, styled after the ICP dashboard. A `renderAt(t)` function draws any moment of the video.
- `render.mjs` renders it frame by frame in headless Chrome, then encodes with ffmpeg.
- `tts.mjs` voices each scene with Gemini TTS. Scenes are timed to the real audio.
- `capture-demo.mjs` takes screenshots of the live demo page. It reads `MCP_TOKEN` from the
  environment or `.env` (quotes are stripped). Each Chrome run uses a throwaway profile with a
  random DevTools port, deleted when the run ends.
- `node video/test-tools.mjs` checks the `.env` reader and that Chrome runs leave no profile behind.

Requirements:
- Chrome, Node ≥ 22 and ffmpeg
- `GEMINI_API_KEY` in the environment, in this project's `.env`, or in `~/.env`

The voice argument is optional (default `Puck`):

```bash
video/build.sh Puck
```

The output goes to `video/out/` (gitignored) and is copied to `video/icp-code-mode-explainer.mp4`.

## Demo page

The canister serves a demo UI at `GET /`:
- local: `http://icp_code_mode.local.localhost:8010/`
- mainnet: `https://<canister-id>.icp0.io/`

Paste the bearer token, then use the buttons:
- **Initialize** and **List tools**
- **Search**, with optional keywords
- **Execute**, with five ready-made scripts: a JS ledger overview, Python balance, Python
  batching with `asyncio.gather`, a Rhai interface read, and a call that policy blocks

Each button sends a real JSON-RPC request to `POST /mcp` and shows the result, the console
output and the raw request/response pair. The page contains no secrets. The token stays in
your browser tab's sessionStorage (gone when the tab closes) and is sent only as the `Authorization` header.
The page is `src/demo.html`, compiled into the Wasm with `include_str!`.

## curl examples (local)

```bash
export MCP="http://localhost:8010/mcp?canisterId=$(icp canister status icp_code_mode --id-only)"
```

**initialize** (answered by the query call, no consensus round):

```bash
curl -s "$MCP" -H "Authorization: Bearer $MCP_TOKEN" -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"curl","version":"1"}}}'
```

```json
{"id":1,"jsonrpc":"2.0","result":{"capabilities":{"tools":{}},"instructions":"…","protocolVersion":"2025-03-26","serverInfo":{"name":"icp-code-mode","version":"0.1.0"}}}
```

**notifications/initialized** returns `202` with an empty body:

```bash
curl -s -o /dev/null -w '%{http_code}\n' "$MCP" -H "Authorization: Bearer $MCP_TOKEN" -H 'Content-Type: application/json' -d '{"jsonrpc":"2.0","method":"notifications/initialized"}'
```

**tools/list**:

```bash
curl -s "$MCP" -H "Authorization: Bearer $MCP_TOKEN" -H 'Content-Type: application/json' -d '{"jsonrpc":"2.0","id":2,"method":"tools/list"}'
```

**tools/call, search.** This is answered by the query call once the target's interface is
cached; the first search after a deploy goes through an update call to fetch it:

```bash
curl -s "$MCP" -H "Authorization: Bearer $MCP_TOKEN" -H 'Content-Type: application/json' -d '{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"search_icp_canisters","arguments":{"query":"balance"}}}'
```

```json
{"id":7,"jsonrpc":"2.0","result":{"content":[{"type":"text","text":"{\"canisters\":[{\"allowed_methods\":[\"icrc1_balance_of\",\"icrc1_symbol\",\"icrc1_decimals\",\"icrc1_metadata\",\"icrc1_total_supply\"],\"canister_id\":\"ryjl3-tyaaa-aaaaa-aaaba-cai\",\"methods\":[{\"name\":\"icrc1_balance_of\",\"signature\":\"(Account) -> (Icrc1Tokens) query\"}],\"types\":{\"Account\":\"record { owner : principal; subaccount : opt SubAccount }\",\"Icrc1Tokens\":\"nat\",\"SubAccount\":\"blob\"}}]}"}],"isError":false}}
```

Arguments are optional:
- `query`: space-separated keywords (at most 256 bytes), all of which must appear in a
  method's name, signature, doc comment or argument names (shown as `doc`). Methods `icp.call`
  can never call (oneway, composite_query, names over 256 bytes, an `empty` argument) are not
  listed.
- `canister_id`: restrict the search to one canister. It is parsed exactly as calls parse it
  (no trimming); an invalid id is reported as such.

With no arguments, it lists everything that's allowed, up to 40 methods.

**tools/call, JavaScript.** The gateway upgrades the request to an update call. The `Promise.all` sends four calls concurrently:

```bash
curl -s "$MCP" -H "Authorization: Bearer $MCP_TOKEN" -H 'Content-Type: application/json' -d @- <<'EOF'
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"execute_icp_script","arguments":{"language":"javascript","code":"const L = 'ryjl3-tyaaa-aaaaa-aaaba-cai';\nconst did = await icp.candid(L);\nconst [sym, dec, supply, bal] = await Promise.all([\n  icp.call(L, 'icrc1_symbol', null), icp.call(L, 'icrc1_decimals', null),\n  icp.call(L, 'icrc1_total_supply', null),\n  icp.call(L, 'icrc1_balance_of', {owner: icp.principal(), subaccount: null})]);\nconsole.log('did bytes', did.length);\nreturn {sym, dec, supply, bal, methods: (did.match(/icrc1_\\w+/g) || []).length};"}}}
EOF
```

```json
{"id":3,"jsonrpc":"2.0","result":{"content":[{"type":"text","text":"{\"bal\":0,\"dec\":8,\"methods\":14,\"supply\":\"299999999995070000\",\"sym\":\"ICP\"}"},{"type":"text","text":"[console output]\ndid bytes 17368"}],"isError":false}}
```

**tools/call, Python.** `icp.call` looks synchronous. `icp.call_async` together with `asyncio.gather` batches calls:

```bash
curl -s "$MCP" -H "Authorization: Bearer $MCP_TOKEN" -H 'Content-Type: application/json' -d @- <<'EOF'
{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"execute_icp_script","arguments":{"language":"python","code":"import asyncio\nL = 'ryjl3-tyaaa-aaaaa-aaaba-cai'\nsym, dec = await asyncio.gather(icp.call_async(L, 'icrc1_symbol'), icp.call_async(L, 'icrc1_decimals'))\nmeta = icp.call(L, 'icrc1_metadata')\nprint('metadata entries:', len(meta))\n{'sym': sym, 'dec': dec, 'keys': [m[0] for m in meta]}"}}}
EOF
```

```json
{"id":4,"jsonrpc":"2.0","result":{"content":[{"type":"text","text":"{\"dec\":8,\"keys\":[\"icrc1:decimals\",\"icrc1:name\",\"icrc1:symbol\",\"icrc1:fee\"],\"sym\":\"ICP\"}"},{"type":"text","text":"[console output]\nmetadata entries: 4"}],"isError":false}}
```

**tools/call, Rhai:**

```bash
curl -s "$MCP" -H "Authorization: Bearer $MCP_TOKEN" -H 'Content-Type: application/json' -d '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"execute_icp_script","arguments":{"language":"rhai","code":"let L = \"ryjl3-tyaaa-aaaaa-aaaba-cai\"; #{sym: icp::call_canister(L, \"icrc1_symbol\"), supply: icp::call_canister(L, \"icrc1_total_supply\")}"}}}'
```

A call blocked by policy comes back as a tool error (`isError: true`). No message is sent, and the violation is audited:

```bash
curl -s "$MCP" -H "Authorization: Bearer $MCP_TOKEN" -H 'Content-Type: application/json' -d '{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"execute_icp_script","arguments":{"code":"return await icp.call(\"ryjl3-tyaaa-aaaaa-aaaba-cai\", \"icrc1_transfer\", {})"}}}'
```

```bash
icp canister call icp_code_mode get_audit_logs '(0, 5)'
```

If the instruction budget runs out after messages were sent, that is not a policy refusal: the
error has no `policy:` prefix and lists the messages that went out, since they may have taken
effect, and the run closes as `Error`.

On mainnet, use `https://<canister-id>.icp0.io/mcp`.

### MCP client config

```json
{ "mcpServers": { "icp": { "type": "http", "url": "http://localhost:8010/mcp?canisterId=<canister-id>",
  "headers": { "Authorization": "Bearer <token>" } } } }
```

## The `icp` SDK

| | JavaScript | Python | Rhai |
|---|---|---|---|
| Interface | `await icp.candid(id)` | `icp.candid(id)` | `icp::candid(id)` |
| Call | `await icp.call(id, m, args)` | `icp.call(id, m, args)` / `await icp.call_async(...)` | `icp::call_canister(id, m, args)` |
| Own principal | `icp.principal()` | `icp.principal()` | `icp::principal()` |
| Output | `console.log`, `print` | `print` | `print`, `debug` |
| Result | `return value` | last expression | last expression |
| Batching | `Promise.all` | `asyncio.gather` | sequential |

How `args` map to the method's arguments:
- `null`: zero arguments.
- An array: positional arguments. For a one-argument method whose argument is itself a vec or
  tuple (or an opt of one), an array is first read as that argument, and `[x]` is a positional
  wrapper only when that reading fails. A reply passed back unchanged therefore encodes to the
  same value.
- Anything else: the single argument, for one-argument methods.

How values are encoded:
- Records are objects, except tuple records (fields numbered 0..n-1 in order, e.g.
  `record { text; nat }`), which are arrays; input accepts either form.
- Record keys and variant tags: a field name, or a numeric id in decimal (`5`, or `1_000` as
  search prints it) or as `_5_`. Each field takes one key: `05` and `+5` are not ids, and two
  keys for one field are an error. Replies write a name that reads as a number (`"1"`) as
  `_<its id>_`, which is accepted back.
- Variants are `{"Tag": value}`; for a null payload, `"Tag"` alone also works.
- Opt is a value or `null`. `null` slots take only `null`.
- Principals are text. A func is `{"principal", "method"}` and nothing more.
- Blobs are byte arrays, or a hex string on input. Each byte counts toward the 200,000-element
  value limit, so a blob reply or byte-array argument over about 200 KB is refused (a reply
  after the call executed: fetch large blobs in chunks). For a large blob argument, pass a hex
  string, also as the single element of the argument list (`["abcd..."]`).
- nat/int in Candid replies come back as numbers when JS-safe (±2⁵³) and as decimal strings
  otherwise. On input, nat/int take integers, including exact 64-bit integers from Python or
  Rhai, and decimal strings (`_` only between digits, at most 1,000 digits). A float beyond
  2⁵³ (any JavaScript number that large) has already been rounded and is refused, so pass large
  values as decimal strings.
- Floats are rounded once to their width. Infinities and NaN travel as the strings `"inf"`,
  `"-inf"` and `"NaN"`, in replies and on input, so an overflow is an error, never a silent
  infinity.
- The tool result is text: a string result is sent as its raw text (so `"42"` and `42` look the
  same), any other value as JSON.

Script results and call arguments follow one rule in every language where JSON has none:
- In a result, Python and Rhai integers beyond 2⁵³ leave as decimal strings (as call arguments
  they are sent exact); JavaScript numbers are doubles and stay so.
- NaN and the infinities leave as `"NaN"`, `"inf"` and `"-inf"`. -0 keeps its sign.
- At most 128 levels of arrays and objects; scalars add no level.
- Errors and exceptions, caught host errors included, leave as their text, `Type: message`
  (JavaScript `Error: …`, Python e.g. `RuntimeError: …`; Rhai's caught values are already
  strings).
- Bytes, typed arrays, ArrayBuffers and DataViews leave as arrays of their bytes or elements.
  Lone surrogates in JavaScript strings and keys become U+FFFD, as Python's do.
- Functions are the one difference: JavaScript drops them, as `JSON.stringify` does (`null` in
  arrays, the key left out of objects); Python and Rhai refuse them (`a <type> value cannot
  leave the sandbox`). JavaScript BigInts are refused; convert them to strings.
- Python results and arguments must be str, int, float, bool, None, bytes, list, tuple, set or
  dict (with str, int or bool keys, bool as `"true"`/`"false"`, as `json.dumps` does). Decimal
  and date/time values leave as their repr; class instances, `range` and similar are errors.

## Guardrails

| Guardrail | Value |
|---|---|
| Allowlist | `Principal → ["method" \| "*"]`, deny by default. Checked before any message is sent, including interface fetches, and again right before each send. `"*"` leaves out delegating methods (see above). The management canister and this canister are always refused |
| Calls per script | 25 (`set_limits`). Counts every `icp.call` attempt that passes policy and every interface-fetch message (a cold fetch sends 2 or 3). Reserved before the await, so concurrent batches count |
| Instruction budget | 20B per script (at most 30B; `set_limits` clamps larger values, and stored values are clamped on read), measured from the script's own start. Checked before each host request and each send, every 64 JS VM steps and inside Boa's built-in loops, on every Rhai operation, and by the Python VM, which gets a fresh allowance from the live counter before every resume (each Python turn is also capped at 20B, so a turn plus one step stays under the IC's 40B). Spent before any message was sent: a `policy:` refusal (`PolicyViolation`); after: an `Error` that lists the messages sent |
| Run time | 600 s per run (scripts, and searches that fetch). Each message's timeout is cut to the time left; after that no new message is sent (`policy: the run reached its 600-second time limit`) |
| Rate | 120 update calls per minute per token; every authenticated update call counts, tool runs and (with `certified_reads`) `initialize`, `tools/list` and `ping` alike (one comparison on the demo page uses up to 17) (`tool_calls_per_minute`). Over the limit, requests are refused before they run: by the query (HTTP 429), by `inspect_message`, and by the update itself |
| Requests | At most 50 messages per JSON-RPC batch. One `tools/call` per HTTP request, on both routes (further ones get `-32001`, the code for every server limit: body size, batch size, one tool call). At most 8 runs (scripts, and searches that must fetch an interface) at once across all requests, and at most 4 per token; more get a "busy" error, as does any new run while the heap is over half its 1 GiB ceiling. JavaScript and Python: at most 64 concurrent requests per batch (`Promise.all` / `asyncio.gather`), and JavaScript at most 64 rounds. Rhai: at most 64 requests per script |
| Messages in flight | At most 64 outbound messages await replies at once, across all runs and searches, and at most 32 per token (its share, in the same proportion as its 4 of the 8 run slots). Past these, or while the canister holds under 100B liquid cycles, no message is sent (a `policy:` error), and under 100B no new run starts (`canister is low on cycles`). A run whose token is revoked, or whose target leaves the allowlist, sends nothing more after its current await; a stop is honoured before every new message |
| Values | Any value leaving a script (result and call arguments in all three languages; values printed with `console.log` in JavaScript, while Python and Rhai print text capped at 64 KiB), and every call reply entering one, is limited to 128 levels, 200k elements and 4 MiB of text, so every language accepts the same replies. A reply is checked after the call executed, and an over-limit reply's error says so. A reply may also hold at most 16,384 keys in total across objects of more than 64 keys |
| JS (Boa, patched) | 30M VM steps per run (across all functions, callbacks and promise jobs; there is no separate per-loop limit), recursion 256, 100k promise jobs, and a 96 MiB heap cap per run (across awaits). The instruction budget and the heap cap are checked every 64 VM steps, every 4,096 property accesses or calls into script code, and every 4,096 element moves while sorting, with an uncatchable `policy:` error. Source nested deeper than 128 levels is refused before parsing; brackets, chained operators, and statements that nest without brackets (`if`/`for`/`while`/`with`/`do`/`extends`, labels, `else if` chains) all count. Built-ins on deeply nested data throw `RangeError: native stack limit exceeded` (or, inside a built-in that Boa assumes cannot fail, end the run with `EnginePanic: … (RangeError: native stack limit exceeded)`), and Proxy target chains are limited to 1,000. ArrayBuffers over 96 MiB are a `RangeError`; pre-sizing from an array-like's script-chosen length is capped, so `{length: 2**45}` costs only what is actually read. `Math.random`, `eval`/`Function`, `Atomics`, `SharedArrayBuffer`, `WeakRef` and `Symbol.for`/`keyFor` removed; dynamic `import()` rejects. `icp.call`/`icp.candid`/`icp.principal` refuse extra arguments. A request started and never awaited is an error, not silently dropped. Fixed clock = block time, UTC. `JSON.stringify` is held to the value limits too, replacer included (an array replacer reads only the listed keys). Thrown values are cut to 8 KiB of text |
| Python (Monty, patched) | Recursion 200, 64 MiB memory (kept replies count too), 256 suspensions. The VM stops when the policy's instruction budget is spent (see above). Results of integer arithmetic (and `math.gcd`/`lcm`/`isqrt`/`pow` operands) are capped at 32,768 bits; ints from other sources (`int(Decimal)`, long hex literals) can be larger, but the costly operations on them are refused. `decimal` precision is capped at 10,000 and one operation at 65,536 digits. A regex with lookaround or backreferences takes at most 5,000 bytes of text and 10,000 backtracks per match, and polls the time limit before every search and every match. `eval` and `exec` are refused by name, and so is `typing.ForwardRef(...).evaluate()`. String annotations are still evaluated (for example by `typing.get_type_hints`), so text built at run time can still be compiled that way, at most 16 KiB per snippet, with the time limit polled. `print` past 64 KiB is cut with `…[truncated]`, as in the other languages, and later prints are not formatted. Raised error text is cut to 8 KiB. Fixed `datetime` (block time to the millisecond), zero sleep, seeded `random` (the same sequence every run), no OS access |
| Rhai | 1M operations per run, 32 call levels, 128 MiB memory per run (map keys, values captured by closures and cached replies count), expressions nested at most 128 levels (64 inside a function; each operator in `a + b + c` is a level, so split very long expressions into `let` bindings), and size limits on strings/arrays/maps equal to the value limits (`replace` and array `pad` check the size before building). `eval`, `curry`, `sleep` and modules disabled. The sorts and `dedup` are replaced by versions that pass comparer errors through and cannot panic |
| Interfaces | ≤ 512 KiB, type nesting ≤ 256 levels per definition and ≤ 2,000 summed over all definitions (every definition counts at least 1), ≤ 1,024 fields per record, variant or service, alias chains (`type A = B;`) ≤ 64, record field ids below 2³²−1, no `import`. All checked before parsing (deeper nesting overflowed the stack). Replies: up to the IC's 2 MiB, at most 1,024 levels of Candid nesting while decoding (the vendored candid's bound), then the value limits above. A reply whose types differ from the cached interface in a way Candid would silently turn into `null` is an error naming the mismatch, and the cached interface is dropped. A reply error says the call already executed on the target |
| Heap | 1 GiB for the whole canister. JavaScript's 96 MiB, Python's 64 MiB and Rhai's 128 MiB per-run limits are checked between steps (eight Rhai runs at their cap can reach the ceiling), so this ceiling bounds what one built-in call can allocate; past it the message traps and the growth is rolled back |
| Output | Script source ≤ 16 KiB (compiling is superlinear in size and cannot be interrupted); request body ≤ 128 KiB. Captured logs ≤ 64 KiB, measured as JSON-escaped text with each line also counting its newline (`\n`, 2 bytes), enforced while running; whenever output is dropped, the last kept line ends with `…[truncated]`. Result ≤ 1 MB of *escaped* JSON; a bigger one is an error (recorded as `Error`), never cut short. Any reply that would exceed 1.9 MB, whatever it contains, is replaced by a small error, so replying can never trap |
| Audit | `StableLog`, appended to only by the canister code: time, token-hash prefix, language, script SHA-256, canisters, methods, calls, instructions, status, violations (≤ 16, each ≤ 512 B: the first 8 and the latest 8, then "…and N more"), error (≤ 4 KiB). One entry per outbound message, written as the system accepts it, plus one closing entry per run (`Trapped` if the run was cut short). Each `Started`/`Call` entry for a script call also records its Candid-encoded argument in `args` (hex, which `didc decode` reads; past 1 KiB, a SHA-256 plus the first 512 bytes). A search is logged with language `search` and script SHA-256 = sha256(`search:<query>:<canister id>`), with the query trimmed and the id in canonical text. Each upgrade writes one entry: language `upgrade`, caller `controller`, status `Ok`. Pages are at most about 1.5 MB; long lists are cut on read (128 items, 2 KiB each, plus "…and N more"), and an entry is never left out of a page |
| Unpaid-call filter | `inspect_message` admits an update call only if it carries a registered token within its rate limit, is well-formed and contains a `tools/call` that parses (with `certified_reads`, any request), or is an admin call from a controller. Other ingress is rejected before the canister is charged for it. `inspect_message` sees ingress only: a call from another canister skips it, so the canister pays to refuse it (the base update fee plus decoding, about 5-6M cycles for a call without a token). The update path itself refuses a request with no `tools/call` (400, `-32600`) unless `certified_reads` is on |
| Browsers | A request with an `Origin` header must come from the canister's own origin (MCP's DNS-rebinding rule); others get 403. Non-browser clients send no `Origin` |
| Stack | 8 MiB wasm stack (`.cargo/config.toml`), up from Rust's 1 MiB default |

Policy, tokens and the audit log live in `ic-stable-structures`, so they survive upgrades
without serialization. `pre_upgrade` refuses an upgrade while runs are waiting on replies
(`pending_run_ids` is not empty) unless the canister is stopped, so stop it first
(`install_code`'s `skip_pre_upgrade` bypasses this check). `post_upgrade` accepts the same
optional `InitArgs` type as `init` and ignores it; change tokens, allowlist and limits with the
admin methods.

### How to read the audit log

Every entry of one run carries the same `run_id` (the id of the run's first entry);
`get_audit_run` reads one run. The script hash can't link entries: the same code can run
several times, and runs interleave across awaits.

**Entries for messages.** Each outbound message gets its own entry, naming the target and
method in `methods`: `Started` for the run's first message, `Call` for each later one. This
includes interface fetches: `aaaaa-aa.canister_info(<id>)`, then
`aaaaa-aa.canister_metadata(<id>)` and, if needed, `<id>.__get_candid_interface_tmp_hack`.

- The entry is written at the moment the system accepts the message. If the system refuses
  it on the spot (no cycles, queue full), or the arguments fail to encode, nothing was sent
  and there is no entry.
- The entry and the message are committed together when that message execution ends. If
  the execution traps first, the IC discards both, so the log never shows a message that was
  not sent, and never misses one that was. If stable memory cannot grow, the append traps, so
  the message is discarded too and nothing goes out unaudited.
- An entry means the system accepted the message. Whether the target executed it shows in the
  script's result or error (a reject, "outcome unknown" after a timeout, or a reply that
  arrived but could not be decoded or was over the limits, whose error says the call
  executed) only if the script got to use the reply. If the run failed, trapped or hit a limit
  after sending, the entry is the only record that the message went out; check its effect on
  the target itself.
- `violations` and the labels in `methods` quote script- or target-authored text, escaped and
  in quotes. `error` holds a script's own error or a target's reject, with control and bidi
  characters escaped (quotes are kept), so it stays on one line. A script can still write any
  one-line text there, so judge a run by `status` and `violations`, not by what `error` says.
  `status`, `calls`, `canisters` and `run_id` are structural.

**Closing entries.** A script run ends with exactly one of these:

| Closing entry | Meaning |
|---|---|
| `Ok` | The script returned a result |
| `Error` | The script failed; a search could not fetch an interface or returned a failure remembered from a recent fetch; or the instruction budget ran out after messages had been sent (the error lists them) |
| `PolicyViolation` | A call was refused by policy, or the instruction budget ran out before any message was sent |
| `Trapped` | The canister trapped while handling a reply, after messages had gone out. The run's own closing entry was rolled back with the trap, so this one is written during cleanup. It lists the messages sent and the violations recorded up to the last message. Its `instructions` leaves out the trapped execution (that work was rolled back with the trap), so it is a lower bound |

`calls` counts messages sent, and `methods` lists them in order.

Two kinds of request write no entries, because they send nothing: a search answered entirely
from cached interfaces, and a run (script or fetching search) refused as "busy" or for low
cycles.

**A run with no closing entry** is one of three things:

- **Pending:** it is still waiting on a reply. `get_policy` lists these runs in
  `pending_run_ids`. Every message is a bounded wait (at most 300 s) and a run lasts at most
  600 s, so no run stays pending longer than that. A run that trapped stays listed for 6
  minutes after its `Trapped` entry, while calls it sent alongside the one that trapped may
  still be outstanding.
- **Unknown:** the closing `Trapped` entry could not be written (stable memory full;
  `get_policy().lost_audit_entries` counts these since the last upgrade, and the run's slot and
  pending id are still released), or the canister was upgraded with `skip_pre_upgrade` while
  the run was waiting. The pending list is kept in memory and is empty after an upgrade.
- **Not in the log at all:** a run that traps before sending any message leaves no entries,
  because the IC rolls back that whole request. It changed nothing and returned nothing.

So a missing closing entry is "pending or unknown", never proof of a trap. A trap is
recorded as `Trapped`.

**Options** (set with `set_limits`):

- `call_timeout_seconds`: how long a message waits for its reply, 1-300 s (other values are
  clamped); `null` keeps the current value, 300 by default. Every message, interface fetches
  included, has this timeout, cut to what is left of the run's 600-second limit. A stop is
  honoured before every new message, so it waits for at most one outstanding timeout. A
  timed-out call may still have executed on the target; the error says so.
- `certified_reads`: by default `initialize`, `tools/list` and cached searches are answered by
  query calls. Those are fast and free but not certified, so a single malicious node could
  alter them (the allowlist is still enforced on-chain when a script runs). With
  `certified_reads = opt true` every authenticated `/mcp` request goes through an update call
  (about 0.2 s locally, 1-2 s on mainnet) and the demo page, which is uncertified HTML that
  asks for the token, is switched off. Read the audit log with an update call for the same
  reason.
- `tool_calls_per_minute`: update calls each token may make per minute; `null` keeps the
  current value, 120 by default.

Changes apply to runs that start afterwards; a running script keeps the limits it started with.

Fetched interfaces are cached for one hour, then fetched again, so a target's interface
change is picked up without upgrading this canister. Calls in one run to a canister whose
interface is not cached (`Promise.all`/`gather`) share one fetch and its outcome, failures
included. A failed fetch is remembered for five minutes when the target's answer is
definitive (no such method or canister, a reject, an invalid or oversized interface, or a
reply whose handling trapped), so searches and scripts don't repeat the same calls. A refusal
here, a stopped or out-of-cycles target, a transient reject or a timeout is not remembered,
and an expired interface is served instead if one is cached. The error names both attempts
(`canister_metadata: …; __get_candid_interface_tmp_hack: …`). `set_allowlist` and
`remove_allowlist` drop both. Interfaces over 512 KiB are refused.

Canister ids are canonicalised (`RYJL3-…` and `ryjl3-…` are the same principal) before the
allowlist check, the interface cache and the override lookup.

## Operations

- `get_metrics` (controller query): cycle balance, Wasm memory, live heap, stable memory,
  audit log length, runs in flight and pending runs. Like every query, it is refused while the
  canister is frozen or stopping; use `icp canister status` then.
- Cycles: top up with `icp canister top-up`. `icp.yaml` pins `freezing_threshold` at 30 days
  (the IC default). Below 0.1T liquid cycles no new run starts and no message is sent.
- Memory: Wasm memory grows and never shrinks. `icp.yaml` sets `wasm_memory_threshold` to
  1 GiB, so once less than that is left under the 3 GiB `wasm_memory_limit`, the
  `on_low_wasm_memory` hook frees the shared JavaScript heap and writes a line to the canister
  log (`icp canister logs`). `log_memory_limit` is 2 MiB, so trap and low-memory lines are
  kept.
- Traps: a trap after a run sent a message is a `Trapped` audit entry. A trap before any
  message is rolled back and appears only in `icp canister logs`.
- Stuck in Stopping: every message times out within 300 s and a run lasts at most 600 s, so a
  stop completes. If it gives up, `icp canister start` brings the canister back.
- Upgrades: stop the canister first, since `pre_upgrade` refuses while runs are pending. Take a
  snapshot (`icp canister snapshot create`) while it is stopped.

## Known limitations

- **JavaScript's heap cap is checked between engine steps.** Each run may hold 96 MiB, checked
  every 64 VM steps and every 4,096 property accesses, across awaits. One built-in call that
  allocates a lot at once (for example one huge string concatenation) can still reach the
  canister-wide 1 GiB heap ceiling, which traps (a 503 for the client) and rolls the growth
  back. `padStart`, `padEnd` and `repeat` refuse results over 16M characters with a
  `RangeError`. The ceiling assumes `wasm_memory_limit` stays well above 1 GiB (icp.yaml sets
  3 GiB). While the heap is over half the ceiling, new runs are refused as busy.
- **The JavaScript nesting check is a lexical scan.** It refuses deep source before Boa's
  parser recurses on it, and it guesses whether `/` right after `)` or `}` starts a regex or
  divides. Source crafted around that guess (for example a regex containing a quote right
  after a block) can hide nesting from the scan. If the parser then runs out of stack, the
  canister traps (a 503).
- **One native operation can still hit the IC's 40B-instruction trap.** The engines check
  budgets between steps (JavaScript also inside built-in loops over objects), not inside one
  built-in call: in JavaScript a regular expression with heavy backtracking, in Python a linear
  operation on a large value (for example `unicodedata.normalize` on megabytes of text; the
  largest such step measured cost about 13B instructions, which the 20B turn cap leaves room for). Python's superlinear operations are capped in
  the patched Monty: big integers, `decimal`, backtracking regexes, colliding hash keys and
  run-time compiled text. The script size limit, value limits, and caps on integer text (1,000
  digits in, 32,768 bits out of Python, 100,000 bits in a reply) bound the rest. Such a trap is
  rolled back: nothing changed. If it happens before any outbound message, nothing is logged
  and the client gets a 503. If it happens after one, the entries for every message already
  sent remain and the run is closed with a `Trapped` entry, whose `instructions` leaves out
  the trapped execution. The per-token rate limit bounds how often anyone can cause this.
- **Some Python built-ins end in a trap, not a Python error.** `json.dumps` with a huge
  `indent`, or `re.compile` of a multi-megabyte pattern, allocates outside Monty's 64 MiB
  tracker. The 1 GiB heap ceiling stops it with a trap (HTTP 503), which rolls the growth back,
  but the script gets no `ValueError` or `PatternError`.
- **Rhai does not bound how deeply values nest.** An array or map nested inside itself tens of
  thousands of times (`x = [x]` in a loop), or a chain of closures each capturing the previous
  one, can exhaust the stack. The message then traps, which rolls it back. Rhai 1.27 offers no
  hook to limit this without patching the crate. Values crossing to the host are still limited
  to 128 levels.
- **Shared engine state is charged to whoever triggers it.** Boa's garbage collector is
  process-wide. A run collects at its end only when no other JavaScript run is live, but an
  automatic collection during a run (or one forced by the heap cap) still sweeps other runs'
  garbage and is billed to that run.
- **A run that traps keeps its run slot, and stays in `pending_run_ids`, for 6 minutes after
  the trap.** Calls it sent alongside the one that trapped may still be outstanding, and each
  ends within its timeout (at most 300 s). The slot is held even when nothing else was
  outstanding, so a token whose scripts keep trapping can tie up its own 4 slots for that long.
- **A trap while handling an interface fetch's reply marks that interface failed for 5
  minutes, for every token.** Only a trap in the callback for one of that fetch's own replies
  counts (for example decoding or parsing a hostile interface ran out of instructions), but a
  trap from other work in the same callback (the script continuing after that reply) also
  counts. A controller can set an override, or wait out the 5 minutes.
- **Concurrent cold fetches are merged within a run, not across requests.** Separate HTTP
  requests each fetch on their own: ic-cdk cannot resume one update call from another's reply.
- **A hostile allowlisted target can make replies expensive to decode.** Candid's decoder has
  work quotas, but some shapes still cost far more than their size. Allowlist only canisters
  you trust to answer sanely.
- **A target upgrade can turn changed `opt` arguments into null.** Interfaces are cached by
  age, not by the target's module hash. For up to one hour after a target is upgraded
  (indefinitely while an override is set), an argument whose `opt` type changed reaches the
  target as `null`, because Candid's opt rule applies on the target's side. Replies are
  checked against the types actually sent, so a stale interface is caught there and dropped.
  Following Candid's upgrade rules, reply fields and extra return values newer than the cached
  interface are dropped silently. Re-register an override after upgrading its target.
- **Blobs over about 200 KB cannot pass through a script.** A decoded blob is one array element
  per byte, so a larger reply is refused after the call executed, in every language (Rhai's
  arrays are capped at the same 200,000 elements). Fetch large blobs in chunks; send large
  blob arguments as hex strings.
- **Bearer tokens are visible to the subnet's node operators and the HTTP gateway**, because
  request contents are replicated to the nodes that run the canister. That is how HTTP
  requests work on ICP. Treat tokens as revocable credentials: one per client, rotated, and
  revoked with `revoke_api_token_hash` when in doubt.
- **All tokens act as one on-chain identity:** calls go out as this canister's principal.
  Tokens are told apart only in the audit log.
- **No idempotency.** A script keeps running, and its calls take effect, even if the client
  times out or cancels (`notifications/cancelled` is ignored). Calling again runs everything
  again. Before retrying a script that writes, read the target's state; for ICRC-1/2 transfers
  set `created_at_time` (and a memo) so the ledger rejects a duplicate. Run outcomes are visible
  only to controllers (`get_audit_logs`).
- **Target text reaches the model verbatim.** Interface text (including `.did` comments
  returned by `icp.candid`), method, type and field names, reject messages and call results
  come from the target canisters. The tool descriptions tell the model they are untrusted
  data; calls stay bounded by the allowlist.
- **Rhai replays from the top** for each host request. That costs O(requests²) operations,
  bounded by the 64-request cap and the instruction budget.
- **`opt opt T`:** `?null` and `null` both decode to JSON `null`, and you can't send `?null`.
- **The interface cache lives on the heap**, so it is also re-fetched after an upgrade.
  `canister_metadata` is tried first (after `canister_info`); `__get_candid_interface_tmp_hack`
  is the fallback when it fails or is skipped.
- **Adding the `Started`, `Call` and `Trapped` audit statuses were Candid-breaking changes** for
  `get_audit_logs` clients compiled against an older `.did`. Upgrading needed `icp deploy --yes`.
- **`AuditEntry.args`, `PolicyView.lost_audit_entries`, `get_audit_logs_before`,
  `get_audit_run` and `get_metrics` are new.** They are additive (an `opt` field, a record
  field in a reply, new methods). A client with an older `.did` that decodes `PolicyView`
  strictly should regenerate its bindings.
- **The audit log is never pruned.** The rate limit bounds how fast it grows: one token's runs
  can write at most 120 × (calls per script + 2) entries a minute. There is no
  export-and-prune endpoint. Each new 8 MiB of stable memory raises the freezing threshold, so
  watch the cycles balance and `get_audit_log_len`.
- **Controllers can rewind the audit log and token set:** loading a canister snapshot or
  reinstalling restores the earlier log, and new entries then reuse its ids. Export
  `get_audit_logs` before loading a snapshot, and re-apply token revocations after.
- **Take and load canister snapshots only while the canister is stopped.** Run slots and the
  pending list are kept on the heap. A snapshot taken with runs in flight restores them, and
  they are freed only once stale (16 minutes after the run started). Until then the canister
  may answer "busy", and it refuses upgrades unless stopped. Loading a snapshot while calls
  are open makes their replies trap.
- **A busy canister is found out in the update call.** The query sees the number of running
  scripts as of the last committed state, which can change before the update runs, so the
  check stays in the update. A request that gets a "busy" error has still cost one update
  call.
- **The shipped Wasm has not been run under CI yet.** The PocketIC harness and the CI workflow
  exist but have not run remotely, so wasm32-only traps are still checked by hand. Release
  hashes are not published: icp.yaml remaps the cargo home and checkout paths
  (`--remap-path-prefix`), so builds of the same commit with ic-wasm 0.11.1 should match, but
  this has not been checked across machines.
- **Two unmaintained crates ship in the Wasm.** `smartstring` 1.0.1 (RUSTSEC-2026-0249) is a
  non-optional dependency of rhai 1.27, and `chrono` (RUSTSEC-2026-0543) comes in through monty
  and is vendored as `vendor/chrono`. Both are unmaintained notices, not vulnerabilities, and
  are accepted in `deny.toml`. cargo-deny only scans registry crates, so re-check
  `vendor/chrono` against the advisory database by hand whenever monty is bumped.

## Where this differs from the original spec, and why

- **icp-cli instead of dfx.** `icp.yaml` uses explicit script build steps instead of the
  `@dfinity/rust` recipe, so Cargo features can be passed.
- **getrandom uses the `custom` backend, not `wasm_js`.** `wasm_js` links wasm-bindgen
  imports, and the IC refuses to install a module that imports anything outside `ic0`.
  The custom backend is a fixed deterministic stream: every replica must compute the same
  state. It is used only for hash seeding and Monty's internal ids, never for secrets. The
  stream is public, so hash seeds are predictable: input caps, not the seed, bound
  hash-collision work.
- **Why the patches.** Two keep wasm-bindgen imports out of the Wasm: monty-types reads
  `web_time::Instant` on every execution window, and chrono's default `wasmbind` feature pulls
  in `js_sys::Date`. Three bound work the engines would otherwise run until the IC traps the
  message (instruction limit, heap ceiling, stack overflow, or a panic under
  `panic = "abort"`), which discards the audit entry. Those three (Boa, Monty, candid) each
  list their changes in a `PATCHES.md`, and each change is marked `icp-code-mode patch` in the
  source; chrono's one change is described at the top of its `Cargo.toml`.
  - `shims/web-time` backs `Instant` with the IC instruction counter. This also turns Monty's
    duration limits into deterministic instruction budgets.
  - `vendor/chrono` is chrono 0.4.45 with `wasmbind` removed from its default features.
  - `vendor/boa_engine` is boa_engine 0.22.0 with an interrupt the host polls for its
    instruction deadline and heap cap, a native stack floor on property access and calls, a
    merge sort that tolerates inconsistent comparators, capped pre-sizing from array-like
    lengths, and a 1,000-deep Proxy chain limit.
  - `vendor/monty` is monty 1.6.0 with caps on its uninterruptible native work (big integers,
    `decimal`, regex, hash probes, run-time compiled text), time polls before expensive
    operations, the VM's resource tracker reachable at every kind of suspension, and a probe
    that lets `print` stop formatting once output is full.
  - `vendor/candid` is candid 0.10.40 with a fixed recursion bound (1,024) on every target,
    because upstream's guard does nothing on wasm and a 20,000-level reply overflowed the stack
    inside the decoder, and with the decoding quota charged for the subtype memo copied per
    `opt` value.
  - The release Wasm imports only `ic0.*`; CI checks this for every feature set with
    `scripts/check-wasm-imports.sh <wasm>` (needs `wasm-objdump` from wabt).
- **In Rhai, the call function is `icp::call_canister`**, because `call` is a reserved Rhai keyword.
  Rhai natives are synchronous, so the runtime uses deterministic replay. When a script needs
  an answer it does not have, the run is stopped (uncatchably) and the call is made. The answer
  is cached, and the script re-runs from the top. Cost grows quadratically with the number of
  calls, bounded by the call cap.
- **Interface discovery:** an admin-supplied `set_candid_override` wins when set (nothing is
  fetched or refreshed for that canister, and `remove_allowlist` keeps it). Scripts can read it
  with `icp.candid` only while the canister is allowlisted. An override over 512 KiB (stored
  before that limit existed) is ignored; `get_policy` still lists it with its size so it can be
  cleared. Otherwise:
  1. the management canister's `canister_info`, then `canister_metadata` (the `candid:service`
     section). `canister_metadata` also returns *private* sections to a controller, so it is
     skipped when this canister controls the target (or `canister_info` fails); a cold fetch
     costs one more message for this,
  2. the target's `__get_candid_interface_tmp_hack`.

  A failure is remembered for 5 minutes only when every step failed for a lasting reason (no
  such method or section, no such canister). If `canister_info` failed, `canister_metadata` is
  not tried, and the failure is not remembered.
- **Skip-certification.** With `certified_reads` off, query responses (`initialize`,
  `tools/list`, cached searches) carry skip-certification headers, so they work on the certified
  gateway domain, not just `raw`. With it on, the canister certifies nothing, so the gateway
  rejects any query response except an upgrade, and a single node cannot answer in the
  canister's name. Script results always come from the update call, which consensus certifies.
