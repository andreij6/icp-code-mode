# Variant sweep r2-3: limits the dependency scales differently

Run `2026-11-17-295d64b`, Phase 6, round 2. Pattern (from the confirmed DECODING_QUOTA
finding, not re-reported):

> A limit tuned without regard to how the dependency scales it, refusing valid data far
> below the transport limit; an error message that hides the real cause.

Question asked of every inventory row: does the app set a number (size, count, depth,
iterations) that a dependency or conversion layer then counts in a different unit
(summed, multiplied, per-frame, per-byte), so valid input well under the transport or
documented limit is refused, and does the error say why?

Evidence: **R** = reproduced natively at `295d64b` in a private copy (probe tests added to
`src/runtimes/mod.rs` of the copy, `cargo test --lib sweep_probe`, debug profile, shared
`CARGO_TARGET_DIR`); replies were built by encoding real Candid and decoding through
`Service::decode_reply`, the production reply path. **C** = code path. The copy was deleted.

## New candidates

| Id | Sev | Candidate | Evidence |
|---|---|---|---|
| SW2-3-1 | Medium | Rhai's array/map/string limits are summed over the whole nested value by Rhai (`calc_data_sizes`), while the app set them as per-container caps (10,000 / 10,000 / 1 MiB). A demo_store-shaped `vec Order` reply of 1,112 single-item orders (39,008 B on the wire) fails with `Size of object map too large`; 1,111 passes. `vec Product` fails at 2,600 (76,925 B). `vec vec nat64` 101x100 (80,913 B) fails with `Size of array/BLOB too large`. JS and Python accept all of them. The call has already executed. No map in the reply has more than 6 keys, so the message points at the wrong thing. Script-built data is hit the same way (1,700 six-field records). | R |
| SW2-3-2 | Low | Boa's 1M loop limit is one counter per call frame for all its loops, never reset, and `String.prototype.repeat` charges one iteration per repetition. Refused: `'ab'.repeat(1100000).length` (no loop in the script); two 600k loops in one frame (the same loops in two functions pass); three passes over a 400,000-element reply (the reply decodes, at the decoding quota); 500k iterations before an `await` plus 600k after. After an await, the error is the generic "a runtime limit (loop iterations, recursion, stack) was hit after an await, or it awaits a promise that never settles". README:370 says "1M loop iterations". | R |
| SW2-3-3 | Low | Rhai `set_max_expr_depths(64, 32)` was meant for nesting, but Rhai counts each operator in a flat chain as one level. `x + x + ...` with 66 terms at top level, about 28 in a function, or 25 inside a for/if in a function, fails with `syntax error: Expression exceeds maximum complexity`, which does not name a depth. A 70-term `&&` chain fails at top level. All are tiny sources, far under the 100 KiB code limit. | R |
| SW2-3-4 | Low | `MAX_VALUE_NODES` (200k) counts a blob as one element per byte once `idl_to_json` turns it into a JSON array. A 250,000-byte blob reply decodes, but forwarding it as a call argument fails in JS and Python with `value has more than 200000 elements`. The inter-canister limit is 2 MiB, and Python `bytes` and Rhai blobs are counted as bytes, not elements. Python cannot convert the list (`bytes(list)` gives `TypeError` in Monty). JS can work around it with a hex string (500,004-byte argument sent). | R |
| SW2-3-5 | Low | The interface-fetch fallback hides the first failure. When `canister_metadata` rejects (for example a private `candid:service` section) or its reply does not decode, `fetch_did` (`src/bridge.rs:424-435`) discards that error and returns only the `__get_candid_interface_tmp_hack` error, which is cached in `DID_FAILED` for 5 minutes. This matches only the second half of the pattern (no limit involved). | C |

## Grid

