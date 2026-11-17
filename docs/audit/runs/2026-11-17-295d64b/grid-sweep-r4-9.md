# Variant sweep r4-9: narrowing cast or parse that saturates instead of failing

Run `2026-11-17-295d64b`, Phase 6, round 4. Pattern (seed L7-5, with its siblings L7-11/12/13,
all known and not re-reported):

> A narrowing cast or parse that saturates (or otherwise coerces leniently) instead of
> failing, so a value that does not fit arrives changed and the operation reports success.

Method: `grep -nE "\bas (u8|u16|u32|u64|u128|usize|i8|i16|i32|i64|i128|isize|f32|f64)\b|parse::<|\.parse\(\)|from_str|unwrap_or|try_from|saturating|as_u64|as_i64|as_f64|clamp|\.min\(|\.max\("`
over `src/`, `demo_store/src/`, `shims/web-time/src/`; read every hit; read the four
value converters (B5, B8, B9, B10) and the demo_store write paths; grep `src/demo.html`,
`video/*.mjs`, `scripts/*.sh` for JS numeric coercions. Checked that no dependency turns on
serde_json `arbitrary_precision` (`cargo tree -e features -i serde_json`: `default`, `std`
only), so every `as_f64`/`as_i64` result below is the plain i64/u64/f64 form.
Evidence: R = reproduced (native test in a private copy, shared target dir), C = code path.

## Probes (private copy, deleted afterwards)

- P1 `demo_store` `restock(1, u32::MAX)` on seed stock 21 -> `Ok(Inventory { on_hand: 4294967295 })`;
  21 units silently lost. Then `restock(1, 1000)` -> `Ok`, `on_hand` unchanged (R).
- P2 `json_to_idl` against `record { 5 : nat8; b : nat; c : float64; d : float32; e : int64 }`:
  keys `"+5"` and `"_+5_"` match field 5; `"+7"`, `"+12"`, `"+3"` accepted; `"1e-400"` -> `0.0`,
  `1e-50` (float32) -> `0.0`, `"-1e400"` -> `-inf`, `"infinity"` -> `inf`; `9.2e18` (int64) ->
  exact `9_200_000_000_000_000_000`; `1e19` (int64) -> error `` `1e+19` is not a valid int64 `` (R).

## Cells

