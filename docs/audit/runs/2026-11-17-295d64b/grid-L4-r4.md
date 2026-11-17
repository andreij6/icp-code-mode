# Coverage grid: L4 Multiplication, round 4 (canister callers: the ingress size bound x5)

Run `2026-11-17-295d64b`. Lens question: what happens when this is done N times at once, or N
times in one request? This round asks it of the request size itself. The inventory bounds U1-U5,
W1 and W3 by the 2 MiB ingress limit. A same-subnet canister that holds a token is not bound by
that limit: it is bound by the inter-canister payload limit. `file:line` refers to commit `295d64b`.

Evidence keys: **R** reproduced on the project's local icp-cli network (port 8010), **C** the code
path produces it unambiguously.

## Setup (Live phase)

- Fixture: a purpose-built caller canister (not in the repository; `ic-cdk 0.20.3`, `candid
  0.10.40`, built in a private temp dir with the shared `CARGO_TARGET_DIR`). Its `send(prefix,
  filler, n, suffix, method, token)` method builds the HTTP body inside the canister as `prefix +
  filler*n + suffix`, calls `http_request_update` (or `http_request`) with `Call::unbounded_wait`,
  and returns the status, the first and last bytes of the reply, any reject, and its own cycle
  spend. `blast(n, ...)` / `blast_oneway(n, ...)` send n small calls concurrently. The test token
  went in through an `--args-file` init argument written from `.env` by a script and deleted right
  after install. It was never printed or passed on a command line.
- Targets:
  - the live `icp_code_mode` (`4caro-...`, module `224cfa7a...`, built from HEAD, see
    grid-L13-r2) for the size and search tests. Caller `4zfnl-5t777-77775-aaadq-cai`.
  - a **throwaway copy** of the audited module (`.icp/cache/artifacts/icp_code_mode`, sha256
    `ad3ee221...`, same commit) installed in detached canister `5uljf-s3777-77775-aaaea-cai`,
    with the same token hash and the same 13-method allowlist (ledger 5 + demo_store 8),
    default limits. Caller `5tkpr-7d777-77775-aaaeq-cai`. Used for the memory tests, so the
    shared canister's heap high-water mark would not be pushed toward 3 GiB, and for the cost
    tests, so other agents' traffic would not add noise.
- No setting of `icp_code_mode` was changed, and it was not deployed. Limits read back
  afterwards were the defaults (25 calls, 20e9 instructions, no timeout, no certified reads).
  All three fixture canisters were deleted (cycles recovered). Side effects on the live canister:
  memory size went from 380,391,208 B to 481,120,040 B (heap high-water mark from the 10.48 MB
  search below; it stays until the next upgrade), about 0.2T test cycles spent, and audit entries
  9911-9913, 9976-9978 plus a few in between that are mine (the PolicyViolation search and script
  runs below).

## Measurements