| Row | Result | Note |
|---|---|---|
| C1 runtimes (C1a subsets) | finding SW2-3-1..4 | see above |
| C2 demo_store | holds (C) | args are typed; ic-cdk skipping quota 10,000 is charged 50x but these types have no skipped fields. Its growing `list_orders` is what triggers SW2-3-1 for Rhai callers |
| C3 web-time shim | holds for this pattern | instruction-to-Duration scaling is 1:1. Known budget findings not repeated |
| C4 vendored chrono | n/a | no limits |
| C5 demo page, C6, C7-C10, C12-C15 | n/a | no limit applied to runtime data. Note: Monty `MAX_NESTING_DEPTH` is 30 in debug builds (what tests run) and 200 in release (`monty-1.6.0/src/parse.rs:41-47`), so `x + ... ` with 30 terms is a `SyntaxError` in tests but not on chain; test/release divergence, not this pattern |
| C11 .did | n/a | |
| A1-A12 | n/a | actors; covered through entry points |
| E1/E2 InitArgs | holds (C) | typed decode, no app limit |
| E3 inspect_message | holds for this pattern | default `DecoderConfig` (no quota). The quota mismatch with the method is known |
| E4/E5 http_request(_update) | holds (C) | `HttpRequest` declares `certificate_version`, so nothing is skipped under ic-cdk's 10,000 skipping quota; `HttpUpdateRequest` has the 4 gateway fields |
| E5a run_script | finding SW2-3-1..4 | |
| E5b search | holds | no decode of replies, limits are app-level (40 methods). Known keyword/render findings not repeated |
| E5c/E5d callbacks | n/a | |
| E6-E14 admin | holds (C) | typed args; unbounded sizes are known |
| E16 JS SDK | finding SW2-3-2, SW2-3-4 | P4: `v.map(walk)` recursion over a 120-deep reply, direct recursion and returning it all pass, so recursion 256 / stack 10,240 hold at the depth replies can reach |
| E17 console | holds | snapshot bounds; known UTF-16 finding |
| E18 Python SDK | finding SW2-3-4 | recursion 200 vs reply depth 128: holds |
| E19 Rhai SDK | finding SW2-3-1, SW2-3-3 | |
| E20-E25 | n/a | demo_store has no app limits |
| U1-U4 HTTP | holds (C) | serde_json recursion 128 only affects nesting inside `arguments`, whose valid fields are flat strings |
| U5/U6 script source | finding SW2-3-3 (Rhai). JS and Python hold in release | Python's flat-chain limit is 200 in release (Monty's own constant, documented in the crate) |
| U7 script values | finding SW2-3-4 | known UTF-16 finding not repeated |
| U8 reply bytes | finding SW2-3-1 (Rhai), SW2-3-4 | the decoding-quota source finding is not repeated |
| U9 rejects | holds | passed through as text |
| U10 interface text | finding SW2-3-5 | the 512 KiB check is on bytes and its message is explicit (holds) |
| U11 admin inputs | holds | |
| U12 | holds | see E3 |
| U13 demo_store args | holds | |
| U14-U16 | n/a | |
| O1 calls | holds | args up to 2 MiB pass when given as a hex string (P7) |
| O2 interface fetch | finding SW2-3-5 | `canister_metadata` reply decoded with `decode_one` (no quota), so no hidden refusal there |
| O3-O10 | n/a | no limit is scaled by a dependency |
| D1-D10 | holds (C) | `Bound::Unbounded` Storables; no dependency-side size limit |
| V1-V11 | n/a | |
| K1-K12 | n/a | |
| LIM1-LIM4 | holds for this pattern | known findings not repeated |
| LIM5 | known | |
| LIM6 MAX_RESULT_BYTES | holds (C) | `truncate_escaped` counts the one escaping the text gets when embedded in the reply; the raw text's own backslashes are counted at 2 |
| LIM7, LIM8 | holds | |
| LIM9-LIM12 | n/a / known | |
| LIM13, LIM14 | known | |
| LIM15 | known (Python MemoryError) | |
| LIM16 | finding SW2-3-4 | |
| LIM17 | holds | |
| LIM18 | n/a | |
| LIM19 | holds | measured in bytes, explicit message |
| LIM20 | source finding | |
| LIM21 | known | |
| LIM22 | n/a | |
| LIM23 | holds | |
| LIM24 | finding SW2-3-2 (loop); recursion and stack hold (P4) | |
| LIM25 MAX_PROMISE_JOBS | holds (R) | 99,000 awaits of an async helper and 99,000 `await i` in one script all pass, so this is not reached at reply-sized loops |
| LIM26 | holds for this pattern | recursion 200 > reply depth 128; known memory/budget findings not repeated |
| LIM27 | finding SW2-3-1, SW2-3-3 | |
| LIM28-LIM30 | known | |
| LIM31, LIM32 | holds | |
| W1-W16 | covered by the LIM/U rows above | |
| F1-F16 | n/a | about traps, not refusals |
| B1-B4 | holds | |
| B5 json_to_idl | holds | hex blob path takes up to 2 MiB |
| B6 idl_to_json | finding SW2-3-4 (blob becomes per-byte elements) | |
| B7 | known (parser stack) | |
| B8 JS<->JSON | finding SW2-3-4 | |
| B9 Python<->JSON | finding SW2-3-4 (`bytes` counted as bytes, but replies arrive as lists) | |
| B10 Rhai<->JSON | finding SW2-3-1 | |
| B11 | holds | see LIM6 |
| B12-B17 | n/a | |
| VF1-VF8 | n/a | |
| AD1-AD9 | holds / known | |
| S1-S10 | n/a | dependency limits were examined through their rows above |
| CL10 | finding SW2-3-1, SW2-3-4 | the value limits stated "in all three languages" are applied in different units |
| CL11 | finding SW2-3-2 | "1M loop iterations" |
| CL13 | finding SW2-3-1, SW2-3-3 | "size limits on strings/arrays/maps" |
| other CL | n/a | |
| T1-T12 | gap | no test covers a Rhai reply of many small records, a top-level JS script doing more than 1M iterations in total, a long operator chain in Rhai, or forwarding a blob reply |
