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
  that must first fetch an interface runs as an update call: audited, and limited like a
  script's calls.
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
- **Deny-by-default safety & auditability:** Controllers restrict access per `(Principal, method)`, cap calls/instructions, and inspect an append-only on-chain `StableLog`.
- **No local MCP runtime:** Any HTTP-capable MCP client (Claude, Gemini, etc.) connects directly to the canister URL with a Bearer token.

## Comparison with Cloudflare Code Mode

Cloudflare's Code Mode introduced the pattern of letting an LLM write code against a typed SDK inside a sandbox rather than calling many fine-grained MCP tools. **ICP Code Mode** brings that same search-and-execute pattern natively onto the Internet Computer, with several advantages for ICP workflows:

- **On-chain, consensus-certified execution:** Scripts run inside the canister's deterministic Wasm sandbox (`http_request_update`) and make native inter-canister calls, rather than running in an off-chain isolate talking to external RPCs.
- **Native Candid bridge:** Built-in `.did` parsing and type-directed JSON ⇄ Candid conversion—no off-chain IDL tooling or generated client bindings required.
- **Multi-language runtime:** Supports **JavaScript** (Boa), **Python** (Monty), and **Rhai** in one canister (or per-language feature builds).
- **On-chain policy & immutable audit log:** Allowlists, SHA-256 token hashes, instruction budgets, and `StableLog` audit entries live in canister stable memory and survive upgrades.
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
4. **Connect your MCP client** ([client config](#mcp-client-config)) or **test with `curl`** ([curl examples](#curl-examples-local)) against `http://icp_code_mode.local.localhost:8010/mcp` (or `https://<canister-id>.icp0.io/mcp` on mainnet).

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
                                              └─▶ ic_cdk::call::Call::unbounded_wait ─▶ target canister
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
| `scripts/demo-setup.sh` | Allowlists `demo_store` in `icp_code_mode` |
| `video/` | Explainer video: script, HTML composition, Gemini TTS and render pipeline, final MP4 |
| `src/search.rs` | `search_icp_canisters`: allowlist × interface → filtered signatures and referenced type definitions |
| `src/policy.rs` | Stable-memory guardrails: allowlist, limits, SHA-256 token hashes, append-only audit log |
| `src/runtimes/` | `LanguageRouter` + `js.rs` (Boa), `python.rs` (Monty), `rhai_rt.rs` (Rhai) |
| `icp_code_mode.did` | Canister interface. A test checks that it matches `export_candid!` |
| `icp.yaml` | icp-cli project (build steps, local network on port 8010) |
| `shims/web-time`, `vendor/chrono` | `[patch]`es that keep wasm-bindgen imports out of the Wasm (see "Why the patches") |

## Build and test

Requirements:
- Rust 1.100, pinned in `rust-toolchain.toml` (monty 1.6 needs ≥ 1.98)
- the `wasm32-unknown-unknown` target
- [icp-cli](https://cli.internetcomputer.org) ≥ 1.7
- `ic-wasm`

```bash
cargo test
```

```bash
cargo build --target wasm32-unknown-unknown --release --locked
```

Engines are Cargo features (`default = ["js", "python", "rhai"]`). To build a dedicated
per-language worker, for example a Rhai-only one:

```bash
cargo build --target wasm32-unknown-unknown --release --no-default-features --features rhai --target-dir target/rhai
```

To deploy a worker, copy the canister entry in `icp.yaml` under a new name and use that build command.
The all-in-one build is about 12 MB (3.6 MB gzipped). JS + Rhai without Python is about 6 MB.

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

The gateway URL is printed on deploy: `http://icp_code_mode.local.localhost:8010/`.
The network uses port 8010 (set in `icp.yaml`) so it can run next to other projects on the default 8000.

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

Use `vec {"*"}` to allow every method of a canister. The other admin endpoints are controller-only:
- `set_limits(record { max_calls_per_script; max_instructions; call_timeout_seconds; certified_reads; tool_calls_per_minute })`
  replaces all of them (an omitted optional field means its default) and returns the previous
  values; see "Options" below
- `remove_allowlist`
- `revoke_api_token_hash`
- `set_candid_override(principal, opt text)`
- `get_policy`, `get_audit_logs(offset, limit)`, `get_audit_log_len`

## Demo store and side-by-side comparison

`demo_store/` is a second canister: a small shop with 10 methods (7 reads, 3 writes) and
deterministic seed data (12 products, 8 customers, 60 orders). It exposes its interface
through `__get_candid_interface_tmp_hack`. Deploying it and allowlisting its reads plus
`create_order` takes two commands; `cancel_order` and `restock` stay blocked, so the
guardrail is visible:

```bash
icp deploy
```

```bash
scripts/demo-setup.sh
```

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
- `capture-demo.mjs` takes screenshots of the live demo page.

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
export MCP=http://icp_code_mode.local.localhost:8010/mcp
```

**initialize** (answered by the query call, no consensus round):

```bash
curl -s $MCP -H "Authorization: Bearer $MCP_TOKEN" -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"curl","version":"1"}}}'
```

```json
{"id":1,"jsonrpc":"2.0","result":{"capabilities":{"tools":{}},"instructions":"…","protocolVersion":"2025-03-26","serverInfo":{"name":"icp-code-mode","version":"0.1.0"}}}
```

**notifications/initialized** returns `202` with an empty body:

```bash
curl -s -o /dev/null -w '%{http_code}\n' $MCP -H "Authorization: Bearer $MCP_TOKEN" -H 'Content-Type: application/json' -d '{"jsonrpc":"2.0","method":"notifications/initialized"}'
```

**tools/list**:

```bash
curl -s $MCP -H "Authorization: Bearer $MCP_TOKEN" -H 'Content-Type: application/json' -d '{"jsonrpc":"2.0","id":2,"method":"tools/list"}'
```

**tools/call, search.** This is answered by the query call once the target's interface is
cached; the first search after a deploy goes through an update call to fetch it:

```bash
curl -s $MCP -H "Authorization: Bearer $MCP_TOKEN" -H 'Content-Type: application/json' -d '{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"search_icp_canisters","arguments":{"query":"balance"}}}'
```

```json
{"id":7,"jsonrpc":"2.0","result":{"content":[{"type":"text","text":"{\"canisters\":[{\"allowed_methods\":[\"icrc1_balance_of\",\"icrc1_symbol\",\"icrc1_decimals\",\"icrc1_metadata\",\"icrc1_total_supply\"],\"canister_id\":\"ryjl3-tyaaa-aaaaa-aaaba-cai\",\"methods\":[{\"name\":\"icrc1_balance_of\",\"signature\":\"(Account) -> (Icrc1Tokens) query\"}],\"types\":{\"Account\":\"record { owner : principal; subaccount : opt SubAccount }\",\"Icrc1Tokens\":\"nat\",\"SubAccount\":\"blob\"}}]}"}],"isError":false}}
```

Arguments are optional:
- `query`: space-separated keywords, all of which must appear in a method's name or signature.
- `canister_id`: restrict the search to one canister.

With no arguments, it lists everything that's allowed, up to 40 methods.

**tools/call, JavaScript.** The gateway upgrades the request to an update call. The `Promise.all` sends four calls concurrently:

```bash
curl -s $MCP -H "Authorization: Bearer $MCP_TOKEN" -H 'Content-Type: application/json' -d @- <<'EOF'
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"execute_icp_script","arguments":{"language":"javascript","code":"const L = 'ryjl3-tyaaa-aaaaa-aaaba-cai';\nconst did = await icp.candid(L);\nconst [sym, dec, supply, bal] = await Promise.all([\n  icp.call(L, 'icrc1_symbol', null), icp.call(L, 'icrc1_decimals', null),\n  icp.call(L, 'icrc1_total_supply', null),\n  icp.call(L, 'icrc1_balance_of', {owner: icp.principal(), subaccount: null})]);\nconsole.log('did bytes', did.length);\nreturn {sym, dec, supply, bal, methods: (did.match(/icrc1_\\w+/g) || []).length};"}}}
EOF
```

```json
{"id":3,"jsonrpc":"2.0","result":{"content":[{"type":"text","text":"{\"bal\":0,\"dec\":8,\"methods\":14,\"supply\":\"299999999995070000\",\"sym\":\"ICP\"}"},{"type":"text","text":"[console output]\ndid bytes 17368"}],"isError":false}}
```

**tools/call, Python.** `icp.call` looks synchronous. `icp.call_async` together with `asyncio.gather` batches calls:

```bash
curl -s $MCP -H "Authorization: Bearer $MCP_TOKEN" -H 'Content-Type: application/json' -d @- <<'EOF'
{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"execute_icp_script","arguments":{"language":"python","code":"import asyncio\nL = 'ryjl3-tyaaa-aaaaa-aaaba-cai'\nsym, dec = await asyncio.gather(icp.call_async(L, 'icrc1_symbol'), icp.call_async(L, 'icrc1_decimals'))\nmeta = icp.call(L, 'icrc1_metadata')\nprint('metadata entries:', len(meta))\n{'sym': sym, 'dec': dec, 'keys': [m[0] for m in meta]}"}}}
EOF
```

```json
{"id":4,"jsonrpc":"2.0","result":{"content":[{"type":"text","text":"{\"dec\":8,\"keys\":[\"icrc1:decimals\",\"icrc1:name\",\"icrc1:symbol\",\"icrc1:fee\"],\"sym\":\"ICP\"}"},{"type":"text","text":"[console output]\nmetadata entries: 4"}],"isError":false}}
```

**tools/call, Rhai:**

```bash
curl -s $MCP -H "Authorization: Bearer $MCP_TOKEN" -H 'Content-Type: application/json' -d '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"execute_icp_script","arguments":{"language":"rhai","code":"let L = \"ryjl3-tyaaa-aaaaa-aaaba-cai\"; #{sym: icp::call_canister(L, \"icrc1_symbol\"), supply: icp::call_canister(L, \"icrc1_total_supply\")}"}}}'
```

A call blocked by policy comes back as a tool error (`isError: true`). No message is sent, and the violation is audited:

```bash
curl -s $MCP -H "Authorization: Bearer $MCP_TOKEN" -H 'Content-Type: application/json' -d '{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"execute_icp_script","arguments":{"code":"return await icp.call(\"ryjl3-tyaaa-aaaaa-aaaba-cai\", \"icrc1_transfer\", {})"}}}'
```

```bash
icp canister call icp_code_mode get_audit_logs '(0, 5)'
```

On mainnet, use `https://<canister-id>.icp0.io/mcp`.

