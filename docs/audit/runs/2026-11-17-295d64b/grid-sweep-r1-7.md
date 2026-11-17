# Variant sweep r1-7: cleanup-time records that read rolled-back state

Run `2026-11-17-295d64b`, Phase 6. Pattern: "A cleanup-time record reads state that the trap
rolled back, so it understates the work it describes." Seed findings (not re-reported): the
`Trapped` entry's `instructions` omits the trapped execution; the `Trapped` entry drops the
"…and N more" violation count; a trap inside cleanup leaks the slot and pending id.

Method: listed every code path that runs during ic-cdk's trap cleanup (`grep` for `impl Drop`
and `is_recovering_from_trap` in `src/`, `demo_store/src/`, `shims/`, plus ic-cdk 0.20.3
`call.rs:943` and `futures.rs:108`), then for each inventory row asked whether it writes or
feeds a record in cleanup, and whether that record reads heap state from the rolled-back
execution. Only three app Drop impls exist: `IcBridge` (`src/bridge.rs:243`), `RunSlot`
(`src/lib.rs:212`), `test_serial::Turn` (`src/runtimes/python.rs:184`, `#[cfg(test)]`).
Evidence is code path (C) unless marked; no request was sent to any running instance.

Candidate ids (SW7-n) refer to the candidates returned with this grid.

| Row(s) | Verdict | Reason |
|---|---|---|
| E5d cleanup callback / K7 | **finding SW7-1**; seeds known | Cleanup drops the whole `http_request_update` future as it was at the last commit. In a batch, an earlier tool call that finished in the trapped execution is still "unfinished" in that state, so its bridge writes the `Trapped` entry, and the tool call that actually trapped (never sent) leaves nothing |
| `Trapped.violations` (bridge.rs:261) | holds (documented) | Violations raised after the last message are rolled back; README.md:407 says the entry lists violations "recorded up to the last message". Same mechanism as the seeds, but disclosed. Dropped count: seed |
| `Trapped.calls` / `methods` / `canisters` | holds | `messages` and `touched` are only written in `record_message`, in the same execution as the send (K1); a send in the trapped execution is rolled back with its message, so the counts match what went out |
| `Trapped.timestamp_ns`, `caller`, `language`, `script_sha256`, `run_id` | holds for a single tool call; see SW7-1 for batches | Taken from `audit_key` / `started`, set before the first commit |
| `Trapped.instructions` | seed (known) | |
| `Trapped.error` | holds | Constant text; for a search it says "the script's result", wording only |
| E5c reply callbacks | holds | A callback after cancellation traps "Call already trapped" and returns early in cleanup (ic-cdk `call.rs:943-960`), no second record |
| K1-K4 per-message entries | not applicable | Written in the sending execution, not in cleanup |
| K5 `run_search` / `fetch_did` | **finding SW7-1** (when followed by a script in the same request); holds alone | A search that traps after its fetch gets a correct `Trapped`; failure not negative-cached is known |
| K6 closing entry | **finding SW7-1** | The closing entry of a tool call that completed is rolled back by a trap in a *later* tool call of the same request; cleanup then writes `Trapped` for the completed one |
| K8 / K9 | holds (register #11, accepted, README "Not in the log at all") | No cleanup record exists; nothing understated |
| K10, K12 | not applicable | Single-execution updates; no cleanup callback |
| K11 upgrade | not applicable | Callbacks deleted on upgrade, no cleanup record (documented "unknown") |
| V1 `ACTIVE_RUNS` / `RunSlot` | holds | Decrements the committed value, which includes this run's increment; a slot taken in the trapped execution is rolled back with it |
| V3 `PENDING_RUNS` | holds | Removal reads `started`, set in a committed execution |
| V2 / V4 / V5 caches | not applicable (not a record) | Failure not cached after a trap is known (register list) |
| V6 Python memory atomics | not applicable | Rolled back with the heap they count; cleanup frees the VM consistently; not a record |
| V7 getrandom state | not applicable | Rolled back; stream carry-over is known; not a record |
| V8 Boa GC, V9 executor, V10 engine statics | not applicable | Not records; engine drop order in cleanup feeds the known L2-5 path only |
| V11 / D8 demo_store | not applicable | No awaits, no cleanup callbacks |
| D1-D3, D5-D7 | not applicable | Written only by single-execution admin updates or lifecycle hooks |
| D4 `AUDIT` | **finding SW7-1** | The only durable record written in cleanup |
| D9, D10 | not applicable | Browser/disk, no trap semantics |
| O3 | **finding SW7-1** | Cleanup append |
| O1, O2, O4-O10 | not applicable | No cleanup-time write |
| F4 append trap | known (L2-5 / L9-9) | |
| F1-F3, F5-F16 | not applicable | Abort sites, not cleanup records; F12/F14 are the triggers SW7-1 uses (known) |
| E1-E4, E6-E15, E16-E28 | not applicable | Queries, admin updates, lifecycle and host functions have no cleanup record of their own; host functions feed `messages`/`violations` (rows above) |
| E5, E5a, E5b | **finding SW7-1** | `http_update` runs tool calls sequentially in one future (`src/mcp.rs:397-418`) |
| LIM9, LIM10 | **finding SW7-1** (enabler) | One script plus up to five searches per request share executions |
| LIM13, LIM14 | holds / seed | Constant error; violation cap seed |
| LIM1-LIM8, LIM11, LIM12, LIM15-LIM32 | not applicable | Limits; none is read by a cleanup record except `max_instructions` (not written to the entry) |
| U1-U16 | not applicable | Inputs; none is read by the cleanup record except through `audit_key` |
| W1-W16 | not applicable | Work rows; W3/W8 can be the trap trigger (known) |
| B1-B17 | not applicable | No conversion runs in cleanup except `Vec<String>` clones |
| VF1-VF8 | not applicable | No value moves in cleanup |
| AD1-AD9 | not applicable | Admin updates are single executions |
| S1-S10 | not applicable | Build rows |
| C1-C15, A1-A12 | covered through E/K/D rows | |
| CL15, CL17, CL18 | holds for single tool calls | |
| CL19, CL20 ("A trap is recorded as `Trapped`"; "Not in the log at all ... changed nothing") | **finding SW7-1** | In a mixed batch the trap is recorded against the wrong run, and the request did change state (the search's message) |
| CL1-CL14, CL16, CL21-CL37 | not applicable | No claim about cleanup records |
| T1-T12 | not applicable / gap | No test drives the real bridge (register #58), so neither the seeds nor SW7-1 can be tested natively |
