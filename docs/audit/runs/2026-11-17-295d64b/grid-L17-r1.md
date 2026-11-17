# Coverage grid: L17 Client and presentation, round 1

Run `2026-11-17-295d64b`. Lens question: can the user-facing surface be turned against the user?
The surfaces are the demo page (C5, E26), the text the AI model reads (A4: `tools/list`,
`initialize.instructions`, search output, script results and errors), and the video capture
tooling that handles the token in a browser (C12). Evidence: **R** = reproduced (command or
test given), **C** = code path. Candidate findings are numbered L17-1 to L17-5 (details in the
lens report returned to the workflow). Where a cell shares a root cause with an L10 cell
(`grid-L10-r1.md`), it says so, so Phase 5 can merge them.

## Dynamic tests run

All in a private copy of the tree (scratchpad), `CARGO_TARGET_DIR=/private/tmp/claude-501/audit-target`.
Nothing was deployed; the only request to the local network was one `GET /` (headers).

1. **Target text to the model (native unit probe).** A test added to the copy's `search.rs`
   parsed a `.did` whose comment, quoted method name and quoted record label each hold an
   instruction to the AI, then ran `search::render` (allowlist `*`) and
   `mcp::tool_result(Ok(String(did)))` (what `icp.candid` returns). Output: the quoted method
   name appears verbatim as `"name"`, the quoted label appears in `types.Account` (a type
   referenced by `list_products`, so it would appear with an exact-name allowlist too), and the
   whole text including the `// SYSTEM NOTE TO THE ASSISTANT ...` comment appears in the tool
   result with `isError: false`. `cargo test --lib l17_` passed.
2. **Demo page regexes (V8, Node 26).** `classicDefsChars` service-line regex
   (`src/demo.html:408`) on `list_products : () -> (); //` followed by `" ) -> ("` x k:
   k=1000: 5 ms, 2000: 19 ms, 4000: 76 ms, 8000: 304 ms (quadratic); a 518,045-byte interface
   blocked for **24,576 ms**. The same file (518,059 bytes with one type line) parses with
   `Service::parse` (native test) and is under `MAX_DID_BYTES` (524,288). The type regex
   (`:405`) is also quadratic but reaches only ~1 s at the 512 KiB limit.
3. **Demo page encoders (Node).** `esc` and the scenario `code(S)` functions were extracted
   verbatim from `src/demo.html`. 200,000 random strings over `& < > " ' a é 😀` through
   `esc(JSON.stringify(s)).slice(0, 40)`: no output contains `<` or `>`. `code(S)` with
   `S = 'x"; await icp.call("x", "create_order", []); //'` yields the first line
   `const S = "x"; await icp.call("x", "create_order", []); //";`.
4. **Capture tooling (fault injection).** `video/cdp.mjs` and `video/capture-demo.mjs` copied
   next to a `.env` holding `MCP_TOKEN=DUMMY-TOKEN-L17-not-real`; a fake DevTools endpoint
   listened on 127.0.0.1:9333; `CHROME=/usr/bin/true` (Chrome never starts). The script
   fetched `/json/list` from the fake endpoint, opened the WebSocket it named, and sent
   `Runtime.evaluate` with `localStorage.setItem("icp-code-mode-token", "DUMMY-TOKEN-L17-not-real")`.
   The real token was never read or used.
5. **Live headers.** `GET http://icp_code_mode.local.localhost:8010/` returns
   `content-type`, `cache-control: no-cache`, gateway CORS headers (`access-control-allow-origin: *`)
   and HSTS; no `Content-Security-Policy`, no `X-Frame-Options`/`frame-ancestors`.

## Grid