### MCP client config

```json
{ "mcpServers": { "icp": { "type": "http", "url": "http://icp_code_mode.local.localhost:8010/mcp",
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
- An array: positional arguments.
- Anything else: the single argument, for one-argument methods.

How values are encoded:
- Records are objects.
- Variants are `{"Tag": value}`; for a null payload, `"Tag"` alone also works.
- Opt is a value or `null`.
- Principals are text.
- Blobs are byte arrays, or a hex string on input.
- nat/int come back as numbers when JS-safe (±2⁵³) and as decimal strings otherwise. Strings are accepted on input.

## Guardrails

| Guardrail | Value |
|---|---|
| Allowlist | `Principal → ["method" \| "*"]`, deny by default. Checked before any message is sent, including the interface fetch |
| Calls per script | 25 (`set_limits`). Counts every `icp.call` attempt that passes policy and every interface fetch that misses the cache. Reserved before the await, so concurrent batches count |
| Instruction budget | 20B per script (at most 30B; `set_limits` clamps larger values), measured from the script's own start. Checked before each host request, every 16 JS promise jobs, between JS rounds, and on every Rhai operation; the Python VM stops when it is spent |
| Rate | 120 tool calls per minute per token (one comparison on the demo page uses up to 17) (`tool_calls_per_minute`). Over the limit, requests are refused before they run: by the query (HTTP 429), by `inspect_message`, and by the update itself |
| Requests | At most 50 messages per JSON-RPC batch. One `tools/call` per HTTP request, on both routes (further ones get `-32600`). At most 8 scripts run at once across all requests; more get a "busy" error. JS: at most 64 concurrent requests per round and 64 rounds. Rhai: at most 64 requests |
| Values | Any value leaving a script (result, call arguments, printed values) is limited to 128 levels, 200k elements and 4 MiB of text, in all three languages |
| JS (Boa) | 30M VM steps per run (across all functions, callbacks and promise jobs), 1M loop iterations per loop, recursion 256, 100k promise jobs. `Math.random`, `eval`/`Function`, `Atomics`, `SharedArrayBuffer`, `WeakRef` and `Symbol.for`/`keyFor` removed. A request started and never awaited is an error, not silently dropped. Fixed clock = block time, UTC. `JSON.stringify` is held to the value limits too, replacer included |
| Python (Monty) | Recursion 200, 64 MiB memory (replies count too), 256 suspensions. The VM stops when the policy's instruction budget is spent, checked between steps and at every suspension. `eval` and `exec` refused. `print` past 64 KiB is cut, as in the other languages. Ints over 100,000 bits are not exported. Fixed `datetime`, zero sleep, seeded `random` (the same sequence every run), no OS access |
| Rhai | 1M operations per run, 32 call levels, size limits on strings/arrays/maps (`replace` checks the size before building). `eval`, `sleep` and modules disabled. The sorts and `dedup` are replaced by versions that pass comparer errors through and cannot panic |
| Interfaces | ≤ 512 KiB, type nesting ≤ 256 levels per definition and ≤ 2,000 summed over all definitions, checked before parsing (deeper nesting overflowed the stack). A reply whose types differ from the cached interface in a way Candid would silently turn into `null` is an error, and the cached interface is dropped |
| Heap | 1 GiB for the whole canister. Python's 64 MiB limit is checked between steps and JavaScript has none, so this ceiling bounds what one built-in call can allocate; past it the message traps and the growth is rolled back |
| Output | Script source ≤ 16 KiB (compiling is superlinear in size and cannot be interrupted); request body ≤ 128 KiB. Captured logs ≤ 64 KiB, enforced while running. Result ≤ 1 MB of *escaped* JSON; a bigger one is an error (recorded as `Error`), never cut short. Any reply that would exceed 1.9 MB, whatever it contains, is replaced by a small error, so replying can never trap |
| Audit | Append-only `StableLog`: time, token-hash prefix, language, script SHA-256, canisters, methods, calls, instructions, status, violations (≤ 16, each ≤ 512 B), error (≤ 4 KiB). One entry per outbound message, written as the system accepts it, plus one closing entry per run (`Trapped` if the run was cut short) |
| Unpaid-call filter | `inspect_message` admits an update call only if it carries a registered token within its rate limit, is well-formed and contains a `tools/call` (with `certified_reads`, any request), or is an admin call from a controller. Everything else is rejected before the canister is charged for it |
| Browsers | A request with an `Origin` header must come from the canister's own origin (MCP's DNS-rebinding rule); others get 403. Non-browser clients send no `Origin` |
| Stack | 8 MiB wasm stack (`.cargo/config.toml`), up from Rust's 1 MiB default |

Policy, tokens and the audit log live in `ic-stable-structures`, so they survive upgrades
without `pre_upgrade` code. `post_upgrade` accepts the same optional `InitArgs` as `init`.

### How to read the audit log

Every entry of one run carries the same `run_id` (the id of the run's first entry). The
script hash can't link entries: the same code can run several times, and runs interleave
across awaits.

**Entries for messages.** Each outbound message gets its own entry, naming the target and
method in `methods`: `Started` for the run's first message, `Call` for each later one. This
includes interface fetches, which go to the management canister and, if needed, the target.

- The entry is written at the moment the system accepts the message. If the system refuses
  it on the spot (no cycles, queue full), or the arguments fail to encode, nothing was sent
  and there is no entry.
- The entry and the message are committed together when that message execution ends. If
  the execution traps first, the IC discards both, so the log never shows a message that was
  not sent, and never misses one that was.
- An entry means the system accepted the message. Whether the target executed it shows in the
  script's result or error (a reject, or "outcome unknown" after a timeout) only if the script
  got to use the reply. If the run failed, trapped or hit a limit after sending, the entry is
  the only record that the message went out; check its effect on the target itself.

**Closing entries.** A script run ends with exactly one of these:

| Closing entry | Meaning |
|---|---|
| `Ok` | The script returned a result |
| `Error` | The script failed, or a search could not fetch an interface |
| `PolicyViolation` | A call was refused by policy, or the instruction budget ran out |
| `Trapped` | The canister trapped while handling a reply, after messages had gone out. The run's own closing entry was rolled back with the trap, so this one is written during cleanup. It lists the messages sent and the violations recorded up to the last message |

`calls` counts messages sent, and `methods` lists them in order.

Two kinds of request write no entries, because they send nothing: a search answered from
cached interfaces, and a script refused as "busy" when 8 are already running.

**A run with no closing entry** is one of three things:

- **Pending:** it is still waiting on a reply. `get_policy` lists these runs in
  `pending_run_ids`. With the default unbounded wait, a target that never answers leaves a
  run pending indefinitely; `call_timeout_seconds` prevents that.
- **Unknown:** cleanup after a trap did not complete (it can itself fail), or the canister
  was upgraded while the run was waiting. The pending list is kept in memory and is empty
  after an upgrade.
- **Not in the log at all:** a run that traps before sending any message leaves no entries,
  because the IC rolls back that whole request. It changed nothing and returned nothing.

So a missing closing entry is "pending or unknown", never proof of a trap. A trap is
recorded as `Trapped`.

**Options** (set with `set_limits`):

- `call_timeout_seconds`: how long a call waits for its reply, 1-300 s (other values are
  clamped); `null` means 300. Every call has a timeout, so a canister that never answers holds
  a run slot, and this canister's stop, for at most that long. A timed-out call may still have
  executed on the target; the error says so.
- `certified_reads`: by default `initialize`, `tools/list` and cached searches are answered by
  query calls. Those are fast and free but not certified, so a single malicious node could
  alter them (the allowlist is still enforced on-chain when a script runs). With
  `certified_reads = opt true` every request goes through an update call (about 0.2 s locally,
  1-2 s on mainnet) and the demo page, which is uncertified HTML that asks for the token, is
  switched off. Read the audit log with an update call for the same reason.
- `tool_calls_per_minute`: tool calls each token may make per minute; `null` means 120.

Fetched interfaces are cached for one hour, then fetched again, so a target's interface
change is picked up without upgrading this canister. A failed fetch is remembered for five
minutes, so searches and scripts don't repeat the same calls to a target that exposes no
interface; a fetch that never reached the target (refused by policy or on the spot by the
system) is not remembered. `set_allowlist` and `remove_allowlist` drop both. Interfaces over
512 KiB are refused.

Canister ids are canonicalised (`RYJL3-…` and `ryjl3-…` are the same principal) before the
allowlist check, the interface cache and the override lookup.

## Known limitations

- **JavaScript has no memory cap of its own.** Python is limited to 64 MiB and Rhai by its
  size limits, but Boa offers no memory limit. The canister-wide 1 GiB heap ceiling stops a
  script that hoards memory with a trap (a 503 for the client), which rolls back the growth.
  `padStart`, `padEnd` and `repeat` refuse results over 16M characters with a `RangeError`
  instead. Boa still pre-allocates for array-likes (`Array.prototype.sort.call({length: 2**32 - 1})`),
  which ends in the same trap.
- **One native operation can still hit the IC's 40B-instruction trap.** The engines check
  budgets between steps, not inside one built-in call: in JavaScript `new Array(2e8).fill(0)`,
  in Python big-integer arithmetic (`**`, `*`, `//`, `math.gcd`, `math.isqrt`), regular
  expressions with backtracking, `decimal` at very high precision, or a dict whose keys all
  collide. The script size limit, value limits, and caps on integer text (1,000 digits in,
  100,000 bits out) bound the common cases; the rest needs patched engines. Such a trap is
  rolled back: nothing changed. If it happens before any outbound message, nothing is logged
  and the client gets a 503. If it happens after one, the entries for every message already
  sent remain and the run is closed with a `Trapped` entry, whose `instructions` leaves out
  the trapped execution. The per-token rate limit bounds how often anyone can cause this.
