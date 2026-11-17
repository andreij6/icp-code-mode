# Variant sweep r5-7: ambiguous input shape resolved differently from the documented rule

Run `2026-11-17-295d64b`, Phase 6, round 5. Pattern:

> An input whose shape admits more than one reading is resolved by the code (or by a
> documented recipe) in a way the documented rule does not say, so the same value means
> different things on different paths.

Seeds (known, not re-reported): L7-15 (`[]` for one `opt vec` argument), the single-argument
`[x]` heuristic, the `_n_` / quoted-number variant tag finding, L7-13 (aliased record keys),
L7-11 (null/reserved slots), L7-16 (raw string results), register #42 (opt opt; also opt
null / opt reserved, per grid-L7-r1), the inputSchema finding (non-object `arguments`,
`language: null`, aliases), whitespace-padded / invalid search `canister_id`.

Method. Code read at `295d64b` from a `git archive` copy (the working tree's `src/` carries
someone else's uncommitted test additions; non-test lines are unchanged). Every place a
caller, script, operator, target or developer supplies a value whose shape is open to more
than one reading was listed, and the reading the code takes was compared with the README,
the `.did`, the tool descriptions shown to the AI and the code comments. Probes were run
natively in `/private/tmp/claude-501/sweep-r5-7.*` (shared `CARGO_TARGET_DIR`, every source
touched first so nothing stale was reused; `cargo test --lib`), and with `sh`/`node` for the
two tooling rows. The copy was deleted afterwards. No deployment, no live request.

Evidence: **R** reproduced here; **C** the code path produces it unambiguously; **known** in
the task's known list; **r5-N** found by another round-5 sweep (not repeated).

## New candidates

| Id | Sev | Rows | Summary | Ev |
|---|---|---|---|---|
| SW5-7-1 | Low | B6, B5, CL (README:354, `SEARCH_DESCRIPTION` `src/mcp.rs:75`) | A record whose labels are 0..n-1 (a Candid tuple, e.g. ICRC-1 `icrc1_metadata`'s `vec record { text; Value }`) leaves the bridge as a JSON **array**; a record with numeric labels that are not contiguous leaves as an **object** keyed `"0"`, `"2"`. README ("Records are objects") and the search description shown to the AI ("records are objects") state one rule; the code applies a second, undocumented one on output, while input accepts both forms | R |
| SW5-7-2 | Low | E1/E2, AD1, AD5, CL (`icp_code_mode.did:61`, `src/lib.rs:20`) | The token-hash recipe in the `.did` and `InitArgs` doc, `echo -n "$TOKEN" \| shasum -a 256`, depends on the shell. Under POSIX `sh` (macOS `/bin/sh`, every `make` recipe) `-n` is printed as text, so the digest is of `-n TOKEN\n`. The canister accepts it (it is 64 hex) and the token then gets 401. The README uses `printf %s` and gets the right digest. The recipe also omits `cut -d' ' -f1`, so pasting its output whole (`<hex>  -`) into `InitArgs` traps the install (F1) | R |
| SW5-7-3 | Low | C12, D10, U16, E28 | The two `.env` readers in `video/` resolve a quoted value differently. `tts.mjs:11` strips quotes from `GEMINI_API_KEY="…"`; `capture-demo.mjs:7` keeps them in `MCP_TOKEN="…"`, writes `"…"` (with quotes) into the page's localStorage, every MCP request is 401, and the script still writes screenshots and `demo-results.json` of the error state and exits 0. It also ignores `process.env.MCP_TOKEN`, which the README's quick start exports | R (parsers), C (401 path) |

### SW5-7-1 details

- Where: `src/candid_bridge.rs:339-350` (`idl_to_json`: tuple test `Unnamed(n)|Id(n) if n == i`
  for every field, else an object keyed by `label_name`). Input side `:256-273` accepts an
  array (tuple only) or an object keyed `"0"`, `"_0_"`. Docs: README.md:353-354 ("How values
  are encoded: Records are objects"), `SEARCH_DESCRIPTION` (`src/mcp.rs:75`, shown to the AI:
  "records are objects"). `TOOL_DESCRIPTION` says nothing about records.
- Repro (native, copy of 295d64b): interface
  `type R = record { text; nat }; type S = record { 0 : text; 2 : nat }; service : { f : (R, S) -> () }`.
  `encode_args("f", [{"0":"a","1":1}, {"0":"b","2":2}])` is accepted (records given as
  objects, as documented). Decoding the same bytes as a reply gives `[["a",1],{"0":"b","2":2}]`.
  `S` given as an array is refused ("expected record object, got array").
- Effect: a script that follows the documented rule and reads a tuple record by key gets the
  wrong type. In Python, `m["0"]` on a reply element is a `TypeError` (it is a list); in Rhai,
  map access on an array is an error; JS `m["0"]` happens to work. Whether the same kind of
  value (a record with numeric labels) is an array or an object depends on whether the labels
  are contiguous from 0, which the AI cannot see from the rule it is given. Loud failure, no
  wrong data: Low (L16, honest interface). The README's own Python example (`m[0] for m in
  meta`) relies on the undocumented array form.
- Fix: document the rule ("records whose fields are 0..n-1 are arrays, both ways") in README
  and both tool descriptions; test with a `record { text; nat }` reply.

### SW5-7-2 details

- Where: `icp_code_mode.did:61` and `src/lib.rs:20`. README.md:54 and :148 use
  `printf %s "$MCP_TOKEN" | shasum -a 256 | cut -d' ' -f1`.
- Repro (macOS, dummy token `dummy-token`, nothing sent anywhere):
  `sh -c 'echo -n "$1" | shasum -a 256' _ dummy-token` gives `1b9742e5…`;
  `sh -c 'printf %s "$1" | shasum -a 256'` and `zsh -c 'echo -n …'` give `fe0c8f2d…`.
  `sh -c 'echo -n abc' | od -c` prints `- n   a b c \n` (`/bin/sh` is bash 3.2 in POSIX mode).
- Effect: an operator who scripts `InitArgs` or `add_api_token_hash` from the interface's own
  comment in a `sh` script or Makefile registers a digest no token matches. `add_token_hash`
  accepts any 64-hex string (`src/policy.rs:208-213`), so nothing flags it until clients get
  401. Fails closed; the operator is misled about why. Low (L16).
- Fix: use the README's `printf %s … | cut -d' ' -f1` recipe in the `.did` and `lib.rs`.

### SW5-7-3 details

- Where: `video/capture-demo.mjs:7` (`.match(/^MCP_TOKEN=(.*)$/m)[1].trim()`), `video/tts.mjs:11`
  (`.match(/^GEMINI_API_KEY=(.+)$/m)…trim().replace(/^["']|["']$/g, "")`).
- Repro: a scratch `.env` with dummy values `MCP_TOKEN="dummy"` and `GEMINI_API_KEY="dummykey"`, run
  through the two regexes copied from 295d64b: `{"capture_demo_token":"\"dummy\"","tts_key":"dummykey"}`.
  The real `.env` was not read.
- Effect (C): the page sends `Authorization: Bearer "dummy"`, `authenticate` hashes the quoted
  text (`src/mcp.rs:266-290`) and answers 401. `capture-demo.mjs:24-37` does not check the
  comparison status, so it overwrites `video/assets/*.png` and `demo-results.json` with the
  error state and exits 0. An `export MCP_TOKEN=…` line (the README's quick-start form) does
  not match at all and the script crashes with a `TypeError` (loud). Dev tooling only: Low.
- Fix: one shared `.env` reader that strips `export` and quotes, falling back to
  `process.env.MCP_TOKEN`; fail if the page shows an error.

## Grid (every inventory row against the pattern)

| Rows | What can be ambiguous | Result | Note |
|---|---|---|---|
| C1, C1a | per-language builds and the `language` default | known | default `javascript` on builds without JS is known |
| C2 / E20-E25 demo_store args | `list_products(opt text)` category case; `search_products(text)` case; `list_orders(opt principal)` null | holds (C) | category compare `eq_ignore_ascii_case`; search lowercases the query and the name, categories are lowercase ASCII seed constants that no method changes; `null` means all, as the `.did` says. Duplicate/zero lines, saturation: known |
| C3 web-time, C4 chrono | none from callers | n/a | no caller-supplied shape reaches them |
| C5 / E26 / B17 demo page | token text, search reply, `.did` text | holds / known | token trimmed on both sides (`demo.html:208, 212` and `mcp.rs:285`); canister-id splicing, regex parsing and method counts are known |
| C6 / E27 demo-setup.sh | env var vs argument for the environment | known | |
| C7 icp.yaml `init_args: "(null)"` | `(null)` vs no argument | known | post_upgrade re-applying InitArgs is known |
| C8-C10, S1-S10 | build flags | n/a | no input shape |
| C11 `.did` comments | token-hash recipe | **SW5-7-2** | |
| C12 / U16 / E28 / D10 video `.env` | quoted / `export` lines | **SW5-7-3** | |
| C13 audit workflow | finding keys | r5-1 (SW5-1-2) | |
| C14 per-customer doc | none | n/a | |
| C15 exports | none | n/a | |
| A1-A12 | | n/a | the pattern is about values, not actors |
| E1/E2 `apply` | duplicate allowlist entries; opt Limits | r5-3 (SW5-3-3) / known | |
| E3 / U12 inspect_message | same `authenticate` as the method | holds (C) | identical header, scheme and path rules on both (`lib.rs:276-278`) |
| E4/E4a/E5 routes, U1 | `/mcp/`, `/mcp?x`, method case | holds (C) | trailing `/`, query and fragment ignored; `POST`/`GET` compared case-insensitively; README names only `POST /mcp`, `GET /`, `GET /demo`, all honoured. `/demo/` is a 404 (no rule says otherwise) |
| U2 Authorization | scheme case, extra spaces, several headers | holds (C) | scheme case-insensitive (RFC 7235); token trimmed; first `authorization` header used on both routes |
| U3/U4 / B1 body, ids, notifications | missing id, `id: null`, responses, empty batch | holds / known | missing id = notification (202, JSON-RPC rule); `id: null` -32600 (MCP rule); id forms known (L7-9); result/error-carrying invalid requests known |
| U5 / E5a `language`, `code` | aliases, case, null | known | inputSchema finding |
| U5 / E5b search `query` | empty, whitespace, case | holds (C) | `split_whitespace` + lowercase on both sides, as `SearchArgs` documents; empty = no filter |
| U5 / E5b search `canister_id` | `""`, padded, invalid | known | `""` gives "canister  is not allowlisted" (R here), merged into the known invalid/padded id finding |
| E16 JS `icp.call` arity | 4th argument | r5-3 (SW5-3-1) | R here as well: `icp.call(L, m, null, {owner})` sends `null` and returns Ok; Python `TypeError`, Rhai "Function not found" |
| E16/E18/E19 two-argument form, `undefined`/`None`/`()` | omitted args | holds (R) | all three send `null` |
| E18 Python kwargs | `icp.call(c, m, args=x)` while the help/error text shows `args=None` | holds | refused loudly ("positional arguments only"), not misread |
| E17 console | `"42"` vs `42` in logs | n/a | standard print semantics in every language |
| U6 script source | last expression vs `return` | holds | documented per language |
| U7 / B8 JS values | Map/Set, typed arrays, `__proto__` keys, index keys | holds / r5-8 | Map/Set/typed arrays as `JSON.stringify` (r5-8, not raised); `JsValue::from_json` defines own data properties and canonicalises index keys (`boa_engine property/mod.rs:695-699`), as `JSON.parse` does |
| U7 / B9 Python values | tuple/list/set, bytes, bigint in a text slot | known | repr/bigint findings (L7-6, py_repr rows); bytes -> blob documented |
| U7 / B10 Rhai values | blob, unit, char | holds | blob -> array documented; unit -> null |
| U8 / B6 replies | tuple vs sparse numeric records | **SW5-7-1** | |
| U8 / B6 replies | opt null / opt reserved / zero returns / multi-returns | known | #42, r5-8 |
| U9 reject text | none | n/a | |
| U10 / B7 interface text | class, named service, `import` | holds / r5-4 | class unwrapped, `service : X` traced; `import` is SW5-4-1 |
| U11 admin inputs | token hash vs plaintext token (both 64 hex with the README's `openssl rand -hex 32`) | holds | indistinguishable by shape; resolved as a hash, as documented; a pasted plaintext fails closed (401) |
| U11 revoke by shown prefix | 16-hex prefix vs 64-hex hash | r5-1 (SW5-1-1) | R here as well: `revoke(prefix) = false`, token still valid |
| U11 allowlist `"*"` | wildcard vs a method named `*` | r5-5 (SW5-5-1) | |
| U11 set_limits opt fields, override `Some("")` | | known / holds | omitted opt fields known; empty override text refused |
| U13 demo_store | see C2 | holds / known | |
| U14 page token | see C5 | holds | |
| U15 platform values | | n/a | trusted |
| O1 / B5 `encode_args` shapes | `null`, `{}`, `[]`, `[x]`, short arrays | known / holds | `[x]`/`[]` heuristic known; `{}` for a 0-arg method and padding a short array with `null` match Candid's rule that missing trailing opt arguments read as null (no different effect at the target) |
| B5 variants | `"Tag"` for an opt payload | holds | becomes `Tag(null)`, the only reading |
| B5 numbers, blobs, principals, func | text numbers, hex blobs, func array form | known / holds | lenient parsing L7-12; func object extra keys r5-3 (SW5-3-2); func `[p, m]` form is the only array reading |
| O2-O10 | | n/a / known | no further caller-shaped input |
| D1-D9 | stored formats | known | opt decoding as null, schema rollbacks known |
| V1-V11 | | n/a | not input shapes |
| K1-K12 | | n/a | |
| LIM1-LIM32 | shape of the value counted | n/a for this pattern | counting rules swept in r5-0 |
| W1-W16 | | n/a | cost, not shape |
| F1-F16 | F1 trap on a pasted `<hex>  -` line | **SW5-7-2** (symptom) | |
| B2 HttpUpdateRequest decode | | known | inspect quota |
| B3 token -> hash, caller prefix | | **SW5-7-2** / r5-1 | |
| B4 principal text | case, dashes | holds (C) | `Principal::from_text` lowercases and requires canonical grouping; trim only in search (known) |
| B11 tool result text | string vs value | known | L7-16 |
| B12 stable Candid | | known | |
| B13 language | | known | |
| B14 time | | known | seconds vs ms |
| B15 certificate header | | n/a | fixed |
| B16 search JSON | `1_000` / bool spellings | known | |
| VF1-VF8 | | n/a | |
| AD1-AD9 | | see U11, SW5-7-2 | |
| CL1-CL37 | README:349-359 argument and encoding rules; `.did` recipe; tool descriptions | **SW5-7-1**, **SW5-7-2** / holds | argument-mapping rules hold except the known heuristics |
| T1-T12 | tests | gap | No test decodes a tuple record and checks the documented shape; no test of the `.did` hash recipe |
| History | L7 class, #42 | covered | |

Not tested: the 401 and file overwrite of SW5-7-3 were not run against the live page (it
would need the real token in a quoted `.env`); SW5-7-1 on a live ICRC-1 ledger was not run
(the decoding path is the same code).
