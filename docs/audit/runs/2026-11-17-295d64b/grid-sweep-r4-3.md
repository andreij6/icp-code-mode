# Variant sweep r4-3: one limit, different operators or different check positions

Run `2026-11-17-295d64b`, Phase 6, round 4. Pattern:

> The same limit compared with different operators, or counted at different positions.

Seeds (known, not re-reported): the depth limit is off by one between runtimes (JS
`d >= 128` on containers, Python/Rhai `depth > 128` on every value); `MAX_DEPTH` counts Candid
levels in `candid_bridge` and JSON levels in the runtimes (SW3-8-1); the log cap is 64 KiB raw
in the runtimes and 250,000 escaped bytes in `tool_result` (SW3-8-3). Round 3's units sweep
(r3-8) covered "different units". This sweep looks only at the operator (`>` / `>=` / `<`,
inclusive or exclusive) and at **where** the count is added versus where it is compared.

Method: for every limit row, `grep` every comparison against it in `src/`, `demo_store/` and
`src/demo.html`, then compare each comparison's operator and position. Also check that every
place that adds to a running total is followed by a comparison. Reproductions were native
(`cargo test --lib`) in a private copy (no `target/`, `design/`, `.env`), with the shared
`CARGO_TARGET_DIR=/private/tmp/claude-501/audit-target`. The copy was deleted afterwards.
Nothing was deployed and no live setting was changed.

Evidence keys: **R** = reproduced natively; **C** = the code path produces it unambiguously.

## New candidate

| Id | Severity | Candidate | Evidence |
|---|---|---|---|
| SW4-3-1 | High | **JS object-key text is added to the 4 MiB value budget but never compared against it.** In `SNAPSHOT_JS` (`src/runtimes/js.rs:58-66`), `bytes > MAXB` is tested only in the string-value branch (`:58-60`). The key loop (`:64-66`) runs `bytes += k.length` and never checks. Python (`python.rs:91`) and Rhai (`rhai_rt.rs:105`) charge keys through `ValueBudget::bytes`, which compares on every add. So in JS, an object whose values are numbers, booleans, null or nested objects can carry any amount of key text, as long as no string value follows. One large string used as the key of many objects leaves the sandbox as one host copy per object. This brings back register #21 (fixed, High) for JS keys. It affects every path that uses the snapshot: the script result, `icp.call` arguments (`request_fn` -> `to_json_safe`), `console.log` of objects (`display`), and the `JSON.stringify` wrapper. CL10 (README:369, "4 MiB of text, in all three languages") and CL11 ("JSON.stringify is held to the value limits too") are false for JS | **R**: `let k='a'; for (let i=0;i<20;i++) k+=k; return Array.from({length:40},()=>({[k]:1}))` returns `Ok` with 41,943,040 bytes of key text (limit 4,194,304). With `{[k]:'x'}` it fails with `RangeError: value holds more than 4194304 bytes of text`. Python `[{k: 1} for _ in range(40)]` and the Rhai equivalent both fail with the same limit error. `icp.call(L, 'icrc1_balance_of', o)` with 8 keys of 1 MiB: the host received 8,388,629 bytes of key text, and the call was sent. `JSON.stringify(a).length` = 41,943,321. With length 200, the result is `Ok` with about 200 MiB of key text. That took 8.8 s natively, with a peak RSS of 675 MB for the test process. **Inferred, not run**: at about 1,000 x 1 MiB, the host copies pass the 3 GiB Wasm memory limit or the 40e9 instruction limit. The canister traps, and when the trap comes before the first send, no audit entry is written (K8) |

## Limits: every comparison site