| # | Request from the caller canister (token unless stated) | Body bytes | Result | Target paid (cycles) | Caller paid | Audit |
|---|---|---|---|---|---|---|
| S1 | `ping` + trailing spaces | 3,000,040 | 200 `{"result":{}}` | 0.24B | 3.71B | none |
| S2 | same | 10,484,040 | 200 | 0.81B | 12.93B | none |
| S3 | same | 10,486,000 + Candid envelope = 10,486,231 | refused by the platform on the caller's side: "payload size of 10486231 ... exceeds the allowed limit of 10485760" | - | - | none |
| K1 | search, `query` = `"( "` x 2,000,000 (live) | 4,000,112 | success | 10.79B | - | none |
| K2 | `"( "` x 4,000,000 | 8,000,112 | `policy: instruction budget exceeded (20359710740 > 20000000000)` | 21.6B | 9.15B | 1 PolicyViolation |
| K3 | `"( "` x 5,240,000 | 10,480,112 | `... (26625395028 > 20000000000)` | 28.2B | 11.99B | 1 PolicyViolation |
| K4 | `"-> "` x 2,000,000 | 6,000,112 | `... (26704841870 > 20000000000)` (about 13,350 instructions per keyword) | 27.6B | 6.69B | 1 PolicyViolation |
| **K5** | `"-> "` x 3,000,000 (live) | 9,000,112 | **reject IC0522: "Canister exceeded the limit of 40000000000 instructions for single message execution"** | **40.01B** | 10.03B | **none** (audit length 9976 before and after) |
| K6 | `"-> "` x 3,493,000 | 10,479,112 | IC0522, same | 40.4B (others active) | 11.68B | none |
| B0 | Python `while True: x += 1` alone | 176 | `policy: instruction budget exceeded (20000597768 > 20000000000)` | 20.0B | 6.5M | 1 |
| B1 | batch [same script, search `"-> "` x 690,000] (fits in an ingress message) | 2,070,291 | both answered; script `budget exceeded`, search listed 13 methods | 29.5B | - | 1 (the script's) |
| **B2** | batch [same script, search `"-> "` x 1,600,000] | 4,800,291 | **IC0522 (40e9)** | **40.01B** | 5.35B | **none**: the script's PolicyViolation entry is rolled back |
| N1 | 50-message batch, each `{"id":"i" x 209,000, "method":"nope"}` (copy) | 10,452,101 | 200 `-32603 reply exceeded 1900000 bytes` (no trap) | 1.41B | 11.03B | none |
| N2 | same with `ping` | 10,452,101 | same | 1.41B | 11.03B | none |
| N3 | 50 searches, each `"-> "` x 69,000 (copy) | 10,355,651 | 5 answered, 45 `-32600 at most 5 searches` | 4.96B | - | none |
| M1 | `execute_icp_script` `return 1` with `arguments.pad` = `[{"":0},...]` x 299,000 (copy) | 2,093,127 | `1`, success | 2.37B | 2.27B | 1 Ok |
| M2 | same x 1,497,000 (copy) | 10,479,127 | `1`, success | 11.76B | 11.34B | 1 Ok |
| M3 | same with `pad` = `[ {"":{"":...0...}} 120 deep ]` x 17,400 (copy) | 10,474,927 | `1`, success | 14.56B | 11.06B | 1 Ok |
| M4 | after M3: JS pushes 200,000 strings (copy) | 178 | `200000`, success | - | - | 1 Ok |
| Q1 | search `"-> "` x 300,000 sent to the **query** method `http_request` (copy) | 900,112 | answered from cache | 4.03B | - | none |
| Q2 | same x 3,000,000 | 9,000,112 | IC0522 at **5e9** (replicated query limit) | 5.01B | 9.0B | none |

Heap high-water mark of the copy (`canister status` memory size): 73,351,622 B before M1, then
**477,577,670 B after M1** (+404 MB for a 2.09 MB body), **2,084,716,998 B after M2**, and
**2,701,738,438 B after M3**, which is 84% of the 3,221,225,472 B `wasm_memory_limit`. Idle burn
of the copy at 2.70 GB: 69.9B cycles/day. The live canister at 481 MB: 13.2B cycles/day. M4 shows
the canister keeps working: the freed heap is reused.

Purpose-built attacker for L5-3 (copy, 400 concurrent calls each, small `ping` body):

| Call | Target paid per call | Attacker paid per call | Ratio |
|---|---|---|---|
| `http_request_update`, no token (401) | 5,726,507 | 5,754,415 | 1.00 |
| same, `oneway` (no reply handling) | 5,752,696 | 5,582,907 | 1.03 |
| `http_request` (query method called as an update), no token | 5,606,982 | 5,753,534 | 0.97 |
| `http_request_update`, token, `ping` | 5,851,404 | 5,816,538 | 1.01 |

## Cells

| Row | Verdict | Reason / evidence |
|---|---|---|
| A8 other canisters | **finding L4r4-1, L4r4-2**; L5-3 amplification measured (1.0x) | R (S2/S3): a canister on the same subnet delivers bodies up to 10,485,760 B minus the Candid envelope, so 10,484,040 B was accepted. That is 5x the ingress bound every size row assumed. Cross-subnet callers are capped at 2 MiB (platform, unverified here: single-subnet network). With a token the caller reaches everything an ingress caller reaches, at 5x the size: K5/K6/B2 (traps) and M2/M3 (heap). Without a token, see VF3 |
| U1 HTTP `method`, `url` | holds | `authenticate` (`src/mcp.rs:266-290`) does one `split` and a `trim_end_matches` on the path and echoes neither. A 10 MiB url costs a linear scan and a 404 (C). The bound is the 10 MiB payload limit, not 2 MiB, for A8; no multiplication |
| U3 HTTP body | **finding L4r4-1 (as the size source), L4r4-2** | The inventory's "ingress message limit (~2 MiB) only" is wrong for A8: 10 MiB (R, S2). `MAX_BATCH` holds (LIM8). What the body size multiplies: per-search render work (L4r4-1) and the parsed copies of `params` (L4r4-2) |
| U5 `arguments.*` | **finding L4r4-2** (`arguments` is deep-cloned whole); otherwise holds | `parse_tool_call` clones all of `arguments` (`src/mcp.rs:212`) to read one or two strings, after `classify` cloned all of `params` (`:255`). Unknown fields such as `pad` are kept until the clone is dropped (M1-M3). `code` over 100 KiB is refused after parsing (known; no multiplication). `query`/`canister_id` unbounded: L4r4-1 for `query`; the `canister_id` echo is truncated by `tool_result` to 1 MB (known echo class) |
| W1 `parse_body` | **finding L4r4-2**; no trap on parse at 10 MiB | R: 10.47 MB at the depth limit (nested single-key objects 120 deep, inside a batch element) parses and returns a result. 128+ levels: `-32700 recursion limit exceeded` (R). Parse plus the two clones grow the heap to 2.70 GB (M3). No trap was reached at 10 MiB. The densest shape found peaks below 3 GiB on an empty heap, but it leaves about 0.5 GB for anything else in flight |
| W3 `search::render` | **finding L4r4-1** | R: one search of 3.0M `"-> "` keywords (9.0 MB) over the default 13 allowed methods traps the message at 40e9 (K5). At 6 MB the same search ends in a recorded `PolicyViolation` (K4). Above about 9 MB the record and the clean error are gone. Ingress tops out near 0.7M such keywords (about 9.3e9), so this needs a canister caller at the demo allowlist size. The query route is bounded by the 5e9 replicated-query limit (Q2), holds |
| LIM8 `MAX_BATCH` 50 | holds | R (N1, N2, N3): a 10.45 MB 50-message batch is parsed, then answered or refused; 51+ is refused after parsing (known). The reply re-measure (`src/mcp.rs:405`) and `finish` (`:306-318`) serialize about 10 MB of echoed ids twice and return a small `-32603` without trapping (N1/N2: 1.41B cycles) |
| LIM9 `MAX_SEARCHES` 5 | holds (update path) | R (N3): 50 searches in 10.36 MB, 5 run, 45 refused cheaply. The cap bounds search count, not keyword bytes: the 5 searches share one body, so splitting does not multiply render work beyond the body size. One search already traps at 9 MB (L4r4-1). The query-path exemption is known (L4-3) |
| VF3 calls from other canisters | **measured, L5-3 not amplified**; **finding L4r4-1** for token holders | R: for a tokenless or small call, a purpose-built attacker pays as much as the target (0.97-1.03x, even with `oneway`, because the caller is charged for the call, the bytes and its response execution). Body bytes cost the caller about 1,233 cycles/byte (S2) and the target about 78 cycles/byte, the reverse of ingress, where the target pays about 2,080 cycles/byte (L5-1). So L5-3 is a cost of about 5.7M per call that the canister pays with no amplification. The comment's "they pay for themselves" is wrong only in that the canister pays its share as well. With a token the amplification is the work per message: K5 4.0x (40.0B vs 10.0B), B2 7.5x (40.0B vs 5.35B), M3 1.3x at once plus about 57B cycles/day of extra idle burn until the next upgrade (L4r4-2) |
| I4 bounded cost | **violated (new triggers L4r4-1, L4r4-2)**; known parts unchanged | Per message the canister can be driven to the platform's 40e9 limit with no record (K5, B2). One request raises the permanent heap high-water to 2.70 GB (M3). Neither bound comes from the code: the instruction bound is the platform trap, and the memory bound is the 10 MiB inter-canister payload times the parse amplification |

## Candidate findings

### L4r4-1 Canister callers send 10 MiB bodies; one search at the default allowlist traps at 40e9 and leaves no record (Medium, R)

- Lens L4 (and L3/L5). Invariants I4 (bounded cost), I5 (clean failure), I3 (a run that ended
  over budget has no entry).
- Where: `src/mcp.rs:374-412` (`http_update`: no body-size limit, runs every item in one
  execution), `src/search.rs:80-82` (`keywords`: unbounded count), `src/search.rs:116-121`
  (every keyword checked against every allowed method), `src/lib.rs:150-154` (budget compared
  only after `render` returns).
- Repro (R, live `icp_code_mode`, default limits and allowlist): from a same-subnet canister
  holding the token, call `http_request_update` with body
  `{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"search_icp_canisters","arguments":{"query":"-> -> ... (3,000,000 times)"}}}`
  (9,000,112 B). Reply: reject `IC0522 ... exceeded the limit of 40000000000 instructions`.
  Target paid 40.01B cycles, the caller 10.03B, and the audit length was unchanged. At
  2,000,000 keywords (6 MB) the same request returns `policy: instruction budget exceeded
  (26704841870 > 20000000000)` and writes a `PolicyViolation` entry.
- Batch variant (R, B2, known root SW3-5-2): `[Python while-loop script, search "-> " x 1.6M]`
  (4.8 MB) traps. The script, which spent its 20e9 and would have closed `PolicyViolation`
  (B0), leaves no entry. The same pair at ingress size (B1, 2.07 MB) completes and is audited.
- Why it is new: SW3-5-2 and L5-4 judged reachability against "the HTTP body caps the keywords
  at about 1M" and estimated that about 85 allowed methods were needed. A canister caller gets
  5x the bytes, and `"-> "` costs about 13,350 instructions per keyword over 13 methods (vs
  about 5,000 for `"( "`). So a single search traps at the demo's 13 methods with no
  configuration change.
- Refutation tried: `"( "` at the maximum size stays at 26.6e9 (K3, no trap); the query route
  stops at 5e9 (Q2); `MAX_SEARCHES` does not help because one search suffices; the caller pays
  about 1,233 cycles/byte, so the cost amplification (4x-7.5x) is lower than an ingress token
  holder gets for free. The finding is the unrecorded trap at default settings, not the cost.
- Fix: bound the work before doing it. Cap keyword count and length in `parse_tool_call` or
  `keywords`, check the instruction counter inside the `render` loop, and enforce one per-request
  budget across all items (the SW3-5-2 fix). Also cap the body size in `http_update` (for
  example 2 MiB, matching ingress), so canister callers get no larger inputs than gateway callers.
- Pattern for the sweep: *a bound taken from one entry path's transport limit (ingress 2 MiB)
  and assumed for every caller of the same method*.

### L4r4-2 The request parser keeps three full copies of untrusted `params` as `serde_json` trees: about 1,300 bytes of heap per 5 body bytes, 2.70 GB high-water from one request (Medium, R)

- Lens L4/L3 (L18 cost). Invariant I4 (no caller can make the canister hold unbounded memory).
- Where: `src/mcp.rs:293-304` (`parse_body` builds the whole `Value` tree), `src/mcp.rs:255`
  (`classify`: `msg.get("params").cloned()`), `src/mcp.rs:212` (`parse_tool_call`:
  `params.get("arguments").cloned()`). The tree is a `BTreeMap` per object (no
  `preserve_order`), so a one-key object costs a full leaf node.
- Repro (R, throwaway copy of the audited module, empty-ish heap of 73 MB):
  `{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"execute_icp_script","arguments":{"code":"return 1","pad":[ ... ]}}}`
  - `pad` = `{"":0}` x 299,000 (2.09 MB, fits ingress): memory size 73 MB -> 478 MB, result `1`;
  - `pad` = `{"":0}` x 1,497,000 (10.48 MB, canister caller): -> 2.08 GB;
  - `pad` = 17,400 single-key objects nested 120 deep (10.47 MB): -> 2.70 GB, result `1`.
  The wasm heap never shrinks, so idle burn stays at 69.9B cycles/day until an upgrade. The
  copy at 73 MB burned about 1.9B/day (estimated from the live 481 MB at 13.2B/day). The
  caller paid 11.06B once.
- Not shown: a trap. The densest shape found peaks at about 2.7 GB on a fresh heap. Together
  with suspended runs holding memory (JS is uncapped, register #24) the request would trap
  instead. A trap rolls the growth back, so the lasting effect is the cost, and only a request
  that succeeds leaves the high-water mark behind.
- Relation to known findings: the "high-water never shrinks" cost is L18-8/L8-6, and those
  triggers are script heaps. This trigger runs before any script, budget or language. It needs
  only a valid token, and it works through ingress too (+404 MB at 2.09 MB). A canister caller
  makes it 5x larger.
- Fix: refuse bodies over a fixed size before parsing (`http_update` and `http_query`). Read
  `params`/`arguments` by reference (`get`/`as_str`) instead of cloning. Reject unknown
  `arguments` fields (this also closes the known "unknown fields accepted" finding at
  `src/mcp.rs:202-228`).
- Pattern for the sweep: *untrusted input copied in full where only a few scalar fields are
  read*. Other candidates to check: `classify` `id` clone (bounded by echo/`finish`), JS
  `args_json` + parsed `Value` + `enc.bytes` held across awaits (sweep-r3-2 "not tested").

## Not tested, and why

- **Cross-subnet canister callers** (2 MiB inter-canister limit): the local network has one
  subnet. Mainnet behaviour, including whether the 10 MiB same-subnet limit applies to an
  application subnet's own canisters, is unverified (register #57).
- **Mainnet cost ratios**: cycles here are the local network's prices (about 1.06
  cycles/instruction observed). The ratios in VF3 hold only if mainnet charges caller and callee
  the same way.
- **A parse-path trap** (body peak over 3 GiB): not reached at 10 MiB on an empty heap. It was
  not combined with 8 suspended JS runs holding memory, because doing that on the shared canister
  would block other agents (another lens already saw the 3 GiB limit block every update). On the
  copy it would only add a trap, with no lasting effect beyond L4r4-2.
- **5 searches x 2 MiB each in one batch**: the fixture's `filler` primitive repeats one unit,
  and an ingress argument to the fixture is capped at 4 MiB by the local gateway, so a batch of
  five big searches could not be expressed. A single search already traps (K5), so this adds
  nothing.
- **Rhai/JS budget-burning script + search**: only Python was used for B0-B2. The Python run
  stopped cleanly at its 20e9 budget, and the trap comes from the sum, so the language does not
  matter.
- The live canister's high-water mark grew by about 101 MB from K3 (keyword strings). This was
  not reversed: shrinking it takes an upgrade, which is outside this lens's remit.
