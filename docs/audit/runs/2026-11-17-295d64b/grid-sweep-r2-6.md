# Variant sweep r2-6

Run `2026-11-17-295d64b`, Phase 6. Pattern: **"Storage that grows from unauthenticated or
cheap requests; a read whose reply size grows with storage."** Seeds (known, not
re-reported): demo_store orders / zero-quantity lines and `list_orders(null)`; audit log
growth (register #31); `get_audit_logs` page capped by count, not bytes; Boa `Symbol.for`
registry; admin inputs returned whole by `get_policy`.

Method: code reading at `295d64b` plus a scan of every process-global or thread-local store
in the app and in the dependencies that run inside the canister (`grep` for `thread_local!`,
`static … Mutex/RwLock/OnceLock/LazyLock/RefCell` in boa_engine/boa_gc/boa_string/
boa_interner 0.22.0, monty/monty-types 1.6.0, rhai 1.27.0, candid 0.10.40,
candid_parser 0.4.1, ic-cdk 0.20.3, ic-cdk-executor 2.0.0). Nothing compiled or run (every
cell is a code-path statement, C). For each row the questions were: (a) can an
unauthenticated or cheap request make something that outlives the request grow, and
(b) does any read return, or iterate, an amount that grows with that store?

Result: **no new candidate.** Every growing store or growing read is an already-known
finding, or is bounded by something only a controller sets.

## Stores that outlive one request

| Row | Store | Who can make it grow, and by how much | Read that grows with it | Verdict |
|---|---|---|---|---|
| D4 | `AUDIT` StableLog | token holder, 1 entry per message + 1 closing entry per run/search; no rate limit; entry size grows with `max_calls_per_script` | `get_audit_logs` (100 entries, not bytes) | **known** (#31; page-by-count finding; legacy 8.39 MB entry) |
| D1, D2, D3, D5 | limits, allowlist, tokens, overrides | controller only (`guard = "controller"`, `lib.rs:295-333`) | `get_policy` returns allowlist + tokens whole | **known** ("Admin inputs are unbounded … get_policy returns them in one reply"); not cheap |
| D6, D7 | certified data, settings | system / controller | none | holds (fixed size) |
| D8/V11 | demo_store `STORE.orders` | anyone, `create_order`, any number of lines per order (incl. zero-quantity, duplicates) | `list_orders(null)` and `list_orders(Some(c))` (any seeded customer, since anyone may order for anyone) return all; `sales_summary` iterates every line of every order | **known** (orders grow / zero-quantity lines; one root cause). `list_orders(Some(c))` and `sales_summary`'s per-call cost are further symptoms of the same unbounded `orders` vector, not a separate root cause. `customers`, `products`, `inventory` cannot grow (`create_order` requires a seeded customer and product) |
| D9 | demo page `localStorage` token | page user, one key | none | **known** (indefinite retention); does not grow |
| D10 | `.env` | developer | n/a | not applicable |
| V1 | `ACTIVE_RUNS` | counter | `get_policy.active_runs` (u32) | holds (fixed size); leak on cleanup trap **known** |
| V2 | `DID_CACHE` | token holder triggers fill, but only for canisters that pass `reserve` (allowlisted, `bridge.rs:486`); one entry per canonical principal, ≤ 512 KiB text + parsed service; overwritten on refetch; evicted by `remove_allowlist` | `icp.candid` (≤ 512 KiB), search (40-method cap) | holds: bounded by allowlist size × `MAX_DID_BYTES`, which the controller sets. Post-await insert for a just-removed canister is **known** (allowlist not re-checked after the fetch await) |
| V3 | `PENDING_RUNS` | one id per in-flight run or fetching search; removed in `Drop` | `get_policy.pending_run_ids` (8 B per id) | **known**: grows only with concurrency (uncapped searches, L4-4) or a leaked cleanup (L2-5). With a fetch pending ≤ 300 s (bounded_wait) and `DID_FAILED` stopping later searches after the first failure, reaching a reply-size problem would need ~250k simultaneous fetching searches; not demonstrated, no separate root cause |
| V4 | `DID_FAILED` | inserted only after `reserve` passed (allowlisted), one per principal, message embeds target text | search / script errors | holds for growth (bounded by allowlist); echoed target text is **known** |
| V5 | `OVERRIDES` parsed cache | filled only for principals that have a D5 entry (controller) and reach `override_service` (`call` after `reserve`, or search scope = allowlisted) | none | holds for the pattern: controller-driven. Side note (not a candidate, admin-only): `set_candid_override(p, None)` removes D5 but not the heap parse (`policy.rs:233-238`, eviction only in `evict_interface`); the stale parse is never served (D5 is read first) and is at most one per principal a controller ever overrode, freed on upgrade |
| V6 | `LIVE_MEMORY` / `BASELINE_MEMORY` | atomics | n/a | holds (fixed size) |
| V7 | getrandom `STATE` | `Cell<u64>` | n/a | holds (fixed size); stream carry-over **known** |
| V8 | Boa GC heap (`boa_gc` thread-local) | per run, `force_collect` after; Wasm memory high-water never shrinks | `memory_size`, idle burn | **known** (L8-6, L18-8, register #24, "JS run can grow the Wasm heap past wasm_memory_limit") |
| V9 | ic-cdk-executor `METHODS`, `TASKS`, `PROTECTED_WAKEUPS`, `MIGRATORY_WAKEUPS` (`machinery.rs:56-64`) | one slot per in-flight method/task, released at completion | none | holds; leak only on a trapping cleanup (**known**, L2-5) |
| V10 | Engine statics | Boa: `GLOBAL_SYMBOL_REGISTRY` (**known**); `CANNOT_BLOCK_COUNTER`, `CODEBLOCK_ID_COUNTER`, `ASYNC_EVAL_QUEUE_INDEX` (counters, fixed size; modules disabled by `IdleModuleLoader`); `SEEN` (Debug printing, cleared); futex `CRITICAL_SECTION` waiters (unreachable: `Atomics` and `SharedArrayBuffer` deleted, `js.rs:314`); `RAW_STATICS_CACHE` (immutable). Monty: `CORE_ENTRIES`, `RESERVED_STRING_HASHES`, `DEFAULT_OS_POLICY`, `INT_MAX_STR_DIGITS_THRESHOLD` (all initialised once, fixed). Rhai: `HASHING_SEED`, `INDEXER_HASHES` (once), keyword tables; `Engine::new_raw()` per run (`rhai_rt.rs:120`). candid: `ENV`/`DOC_ENV`/`ID`/`NAME` keyed by Rust `TypeId` (static types only, `types/mod.rs:45-48`), never by parsed `.did` input | — | holds except the known Boa symbol registry |
| — | Canister log (ic-cdk `debug_print` on panic) | platform ring buffer, `log_memory_limit` 4096 B | `fetch_canister_logs` | holds (platform-bounded) |
| — | Stable-memory buckets (`MemoryManager`) | grow with D4 (known) and admin maps | n/a | holds beyond #31 |

## Per-request and per-run state (does not outlive the request)

| Row | State | Verdict |
|---|---|---|
| IcBridge `messages`/`touched`/`violations`/`logs` | capped per run (call cap, 16 violations, 64 KiB logs) | not applicable (dies with the run); cap overruns by markers **known** |
| JS `State`, Python `out`/`queued`, Rhai `answers` (W11) | per run; Rhai `answers` × 8 runs is a memory (L3/L4) item, **known** (W11 / Python host-reply finding) | not applicable to durable growth |
| E4 query path (`http_request`) | query state is discarded; `override_service` inserts into `OVERRIDES` there are discarded too | holds |

## Entry points: unauthenticated or cheap writes

| Row | Writes? | Verdict |
|---|---|---|
| E1/E2 | install/upgrade args (controller) | not cheap |
| E3 `inspect_message` | none (decode + hash only) | holds |
| E4/E4a/E4b | none persistent | holds |
| E5/E5a/E5b/E5c/E5d | audit, caches; `authenticate` runs before `parse_body` and before any tool (`mcp.rs:367-374`), inter-canister callers without a token get 401 and write nothing | **known** (#31, unaudited paid updates); no unauthenticated write |
| E6-E11 | controller only | not cheap |
| E12-E14 reads | `get_policy`, `get_audit_logs` grow with stores | **known** |
| E16-E19 host functions | reach storage only through E5 (audit, `DID_CACHE`) | covered above |
| E20 demo_store reads | `list_orders` grows (known); `sales_summary` work grows with total lines (same root cause); others read fixed-size seed vectors | **known** |
| E21-E23 demo_store writes | `create_order` appends (known); `cancel_order` flips status; `restock` saturating add | **known** / holds |
| E24, E25 | `__get_candid_interface_tmp_hack` fixed text; heap reseeded | holds |
| E26-E28 | demo page, setup script, video tooling: no shared store grows from a request (Chrome profiles in TMPDIR **known**) | holds |

## Other inventories

| Rows | Verdict |
|---|---|
| C1-C15, A1-A12 | Covered through the entry points and stores above; A1/A2/A8 (anonymous) cannot write C1 state; A10 writes demo_store (known) |
| U1-U16 | Inputs that reach a store: U5 search `query` (hashed into `script_sha256`, not stored), U7 method names (≤ 256 B into `methods`), U9/U10 target text (DID_FAILED / errors, known), U11 admin (controller), U13 demo_store (known). None new |
| O1-O10 | O3 is the audit store (known); O4 admin; O9 demo_store (known) |
| K1-K12 | Commit points only decide when D4 entries persist; no extra store |
| LIM1-LIM32 | Missing-kind list (`inventory.md` §9) re-checked: allowlist/token/override counts (admin, known), `query` keywords (cost, known), `canister_id` length (not stored), audit size (#31), concurrent searches / PENDING_RUNS (known), demo_store orders (known) |
| W1-W16 | W3/W4 search cost grows with allowlist and keywords (known); W13 audit page (known); W14/W15 (known) |
| F1-F16 | F4 append trap when stable memory cannot grow (known) |
| B1-B17 | No conversion stores data across requests |
| VF1-VF8 | VF1/VF5 storage cost of audit growth (known #31) and heap high-water (known L18-8/L8-6) |
| AD1-AD9 | Admin-sized stores (known) |
| S1-S10 | not applicable |
| CL1-CL37 | No claim bounds a store's size; CL32 "limit is capped at 100" is literal (known page-by-count finding) |
| T1-T12 | No test fills any store (register #54, known testability gap) |

## Not tested

Nothing was run: no live fill of `PENDING_RUNS` or demo_store to measure where
`get_policy` or `sales_summary` stop working; both are symptoms of known root causes.
Dependency statics were found by `grep` over the crate sources; a store hidden behind a
non-`static` global (for example, a leaked `Box` reachable from a once-initialised value)
would not be found that way.