| Row | Sites (operator, position) | Verdict |
|---|---|---|
| LIM16 text 4 MiB | `ValueBudget::bytes` add-then-`>` (Python str, bytes, dict keys; Rhai blob, keys, strings); JS string `bytes += len; > MAXB`; **JS keys `bytes += k.length` with no compare** | **SW4-3-1** (R) |
| LIM16 200k nodes | `ValueBudget::node` `+1; >`; JS `++nodes > MAXN`, after `toJSON` and replacer, for every value | holds: same operator. Counting after `toJSON` is the spec order |
| LIM16/LIM21 depth 128 | JS `d >= MAXD` (containers only); Python/Rhai `depth > 128` (every value); `json_to_idl`/`idl_to_json` `depth > 128` (every Candid level, both directions start at 0) | runtime off-by-one **known**; Candid vs JSON levels **known** (SW3-8-1); encode and decode symmetric (R in r3-8: 64 pass, 65 fail both ways) |
| LIM15 log 64 KiB | JS `>=` pre-check, `+len >` then truncate to the remainder; Rhai `>=` pre-check then truncate to the remainder (marker counted); `LogBuf` `>=` and `+len >` (pushes marker only); Monty `CollectString` raises | markers past the cap, LogBuf dropping the cut line and the Python raise: all **known** |
| LIM6 result 1 MB / logs 250k | `truncate_escaped` `used > max`, cut before the char that crosses | marker overshoot **known**; escaped vs raw re-cut **known** (SW3-8-3) |
| LIM7 reply 1.9 MB | `http_update` sum of replies `>` (framing excluded); `finish` body `>` | **known** (framing) |
| LIM8 batch 50 | `parse_body` `a.len() > 50` after the parse (shared by both routes) | holds for this pattern; parse-before-check **known** (W1) |
| LIM9 5 searches | update path `searches >= 5` counted only for valid searches; query path none | **known** |
| LIM10 one script | `ran_script` set when a valid script runs | holds |
| LIM11 rpc 300 B, LIM13 4096, LIM14 512, LIM31 80 | `bridge::truncate` `len > max` everywhere | holds (marker overshoot **known**) |
| LIM12 8 runs | `RunSlot::take` `< 8` only; the query route does not check | **known** |
| LIM1 call cap | `authorize` `calls_made >= max` on slots, before fetch and send; the audit counts messages | operator holds; slots vs messages **known** |
| LIM2 instructions | `authorize` `used > max`; `budget_error` `counter > base+max` (JS console/request/jobs/rounds, Rhai `on_progress`); `run_script`/`run_search` `used > max` after the work; Python feed `deadline - counter` | one operator throughout (the deadline is `base + max`, so `>` matches `used > max`). Positions (after render, before send) **known** |
| LIM17 method 256 B | `IcBridge::call` `len > 256` only | holds. Refused without a violation entry: same family as the **known** composite_query refusal |
| LIM18 TTLs | `cache_get` and `failed_get` both `<= TTL` with `saturating_sub` | holds |
| LIM19 512 KiB | fetch `n > MAX` (after the reply arrives); override `d.len() > MAX` | holds (same operator; position after receipt **known**) |
| LIM20 decoding quota | one site (`decode_reply`) | n.a. |
| LIM22 2^53 | `big` `m <= 2^53-1`; `normalize_numbers` `abs <= 2^53-1`; `num_text` `abs < 1e19` | first two agree. `num_text` **known** (L7-3) |
| LIM23 100 KiB code | `LanguageRouter::execute` `len > MAX` only | holds |
| LIM24 JS loop/recursion/stack | Boa internals | per-frame counting **known** |
| LIM25 JS rounds/pending/jobs | `for 0..64` rounds (the last round's replies are dropped); `pending.len() >= 64`; jobs `n > 100_000` | round limit off by one **known**; others hold |
| LIM26 Python suspensions | host `for 0..256` progress events plus Monty `max_suspensions(256)` | **known** (SW3-8, register) |
| LIM27 Rhai requests | `answers.len() >= 64` before dispatch, so a 64-request script completes | holds; differs from JS/Python at their limit (**known**) |
| LIM32 search 40 methods | `budget == 0` then `cut += 1` | holds |
| LIM5 audit page 100 | `limit.min(100)` | holds |
| LIM3/LIM4 | not compared against a count | n.a. |
| LIM28-LIM30 | platform | n.a. |
| demo_store stock (VF6, E21) | `quantity > available` per line; `reserved` and `on_hand` saturate | per-line check **known**. The demo page "needs restocking" filter uses `available < reorder_level` in both comparison scripts (`demo.html:319, 347`), the same as the seed test (`demo_store/src/lib.rs:362`): holds |

## Other rows

| Rows | Verdict |
|---|---|
| C1-C15 | C1 covered above. C2: see demo_store row. C5: holds (see the demo_store row). C3, C4, C6-C15: n.a. (no limit comparisons) |
| A1-A12 | n.a. |
| E1-E28 | E5a/E16/E17 reach SW4-3-1 (result, `icp.call` args, `console.log`, `JSON.stringify`). Other entry points route into the limits above |
| U1-U16 | U7 (script values): SW4-3-1 for JS keys. Others bounded as listed |
| O1-O10 | O1: SW4-3-1 sends unbounded key text as call arguments (the payload-limit trap is **known**) |
| D1-D10, V1-V11, K1-K12 | n.a. (no limit comparison) |
| W1-W16 | W8 (JS snapshot): SW4-3-1. W12: host copy of the oversized result, then cut (L7-1 **known**) |
| F1-F16 | F13 (out of memory): SW4-3-1 is a new driver (inferred at about 1 GiB of key text) |
| B1-B17 | B8: SW4-3-1. B9/B10: keys charged. B5/B6: depth/2^53 as above |
| VF1-VF8 | VF1/VF5: SW4-3-1 host work is paid by the canister |
| AD1-AD9 | n.a. (no new comparison; unvalidated limits **known**) |
| S1-S10 | n.a. |
| CL1-CL37 | CL10 and CL11 violated for JS keys by SW4-3-1. CL9/CL14/CL15 operators as above (**known**) |
| T1-T12 | T8 `stringify_replacer_and_size_limits` tests large strings as values only. No test has large keys |

## Not tested

- The on-chain trap for SW4-3-1 (about 1,000 objects sharing one 1 MiB key). It would need a
  deployment, which is the Live phase's job. Native cost grows linearly: 40 MiB of key text took
  1.8 s, and 200 MiB took 8.8 s with a peak RSS of 675 MB.