- **Shared engine state is charged to whoever triggers it.** Boa's garbage collector is
  process-wide, so a run's `instructions` can include collecting other runs' garbage.
- **A trap in one reply of a batch ends the run's bookkeeping early.** Its run slot is freed and
  it leaves `pending_run_ids` while the batch's other calls may still be outstanding; their
  entries remain, and they finish on their own.
- **Concurrent cold fetches are not merged.** `Promise.all` over several calls to a canister
  whose interface is not cached yet sends one fetch per call, each counted against the call
  cap. Fetch once (`icp.candid`) before the batch, or run any search first.
- **A hostile allowlisted target can make replies expensive to decode.** Candid's decoder has
  work quotas, but some shapes (many `opt` values against a large interface) still cost far
  more than their size. Allowlist only canisters you trust to answer sanely.
- **Bearer tokens are visible to the subnet's node operators and the HTTP gateway**, because
  request contents are replicated to the nodes that run the canister. That is how HTTP
  requests work on ICP. Treat tokens as revocable credentials: one per client, rotated, and
  revoked with `revoke_api_token_hash` when in doubt.
- **All tokens act as one on-chain identity:** calls go out as this canister's principal.
  Tokens are told apart only in the audit log.
- **Rhai replays from the top** for each host request. That costs O(requests²) operations,
  bounded by the 64-request cap and the instruction budget.
