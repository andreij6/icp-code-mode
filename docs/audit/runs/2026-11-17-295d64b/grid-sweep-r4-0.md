# Variant sweep r4-0: two paths answering one request from different subsets of one cache

Run `2026-11-17-295d64b`, Phase 6, round 4. Pattern under test:

> Two paths answering the same request from different subsets of the same cache.

Generalised for the sweep: a piece of shared state is layered (D5 overrides, V5 parsed
overrides, V2 `DID_CACHE` with TTL, V4 `DID_FAILED` with TTL, the allowlist, the limits,
heap counters, the durable log), and two paths that answer the same question (query vs
update, `icp.candid` vs `icp.call` vs search, admin view vs runtime view, Classic vs Code
Mode on the demo page) each consult a different subset of the layers, so the answers differ
for the same state.

Method: code reading at `295d64b` (working-tree changes under `src/` and `demo_store/` were
checked with `git diff 295d64b` and are test-module additions only, so non-test line numbers
match the commit). One computation: the demo page's own `classicDefsChars` code, copied
verbatim into a Node script in the scratchpad, run on `demo_store/demo_store.did` (the file
`icp.yaml` embeds as the `candid:service` metadata that `icp.candid` returns). Nothing was
built, deployed or sent.

Known and not re-reported: query search ignores `DID_FAILED`; query search skips
`MAX_SEARCHES` and the instruction budget; `icp.candid` serves overrides/cache without an
allowlist check; overrides outlive `remove_allowlist`; overrides consulted first (README says
third); eviction race; no single-flight; expired entry discarded on a failed re-fetch;
query-path override re-parse under the 5B limit; search trims `canister_id`, scripts do not;
`active_runs` excludes searches while `pending_run_ids` includes them; snapshot vs live
limits; latent override-parse divergence between `cached_service` and `IcBridge::candid`
(r3-1).

## New candidates

| Id | Sev | Title | Ev |
|---|---|---|---|
| SW4-0-1 | Medium | Demo comparison (and README benchmark table) builds Classic tool definitions from `icp.candid` (all 10 store methods) but Code Mode's side from search (the 8 allowlisted ones). With the same 8-method surface, "What needs restocking?" flips from README's "1.2x fewer" to 0.96x (Code Mode uses more input tokens); Top 3 2.6x -> 2.38x, Tea 1.4x -> 1.31x | R (computed with the page's own code; base figure matches L16-r4's live 1,203 tok exactly) |
| SW4-0-2 | Low (may merge into the overrides findings) | `get_policy` answers "what governs this canister" from limits, allowlist and tokens only; `CANDID_OVERRIDES` (D5) has no read or list path at all, yet every serving path consults it first. A controller cannot discover a forgotten override that shadows a target's real interface (with the known stale-interface `opt` -> null effect) | C |

### SW4-0-1 detail

- Classic side: `classicDefsChars` (`src/demo.html:401-417`) runs
  `icp.candid(S)` and makes one tool per service line: 10 tools, including `cancel_order`
  and `restock`, which `scripts/demo-setup.sh:2-3` keeps blocked "so the demo can show the
  guardrail".
- Code Mode side: `search_icp_canisters({canister_id: S})` (`src/demo.html:455`) renders
  only `e.allowed` methods (`src/search.rs:103-109`): 8 methods and their types.
- Same interface, two subsets. Computation (scratchpad `defs.mjs`, the page's regexes and
  object shape verbatim): all methods 4,810 chars (1,203 tok, matching L16-r4's live
  "10 classic tools ≈1,203 tok"); the 8 allowlisted methods 3,733 chars (933 tok). The page
  counts definitions once per Classic LLM turn (`simulate`, `src/demo.html:425-430`); Classic
  turns are 3 / 3 / 2. Re-scoring L16-r4's live inputs with 270 tok less per turn:
  Top 3 9,954/3,838 = 2.594 -> 2.383; Restocking 4,515/3,863 = 1.169 -> 0.960; Tea
  5,609/3,884 = 1.444 -> 1.305.
- README.md:194-203 presents the ratios as Code Mode's advantage. The page discloses
  "10 tools" in its "How the numbers are counted" note, but neither it nor the README says
  that 2 of the 10 are methods Code Mode refuses, and the restocking claim depends on them.
