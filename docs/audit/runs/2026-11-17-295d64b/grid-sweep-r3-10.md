# Variant sweep r3-10: engine limits applied to host-supplied data in one runtime only

Run `2026-11-17-295d64b`, Phase 6, round 3. Source finding (known, not re-reported): Rhai
applies its engine size limits (`set_max_string_size(1 << 20)`, `set_max_array_size(10_000)`,
`set_max_map_size(10_000)`, summed over the whole value) to replies the host hands it, after
the call has executed, while JS and Python bound only data the script produces.

Pattern, stated generally: **a sibling runtime applies its own engine limits to host-supplied
data, while the other runtimes bound only script-produced data.**

For each row the question is: does host-supplied data (replies, interface text, error
strings, principal, time) enter a script engine here, and does that engine apply a limit of
its own that the sibling engines do not apply to the same data?

Evidence key: **R** = reproduced here (native differential test, below); **CP** = code path
unambiguous; **known** = already reported, not re-reported.

## Result

**No new candidate.** Every host-to-engine path was enumerated (JS `JsValue::from_json` and
`JsNativeError` messages; Python `from_json` → `MontyObject` and `MontyException` messages;
Rhai `rhai::serde::to_dynamic` and `ErrorRuntime` strings; `icp.candid` text; principal;
block time). The only engine limits that bite host data are the Rhai size limits already
reported. Engine limits that could in principle apply to host data were tested and hold:

| Probe (same mock reply, all three runtimes) | JS | Python | Rhai |
|---|---|---|---|
| Nested arrays / objects of depth 100, 120, 126, 127, 128, returned as-is | Ok | Ok | Ok |
| Same, only bound to a variable | Ok | Ok | Ok |
| Same, printed (`console.log` / `print`) | Ok | Ok | Ok |
| Same, received inside script recursion (JS/Py depth 150, Rhai 25 of 32 levels) | Ok | Ok | Ok |
| Reply string of 1.2 MB UTF-8 (600k UTF-16 units) | Ok | Ok | refused: "Length of string too large" (**known**) |
| `icp.candid` text of 500,000 bytes | Ok | Ok | Ok |
| Reply record with `__proto__` / `constructor` labels | own keys kept | kept | kept |

So Boa's recursion (256), stack-size (10,240) and loop (1M) limits are not charged by
`JsValue::from_json` or by the snapshot walk of a reply at the 128-level value depth, and
Monty's recursion limit (200) is not charged when it imports a reply, even with 150 script
frames already on the stack.

Repro: test module `src/runtimes/sweep.rs` in a private copy (a `DataBridge` whose `call`
returns a fixed JSON value and whose `candid` returns 500,000 bytes), run with
`cargo test --lib sweep -- --nocapture` against the shared target dir. The copy was deleted
afterwards.

## Cells

### Components (C)

| Row | Cell |
|---|---|
| C1 icp_code_mode | **known** (Rhai size limits on replies). JS and Python checked: R, table above |
| C1a feature subsets | holds: each subset compiles the same runtime modules, so a single-language worker has the same host-data handling as the full build; the Rhai-only worker inherits the **known** Rhai refusal |
| C2 demo_store | n/a: no script engine |
| C3 web-time shim | n/a: supplies time, not host data subject to engine size limits |
| C4 vendored chrono | n/a: Monty date arithmetic on a fixed block time (`now_ms()/1000`); no engine limit on it |
| C5 demo page | n/a: browser, no sandbox engine limits on server replies (regex cost **known**) |
| C6-C15 | n/a: no script engine |

### Actors (A)

| Row | Cell |
|---|---|
| A1, A2, A5, A8-A12 | n/a: do not supply data into a script engine |
| A3 token holder | holds for JS/Python (R); **known** for Rhai |
| A4 AI model | n/a |
| A6 allowlisted target | the only supplier of reply and interface data. Depth, width and text probes above: only the **known** Rhai limits refuse |
| A7 management canister | holds: supplies interface text (`canister_metadata`), bounded by `MAX_DID_BYTES` 512 KiB before any engine sees it; 500,000-byte text accepted by all three (R) |

### Entry points (E)

| Row | Cell |
|---|---|
| E1-E4, E6-E15, E20-E28 | n/a: no script engine receives host data |
| E5, E5a | covered by E16-E19 |
| E5b search | n/a: no engine; `MAX_METHODS` is a display cap applied to every caller alike |
| E5c reply callbacks | holds: hand the reply to the runtime's converter (E16-E19) |
| E5d cleanup | n/a |
| E16 JS host fns | holds (R): `JsValue::from_json` is native, charges none of Boa's runtime limits; host errors become `JsNativeError::error().with_message(msg)` with no size check; `icp.principal` short |
| E17 JS console | n/a for host data (prints script values; formatting cost **known**) |
| E18 Python host fns | holds (R): `from_json` → `MontyObject`; import does not consult `max_recursion_depth`. `max_memory` would count an imported reply, but one reply is quota-bounded: decoding cost is ×50 untyped and label names are charged by length (`candid-0.10.40/src/de.rs:1491`), so one reply yields well under 64 MiB of Monty heap (CP). Holding many replies under 64 MiB is a script choice, not this pattern |
| E19 Rhai host fns | **known** for `to_dynamic` of replies (`src/runtimes/rhai_rt.rs:73`). `icp::candid` text: holds, `MAX_DID_BYTES` (512 KiB) < `max_string_size` (1 MiB, bytes) (R at 500,000 B). Host error strings: holds, `ErrorRuntime(e.into())` carries no size check and bridge errors are bounded (`set_full_error_message(false)`, `src/candid_bridge.rs:102`) |