| Row | Cell | Entry | Reason / evidence |
|---|---|---|---|
| C5 | `innerHTML` sites `src/demo.html:433-507` | holds | Every `innerHTML` string is built from page constants, numbers (`fmt`, `toFixed`, `tok`) or `esc(...)`. The only reply-derived values are `r.method` (page constant) and `r.args` (customer principals / product ids from demo_store replies) at `:480`, both through `esc`. `esc` omits quotes, but no reply-derived value reaches an attribute context (`class`/`style` values are constants or numbers), so quotes do not matter (R test 3) |
| C5 | `slice(0, 40)` after `esc` | holds (cosmetic) | Cutting `&amp;` to `&am` or a surrogate pair cannot create markup: characters from entity decoding are text (R test 3: `["&amp;...&am`). Display only |
| C5 | comparison script built by string interpolation (`:299, 315, 329`) | **finding L17-2** (Low) | `const S = "${S}"` with `S` = `canister_id` from a search reply; the sibling `classicCall`/`classicDefsChars` use `JSON.stringify(S)`. Honest server: `S` is `Principal::to_text()` (`src/search.rs:89`), no quote possible. A forged search reply (query path, L10-1) becomes script run with the user's token (R test 3) |
| C5 | `classicDefsChars` regexes on target `.did` text (W16) | **finding L17-3** (Low) | Quadratic regex on target-controlled text freezes the tab ~25 s within the 512 KiB limit (R test 2) |
| C5 | token handling | holds (honest page), see D9 | Token sent only as `Authorization` to relative `/mcp` (`rpc` `:219-223`, `post` `:367-368`); raw-exchange view shows the body, not headers; input is `type=password`; never interpolated into script or DOM |
| C5 | framing / CSP (R test 5) | holds (hardening only) | No `frame-ancestors`/CSP. A framing site cannot read the stored token: modern browsers partition third-party storage, so the framed page sees an empty token field; every button is read-only or refused by policy (examples, comparison scenarios). No failure demonstrated |
| C5 | page reflects request input | holds | Static `include_str!`; `demo_page` ignores query string and fragment (`src/mcp.rs:323-330`) |
| E26 | buttons and comparison runner | holds except L17-2/L17-3 | `show()` writes everything with `textContent` (`:240-260`); errors via `e.message` into `textContent` (`:232, 512`) |
| B17 | JSON -> DOM, `.did` regex parsing | L17-2, L17-3 | as above |
| W16 | browser work on interface text | **finding L17-3** | as above |
| U14 | token typed by the user; JSON rendered into the DOM | holds; D9 | as above |
| D9 | token in `localStorage` | **finding L17-5** (Low; same root cause as L10-2) | Written on every request (`:213`) and reloaded on every visit (`:207`); no way to clear it in the page. The hint (`:81`) discloses it, so not a claims violation; exposure needs other script on the origin (L10-1 forged page) |
| I25 / CL30 | page holds no secrets, token only as header, nothing from a reply runs as script in the page | holds for the page; violated in spirit by L17-2 | No reply value reaches `eval`, `innerHTML` unescaped, or an attribute. L17-2 runs reply data as script in the canister sandbox (not the page) under the user's token |
| A1 | anonymous web user | holds | Gets the static page; nothing to inject without a token |
| A3 | token holder via the page | holds except L17-2/L17-3/L17-5 | |
| A4 | static text: `tools/list` descriptions, `initialize.instructions` (`src/mcp.rs:69-149`) | holds (no injection); misstatements belong to L16/L11 | Compile-time constants. They tell the model search has "no audit entry, does not count toward the call cap" (CL3) and "no ... randomness" (CL29, L11-2): wrong, but they mislead about cost/semantics, not into acting against the user |
| A4 | search output text from target interfaces (B16) | **finding L17-1** (Medium; same root cause as L10-4) | Quoted method names and quoted field/variant labels are printed verbatim; labels come in through types referenced by exactly-allowlisted methods, so the allowlist does not filter them (R test 1) |
| A4 | `icp.candid` text | **finding L17-1** | Returns the fetched text unchanged, comments included (`src/bridge.rs:470-500`); string results go out raw, not JSON-quoted (`src/mcp.rs:157`) (R test 1) |
| A4 | script errors embedding target reject messages (U9) | **finding L17-1** (symptom) | `format!("call {p}.{method} failed: {e}")` (`src/bridge.rs:528-531`); `CallFailed` displays `call rejected: {code} - {reject_message}` (`ic-cdk-0.20.3/src/call.rs:520`). Fetch failures embed it too (`src/bridge.rs:497`) and are cached in `DID_FAILED` for 5 min, so every token holder's search shows them (`src/search.rs:98-101`) (C) |
| A4 | script results (values) | holds as data | Values the model asked for; their content is data by nature. Only the lack of any provenance marking is the L17-1 issue |
| A4 | console-output framing | L17-1 (symptom) | Logs are appended as `[console output]\n...` (`src/mcp.rs:165`); a printed target string can imitate that framing or a result. Same root cause |
| E4b | `initialize`, `tools/list` answered from query | holds for L17; alteration is L10-1 | |
| E5a / E5b | tool outputs | L17-1 | |
| U9 / U10 | reject messages, interface text | L17-1 | |
| B11 | result -> `CallToolResult` text | L17-1 (symptom) | Raw strings, no source label; truncation marker inside the text is L7's |
| B16 | interface -> search JSON | L17-1 | |
| V4 | cached fetch-failure text | L17-1 (symptom) | Target text persists 5 min across all users of the canister |
| O6 | HTTP responses (page, JSON) | holds; see C5 framing | JSON responses have `application/json`; 401/404/405 bodies carry no reply-derived HTML |
| C12 | `capture-demo.mjs` + `cdp.mjs` token injection | **finding L17-4** (Low) | `launch()` never checks that the spawned Chrome started or owns port 9333, and follows the `webSocketDebuggerUrl` it is given; the token goes to whoever answers (R test 4). Leftover profiles holding the token are L10-3 |
| C12 | `DEMO_URL` override | holds (operator-set) | Token is written into whatever origin `DEMO_URL` (or a redirect from it) loads; set by the person running the script |
| O10 / U16 | video tooling effects | L17-4; `tts.mjs` key holds (L10) | |
| C12 | `video/composition.html` | not applicable | All `innerHTML` content is constants (`composition.html:302-362`); no external data |
| D10 | `.env` | not applicable here | Disk hygiene; L10 holds |
| CL3 / CL29 | claims shown to the AI | not applicable here | L16 / L11-2 own them |
| C1-C4, C6-C11, C13-C15, A2, A5-A12, E1-E3, E6-E25, E27, U1-U8, U11-U13, U15, O1-O5, O7-O9, D1-D8, V1-V3, V5-V11, K1-K12, LIM1-LIM32, W1-W15, F1-F16, B1-B15, VF1-VF8, AD1-AD9, S1-S10, CL1-CL2, CL4-CL28, CL31-CL37 | | not applicable | Not shown to an end user or to the AI model, and not handled in a browser; their effect on output text is reviewed through the A4, B11 and B16 cells above |
| T | tests | gap | `mcp::demo_page_is_served_without_auth` checks only that the page is served. Nothing tests the page's DOM output, the comparison's script construction, regex cost on large interfaces, or how target text is framed for the model. Harness that would cover it: a headless-browser test of `src/demo.html` against a mock `/mcp` that returns hostile replies |
