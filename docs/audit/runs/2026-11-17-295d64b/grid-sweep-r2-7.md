# Variant sweep r2-7: per-key cache filled after an await, no in-flight marker

Run `2026-11-17-295d64b`, Phase 6, round 2. Pattern: **"A per-key cache filled only after an
await, with no in-flight marker, so N concurrent misses do the work N times and each is
charged against a shared budget."** Seed (known, not re-reported): L4-1, concurrent misses on
one interface each send their own fetch (`IcBridge::candid`, `src/bridge.rs:471-508`), within
one JS `Promise.all` / Python `gather` batch and across concurrent scripts and searches; L9-5
(expired entry discarded on a failed refresh).

Method: listed every cache, memo, map and set that outlives one step (`thread_local!`,
`static`, `BTreeMap`/`HashMap`, `cache`, `memo`, `seen`) in `src/`, `src/runtimes/`,
`demo_store/src/`, `shims/`, `src/demo.html`, `video/*.mjs`, `scripts/`; for each, found where
it is read, where it is written, and whether an `await` (or a JS `await`) separates the two;
then looked for concurrent callers of that path and the budget each miss is charged to. Also
asked the inverse question the pattern implies: without an in-flight marker, can a fill that
lands after the await undo something done during the wait (eviction, refresh)? Evidence: C =
code path. Nothing was built and nothing live was touched (`IcBridge` cannot run natively,
register #58). Excluded: `docs/per-customer-canisters-plan.md`, `design/`.

## Candidate findings

| Id | Sev | Title | Evidence |
|---|---|---|---|
| SW7-1 | Low | An interface fetch in flight when `remove_allowlist` runs re-inserts the removed canister's interface into `DID_CACHE` (fresh 1 h timestamp) after the eviction, so `icp.candid` keeps serving it with no allowlist check, and the cache is no longer "bounded by the allowlist size" | C |

### SW7-1 detail

`remove_allowlist` (`src/lib.rs:301-304`) calls `bridge::evict_interface` (`src/bridge.rs:270-275`,
"Forget everything cached about a canister (called when it leaves the allowlist)"), which
removes the key from `DID_CACHE`, `DID_FAILED` and `OVERRIDES`. `IcBridge::candid`
(`src/bridge.rs:486-507`) reserves, awaits `fetch_did` (one or two messages), then in the
reply continuation inserts `DID_CACHE[key] = (did, svc, now)` unconditionally: there is no
in-flight marker or generation for the key, and the allowlist is not re-read. Sequence:

1. A script (or update-path search) calls `icp.candid(X)` / `icp.call(X, ..)` on a cold X;
   `reserve` passes, `canister_metadata` is sent (await).
2. The controller calls `remove_allowlist(X)`; `evict_interface` finds nothing to remove.
3. The fetch reply arrives; line 499 inserts X's interface with `now` as its fetch time.
4. For the next hour, any script's `icp.candid(X)` returns it from `cache_get` at line 478,
   before `reserve`, so with no allowlist check (known finding: `icp.candid` serves cached
   interfaces without an allowlist check; eviction was the only thing bounding it).

A failed fetch lands the same way in `DID_FAILED` (line 504): after `remove_allowlist(X)` +
`set_allowlist(X, ..)` during the window, X is refused for 5 minutes with the stale error.

Window: normally one round trip. X itself controls its length: if X does not publish
`candid:service`, the fetch falls back to `X.__get_candid_interface_tmp_hack` with
`bounded_wait` (ic-cdk default 300 s), so a target being removed for misbehaving can hold the
fill open for up to 300 s. The comment at `src/bridge.rs:192-193` ("bounded by the allowlist
size") also stops being true: repeated add/remove cycles leave entries for canisters that are
not allowlisted, until upgrade. Overlap: the known finding "Allowlist and token checks taken
before a suspension are trusted after it" covers a *call* sent after removal; this is the
cache fill, a different effect of the same missing re-check. A verifier may merge them.
Fix: in the continuation, insert only if `policy::is_allowlisted(p)` still holds (or keep a
per-key generation bumped by `evict_interface` and drop fills from an older generation).
Regression test: needs the `IcBridge` seam asked for by register #58.

## Grid

Cells: **finding** (id above), **known** (in the known list), **holds** (with the reason),
**n/a** (the row has no cache, no await between read and fill, or no concurrency).

### Volatile shared state (the rows that can hold a cache)

| Row | Cell | Reason |
|---|---|---|
| V1 `ACTIVE_RUNS` | n/a | counter, not keyed; taken synchronously before any await (`src/lib.rs:208`) |
| V2 `DID_CACHE` | **known** L4-1, L9-5; **finding SW7-1** | filled after the fetch await, no in-flight marker: duplicate fetches (known); fill after eviction (SW7-1) |
| V3 `PENDING_RUNS` | n/a | per-run set, written synchronously in `record_message` |
| V4 `DID_FAILED` | **known** (L4-1 detail names it); SW7-1 | same fill site as V2: concurrent misses on a failing target each repeat the failing messages until the first failure lands (same root cause as L4-1). Last-writer races between a success and a failure are harmless: `cache_get` is consulted before `failed_get` (`src/bridge.rs:478-485`) |
| V5 `OVERRIDES` | holds (for this pattern) | filled by `Service::parse` with no await between miss and insert (`src/bridge.rs:216-227`); in the query path the insert is discarded, already recorded as an observation in grid-L4-r1 |
| V6 Monty memory atomics | n/a | counters, not a cache |
| V7 getrandom `STATE` | n/a | stream, not keyed |
| V8 Boa GC heap, `Symbol.for` registry | n/a | no await between lookup and insert (Symbol registry growth is known) |
| V9 ic-cdk executor | n/a | |
| V10 engine statics | n/a | none keyed by script input with an await in between (engines run synchronously between host round-trips) |
| V11 demo_store `STORE` | n/a | no awaits in demo_store |
| Rhai `answers` (per run) | holds | keyed by request sequence, one request awaited at a time (`src/runtimes/rhai_rt.rs:211-218`); a replay never resends an answered request |
| JS `State` pending queue / Python `queued` (per run) | holds as a cache (no keyed cache); duplicates are L4-1 | requests are not deduplicated by key, so two `icp.candid(X)` in one round are two fetches: same root cause as L4-1, in `IcBridge::candid` |

### Other inventory sections

| Rows | Cell | Reason |
|---|---|---|
| C1, C1a | known L4-1 / finding SW7-1 | the only await-filled keyed caches are V2/V4 in `src/bridge.rs`, compiled into every feature subset |
| C2 demo_store | n/a | no async code |
| C3 web-time shim, C4 chrono, C8-C11, C15 | n/a | no caches or awaits |
| C5 demo page `cache.store` / `cache.defs` / `cache.codeDefs` (`src/demo.html:361-423`) | holds | filled after a `fetch` await with no in-flight marker, but the only caller, `runComparison`, disables every button first (`src/demo.html:438-439, 514`), so no two misses overlap; `classicDefsChars` and `codeDefsChars` run concurrently but use different keys. Cost if it did overlap would be the client's own requests |
| C6 `scripts/demo-setup.sh` | n/a | sequential CLI calls, no cache |
| C7 `icp.yaml` | n/a | |
| C12 video (`tts.mjs`, `render.mjs`, `stills.mjs`, `capture-demo.mjs`, `cdp.mjs`) | holds | no response cache; every loop awaits sequentially (`tts.mjs:64-65`, `render.mjs:22-24`, `capture-demo.mjs:25-32`); `cdp.mjs:25` `pending` map is keyed by unique message id |
| C13 audit workflow, C14 doc | n/a | |
| A1-A12 | covered via the entry points below | |
| E1-E3, E6-E15 | n/a | no awaits (E7 is the eviction side of SW7-1) |
| E4/E4b query path | holds / known | query reads `cached_service` only; a miss returns `upgrade: true` (no fill in the query). N clients doing the same cold search become N update calls, each fetching: the cross-request half of L4-1 |
| E5, E5a | known L4-1; SW7-1 | |
| E5b `run_search` | known L4-1 (across requests); holds within one search | one search fetches its scope sequentially (`src/lib.rs:129-149`) and `scope` comes from the allowlist map (unique keys), so no duplicate fetch inside one search; searches in one HTTP request run one after another (`src/mcp.rs:401`), so search 2..5 see search 1's fill |
| E5c/E5d callbacks | SW7-1 | the fill runs in the reply callback |
| E16 JS `icp.call`/`icp.candid` | known L4-1 | `join_all` per round (`src/runtimes/js.rs:391-400`) |
| E17, E19 print/debug | n/a | |
| E18 Python `call_async` + `gather` | known L4-1 | `join_all` (`src/runtimes/python.rs:262`) |
| E19 Rhai `icp::call_canister` / `icp::candid` | holds | one request in flight per run; across runs it is L4-1 |
| E20-E25 demo_store | n/a | |
| E26 demo page | holds | see C5. Observation only, not this pattern: the Classic "What needs restocking?" turn sends 12 concurrent `execute_icp_script` requests (12 seed products, `demo_store/src/lib.rs:129`; `src/demo.html:312, 448`) against `MAX_CONCURRENT_RUNS` 8; whether any get "busy" depends on how replies interleave on the subnet. Not tested (see not_tested) |
| E27, E28 | n/a | |
| U1-U16 | n/a except U10 | U10 (interface text) is the value cached at V2: SW7-1 / L4-1 |
| O1 | known L4-1 | each duplicate `icp.call` miss also takes a call slot before its fetch |
| O2 | known L4-1, L4-2; SW7-1 | |
| O3 audit writes | known L4-1 | duplicate fetches write duplicate `Started`/`Call` entries |
| O4-O10 | n/a | |
| D1-D10 | n/a | durable state is written synchronously by admin methods or audit appends; no await-filled durable cache |
| K1-K12 | K5 known; K11 holds | K5: fill at the end of `fetch_did`'s continuation (SW7-1). K11: an upgrade empties V2/V4 and drops in-flight fetches with their callbacks, so no stale fill survives an upgrade |
| LIM1 call cap | known L4-1 | the shared budget the duplicate misses are charged to |
| LIM2 instruction budget | known L4-1 | duplicate fetches each parse the interface in their own continuation (up to 512 KiB, `Service::parse`) and charge the run's budget |
| LIM9 `MAX_SEARCHES` | holds | sequential, see E5b |
| LIM12 run slots | holds for this pattern | not a keyed cache; searches taking no slot is known |
| LIM18 TTLs | known L9-5; SW7-1 | the fill uses `now` at reply time, so a re-inserted entry gets a full fresh hour |
| LIM25 JS `MAX_PENDING` 64 | known L4-1 | up to 64 duplicate fetches of one cold interface per round, each a call slot (capped by LIM1) |
| LIM26 Python suspensions 256 | holds | a duplicate is one host request like any other |
| LIM27 Rhai `MAX_REQUESTS` | holds | sequential |
| other LIM rows | n/a | |
| W1-W16 | W7 known L4-1 (parse repeated per duplicate); W11 holds (per run, by sequence); W16 holds (C5); rest n/a | |
| F1-F16 | n/a | no cache fill near an aborting construct beyond the known parse/decode traps |
| B1-B17 | n/a except B7 (parse at fill, known) | |
| VF1 | known L4-1 | canister pays every duplicate fetch (cycles and audit bytes) |
| VF2-VF8 | n/a | |
| AD2 / AD3 | **finding SW7-1** | eviction is defeated by an in-flight fill |
| AD7 overrides | holds | override lookup has no await; precedence over V2 means an override set during an in-flight fetch still wins (`src/bridge.rs:441, 475`) |
| other AD rows | n/a | |
| S1-S10 | n/a | |
| CL7 call cap claim | known L4-1 | |
| CL22 cache TTLs | SW7-1 | "cached 1 h" holds per entry, but an entry can be re-created for a removed canister |
| other CL rows | n/a | |
| T1-T12 | gap | no test drives `IcBridge::candid` concurrency or eviction (register #58) |

## Refuted while sweeping

- **Stale interface overwriting a newer one across a target upgrade.** Two concurrent fetches
  that straddle the target's upgrade: the later reply wins, and it gets a fresh `now`. Both
  `canister_metadata` requests go from this canister to the management canister, which
  answers in request order, so the later reply is the newer read. Mixed paths (one metadata,
  one `tmp_hack` fallback) could in theory invert the order, but that needs the target to
  drop its `candid:service` section and be upgraded inside one fetch window; not reproduced,
  left as unverified.
- **Success/failure last-writer race on V2/V4.** Harmless: lookups check `DID_CACHE` first.
