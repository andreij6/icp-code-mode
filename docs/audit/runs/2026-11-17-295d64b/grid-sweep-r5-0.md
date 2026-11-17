# Variant sweep r5-0: a limit enforced with different semantics in one runtime than its siblings

Run `2026-11-17-295d64b`, Phase 6, round 5. Pattern (register class L6):

> A limit enforced in one runtime (or route, build, SDK function, output channel) with
> different semantics from its siblings, or missing there.

Method: read `src/runtimes/{mod,js,python,rhai_rt}.rs`, `src/bridge.rs`, `src/lib.rs`,
`src/policy.rs`, `src/search.rs`, `src/mcp.rs:140-200, 330-410`, `demo_store/src/lib.rs` at
`295d64b` (`git archive 295d64b` into `/private/tmp/claude-501/sweep-r5-0`, deleted
afterwards, because the working tree's `src/` has someone else's uncommitted edits). I also
read `monty-types-1.6.0/src/io.rs:100-170, 279-292` and `monty-1.6.0/src/builtins/print.rs:21-84`
for Python's second output stream. For every limit row I listed each sibling that enforces
it and compared the unit counted, when the check runs, what is counted against it, and what
happens at the limit. I then checked each difference against the task's known list and
the earlier sweep and lens grids (r1-r4, L3/L6/L14/L16). Nothing was compiled or run. Every
verdict below comes from the code path (C) or cites an earlier reproduction.

Result: **no new candidate.** Each difference that fits the pattern is already a known
finding or was given a reasoned "holds" in an earlier grid that this sweep re-checked.

## Cells