- Refute attempts: (a) a Classic server "one tool per store method" is the stated model
  (README:185); but the Code Mode side is held to the allowlist while Classic is not, so the
  two sides are not answering the same policy. (b) Checked that no scenario calls
  `cancel_order`/`restock` (`src/demo.html:289-327`), so those definitions only add input.
- Fix: build Classic definitions from the search reply's `allowed_methods` (or filter
  `icp.candid` output by them), re-measure, and update README.md:194-203.

### SW4-0-2 detail

- `src/policy.rs:131` declares `CANDID_OVERRIDES`; the only accessors are
  `set_candid_override` (`:233`) and `candid_override(key)` (`:240`, single-key lookup).
  `PolicyView` (`src/lib.rs:27-36`) and `get_policy` (`src/lib.rs:335-343`) carry limits,
  allowlist, tokens, `active_runs`, `pending_run_ids`: no overrides. `icp_code_mode.did:105`
  has only the setter.
- Serving paths read D5 first: `IcBridge::candid` (`src/bridge.rs:475`), `service`
  (`:441`), `cached_service` (`:207-213`, query and update search).
- Consequence: the controller's view of policy and the runtime's view differ by exactly
  the layer that takes precedence. The only way to learn an override exists is to run a
  script calling `icp.candid` and compare its text with the target's own metadata.
- Fix: add overrides (principal, byte length, hash) to `PolicyView`, or a
  `list_candid_overrides` query.

## Grid

