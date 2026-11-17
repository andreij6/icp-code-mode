# Grid: L7 Data fidelity, round 3 (stale or overridden interface vs. Candid opt-subtyping)

Run `2026-11-17-295d64b`. Reviewer: L7 lens agent, round 3. Question: can a value the target
actually sent (or the script actually passed) reach the other side as `null` or missing while
the run is audited `Ok`, and does the script get any signal?

Method: private copy of the tree (no `target/`, no `design/`, no `.env`), shared
`CARGO_TARGET_DIR`, probe module `src/audit_l7r3.rs` in the copy only (not committed),
`cargo test --lib audit_l7r3 -- --nocapture`. Each probe encodes a value under the target's
NEW type with candid (`IDLArgs::to_bytes_with_types`), checks the new type decodes it back
exactly, then decodes the same bytes with the app's `Service::decode_reply` under the OLD
(cached or override) type. Evidence: **R** = reproduced natively, **C** = code path
unambiguous. Candidate finding: **L7r3-1**.

Probe results (all R, native):

| Probe | Target's new type vs. app's cached type | Reply the target sent | What the app returns |
|---|---|---|---|
| P1 | `info : opt record { amount : nat }` -> `amount : text` | `{"id":1,"info":{"amount":"12 ICP"}}` | `Ok({"id":1,"info":null})` |
| P1c control | same change, not under `opt` | same | `Err("candid decoding failed ...")` |
| P2 | `get_customer : (nat64) -> (opt Customer)`, `Tier` gains `Platinum` | customer 7, tier `Platinum` | `Ok(null)` (customer "does not exist") |
| P2 | same, tier `Gold` | customer 7, tier `Gold` | correct object (loss is value-dependent) |
| P2c control | `list : () -> (vec Customer)` | `[customer 7, Platinum]` | `Err(...)` |
| P2b | `status : opt variant { Pending; Done }` gains `Refunded : nat` | `{"id":3,"status":{"Refunded":500}}` | `Ok({"id":3,"status":null})` |
| P2v | `vec opt variant { A; B }` gains `C` | `[A, C, B]` | `Ok([A, null, B])` (element-wise) |
| P3 | `opt record { a }` gains required `b : text` | `{"a":1,"b":"new"}` | `Ok({"a":1})`, `b` dropped |
| P3r | cached type requires `b`, target dropped it | `{"a":1}` | `Ok(null)`, `a` lost too |
| P3n | `meta : opt record { owner; memo : nat64 }` -> `memo : text` | `{"id":9,"meta":{...}}` | `Ok({"id":9,"meta":null})`, `owner` lost too |
| P4 | lying interface: `(record {id;name})` declared as `(opt record {id;name})` | `{"id":1,"name":"x"}` | correct object |
| P4 | lying interface: declared `(opt record { id : text })` | same | `Ok(null)` |
| P4 | lying interface: declared `(opt nat)` | same | `Ok(null)` |
| P4c control | honest wrong type, no `opt` wrap | same | `Err(...)` |
| P5 | `() -> (nat)` cached, target returns `(nat, text)` | `[5,"lost"]` | `Ok(5)`, second value dropped |
| P5b | `(nat, opt text)` cached, target returns `(nat, nat)` | `[5,6]` | `Ok([5,null])` |
| P6 | P2 through a full run, JS / Python / Rhai, bridge = P2 trip | - | all three `Ok("no such customer")`, logs `[]` |
| P7 | outbound: cached arg `memo : opt nat64`, target now `memo : opt blob` | script passes `memo: 42` | target decodes `memo = null` and would execute the update |

