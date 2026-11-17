# Variant sweep r1-8

Pattern: **a limit marker added in one writer of a record type and missing in a sibling
writer** (seed: the `Trapped` entry drops the `…and N more` violation count that the
`run_script` and `run_search` closing entries add).

Method: grep for every marker and cap (`truncate`, `…[truncated]`, `…[output truncated]`,
`…and N more`, `omitted_methods`, `note`, `slice(`, `MAX_*`), group the writers by the record
they produce, and compare each writer with its siblings. Probes ran natively in a private copy
(`cargo test --lib`, shared `CARGO_TARGET_DIR`), copy deleted afterwards.

## Records and their writers

| Record | Writers | Markers / caps per writer | Result |
|---|---|---|---|
| AuditEntry `violations` | `run_script` (lib.rs:232-235), `run_search` (lib.rs:155-158), `Trapped` drop (bridge.rs:261), `record_message` (empty) | `…and N more` in the two closing writers; per-item 512 B `truncate` in `deny` (the only push site, bridge.rs:356-366) | Trapped gap = known finding. All pushes go through `deny`, so the per-item marker holds everywhere. **holds** otherwise |
| AuditEntry `error` | `run_script` (4096 + marker), `run_search` (4096 + marker on joined fetch errors), `Trapped` (fixed text), `record_message` (None) | | **holds** |
| AuditEntry `methods`, `canisters` | all four writers | no cap in any writer (known: entry size grows with max_calls) | not this pattern |
| Captured logs | JS `console_fn` (js.rs:227-248), Rhai `RunState::log` (rhai_rt.rs:46-54), Python Monty `CollectString` (python.rs:23, 234), then `LogBuf::push` (bridge.rs:106-119) for all three | JS/Rhai: cut line to the remaining budget and append `…[truncated]` (+14 B); LogBuf: drop any line that would cross 64 KiB and push `…[output truncated]`; Python: raise (known) | **finding (new, SW8-2)**: the JS/Rhai marker pushes the cut line past LogBuf's cap, so LogBuf throws the whole cut line away. Reproduced |
| Tool result text / console block | `tool_result` (mcp.rs:156-169), used by query search, update search, scripts, oversized fallback | `…[truncated]` at 1 MB / 250 KB escaped | same function on every path: **holds** (isError:false on truncation = known L7-1) |
| JSON-RPC error message | `rpc_error` (300 B + marker) for every -326xx; authenticate 401/404/405 bodies (fixed text, no echo) | | **holds** |
| Echo of a caller-supplied canister id in an error/diagnostic | `bridge::parse_principal` (80 B + marker, bridge.rs:464-468); `candid_bridge` principal-typed arg (`invalid principal \`{s}\``, candid_bridge.rs:190-191); `search::render` note `canister {id} is not allowlisted` (search.rs:154) | marker only in `parse_principal` | **finding (new, SW8-1)**: both siblings echo the full text. Reproduced (1,000,000-byte id -> 1,000,051 / 1,000,028-byte message vs 94 B) |
| Other echoes of script text in encode errors (`unknown record field`, `unknown variant tag`, number text, float text; candid_bridge.rs:167,173,267,294) | candid_bridge | no cap, no sibling with a cap for these record kinds | listed under SW8-1 as the same root cause (no shared echo helper); not separately reported |
| Search result method list | `search::render` (both paths) | per-canister `omitted_methods` + global `note` when the 40-method cap cuts | single writer used by query and update: **holds** |
| Search per-canister `error` | render (`interface not fetched`, fetch error, `not fetched: {why}`) | none; fetch error size = known (interface parse errors echo ~512 KiB) | not new |
| `DID_FAILED` message | `IcBridge::candid` only | none | single writer; known |
| Policy errors (`check_call`, `check_interface_fetch`) | policy.rs:177-190 | method name capped at 256 before `reserve` (bridge.rs:511); principal is canonical | **holds** |
| `get_audit_logs` page | `audit_page` | 100-entry cap, no "more" marker; paging API returns len separately | single writer; known (paging) |
| `PolicyView.pending_run_ids`, `active_runs` | `get_policy` | no cap | single writer; not this pattern |
| JS printed values | `display` -> `<unprintable value>`; Python/Rhai print raw | known (README:369 finding) | not new |
| Value-limit errors | JS `SNAPSHOT_JS` (js.rs:60-65) and Rust `ValueBudget` (bridge.rs:59-73) | same wording and limits | **holds** |
| Demo page display (`slice(0,200)` js line 372, `slice(0,40)` line 480) | demo.html | no ellipsis in any writer; no sibling adds one | not this pattern (inventory note 12 covers escaping) |
| demo_store records (orders, inventory) | `create_order`, `cancel_order`, `restock` | no limits or markers in any writer | not this pattern (unbounded growth known) |
| Durable admin records D1-D5 | E1/E2, E6-E11 | override 512 KiB check in `set_candid_override`; no marker records | not applicable (no truncation markers) |

## Inventory rows checked

| Row group | Cell |
|---|---|
| C1-C15 | C1 (all writers above), C5 (demo page: no marker siblings), C2 (no markers), others not applicable (no record writers) |
| A1-A12 | actors only select which writer runs; covered via E rows |
| E1-E28 | E5a/E5b/E5d (audit and result writers): SW8-1, SW8-2, known Trapped gap; E4b query search shares `render`/`tool_result`: holds; E6-E14 holds/n.a.; E16-E19: log writers -> SW8-2, call-arg principal echo -> SW8-1; E20-E25 n.a. |
| U1-U16 | U5 `canister_id` -> SW8-1 (search note); U7 principal-typed values -> SW8-1; U9/U10 known; others n.a. |
| O1-O10 | O3 audit writers above; O6 HTTP replies holds |
| D1-D10 | D4 audit (above); others n.a. |
| V1-V11 | V4 DID_FAILED single writer; others hold no markers |
| K1-K12 | K6/K7 closing vs Trapped writer: known gap only |
| LIM1-LIM32 | LIM6, LIM11, LIM13, LIM14, LIM15, LIM31, LIM32 compared across writers: LIM15 -> SW8-2, LIM31 -> SW8-1; rest hold |
| W1-W16 | n.a. (cost, not markers) except W12 (`truncate_escaped`, holds) |
| F1-F16 | n.a. |
| B1-B17 | B5 (candid_bridge echoes) -> SW8-1; B11 holds; B16 holds; B17 n.a. |
| VF, AD, S | n.a. (no record writers with markers) |
| CL14 (logs <= 64 KiB) | SW8-2 contradicts the intent of the JS/Rhai writers; CL15 audit caps: holds apart from known |
| T1-T12 | `console_output_is_capped_while_running` and `print_is_capped_per_run` only assert `total <= MAX_LOG_BYTES + 64`, which a run whose logs were all discarded satisfies; no test for SW8-1 |
