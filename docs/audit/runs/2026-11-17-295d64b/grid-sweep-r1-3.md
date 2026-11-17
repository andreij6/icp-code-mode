# Variant sweep r1-3: stated order or precedence the code does not follow

Run `2026-11-17-295d64b`, Phase 6. Pattern: **"A sentence in the docs that states an order or
precedence the code does not follow."** Seed (known, not re-reported): README:493-496 lists
`set_candid_override` as the third interface source, while the code consults it first.

Method: grep for ordering words (`before`, `after`, `first`, `then`, `fallback`, `in order`,
`newest`, `oldest`, `precedence`, `prefer`, `wins`, `order`) in `README.md`, both `.did` files,
every comment under `src/`, `demo_store/src/`, `shims/`, `scripts/`, `icp.yaml`,
`.cargo/config.toml`, `src/demo.html` and `docs/per-customer-canisters.md`. Each hit was traced
to the code that implements the sequence. Excluded: `docs/per-customer-canisters-plan.md`,
`design/` (someone else's uncommitted work).

Evidence: R = reproduced natively in a private copy (`rsync` without `target/`,
`cargo test --lib sweep3`, shared `CARGO_TARGET_DIR`, copy deleted afterwards) and compared
with Node 26 for the ECMAScript reference behaviour; C = code path. Nothing live was touched.

## Candidate findings

| Id | Sev | Title | Evidence |
|---|---|---|---|
| SW3-1 | Medium | `SNAPSHOT_JS` says it applies the spec's order "toJSON, then the replacer, then the type rules"; it skips the spec's unwrap of boxed primitives, and an array replacer (key filter) is applied after the whole value was walked, not while walking | R |
| SW3-2 | Low | "Checked before every call" / "before each host request" (instruction budget): the check runs before the interface fetch, parse and argument encoding, and is not repeated before the send, so a message can go out after the budget is spent; the run then reports a policy error for a call that executed | C |

### SW3-1 detail

`src/runtimes/js.rs:48-49` (comment inside `SNAPSHOT_JS`): "applied here ... in the order the
spec gives: toJSON, then the replacer, then the type rules". ECMAScript `SerializeJSONProperty`
has a step between the replacer and the type rules: a `Number`, `String`, `Boolean` or `BigInt`
wrapper object is unwrapped to its primitive. `walk` (`js.rs:53-79`) goes from the replacer
straight to `typeof` checks, so wrappers are walked as plain objects. Second deviation:
`SerializeJSONObject` visits only the keys in the property list (array replacer); the wrapper
at `js.rs:85-88` runs `snapshot(v)` over every key first (getters, `toJSON`, node/byte/depth
limits) and lets native `JSON.stringify` drop keys afterwards.

Native probe (Boa, the shipped snapshot) vs Node:

| Script | Sandbox | Spec (Node) |
|---|---|---|
| `JSON.stringify(new Number(5))` | `"{}"` | `"5"` |
| `JSON.stringify(new String("ab"))` | `{"0":"a","1":"b"}` | `"ab"` |
| `JSON.stringify(new Boolean(false))` | `"{}"` | `"false"` |
| `JSON.stringify(Object(1n))` | `"{}"` | `TypeError` |
| `return {n: new Number(5), s: new String("ab")}` (tool result) | `{"n":{},"s":{"0":"a","1":"b"}}` | n/a (values 5, "ab") |
| getter on key `b`, `JSON.stringify(o, ["a"])` | getter runs once | getter never runs |
| `toJSON` on key `b`, `JSON.stringify(o, ["a"])` | `toJSON` runs once | never runs |
| 8 MiB string under key `big`, `JSON.stringify(o, ["a"])` | `RangeError: value holds more than 4194304 bytes of text` | `{"a":1}` |
| 200-deep array under key `deep`, `JSON.stringify(o, ["a"])` | `RangeError: value nested deeper than 128 levels` | `{"a":1}` |

The same snapshot feeds results, call arguments and `console.log`, so boxed primitives also
leave the sandbox as `{}` / index maps, silently, in tool results and in call arguments.
Fix: unwrap wrapper objects right after the replacer step; pass an array replacer into
`snapshot` and walk only the listed keys.

### SW3-2 detail

`src/policy.rs:27` and `icp_code_mode.did:73` (`max_instructions`: "Checked ... before every
call and after the run"), README:367 ("Checked before each host request"). In
`IcBridge::call` (`src/bridge.rs:510-531`) the budget is compared in `reserve` (line 515), then
come `self.service(&p).await` (on a cache miss: up to two fetch messages, then
`Service::parse` of up to 512 KiB in the reply continuation), `encode_args` (`json_to_idl`,
whose cost L3-3 measured in the billions) and the send at line 528, with no second comparison.
If the script is close to `max_instructions` when it calls, the call is sent after the budget
is spent. The next `check_budget` (JS `drive` loop, Rhai `on_progress`, Python VM budget) or
the after-run check in `run_script` (`src/lib.rs:226-231`) then turns the run into
`PolicyViolation` with "policy: instruction budget exceeded" and discards the reply, although
the target executed the call (for example `create_order`). README:318 says of policy errors
"No message is sent", so a client or AI model reading this error can take the call as refused
and retry it. The audit entry does list the message, so the record is correct. Possible overlap
with L3-3 (cost of `json_to_idl`); the root cause here is the placement of the check, and the
fix is a budget comparison immediately before `sent(...)` in `call` and `fetch_did`.

## Grid

Cells: **finding** (id above), **known** (already in the known list), **holds** (the stated
order is what the code does, with the reason), **n/a** (the row states no order).

### Claims (CL) and other order sentences found by the grep

| Row / sentence | Cell | Reason |
|---|---|---|
| README:493-496 interface sources in order (metadata, tmp_hack, override) | known | seed finding |
| `lib.rs:321-322` and `.did` `set_candid_override` comment, `policy.rs:130` `CANDID_OVERRIDES` comment ("for a target that does not expose ... / that expose no interface") | known | same root cause as the seed: the override is consulted first (`bridge.rs:209, 441, 475`) whether or not the target exposes an interface |
| `bridge.rs:205-206` `cached_service`: "an admin override, or the cache" | holds | doc order matches code (`bridge.rs:207-212`) |
| `bridge.rs:421-423` / README:469 "`canister_metadata` is tried first", then tmp_hack | holds | `fetch_did` order matches (`bridge.rs:426-436`); README's reason (local replica lacks it) and demo_store's `lib.rs:338-339` comment are the known L16-11 |
| CL6 README:365 allowlist "Checked before any message is sent, including the interface fetch" | holds / known | `reserve` precedes `service()` and `fetch_did`; not re-checked after the await is known |
| CL7 README:366 "Reserved before the await" | holds | `reserve` runs before `self.service(&p).await` (`bridge.rs:515-516`) and before `fetch_did` (`bridge.rs:486`) |
| CL8 README:367 "Checked before each host request, between JS rounds, and on every Rhai operation" | finding SW3-2; known | check precedes fetch/parse/encode, not the send (SW3-2); JS synchronous code and Python's 10B are known |
| `policy.rs:27`, `.did:73` `max_instructions` "before every call and after the run" | finding SW3-2 | as above; "after the run" holds (`lib.rs:226`, `lib.rs:151`) |
| CL15 / CL17 README:391-396, `bridge.rs:391-394` entry written as the system accepts the message; entry and message committed together, discarded together on a trap | holds | ic-cdk 0.20.3 `CallFuture::poll` (`call.rs:903-922`) calls `call_perform` on the first poll and returns `Pending` only on success; `on_first_pending` records then |
| README:383 / `.did` run_id "the id of the run's first entry" | holds | `append_audit` sets `run_id = id` on an entry without one (`policy.rs:251`); later entries copy `started` |
| README:388, `policy.rs:58-63` `Started` for the first message, `Call` for each later one | holds | `record_message` keys on `started` (`bridge.rs:397-418`); `join_all` polls in batch order, so the first accepted message is `Started` |
| README:409, `.did:5-7` `methods` "in order" | holds | pushed at acceptance (`bridge.rs:399`); `canisters` is a set but no order is claimed for it |
| README:407, `.did` `Trapped` lists messages and violations "up to the last message" | holds | memory rolls back to the end of the execution that sent the last message; violations recorded later in that same execution are also kept, a superset, not an omission. The known Trapped findings cover the omissions |
| CL19 README:411-412 entries-free requests | known | |
| CL20 README:414-426 pending / unknown / not in log | holds | order of commit vs trap matches K1/K7/K8 |
| README:441 "cached for one hour, then fetched again" | known | override shadows it (seed); transient re-fetch failure is known |
| CL23 README:446 ids canonicalised before the allowlist check, the cache and the override lookup | holds | `parse_principal` precedes every lookup in `candid` (`bridge.rs:473-477`) and `call` (`bridge.rs:514-515`); search parses with `Principal::from_text` (`search.rs:39`); admin endpoints take typed `Principal` |
| CL9 README:368 "five searches ... further ones get -32600"; `mcp.rs:390-391` | holds (update) / known (query) | update path refuses in batch order (`mcp.rs:392-397`); query path cap is known |
| CL16 README:375 `inspect_message` "before the canister is charged" | known | canister callers bypass it (known) |
| CL21 `.did` / README:428-439 `certified_reads` "every request goes through an update call" | known | auth failures are answered in the query before the certified check (`mcp.rs:347-352`); forging of query responses is the known certified_reads finding |
| CL28 README:497-499 skip-certification | known | same |
| CL32 `.did:95`, `lib.rs:346`, `policy.rs:260` "Newest first" | holds | `idx = len - (offset + i + 1)` (`policy.rs:265-267`) |
| CL35 / README:488-492 Rhai: stopped uncatchably, call made, answer cached, re-run from the top | holds | `request` suspends with `ErrorTerminated`; `run` performs, pushes into `answers` by sequence, replays (`rhai_rt.rs:61-79, 186-224`) |
| CL11 / `js.rs:48-49` snapshot order "toJSON, then the replacer, then the type rules"; `js.rs:84-85` "an array replacer (a key filter) is safe to pass through" | **finding SW3-1** | R |
| `js.rs:343` "Declared before the Context so it is dropped after it" | holds | `Context` lives in an inner block that ends before `drop(state)` (`js.rs:353-361`) |
| `js.rs:1-3` header: dispatch every pending request, settle, repeat | holds | `drive` (`js.rs:375-406`) |
| `python.rs:1-7` header: suspend, await, resume; `gather` dispatched together | holds / known | un-awaited `call_async` never sent is known |
| `mcp.rs:4-7` query answers read-only, upgrade for tools/call, then update | holds | cached searches also answered in query, which the README states |
| `lib.rs:119-120` run_search "fetch ... then render" | holds | loop then `render` (`lib.rs:129-150`); budget checked only after render is known |
| `lib.rs:202-203` RunSlot returned on drop in cleanup | holds / known | cleanup trap leak is known |
| `lib.rs:265` `inspect_message` "Runs ... before an ingress update call is accepted" | holds | platform order |
| `bridge.rs:24` deadline "before the IC's own 40B trap" | known | JS sync, Python native ops, encode/parse traps are known |
| `python.rs:24-28` 10B budget "well before the IC's 40B per-message trap" | known | |
| `demo_store/src/lib.rs:235`, `.did` `list_orders` "oldest first" | holds | orders only appended; seed `created_at = EPOCH + k*DAY` with EPOCH 2026-01-01, new orders use `time()` |
| `demo.html:196` "refused before any message is sent" | holds | `reserve` precedes `service()` |
| `demo.html:461` "the canister returns object keys sorted" | holds | `serde_json` has no `preserve_order` (no `indexmap` in its lockfile deps), so `Map` is a `BTreeMap` |
| `demo.html:87`, README:187, `mcp.rs:69/77/141` "search first, then the script" | n/a | advice to the user/model, not behaviour |
| `icp.yaml:18-22`, `scripts/demo-setup.sh`, README Quickstart steps | holds | setup order matches what each step needs (`demo-setup.sh` checks `demo_store` exists first) |
| `docs/per-customer-canisters.md` numbered factory steps | n/a | design idea, not built |
| README:74-77 diagram `HostBridge -> policy::check_call -> candid_bridge -> Call` and Layout `policy -> Candid fetch -> call -> decode` | holds / known | order matches `call`; "one entry per run" is known (L16-6) |
| README:456-458 trap "before any outbound message ... nothing is logged; after one ... Trapped" | holds | "outbound message" means a committed one; a trap in the same execution as a send rolls the send back too, consistent with README:395 |
| `candid_bridge.rs:136` key forms "By name, by decimal id, or as `_<hash>_`" | n/a for this pattern | lists accepted forms, not a precedence. Side note, not a candidate here: for a named field only the name matches (`candid_bridge.rs:138-140`), while variant tags accept `_<id>_` for named fields too (`candid_bridge.rs:285`) |
| `candid_bridge.rs:68-69` single-arg `[x]` precedence | known | L7-15 |
| CL1-CL5, CL10, CL12-CL14, CL22, CL24-CL27, CL29-CL31, CL33, CL34, CL36, CL37 | n/a | state no order or precedence (absolutes and numbers, swept by L16) |

### Other inventory sections

| Rows | Cell | Reason |
|---|---|---|
| C1-C15 | n/a / holds | build steps in `icp.yaml` run in the documented order (`cargo build`, `cp`, `ic-wasm metadata`, `shrink`); the stale-`cp` path is known |
| A1-A12 | n/a | actors carry no ordering claim |
| E1/E2 `apply` | n/a | no documented order between tokens, allowlist and limits |
| E3-E5, E5a-E5d | holds / known | see CL rows above |
| E6-E14 | known / holds | override precedence is the seed; `get_audit_logs` order holds |
| E16-E19 SDK | finding SW3-1 (E16/E17 JS values) / holds | Python and Rhai conversion docs state no order |
| E20-E25 demo_store | holds | `list_orders` order holds |
| U1-U16 | n/a | inputs; their ordering claims are covered under CL |
| O1, O2 | finding SW3-2 / holds | O2 fetch order holds |
| O3-O10 | holds / n/a | O3 commit order holds |
| D1-D10 | known / n/a | D5 precedence is the seed |
| V1-V11 | holds / known | V2/V5 lookup order: known (seed); V4 failed cache consulted after the cache and before `reserve`, which no doc contradicts |
| K1-K12 | holds | commit order as documented (see CL15/CL17/CL20 rows) |
| LIM1-LIM32 | holds / finding SW3-2 (LIM2) | LIM9/LIM10 order holds on the update path |
| W1-W16 | n/a | cost rows; no order claims |
| F1-F16 | n/a | |
| B1-B17 | finding SW3-1 (B8) / n/a | B5 single-arg precedence known (L7-15); B6 opt flattening known (#42) |
| VF1-VF8 | n/a | |
| AD1-AD9 | known / n/a | AD7 override precedence is the seed; AD4 `set_limits` reset is known |
| S1-S10 | n/a | |
| T1-T12 | n/a | no test asserts any of the stated orders above except T3 (first-pending hook) and T6 (paging) |

## Not tested

- SW3-2 was not reproduced: the native harness has no instruction counter (register #58), and
  the Live phase is not this task's. A live repro needs a script that reaches just under
  `max_instructions` before an `icp.call` whose fetch/parse/encode crosses it.
- The JS spec comparison used Node as the reference for ECMAScript behaviour; the sandbox side
  ran in Boa 0.22 through the shipped `SNAPSHOT_JS`.