| Row(s) | Result | Reason |
|---|---|---|
| C1, E5, E5a-E5c | known | Script paths: `icp.candid` (D5 text -> V2 -> V4 -> fetch) vs `icp.call` (`service`: V5/D5 parsed -> V2 -> `candid`). Same layers in the same order for one canister; only the allowlist placement differs (known L1-1) |
| C1a | holds | Feature subsets do not change the cache layers |
| C2, E20-E25, D8, V11 | holds | demo_store has no cache; `get_inventory.reserved`, `list_orders` and `sales_summary` read one `STORE`. Consistency of `reserved` with pending orders: maintained by `create_order`/`cancel_order` on the same struct (oversell issues known) |
| C3, C4, C8-C10, S1-S10 | n/a | Build inputs, no runtime cache |
| C5, E26, B17, W16 | **SW4-0-1** | Classic from `icp.candid` (all methods), Code Mode from search (allowlisted). Page-local `cache.store/defs/codeDefs`: one writer, buttons disabled (r2-7), holds |
| C6, E27 | holds | Writes the allowlist only |
| C7, AD9 | n/a | |
| C11, CL25 | n/a | |
| C12, E28, U16, O10, VF8 | n/a | Dev tooling; no shared cache between paths |
| C13, C14, A12, VF7, CL37 | n/a | Docs/tooling |
| C15, V7 | known | getrandom stream shared by all runs |
| A1-A11 | covered by rows below | |
| E1/E2, AD1 | holds | `apply` writes D1-D3; caches are heap and start empty after upgrade, so both search paths see the same empty V2/V4/V5 |
| E3, W2, U12, B2 | known | `inspect_message` vs method decode differences are known; token check uses the same `verify_token` |
| E4, E4a, E4b, CL2, CL3, CL28 | known | Query vs update search: V4 ignored in query (known), searches cap and budget (known). Same `scope(list_allowed())` and `cached_service` on both; TTL evaluated with each path's own `time()`, which can only make the query upgrade, not answer differently |
| E5b `run_search` per-canister choice | holds (minor) | After `halted`, an uncached canister with a fresh `DID_FAILED` entry reads "not fetched: policy …" instead of its remembered failure; the text tells the caller to retry with `canister_id`, which then shows the real error. No wrong data |
| E5b render after `bridge.candid` | latent, known (r3-1) | `candid` returns D5 text unparsed, `cached_service` drops a D5 parse error: unreachable while `set_candid_override` parses with the same parser |
| E5d, K7 | known | `Trapped` built from rolled-back state (known) |
| E6, AD2 | holds | Allowlist read live by `check_*`; `list_allowed()` snapshot per search (mid-search change known) |
| E7, AD3 | known | `evict_interface` clears V2, V4, V5 but not D5 (known) |
| E8, AD4, D1 | known | Snapshot vs live limits (known) |
| E9, E10, D3, AD5, AD6 | holds | Every reader uses `verify_token` on the live map |
| E11, AD7, D5, V5 | holds for serving; **SW4-0-2** for the admin view | All serving paths compare V5 text with D5 on each read, so a changed or cleared override is never served stale. `set_candid_override` does not clear V2/V4; D5 precedence makes that invisible while it exists, and after clearing, V2/V4 resume (same for every path) |
| E12, CL31, V1, V3 | known / **SW4-0-2** | `active_runs` vs `pending_run_ids` subsets known; overrides absent from the view |
| E13, E14, LIM5, W13, D4 | holds | Both read `AUDIT` directly |
| E15 | n/a | |
| E16-E19 | holds | All three runtimes route `candid`/`call` to the same `IcBridge` methods (`js.rs:393`, `python.rs:133`, `rhai_rt.rs:215`) |
| E19 Rhai `answers` replay cache | holds | Keyed by sequence and checked for equality on every replay (`rhai_rt.rs:65-83`); errors cached too, so a replay never sees a different answer |
| U1-U5, U14 | holds | Not cached |
| U6, U7 | n/a | |
| U8, W6 | holds | Decoded with the same `Service` that `icp.call` encoded with |
| U9 | holds | Failure text is the V4 entry for all readers |
| U10, W7 | known | Fetch path; parse cost known |
| U11 | see AD rows | |
| U13 | holds | |
| U15 | holds | `time()` drives both TTLs; consistent within one execution |
| O1 | holds | `reserve` then `service()`; consistent with `candid` |
| O2 | known | Fetch audit labelling known |
| O3, O4 | holds | |
| O5-O9 | n/a | No cache read |
| D2 | holds | |
| D6, D7 | n/a | |
| D9, D10 | n/a | Browser/disk only |
| V2 `DID_CACHE` | holds | Readers: `cached_service`, `service`, `candid`; all use `cache_get` with the same TTL filter. Coexisting fresh V2 + fresh V4 (a slower failing fetch landing after a succeeding one) is read cache-first by every path, so all answer from V2 |
| V4 `DID_FAILED` | known | Read by `candid` only; query search ignores it (known) |
| V5 | holds | See E11 |
| V6, V8, V9, V10 | known | Process-global memory/GC accounting known |
| K1-K12 | holds | Commit points do not choose a cache subset |
| LIM1-LIM4 | known | |
| LIM6-LIM8, LIM10, LIM11 | holds | Same constants on both routes (or update-only, documented) |
| LIM9 | known | |
| LIM12-LIM17 | holds | |
| LIM18 | holds | One TTL constant per layer, applied by its single getter |
| LIM19 | known | 512 KiB applied to both fetch and override; parse limits known |
| LIM20-LIM32 | n/a | Not cache-backed |
| W1, W3, W4, W5, W8-W12, W14, W15 | n/a | Work bounds, not cache subsets (known findings cover them) |
| F1-F16 | n/a | |
| B1-B14, B16 | holds | B16 search JSON and `icp.candid` text come from the same V2/D5 entry; B7 class unwrapping applies to the parsed form only, documented as a rendering choice |
| B15 | n/a | |
| VF1-VF6 | n/a | No cache subsets |
| AD8 | n/a | |
| CL1, CL4-CL27, CL29, CL30, CL32, CL34-CL36 | known or n/a | CL22/CL23 hold for every reader; CL6 known |
| CL33 | **SW4-0-1** | Restocking ratio depends on the Classic side counting two methods Code Mode refuses |
| T1-T12 | testability | No test compares the demo's Classic tool set with the allowlisted set, or checks that `get_policy` exposes every policy layer |
| History (register) | covered | Register classes L1, L6, L8, L16 re-checked against this pattern above |

## Not tested

- SW4-0-1 was computed from the page's code and the committed `.did`, and calibrated to the
  L16-r4 live figure; the page itself was not re-run against the live gateway with a filtered
  definition set.
- Tokenizer counts (the page and README use chars/4).