- **`opt opt T`:** `?null` and `null` both decode to JSON `null`, and you can't send `?null`.
- **The interface cache lives on the heap**, so it is also re-fetched after an upgrade.
  `canister_metadata` is tried first, but the local pocket-ic replica does not implement it
  yet, so the code falls back to `__get_candid_interface_tmp_hack`.
- **Adding the `Started`, `Call` and `Trapped` audit statuses were Candid-breaking changes** for
  `get_audit_logs` clients compiled against an older `.did`. Upgrading needed `icp deploy --yes`.
- **The audit log is append-only and never pruned.** The rate limit bounds how fast it grows:
  one token's runs can write at most 120 × (calls per script + 2) entries a minute.
- **A busy canister is found out in the update call.** The query sees the number of running
  scripts as of the last committed state, which can change before the update runs, so the
  check stays in the update. A request that gets a "busy" error has still cost one update
  call.

## Where this differs from the original spec, and why

- **icp-cli instead of dfx.** `icp.yaml` uses explicit script build steps instead of the
  `@dfinity/rust` recipe, so Cargo features can be passed.
- **getrandom uses the `custom` backend, not `wasm_js`.** `wasm_js` links wasm-bindgen
  imports, and the IC refuses to install a module that imports anything outside `ic0`.
  The custom backend is a fixed deterministic stream: every replica must compute the same
  state, and it is only used for hash seeding.
