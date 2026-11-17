# ICP Code Mode — explainer video script

**Narration style (Gemini TTS):** Confident, crisp developer narrator: brisk and punchy, with a short pause after each number and after 'Refused' so they land, warm but not hype-y, like a sharp conference demo.

**Words:** 115

## Scene 1

**Voiceover:** Same question. Same answer. Two point six times fewer input tokens.

**Visual:** (0-4s) Dark background. Huge bold text appears one phrase at a time: 'SAME QUESTION.', then 'SAME ANSWER.', then a giant '2.6x FEWER INPUT TOKENS' in an accent color that pops in with a quick scale-up. Small subtitle underneath: 'real calls on a local ICP network · tokens estimated'.

## Scene 2

**Voiceover:** Classic MCP: one tool per API method. Raw results flow through the model. One list orders call dumps about twelve thousand characters into context.

**Visual:** (4-13s) Left side: a vertical stack of 10 tool chips (list_orders, get_customer, list_products, ...) labeled 'Classic MCP: one tool per API method'. Right side: just 2 chips labeled 'Code Mode'. The 10-chip stack then collapses into a simple diagram: a 'list_orders' box shoots a fast-scrolling stream of raw JSON order rows into a box labeled 'LLM context', which fills like a progress bar. A counter ticks up to '~12,000 chars'.

## Scene 3

**Voiceover:** Code Mode: two tools. Search and execute. The model writes a short script. It runs in a sandbox inside an ICP canister, and only the answer comes back.

**Visual:** (13-24s) Two tool chips slide in: 'search_icp_canisters' and 'execute_icp_script'. A code card with language tabs 'JavaScript | Python | Rhai' (JavaScript active) types out: const orders = await icp.call(store, 'list_orders', []); then a short reduce/sort and return top3. The card drops into a box labeled 'ICP canister: Wasm sandbox'. Caption under the box: 'canister = a smart contract running WebAssembly'. Inside the box, many rows shrink to 3 lines, and only that small result travels back to the 'LLM' box, tagged '~480 tokens of results'.

## Scene 4

**Voiceover:** Top three customers by spend. Classic: four tool calls, almost ten thousand input tokens. Code Mode: two calls, under four thousand. The gap grows with your data.

**Visual:** (24-35s) Real screenshot of the side-by-side demo page, task 'Top 3 customers by spend'. Columns labeled 'Classic MCP' and 'Code Mode'. Counters animate: left reaches '4 tool calls' and '~9,950 input tokens', right stops at '2 tool calls' and '~3,840 input tokens'. Horizontal token bars grow, with the left bar about 2.6x longer. A green check badge between the columns reads 'Identical answer'. On 'gap grows with your data', a small upward arrow appears beside the bars. Footnote at the bottom: 'Measured on a local ICP network, demo store with 60 orders.'

## Scene 5

**Voiceover:** Guardrails live on chain. A transfer that isn't allowlisted? Refused before anything is sent. Every run is audited.

**Visual:** (35-42s) Three bold badges stack in: 'Allowlist (deny by default)', 'Max 25 calls per script', 'Append-only audit log'. A code card shows icp.call(ledger, 'icrc1_transfer', ...), and a red 'REFUSED: not allowlisted. No message sent.' stamp slams across it. An audit-log row slides in under the stamp: 'script_hash: 9f3a...c21 | canisters: ledger | violation: method not allowlisted'.

## Scene 6

**Voiceover:** Open source. github slash andreij6 slash icp-code-mode.

**Visual:** (42-46s) Closing card: large 'ICP Code Mode' with the subtitle 'MCP server inside an ICP canister'. A bold line reads '2 tools. Not one per method.' Below it, in bold monospace: 'github.com/andreij6/icp-code-mode', with an 'Open source' tag and a subtle star icon pulsing next to the URL.

## Tweet

Same answer. 2.6x fewer input tokens.

Your MCP agent doesn't need a tool per method. It needs 2.

ICP Code Mode: the agent writes a script, it runs in a Wasm sandbox in an ICP canister, only the answer returns.

github.com/andreij6/icp-code-mode