### Untrusted inputs (U)

| Row | Cell |
|---|---|
| U1-U7, U11-U16 | n/a: not host data entering an engine (U6/U7 are script-produced; their bounds are shared `ValueBudget` / snapshot code) |
| U8 reply bytes | **known** (Rhai). JS/Python R |
| U9 reject/error text | holds: passed as an error message in all three; no engine size limit applied |
| U10 interface text | holds: 512 KiB cap precedes every engine; Rhai's 1 MiB string limit is above it |

### Outbound effects (O)

| Row | Cell |
|---|---|
| O1 | **known**: the Rhai refusal comes after O1 executed |
| O2-O10 | n/a |

### Durable state (D), volatile state (V), commit points (K)

| Row | Cell |
|---|---|
| D1-D10 | n/a: not passed into engines |
| V2/V5 DID caches | holds: cached/override text reaches all three through `bridge.candid`, same 512 KiB bound |
| V6 Monty memory atomics | holds: see E18 (memory limit not reachable by one reply) |
| V1, V3, V4, V7-V11 | n/a |
| K2-K4 | holds: each runtime converts the reply after the await; only Rhai refuses (**known**) |
| K1, K5-K12 | n/a |

### Limits (LIM)

| Row | Cell |
|---|---|
| LIM16 value limits | holds as a shared app limit: applied to script-produced values in all three, and to replies only when the script passes them back (blob-element cost **known**) |
| LIM21 depth 128 | holds: a 128-deep reply is accepted and returned by all three (R). Off-by-one at 129 **known** |
| LIM24 JS loop/recursion/stack | holds: not charged by host data (R, including 150 frames deep) |
| LIM25 JS rounds/pending | n/a: counts requests, not data |
| LIM26 Python recursion/memory/suspensions | holds (R for recursion; CP for memory, see E18) |
| LIM27 Rhai string/array/map | **known** |
| LIM19 `MAX_DID_BYTES` | holds: same bound before every engine |
| LIM20 `DECODING_QUOTA` | shared host-side limit, same for all three (its size **known**) |
| LIM1-LIM15, LIM17, LIM18, LIM22, LIM23, LIM28-LIM32 | n/a: not engine limits on host data |

### Unbounded work (W), failure sites (F)

| Row | Cell |
|---|---|
| W6 decode | shared, before any engine |
| W8 JS `from_json` | holds: native, unlimited, no refusal |
| W10 Python `from_json` | holds |
| W11 Rhai `to_dynamic` per replay | **known** (refusal; per-replay size check cost under accepted Rhai O(n²)) |
| W1-W5, W7, W9, W12-W16 | n/a |
| F12/F13 | n/a for this pattern (no engine limit misapplied; native stack/OOM **known**) |
| F1-F11, F14-F16 | n/a |

### Boundary conversions (B)

| Row | Cell |
|---|---|
| B6 Candid → JSON | shared; produces ints ≤ 2^53-1 as numbers and larger as strings, so no runtime-specific integer-range refusal (Rhai i64, Python bigint, JS f64 all accept) (CP, `src/candid_bridge.rs` LIM22) |
| B8 JSON → JS | holds (R) |
| B9 JSON → Python | holds (R) |
| B10 JSON → Rhai | **known** |
| B14 time | holds: fixed time given to each engine; no engine limit (second-truncation **known**) |
| B1-B5, B7, B11-B13, B15-B17 | n/a |

### Value flows (VF), admin surface (AD), supply chain (S)

| Row | Cell |
|---|---|
| VF1/VF4 | **known**: an executed call whose reply Rhai refuses is still paid and its effect stands |
| VF2, VF3, VF5-VF9 | n/a |
| AD7 overrides | holds: 512 KiB bound applies before all engines |
| AD1-AD6, AD8, AD9 | n/a |
| S2 engines | holds: limits each engine applies are set in our code (`js.rs:283-287`, `python.rs:220-225`, `rhai_rt.rs:122-131`); only Rhai's apply to imported data |
| S1, S3-S10 | n/a |

### Claims (CL)

| Row | Cell |
|---|---|
| CL10 value limits "in all three languages" | holds for the script-produced direction; the reply direction differs only by the **known** Rhai limits |
| CL29 "identical semantics in every language" | **known** (Rhai reply refusals) |
| CL1-CL9, CL11-CL28, CL30-CL37 | n/a |

### Tests (T)

| Row | Cell |
|---|---|
| T7 cross-language | no test feeds the same large or deep reply to all three runtimes; covered by the **known** finding's regression test once written |
| T1-T6, T8-T12 | n/a |