| Row | Cell | Reason / evidence |
|---|---|---|
| U8 Candid reply bytes from targets | **finding** L7r3-1 | `decode_reply` (`src/candid_bridge.rs:96-110`) decodes typed against `func.rets` from whatever interface `service()` returned, with skipping on and no check that the wire types match the expected types. Candid's rule (`candid-0.10.40/src/de.rs:1156-1174`, `recoverable_visit_some` :801-842) turns any value under `opt` that does not fit the expected type into `None`; extra fields and extra return values are skipped. Result: real reply data becomes `null` or disappears, returned as `Ok` (R, P1-P5). Outside `opt` the same mismatch is a loud error (R, P1c, P2c, P4c), so the loss is exactly where the README says `null` means "absent" ("Opt is a value or `null`", README.md:356) |
| B6 Candid -> JSON (`idl_to_json`) | **finding** L7r3-1 (via U8) | `idl_to_json` maps `IDLValue::None` to `null` (`src/candid_bridge.rs:331`), identical to a real `null`; the coercion has already happened in candid and leaves no marker, so nothing downstream can tell "absent" from "did not fit" (R, P6: script result and logs identical to a genuine missing customer). Distinct from #42 (`opt opt`) and from L7-r1's L7-14 |
| B7 `.did` -> TypeEnv | holds (parse), feeds L7r3-1 | Parsing is correct and errors loudly on invalid text. Reversing round 1's dismissal: the stale type is produced by the app's own cache (V2) and override precedence (V5/D5), and the app chose typed decoding with no wire-type check; candid provides `subtype_with_config(OptReport::Error, ..)` (`types/subtype.rs:76-86`) that the app does not use. So the loss is app behaviour, not only spec behaviour |
| B5 JSON -> Candid (outbound, same stale type) | **finding** L7r3-1 (outbound symptom) | Arguments are encoded under the cached type (`src/bridge.rs:516-517`). If the target changed the type inside an `opt` field, the target's own decoder turns the script's value into `null` and the method executes (R, P7: `memo: 42` arrives as `memo = null`). The app cannot see this after the fact; only a fresh interface prevents it |
| V2 `DID_CACHE`, TTL 1 h | **finding** L7r3-1 (stale-type source) | `cache_get` (`src/bridge.rs:284-289`) checks only age; nothing ties the entry to the target's module (no module hash, no re-fetch on decode failure or on suspicion). A target upgraded inside the hour is called and decoded with its old interface for the rest of the hour (C). The `.did` comment and README:441-443 present the TTL as the freshness mechanism |
| LIM18 `DID_CACHE_TTL_NS` | **finding** L7r3-1 | Same as V2: the 1 h bound is the window of silent loss, not a correctness bound (C) |
| D5 `CANDID_OVERRIDES` / V5 `OVERRIDES` | **finding** L7r3-1 (amplifier; precedence itself already reported) | An override shadows the target's real interface indefinitely (`override_service`, `src/bridge.rs:216-227`, used first at :439-443; already reported, not repeated). New here: a wrong override whose returns are wrapped in `opt` converts every reply mismatch into `null` instead of an error (R, P4), and a stale override has the P1-P3 effect with no TTL at all (C). No validation of an override against the target's own interface exists (`src/lib.rs:324-333` only parses) |
| A6 Allowlisted target | **finding** L7r3-1 (trigger) | Trigger is an ordinary target upgrade that adds a variant tag or changes a field inside `opt`: routine Candid evolution (adding a variant case to an output type is common), no hostile target needed |
| CL22 "cached for one hour, then fetched again, so a target's interface change is picked up" | **finding** L7r3-1 (claim incomplete) | True as stated, but the README and tool descriptions never say that within the hour, or forever under an override, reply values under `opt` that no longer fit become `null` and new fields/return values are dropped. The code comment at `candid_bridge.rs:99` ("Skipping covers reply fields newer than our copy") shows the skip was intended; the opt-to-null effect is not mentioned anywhere |
| I8 Fidelity invariant | **violated** by L7r3-1 | Data is silently dropped or reinterpreted as `null` while reported as success, in all three runtimes (R, P6), and in the outbound direction (R, P7) |
| O1 inter-canister call | **finding** L7r3-1 (P7) | The effect is sent with a field the script set but the target received as `null`; the audit log records `Call`/`Ok` (C: `record_message` logs the send, closing entry from the script's `Ok`) |
| E5 tools/call (script) and audit `Ok` | **finding** L7r3-1 | The run returns `Ok` and the closing entry is `Ok`; no log line, warning or field marks the coercion (R natively for the result; audit status by code path `src/lib.rs:236-260`) |
| E4 search | not applicable to the loss | Search shows signatures only, no reply values. It does show the stale signature for up to 1 h (C), which is the same source, not a separate loss |

## Candidate finding L7r3-1

- **Title:** A stale or overridden interface makes Candid's opt rule turn real reply data into
  `null` (and drop new fields and return values), and the run is audited `Ok` with no signal.
- **Severity proposed:** High (returns wrong data as if it were right: an existing customer is
  reported as absent, P2/P6). A verifier may argue Medium because the window is 1 h for the
  cache; the override path has no window.
- **Evidence:** reproduced natively (P1-P7 above). Live repetition not done (see below).
- **Root cause:** the app decodes with a cached type whose freshness is judged only by age and
  never checks the reply's wire types against it; candid's `opt` rule hides the mismatch.
- **Fix (proposed, not applied):** before or after typed decoding, check each reply type
  against `func.rets` with `subtype_with_config(OptReport::Error, ..)` (or decode untyped and
  compare); on a mismatch, evict the cache entry, re-fetch once and retry the decode, and if it
  still fails (or an override is in force) return an error naming the field. Key `DID_CACHE`
  to the target's module hash (`canister_info` is callable by any canister) so an upgraded
  target invalidates the entry before the next call, which also covers the outbound (P7)
  direction. Document the behaviour next to CL22.
- **Regression test:** P1, P2, P3r, P4 and P7 as unit tests on `Service` (encode under new,
  decode under old must be an error or a re-fetch, never `null`).

## Not tested

- **Live repetition (upgrade demo_store within the cache hour):** not run. This task is
  Phases 3-4, and the run's rules allow deploys, upgrades and wasm builds only in the Live
  phase; the shared local network is also in use by other agents, so swapping demo_store's
  interface mid-run could corrupt their results. The mechanism is the same native code
  (`decode_reply`, `idl_to_json`); the 1 h window is from code (`cache_get`). A Live-phase
  repro: allowlist demo_store, call `get_customer` once (fills the cache), upgrade demo_store
  with `Tier` gaining `Platinum` and one customer set to it, call `get_customer` for that
  customer within the hour: expected `null`, status `Ok`; then restore demo_store.
- Behaviour of candid's `OptReport::Warning` path at runtime (no logger is wired in the app).
- Whether `canister_metadata` / `canister_info` are available on the local replica for the
  module-hash fix (not needed to establish the finding).