| Row | Site | Verdict | Reason |
|---|---|---|---|
| B5 / F11 `candid_bridge.rs:216` `float(v)? as f32` | json_to_idl float32 | **known** (L7-5) | Saturates to inf, underflows to 0 (P2) |
| B5 `candid_bridge.rs:173` `s.parse::<f64>()` | float text | **known** (L7-5) | `"1e400"`/`"-1e400"` -> +-inf; also `"1e-400"` -> 0.0 and `"infinity"` accepted (P2); same root cause, merge into L7-5 |
| B5 `candid_bridge.rs:156` `{:.0}` for integral f64 below 1e19 | ints from JS numbers | **known** (L7-3) | Exact decimal of the already-rounded f64; above 1e19 `n.to_string()` gives `1e19`, which fails to parse (P2): fails, holds |
| B5 `candid_bridge.rs:160, 167` `s.replace('_', "")` + `FromStr` | int text | **known** (L7-12) | Rust `FromStr` also accepts a leading `+` (`"+7"` for nat8, `"+12"` for nat via num-bigint) (P2). Another lenient spelling, same value: merge into L7-12. Out-of-range text fails (`u8`/`i64` parse errors), so no saturation |
| B5 `candid_bridge.rs:142` `digits.parse::<u32>()` in `key_matches` | record id keys | **known** (L7-13) | `"+5"` and `"_+5_"` are further aliases of id 5 (P2); duplicates resolved first-wins as in L7-13. Merge |
| B5 `candid_bridge.rs:248` `trim_start_matches("0x")` | blob hex | **known** (L7-12) | Repeated prefixes |
| B5 `candid_bridge.rs:202-203` null/reserved | discard | **known** (L7-11) | |
| B5 `candid_bridge.rs:296` `idx as u64` | variant index | holds | usize index of a declared field, fits (C) |
| B5/B6 `candid_bridge.rs:126, 341` `i as u32` | tuple detection | holds | Index compared with field ids; a record with > 2^32 fields cannot be decoded within the quota (C) |
| B6 `candid_bridge.rs:307` `big`: `s.parse::<i64>().unwrap_or_else(Number::from(m))` | reply ints | holds | Reached only when `mag <= 2^53-1`, where the i64 parse always succeeds (C) |
| B6 `candid_bridge.rs:335` `f32 as f64` | reply float32 | holds | Widening, exact (C) |
| B8 `bridge.rs:89` `f as i64` | JS integral numbers | holds | Guarded by `abs() <= 2^53-1` (C) |
| B8 `js.rs:63` BigInt | JS -> host | holds | Refused with a TypeError, not narrowed to a Number (C) |
| B8 Boa `to_json` NaN/inf | JS -> JSON | **known** (L7-4) | |
| B9 `python.rs:70` `as_int().map_or_else(py_repr)` | Python int | holds for this pattern | Beyond i64 becomes an exact decimal string, not a saturated i64 (C); 2^53..2^63 as numbers is **known** (L7-17) |
| B9 `python.rs:71` float -> `Number::from_f64` | Python float | **known** (L7-4) | |
| B9 `python.rs:108-112` `from_json` | reply -> Python | holds | i64, then u64 as bigint (exact), else f64; `unwrap_or(f64::NAN)` is unreachable without `arbitrary_precision` (C) |
| B10 `rhai_rt.rs:76, 117` `to_dynamic` / `from_dynamic` | Rhai | holds | Reply numbers are <= 2^53 or Nat32-sized (`big`), so Rhai's u64 > i64::MAX path is never fed; Rhai INT/FLOAT are i64/f64 (no `f32_float`, no `decimal`) (C) |
| B14 `python.rs:214` `(now_ms() / 1000) as i64` | clock | holds for this pattern | Fits i64; truncation to seconds is **known** (L7-16b) |
| B14 `js.rs:278` `FixedClock::from_millis(now_ms())` | clock | holds | u64 ms, no narrowing (C) |
| B14 `shims/web-time/src/lib.rs:20-35` | Instant | holds | `checked_add`/`checked_sub` return `None`; `saturating_duration_since` matches std semantics; F9 refuted in L14/L15 grids |
| LIM2 `bridge.rs:544` `base.saturating_add(max)`; `python.rs:39` `saturating_sub` | budget | **known** | `max_instructions` near `u64::MAX` saturates to "unlimited": the known "Limits are accepted unvalidated" finding. `saturating_sub` to 0 makes Python fail at once, which is the intended refusal |
| LIM26 `python.rs:224` `budget.min(MAX_TURN_INSTRUCTIONS)` | turn cap | **known** | Intended clamp; its per-turn reset is the known L14-2 correction |
| LIM3 `bridge.rs:525` `change_timeout(secs)` | timeout | holds | IC caps at 300 s silently, documented in README:432 and `.did:70` (L14 grid, L9-r2 live) |
| LIM5 `policy.rs:265` `limit.min(MAX_AUDIT_PAGE)` | audit page | holds | Documented clamp (CL32); offset arithmetic is `checked_*` (C) |
| `bridge.rs:333` `len() as u32`, `bridge.rs:580` `len() as u64` | call counts | holds | Bounded by the call cap / run length (C) |
| `lib.rs:445` `as u8` | getrandom bytes | holds | Intended byte extraction (C) |
| `mcp.rs:179` `c as u32` | escaping | holds | Widening (C) |
| U4 JSON-RPC ids | | **known** (L7-9) | |
| AD1-AD5 admin Candid args | nat32/nat64 | holds | Candid decoding rejects out-of-range values; no app-side narrowing (C) |
| E23 demo_store `restock` `on_hand.saturating_add(quantity)` (`demo_store/src/lib.rs:334`) | stock count | **finding SW4-9-1** (R, P1) | Excess quantity is discarded and the call returns `Ok`. Earlier sweeps r3-4/r3-5 marked this "holds" for growth and wrap patterns; for this pattern it is the hit |
| E22 demo_store `cancel_order` `reserved.saturating_sub` (`:324`) | stock | **known** root cause | Only reachable when `reserved < quantity`, i.e. after the known oversell / `reserved` wrap; sweep r3-5 already ties it there |
| E21 demo_store `create_order` `available = on_hand.saturating_sub(reserved)` (`:290`) | stock | holds | Saturating to 0 available is the correct reading of an oversold product (C) |
| E21 demo_store `create_order` `total += cents * quantity` (`:294`) | order total | **known** root cause | Wraps (not saturates) only with release overflow checks off plus unbounded duplicate lines: the known oversell and overflow-checks findings |
| E20 demo_store `sales_summary` | revenue | **known** | |
| W15 / E20 `seed()` casts (`:126, 135, 147, 152, 154, 175`) | seed | holds | Constant inputs (C) |
| C5 demo page numeric code (`demo.html:225, 302, 334, 342, 354, 358, 495-506`) | display | holds for this pattern | Display arithmetic on reply numbers; no narrowing cast |
| C12 video scripts (`render.mjs:10`, `tts.mjs:27, 62, 68`) | dev tooling | not applicable | Local CLI parsing of the developer's own arguments and ffprobe output |

## New candidate

**SW4-9-1 (Low).** `demo_store.restock(product_id, quantity)` adds with
`on_hand.saturating_add(quantity)` (`demo_store/src/lib.rs:334`) and returns `Ok`. When the
sum passes `u32::MAX`, the excess units are silently dropped instead of the call failing.
P1: `restock(1, 4294967295)` on seed stock 21 returns `Ok` with `on_hand = 4294967295` (21 units
lost); a further `restock(1, 1000)` returns `Ok` and changes nothing. Anyone can call it
(demo_store has no access control, known). The returned `Inventory` shows the capped value, so the
caller can see it only by comparing. Fix: `checked_add(quantity).ok_or("stock would exceed u32::MAX")?`.

## Merged, not new

`+` signs in numeric text and in record id keys, and float64 text underflow to 0 / `"infinity"`,
are further spellings of the known L7-12, L7-13 and L7-5 (P2). They share one root cause:
`json_to_idl` delegates to Rust's `FromStr`, which accepts these spellings, and casts without
range checks.
