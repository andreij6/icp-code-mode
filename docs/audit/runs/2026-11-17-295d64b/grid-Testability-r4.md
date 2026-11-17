# Coverage grid: Testability (register class), round 4

Run `2026-11-17-295d64b`, fourth round for this lens. Question: with interleavings, trap
injection, refused sends and a controllable clock added to the round-3 Node `ic0` emulator,
do the run-record and cache invariants hold on the deployed bytes? Rows: K1-K7, V1-V4, I3,
I12, I14, I21, CL17, CL18, CL22, LIM18, register #45/#46/#54/#58.

## Method

1. **Harness (scratch only, nothing in the repository changed).** Session scratchpad `t4/`:
   `emu4.mjs` (emulator), `prop.mjs` (property test), `ttl.mjs`, `exhaustive.mjs`,
   `bounds.mjs`, `didsize.mjs`, `unawaited.mjs`. Module: round 3's `harness.wasm` (deployed
   bytes, SHA-256 `224cfa7a…c39eec769`, plus the table export and the 3 GiB memory maximum)
   with one more export: the module's only global (`__stack_pointer`, mutable i32). Code and
   data sections unchanged. Additions over round 3:
   - **IC rollback per message execution:** before every execution the harness snapshots
     wasm memory, stable memory and the stack pointer; a trap restores all three and discards
     the execution's outgoing calls and reply (round 3 replaced the instance instead).
     `http_request` queries are always rolled back. `get_policy`/`get_audit_logs` are not
     rolled back (they are read-only).
   - **Cleanup callbacks:** when a reply/reject callback traps, the `call_on_cleanup`
     function is run in its own execution on the rolled-back state; if it traps, it is rolled
     back too.
   - **Refused sends:** `call_perform` returns 1 on a chosen predicate (no call is recorded).
   - **Trap injection:** a trap at the k-th `ic0` call of any execution (ingress, callback,
     cleanup). This stands in for an instruction-limit or memory trap at an arbitrary point.
   - **Clock:** `ic0.time` returns a value the test sets, constant within one execution.
   - Candid decoding of `get_policy`/`get_audit_logs`/`HttpResponse` by `@icp-sdk/core` 6.1.0's
     IDL (a local copy from another project's `node_modules`, read-only).
2. **Property test (`prop.mjs`).** Setup: three allowlisted targets, `ryjl3…` and `r7inp…`
   without an override (their interfaces are fetched) and `rkp4c…` with an override. At each step
   one of the following, chosen at random: submit a script (JS sequential / `Promise.all` /
   try-catch / `icp.candid` / `put`; Python sequential / `asyncio.gather` / `icp.candid`; Rhai
   sequential / `icp::candid`), a search (all canisters, or one), or a JSON-RPC batch (one
   script + 1-3 searches in random order); deliver the answer to a random outstanding call
   (valid, reject or wrong-type reply, chosen per method), in any order; send a query-path
   search; advance the clock (1 ns, 1 s, 60 s, 299 s, 301 s, 3599 s, 3601 s). Faults: trap in
   the callback (12-25 %, index 0-200), in the cleanup (0 or 30 %), in the ingress (4 %),
   refused send (8 % of `call_perform`). Up to 11 requests in flight (8 scripts + searches).
   **After every step** the oracle checks:
   - CL17: the `Started`/`Call` entries equal the accepted `call_perform` calls, in order, with
     the same `canister.method` label; every message of a run comes from one call context;
   - CL18: every run (a `Started` id) has exactly one closing entry (`Ok`/`Error`/
     `PolicyViolation`/`Trapped`) or is in `pending_run_ids`, never both; the closing entry's
     `calls` and `methods` equal the run's message entries; a pending run's request can still
     make progress (not replied, not cancelled, has an outstanding call);
   - `active_runs` equals the scripts that hold a slot and can still progress (for batches:
     between the scripts and the scripts plus live batches);
   - DID caches (V2/V4, CL22): a model of the documented rules (success cached 1 h from the
     reply, failure cached 5 min, success clears failure, updated only by committed
     executions). Every interface fetch must start only when neither entry is fresh; every
     direct call to a fetched target needs a fresh interface; a query-path search must answer
     iff every allowlisted interface is fresh.
   **Oracle sensitivity (mutation check):** with the model's TTL changed from 3600 s to 3602 s,
   6/6 seeds report `CL22-refetch`; with stable-memory rollback disabled in the emulator, 3/4
   seeds report `CL18-exactly-one` (`Error` + `Trapped` for one run). Both mutants removed after.
3. **Exhaustive fault injection (`exhaustive.mjs`).** For JS (`Promise.all` of 2), Python
   (`gather` of 2), Rhai (2 sequential calls, replay) and a search (fetch with fallback): a trap
   at every `ic0` index of the final callback (14 / 17 / 26 / 21 indices), and at cleanup
   indices 0-11 (the cleanup makes 8 `ic0` calls).

Campaign size (sum of the run totals): 24,347 property steps over 97 seeds; 7,595 scripts,
1,854 searches, 409 batches, 13,129 deliveries, 390 callback traps (each followed by a cleanup),
30 cleanup traps, 149 ingress traps, 1,155 refused sends, 6,126 clock moves, 1,277 query-path
searches, 278 `busy` refusals. Also 78 exhaustive callback-trap runs and 48 cleanup-trap runs.

Evidence key: **E** reproduced on the deployed bytes in the emulator; **C** code path;
**X** another lens/round; **K** already reported (not repeated).

## Results

| Check | Result |
|---|---|
| No faults (3 seeds x 150 steps) | 0 violations; 8 scripts in flight reached |
| Callback, ingress, refused-send faults, no cleanup traps (60 x 300; 8 x 250 with batches) | 0 violations (18,060 + 2,008 steps; 2,834 fetch starts and 6,521 direct calls checked against the cache model) |
| All faults including cleanup traps (20 x 200) | 13/20 seeds: `pending-not-alive` and `ACTIVE_RUNS` (`active_runs` one above the live scripts), each right after an injected cleanup trap (K) |
| All faults, the cleanup-trap leak modelled as known (6 x 250, batches) | 0 other violations |
| Exhaustive callback traps (78 runs) | every index: one `Trapped` closing entry with complete `calls`/`methods`, `active_runs` 0, nothing pending |
| Exhaustive cleanup traps | indices 0-7 (every `ic0` call of the cleanup): no closing entry, run stays in `pending_run_ids`, script slot never returned (search: pending only); 8-11 (beyond the cleanup): `Trapped` (K) |
| TTL boundaries (`ttl.mjs`) | interface fresh at exactly reply + 3600 s (no fetch, query answers), re-fetched at +3600 s + 1 ns (query upgrades); failure returned without messages at exactly failure + 300 s, retried at + 300 s + 1 ns |
| Trap in the callback that completes a fetch | cache not filled (rolled back); next run fetches again |
| Refetch of a stale interface fails | calls fail without messages for 5 min although a stale interface is held (K: negative cache precedence) |
| 512 KiB limit (`didsize.mjs`) | fetched and override interfaces of 524,288 bytes accepted, 524,289 refused with the documented message |
| 8 slots (`bounds.mjs`) | 9th script answered `busy`, audit length unchanged; a search still runs and fetches while 8 scripts hold the slots (K) |
| Cap 2 (`bounds.mjs`) | a refused send takes a slot (documented: "attempts that pass policy"), records no message, `calls` = 1; third call refused by policy; one slot sends 2 fetch messages + the call = 3 messages (K) |
| Unawaited calls (`unawaited.mjs`) | an unawaited JS `icp.call` / Python `call_async` is never sent (K); with `Promise.race` / a rejected `Promise.all`, the run closes only after every sent call is answered, so no reply arrives for a closed run |

## Grid

| Row | Cell | Reason / evidence |
|---|---|---|
| K1 first poll Pending -> entry + `PENDING_RUNS` | holds | E: CL17 checked after each of 24,347 steps: entries equal accepted sends in order and label; refused sends (1,155) leave no entry; a send followed by a trap in the same execution leaves neither message nor entry |
| K2 JS `drive` awaits | holds | E: sequential, `Promise.all`, `Promise.race`, try/catch, interleaved with other runs, traps at every index of the final callback |
| K3 Python awaits | holds | E: sequential `icp.call` and `asyncio.gather`; same checks |
| K4 Rhai replay awaits | holds | E: replayed sequential calls; same checks (26-index exhaustive) |
| K5 `run_search` / `fetch_did` awaits | holds | E: searches alone and in batches, fallback fetch after a rejected, wrong-type or refused `canister_metadata`; closing entry written iff a message was sent or policy refused |
| K6 end of `http_request_update` | holds | E: traps injected after the closing entry is appended (late callback indices) roll it back and the cleanup writes `Trapped`; never two closing entries |
| K7 cleanup callback | finding (K) | E: a trap at any of the cleanup's 8 `ic0` calls leaves no closing entry, keeps the run in `pending_run_ids` and keeps the script's slot until upgrade. Already reported (src/bridge.rs:243-266; src/lib.rs:203-220; src/policy.rs:247-253); reproduced here, not repeated |
| V1 `ACTIVE_RUNS` | holds except K7 (K) | E: equals the live scripts after every step under callback/ingress traps, refused sends, batches and busy refusals; diverges only after a cleanup trap. Searches not counted (K) |
| V2 `DID_CACHE` | holds | E: model check (fetch only when stale, direct calls only with a fresh interface, query path answers iff fresh) after every step; fills rolled back with a trapped callback |
| V3 `PENDING_RUNS` | holds except K7 (K) | E: CL18 check; a pending run always had a request that could still progress, except after a cleanup trap |
| V4 `DID_FAILED` | holds (documented TTL); precedence over a stale success and caching of local refusals are K | E: failure TTL boundary exact; a failure recorded by two refused sends in one execution is modelled and matched |
| I3 records match reality | holds except K7 / #11 (K) | E: (a) CL17, (b) CL18, (c) `calls`/`methods` complete on every closing entry, including `Trapped` ones |
| I12 cap per run | holds as documented; message fan-out K | E: refused sends count as attempts; one slot can send two fetch messages |
| I14 8 scripts, busy writes no entry | holds | E: 9th refused with `busy`, audit length unchanged; property runs reached 8 active many times |
| I21 TTLs and 512 KiB | holds | E: all four TTL boundaries and both size boundaries exact |
| CL17 | holds | E (see K1) |
| CL18 | holds except K7 (K) | E (see I3) |
| CL22 | holds | E: first observation of the 1 h re-fetch and repeated observation of the 5 min retry, both at the nanosecond boundary |
| LIM18 | holds | E: as CL22; anchored at the time of the reply that completed the fetch (failure: of the last failed message) |
| #45 interfaces cached until upgrade (fixed) | still fixed | E: re-fetch at 3600 s + 1 ns. Guard still none in the repository (harness is scratch only; K: the time-dependent code cannot be tested natively) |
| #46 failed fetches repeated (fixed) | still fixed | E: no message within 300 s of a failure, under concurrency and traps |
| #54 no integration tests | still open (K) | This harness drives the real bytes with replies, traps and time, but lives in scratch files |
| #58 bridge not drivable off-chain | still open (K) | Same; the emulator closes it for these invariants if kept in the repository |

## Candidate findings

None new. The only invariant violations found (`pending-not-alive`, `ACTIVE_RUNS`) all
follow a trap in the cleanup callback and are the already-reported "release of shared
resources in the fallible cleanup" finding. This round adds exact evidence: every one of the
cleanup's 8 `ic0` calls is a point where a trap leaks the run's slot and pending id, for all
three languages. A search leaks only the pending id. The 1 h / 5 min TTLs, the 512 KiB
limit, CL17 and CL18 held in every other case.

## Not tested, and why

- Instruction accounting across executions: the emulator's `performance_counter` restarts each
  execution and is scaled from wall time, so the call-context counter (I13) and the real cost
  of a cleanup are not modelled. Whether the replica gives cleanup callbacks a smaller
  instruction budget than other executions was not checked.
- Upgrades with runs in flight (callbacks to a new module, `post_upgrade` with a pending run): K
  (volatile bookkeeping finding); not emulated.
- Real causes of a cleanup trap (stable memory exhaustion, instruction limit) were stood in for
  by injected traps; not provoked on the replica.
- Best-effort timeouts (reject code 6) were not delivered as such; rejects with code 3/4/5 take
  the same `Err` path in `fetch_did`/`call`.
- Insufficient liquid cycles (`check_liquid_cycle_balance_sufficient` failing before
  `call_perform`) was not injected separately from refused sends; the bridge sees both as an
  `Err` from the first poll.
- `remove_allowlist`, `set_candid_override` and `set_limits` during in-flight runs were not
  interleaved (other lenses own them; K).
- The query reads (`get_policy`, `get_audit_logs`) were not rolled back in the emulator; they
  do not write state.
- Nothing was run on the replica this round (no live step needed; no setting changed).
