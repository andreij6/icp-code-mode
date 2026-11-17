# Variant sweep r4-5: altered data returned as success; a smaller limit applied downstream by truncation

Run `2026-11-17-295d64b`, Phase 6, round 4. Pattern:

> Partial or altered data returned as success; a second, smaller limit applied downstream by
> truncation instead of refusal (same class as register #2/#33).

Generalised for the sweep: (a) a stage downstream of a limit that admitted a value cuts,
clamps or replaces it instead of refusing; (b) a conversion that substitutes a default
(`""`, `false`, `[]`, `null`, a placeholder) for data it could not read, and the run still
reports success.

Method: `grep` for `truncate`, `truncate_escaped`, `.take(`, `min(`, `saturating_`,
`unwrap_or_default`, `unwrap_or(`, `filter_map`, `slice(` over `src/`, `demo_store/src/`,
`src/demo.html`, `video/`, `scripts/`; read every hit and every conversion in B1-B17.
Native probes in a private copy (`/private/tmp/claude-501/sweep-r4-5.*`, repo copied
without `target/`, `design/`, the uncommitted plan doc and `.env`; shared
`CARGO_TARGET_DIR=/private/tmp/claude-501/audit-target`; `cargo test --lib` /
`cargo test -p demo_store` with probe tests added to the copy only). Nothing was deployed;
the local network was not touched. Copy deleted afterwards.

Evidence: R = reproduced (native test), C = code path. "known" = in the do-not-re-report
list given to this sweep.

## New candidates

| Id | Severity | Candidate | Evidence |
|---|---|---|---|
| SW4-5-1 | Low | Python `to_json` dispatches on `ObjectRef::type_name()`, which for a script-defined class instance is the class's own name (`monty-types graph.rs:572-575`). An instance of a class named `str`, `bool`, `float`, `list`, `tuple`, `set`, `frozenset` or `NoneType` takes that builtin's arm, whose accessor returns `None`, and `unwrap_or_default()` / `map_or(Null)` substitutes `""`, `false`, `[]` or `null` (`src/runtimes/python.rs:67-88`). The attributes are dropped, the run is `Ok`, and the same substitution is sent as `icp.call` arguments | R: `class str:` ... `{'v': str(), 'n': 1}` -> `{"v":"","n":1}`; `bool` -> `false`; `float` -> `null`; `list`/`tuple`/`set` -> `[]`; `NoneType` -> `null`; `@dataclass class float: amount: int` -> `null`. As call args: `{'flag': bool()}` sent as `"flag": false`, `str()` as `""`, `list()` as `[]`, `float()` as `null`, result `Ok("done")`. Control: `class Point` -> repr string (that is known L7-6) |
| SW4-5-2 | Low | demo_store `restock(product_id, quantity)` adds with `on_hand.saturating_add(quantity)` (`demo_store/src/lib.rs:334`) and returns `Ok(Inventory)`: once `on_hand + quantity` passes `u32::MAX` the extra units are dropped silently, and every later restock of that product is a no-op that still reports success. A clamp in place of the `Err(String)` the method already has | R: `restock(1, u32::MAX)` -> `on_hand 4294967295`; then `restock(1, 1000)` -> `Ok`, `on_hand 4294967295` (unchanged) |
| SW4-5-3 | Low | JS `display` (console.log/info/warn/error/debug, `print`) formats Symbols with `v.to_string(ctx)...unwrap_or_default()` (`src/runtimes/js.rs:218-219`). `ToString(Symbol)` throws per spec, so the argument prints as an empty string with no error: printed output is altered while the run succeeds | R: `console.log("a", Symbol("x"), "b"); print(Symbol.iterator)` -> logs `["a  b", ""]`, result `Ok(1)` |

## Grid

### Components, actors, entry points

| Row | Verdict | Reason |
|---|---|---|
| C1, C1a | see rows below | |
| C2 demo_store | **SW4-5-2** | `restock` clamps; `cancel_order` `saturating_sub` hides the known `reserved` wrap (same root cause as the known oversell, per r3-5); `create_order` `total` u64 cannot wrap within one ingress (max 12,900 cents x u32 x ~174k lines < 2^64); `sales_summary` wrap known |
| C3 web-time shim | holds | Instant/Duration arithmetic saturating, not a data path |
| C4 vendored chrono | n/a | no conversion of script or reply data reaches it for this pattern |
| C5 demo page | holds | `pretty()` falls back to raw text; `isError` shown; `slice(0, 40)` of call args is display only (L17-r1 holds); comparison runner parses results and fails loudly (`JSON.parse`) on truncated text |
| C6 demo-setup.sh, C7 icp.yaml, C8-C10 build flags | n/a / known | release overflow-check flag known; no data truncation |
| C11 .did files | holds | `get_audit_logs` cap of 100 is documented (`.did`, `lib.rs:344`) |
| C12 video tooling | holds | only `slice` of argv and of an error message (`tts.mjs:56`, dev only) |
| C13, C14 | n/a | docs/tooling |
| C15 Wasm exports | n/a | |
| A1-A12 | n/a | actors; covered through the rows they reach |
| E1/E2 init/post_upgrade | holds (pattern) | token hash refused (trap) when malformed; replacement of omitted opt limits is known |
| E3 inspect_message | holds | accept/reject only |
| E4/E4a/E4b query route | known | search result through `tool_result` 1 MB cut = L7-1; oversized reply -> single `id:null` error = known |
| E5/E5a run_script | known | L7-1 (result), seed (logs 64 KiB vs 250,000 escaped), audit `error` 4 KiB documented |
| E5b run_search | holds / known | `halted` canisters carry a per-entry `error`; `MAX_METHODS` cut carries `omitted_methods` + `note` (#41); size cut = L7-1 |
| E5c/E5d callbacks | n/a | no conversion |
| E6-E11 admin updates | holds | inputs stored as given or refused (`MAX_DID_BYTES`, token hash format); `set_limits` opt reset known |
| E12 get_policy | holds | whole lists returned; token hash prefix by design and documented |
| E13 get_audit_logs | holds | `limit.min(100)` documented; `filter_map(l.get)` skips nothing for valid indices |
| E14 | n/a | |
| E16 JS host fns | known / **SW4-5-3** | args reparse to `null` at 128 levels = known L7-2; strings unbudgeted = known; Symbol display = SW4-5-3 |
| E17 JS console | **SW4-5-3**; known | per-line cut + LogBuf discard known |
| E18 Python host fns | **SW4-5-1**; known | Monty export guard markers, py_repr fallback, dict-key collisions known |
| E19 Rhai host fns | holds | closure-shared values convert intact (R: shared array, map, string, blob, nested, and as call args); engine size limits refuse (known) |
| E20-E23 demo_store | **SW4-5-2** (E23); known (E21, E22, E20 sales_summary) | |
| E24, E25 | n/a | |
| E26 demo page buttons | holds | see C5 |
| E27, E28 | n/a | |

### Inputs, effects, state

| Row | Verdict | Reason |
|---|---|---|
| U1/U2 method, url, headers | holds | path split/trim, bearer `trim`: normalisation, not truncation |
| U3/U4 body, id, method, params | known | id alteration L7-9; rpc error text cut to 300 B is on an error path |
| U5 tool args | known | non-string `query`/`canister_id` are refused (`opt_string`); schema laxness known |
| U6 source | holds | > 100 KiB refused |
| U7 script values | **SW4-5-1**, **SW4-5-3**; known | |
| U8 reply bytes | known | quota refusal, depth refusal, opt-to-null on stale interface known |
| U9 reject text | holds (pattern) | error path; audit 4 KiB documented |
| U10 interface text | holds | > 512 KiB refused; non-UTF-8 metadata refused (`bridge.rs:428`) |
| U11 admin inputs | holds | |
| U12 | holds | |
| U13 demo_store args | **SW4-5-2** | |
| U14 demo page | holds | |
| U15 platform values | known | block time truncation to seconds (Python) known |
| U16 | n/a | |
| O1 script calls | **SW4-5-1** (Python args), known (JS L7-2, Python export guard) | |
| O2 interface fetch | holds | refused on size/UTF-8 |
| O3 audit writes | holds / known | `error` and `violations` caps documented with markers; marker overshoot known |
| O4-O8 | holds / n/a | |
| O9 demo_store writes | **SW4-5-2** | |
| O10 | n/a | |
| D1-D5 | holds | stored as accepted, read whole |
| D6-D10 | n/a | |
| V1-V11 | n/a / holds | caches hold whole values; V7 known |
| K1-K12 | n/a | commit points; no conversion |

### Limits

| Row | Verdict | Reason |
|---|---|---|
| LIM1, LIM2, LIM12, LIM23-LIM27 | holds | refusals |
| LIM3 timeout | holds | > 300 s clamped by the IC, documented in README and `.did` |
| LIM4 | n/a | |
| LIM5 audit page | holds | documented cap |
| LIM6 result 1 MB | known (L7-1) | |
| LIM7 reply 1.9 MB | known | |
| LIM8-LIM10 | holds | refusals |
| LIM11 rpc error 300 B | holds | error path |
| LIM13/LIM14 audit caps | holds / known | documented, marked; marker overshoot known |
| LIM15 logs | known (seed, LogBuf discard, Python MemoryError) | |
| LIM16 value limits | known (Monty export guard, JS reparse depth) | JS/Rhai refuse |
| LIM17 method length | holds | refused |
| LIM18 TTLs | n/a | |
| LIM19 interface size | holds | refused |
| LIM20 decoding quota | known | |
| LIM21 depth | known (Candid vs JSON levels) | |
| LIM22 safe int | holds | documented string form |
| LIM28-LIM30 | n/a | |
| LIM31 principal 80 B | holds | error text |
| LIM32 search methods | holds | counted and noted |

### Work, failures, conversions, flows, admin, supply chain

| Row | Verdict | Reason |
|---|---|---|
| W1-W16 | n/a for this pattern | cost rows; W3/W12 output size = L7-1 |
| F1-F10, F12-F15 | n/a | aborts, not silent alteration |
| F11 casts | known | float32 saturation (L7-5); `len as u32` cannot exceed u32 |
| F16 demo_store arithmetic | known / **SW4-5-2** | |
| B1, B2, B3, B4, B7, B12, B13, B15 | holds | |
| B5 JSON -> Candid | known | L7-3, L7-5, L7-11, L7-12, L7-13; `num_text` refuses non-integral and >= 1e19 |
| B6 Candid -> JSON | known / holds | opt flattening accepted (#42); L7-14 |
| B8 JS value -> JSON | known / **SW4-5-3** | |
| B9 Python value -> JSON | **SW4-5-1**; known (L7-6, L7-7, L7-17, export guard) | |
| B10 Rhai | holds (R, closure-shared values) / known | |
| B11 tool result | known (L7-1, L7-16, seed) | |
| B14 time | known | |
| B16 search JSON | holds / known | |
| B17 demo DOM | holds | display only |
| VF1-VF8 | n/a | no value conversion; VF6 demo_store = **SW4-5-2** |
| AD1-AD9 | holds | AD4 opt reset known; timeout clamp documented |
| S1-S10 | n/a | |

### Claims and tests

| Row | Verdict | Reason |
|---|---|---|
| CL10 values limited in all three languages | **SW4-5-1** relevant | the limit holds, but Python can emit a default instead of the value |
| CL14 output caps | known (seed, L7-1) | |
| CL29 "identical semantics in every language" | **SW4-5-1** | same class shape gives an object in JS (`{x:41,...}` via snapshot) and `""`/`false`/`[]` in Python |
| other CL rows | n/a for this pattern | |
| T9 Python tests | no test covers user classes in `to_json` | |
| T12 demo_store test | no test covers restock overflow | |
| T8 JS tests | no test covers non-string primitives in console output | |

## Refuted / checked and holding

- Rhai closure-shared (`Dynamic::Shared`) values: `is_array`/`into_array`/`cast::<Map>`/
  `into_immutable_string`/`into_blob` read through the share; results and call args were
  intact (R).
- demo_store `create_order` order total: cannot wrap within one ingress message (C).
- `get_audit_logs` silently returning at most 100: documented in `.did` and `lib.rs`.
- `call_timeout_seconds` > 300 clamped: documented.

## Not tested

- SW4-5-1 on the deployed canister (native probe only; the conversion code is the same on
  wasm32). SW4-5-2 and SW4-5-3 likewise native only.