- **Why the patches.** monty-types reads `web_time::Instant` on every execution window, and
  chrono's default `wasmbind` feature pulls in `js_sys::Date`. Both add wasm-bindgen imports.
  - `shims/web-time` backs `Instant` with the IC instruction counter. This also turns Monty's
    duration limits into deterministic instruction budgets.
  - `vendor/chrono` is chrono 0.4.45 with `wasmbind` removed from its default features.
  - The release Wasm imports only `ic0.*` (`wasm-objdump -x -j Import`).
- **In Rhai, the call function is `icp::call_canister`**, because `call` is a reserved Rhai keyword.
  Rhai natives are synchronous, so the runtime uses deterministic replay. When a script needs
  an answer it does not have, the run is stopped (uncatchably) and the call is made. The answer
  is cached, and the script re-runs from the top. Cost grows quadratically with the number of
  calls, bounded by the call cap.
- **Interface discovery:** an admin-supplied `set_candid_override` wins when set (nothing is
  fetched or refreshed for that canister, and `remove_allowlist` keeps it). Otherwise:
  1. the management canister's `canister_metadata` (the public `candid:service` section; not available on every replica yet),
  2. the target's `__get_candid_interface_tmp_hack`.
- **Skip-certification.** With `certified_reads` off, query responses (`initialize`,
  `tools/list`, cached searches) carry skip-certification headers, so they work on the certified
  gateway domain, not just `raw`. With it on, the canister certifies nothing, so the gateway
  rejects any query response except an upgrade, and a single node cannot answer in the
  canister's name. Script results always come from the update call, which consensus certifies.