| Row | Siblings compared | Verdict | Reason |
|---|---|---|---|
| LIM2 instruction budget | JS (rounds, 256 jobs, console, request), Python (Monty feed/turn clock), Rhai (`on_progress` every op), bridge `reserve`, post-run check | known | Sync JS unchecked; Python soft, relative and turn-reset; snapshot vs live limits; compile and `parse_json` outside the budget; search query path unbudgeted |
| LIM2 catchability | JS budget refusal thrown from `console`/`icp.*` (`js.rs:202-205, 230, 253`) is a catchable RangeError; Rhai's is the uncatchable terminate marker (`rhai_rt.rs:148-151`) | holds | Catching it in JS lets only synchronous code continue, which is already the known "sync JS never checks the budget" finding. The run still ends at the next `check_budget` (`js.rs:376`) or job tick. Host-side `reserve` refusals are catchable in all three by design |
| LIM15 log cap | JS (skips formatting at cap, `[level] ` prefix counted), Rhai (formats, then drops; `[debug pos]` counted), Python (Monty `CollectString`, newline bytes counted, MemoryError), `LogBuf`, `tool_result` 250 KB | known | Python MemoryError; JS formats every argument; marker overshoot; empty lines; 250 KB escaped re-cut |
| LIM15, Python second stream | `print(..., file=sys.stderr)` vs stdout | holds (C) | `PrintWriter::write`/`push` send both streams through the same `CollectString` buffer and `check_print_collect_limit` (`monty-types io.rs:132-136, 159-163`), so stderr cannot get around the 64 KiB cap. JS has `console.error`/`warn` on the same counter; Rhai has `debug` on the same counter |
| LIM16 nodes / bytes / depth | JS `SNAPSHOT_JS`, Python `to_json`, Rhai `to_json_value` | known | UTF-16 vs UTF-8; JS keys unchecked; py_repr uncharged; bytes/blobs as text not elements; depth off by one; Rhai engine limits summed (SW2-3-1) |
| LIM16, scope of the budget per call | JS budgets `args` only; Python one `ValueBudget` across canister, method and args (`python.rs:243-244`); Rhai `args` only (`rhai_rt.rs:160`) | known | JS canister and method strings unbounded (known). Python's wider scope is the stricter one. Rhai strings are capped at 1 MiB by the engine |
| LIM23 code size | `LanguageRouter::execute` once for all (`mod.rs:52`) | holds | The same byte count for every language; compile costs past it are known (Boa, Rhai, Python eval/exec; Monty top-level compile is linear, r2-0 / r3-2 / r3-7) |
| LIM24 / LIM26 / LIM27 recursion | JS 256 frames, Python 200, Rhai 32 call levels | holds | Each is documented with its own number (README:370-372). The Python counter shared with export is known |
| LIM25 / LIM26 / LIM27 request counts | JS 64 per round x 64 rounds, Python 256 progress events, Rhai 64 requests | known / holds | No per-batch cap in Python (T2-1); last round dropped (SW5-1). Python counts `icp.principal()`, name lookups and `import icp` as events, which r1-5 and r3-8 recorded and graded. The hard-coded ceilings below a larger `max_calls_per_script` are documented (L14-r1 AD4) |
| LIM25 promise jobs | JS 100k jobs; Python/Rhai have no queue | holds | No equivalent work queue in the siblings. Their loops are covered by the VM budget (Python) and operations (Rhai) |
| LIM26 memory | Python 64 MiB (counting allocator); JS none; Rhai size limits | known | Register #24; host replies not counted for Python; Rhai keys/FnPtr; Python `re`/`json.dumps(indent)` |
| LIM27 1M operations | per replay (`rhai_rt.rs:19, 197`) vs README "per run" | holds | Recorded in L16-r1 CL13 and r3-6. Each replay re-runs the same logical prefix, so the script's logical work is held to 1M. The absolute deadline is checked every op |
| LIM17 method length | `IcBridge::call` only (`bridge.rs:511`) | holds | The one entry used by all three runtimes. `icp.candid` and search take no method |
| LIM31 principal echo | `parse_principal` 80 B vs candid_bridge and search notes | known | |
| LIM18 TTLs | `cache_get` used by both the query search (`cached_service`) and the update path; `failed_get` only on the update path | known / holds | The cache TTL is the same on both routes (`bridge.rs:207-213, 284-289`). The failed-fetch asymmetry is known. Overrides have no TTL (known) |
| LIM19 512 KiB | fetched (`bridge.rs:490-491`) vs override write (`lib.rs:326`) vs tmp_hack decode | known | Pre-5d856e3 overrides; tmp_hack decode unquoted |
| LIM20 / LIM21 decode | `decode_reply` vs `inspect_message` vs fetch decode | known | |
| LIM1 call cap | script vs search (own cap each, 5 per request) vs fetch (one slot, two messages) | known | L4-r1 E5b, r2-8 |
| LIM3 timeout | script calls vs interface fetches | known | |
| LIM5 audit page | count not bytes | known | |
| LIM6 / LIM7 result and reply caps | query and update both go through `tool_result` (`mcp.rs:155-169, 374, 401`); `http_update` re-measures, query `finish` only | known | Oversized-reply fallback findings |
| LIM8 / LIM9 / LIM10 | query vs update | known / holds | r2-9 |
| LIM12 run slots | scripts only; searches none | known | |
| LIM13 / LIM14 audit caps | `run_script` and `run_search` closing entries vs `Trapped` | known | Trapped drops the "…and N more" count; caps exceed by marker |
| LIM22 2^53 | `idl_to_json` vs `json_to_idl` | known | L7-3, L7-17 |
| LIM28 / LIM29 / LIM30 | platform, shared | known | Native stack, heap, 40B findings |
| LIM32 search 40 methods | count, not bytes | known | SW2-5d |
| C1a feature subsets | constants shared through `crate::bridge` / `candid_bridge` imports | known | Per-language tool description and language validator findings; no limit constant is duplicated per feature |
| C2 demo_store updates | `create_order` (wrapping `+=`) vs `cancel_order` (`saturating_sub`) vs `restock` (`saturating_add`) | known | Wrap, saturate and oversell findings already cover all three arithmetic policies |
| C5 demo page | no client-side limit; server enforces | n/a | Not a second enforcement site |
| E1/E2 vs E6-E11 | `apply` calls the same `policy::set_allowed/add_token_hash/set_limits` | known / holds | Same validation (`lib.rs:38-49`); no override in InitArgs; terse-record/additive findings known |
| E4 vs E5 | query vs update routing | known | MAX_SEARCHES, budget, DID_FAILED, busy |
| E16 / E18 / E19 SDK | arity, types, two-arg `null`, `principal` | holds | `principal` unbudgeted and uncounted in JS/Rhai; in Python it is one counted event (known, see LIM26 row) |
| U/O/D/V/K rows | not limit sites, or limit applied once on a shared path | n/a | Their limit aspects are the LIM rows above |
| W1-W16 | unbounded-work rows | known | Each has a listed finding or an earlier "holds" |
| F1-F16 | failure sites | n/a | Not limits; per-runtime trap differences are the known built-in/stack findings |
| B5-B10 converters | per-runtime value conversions | known | L7-x and SW findings |
| B11 / B14 | result text, time | known | L7-16, L7-16b |
| VF1-VF8, AD1-AD9, S1-S10 | | n/a / known | No per-variant limit beyond those listed (AD4 unvalidated limits known) |
| CL8 / CL9 / CL10 / CL12 / CL13 / CL14 / CL36 | claims about the limits above | known | L16 findings, SW2-5d |
| T1-T12 | tests of these limits | known | Testability T2-1, T5-x |

## Not tested

- Nothing was compiled or run in this sweep. Every "holds" rests on the code path or on an
  earlier round's reproduction, which is cited. The Python stderr cell is code-path only.
- Monty's `warnings`/`logging` modules (if present) were not checked for an output path
  outside `PrintWriter`. On wasm32 a direct `eprint!` would go nowhere, so it could not
  escape the cap.
