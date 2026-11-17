# Variant sweep r4-4

Run `2026-11-17-295d64b`, Phase 6. Pattern: **"A limit applied to new writes but not to
data already stored (L13). A page bounded by count, not bytes."** Seeds (known, not
re-reported): legacy audit entry id 60 (8.39 MB) is unreadable; `get_audit_logs` page capped
at 100 entries, not bytes; `get_policy` returns admin inputs whole; demo_store
`list_orders(null)`.

Method: for the first half, every write-time check in the code was listed and its history
traced with `git log -S` / `git log -L` (when was it added, did the store already hold data
written without it, is it re-applied on read or in `post_upgrade`). For the second half,
every reply or in-memory list that is capped was checked for a count-only cap, skipping rows
already settled by sweeps r2-1 (search 40-method cap) and r2-6 (growing stores and reads).
One native probe was run in a private copy (`cargo test --lib`, shared
`CARGO_TARGET_DIR`); the copy was deleted afterwards. Evidence: R = reproduced, C = code path.

Result: **one new candidate (S4-4-1).**

## Write-time checks on durable state, and whether stored data is held to them

| Row | Store / write path | Check at write | When added | Applied to data already stored? | Verdict |
|---|---|---|---|---|---|
| D5 / AD7 / E11 / LIM19 | `CANDID_OVERRIDES` via `set_candid_override` | `d.len() > MAX_DID_BYTES` (512 KiB) refused; then `Service::parse` | size check added in `5d856e3`; before it, only `Service::parse` (`git show 5d856e3^:src/lib.rs:287-293`), so any override up to the ingress limit (~2 MiB) that parsed was stored | **no**: `override_service` (`src/bridge.rs:216-227`) parses whatever D5 holds, `IcBridge::candid` (`src/bridge.rs:475-477`) returns it as is, `cached_service` serves it to both search paths; `post_upgrade` (`apply`, `src/lib.rs:38-49`) does not touch D5; no endpoint lists D5 (`PolicyView`, `src/lib.rs:27-36`) | **candidate S4-4-1** (R) |
| D3 / AD5 / E9 | `TOKENS` | `trim().to_ascii_lowercase()`, 64 hex | since the first commit `9afe403` (`git log -S is_ascii_hexdigit`) | n/a: no key was ever stored without it, so `list_tokens`' `e.key()[..16]` (F10) is safe | holds (C) |
| D3 labels | token label | none | never | n/a (no limit exists to bypass) | **known** (admin inputs unbounded) |
| D2 / AD2 / E6 | `ALLOWLIST` | none (key is a `Principal`, canonical by type) | `set_allowlist` unchanged since `9afe403` (`git log -L`) | n/a | **known** (admin inputs unbounded; `aaaaa-aa`/self accepted) |
| D1 / AD4 / E8 | `LIMITS` | none | never validated | n/a; schema growth (opt fields) **known** | **known** (limits unvalidated; terse set_limits) |
| D4 / LIM13 / LIM14 | `AUDIT` entries: `error` <= 4 KiB, violations <= 16 x 512 B | at write (`append_audit` callers) | rounds R4-R6 | no: `audit_page` reads any stored size | **known** (seed: legacy entry 60, 8.39 MB) |
| D4 schema | `AuditStatus` variants, `run_id`, `methods` | n/a | `f5305be`, `d508ece`, `cdfa3d9` | old entries decode as `None` | **known** (#51, rollback trap) |
| D5 key form | override key = `Principal::to_text()` | canonical by type | since `9afe403` | n/a | holds (C; L1-r1 probe for uppercase lookup) |
| D6 | certified data | constant | n/a | re-set in `post_upgrade` | holds |
| D7 | canister settings | platform | n/a | n/a | not applicable |
| D8 / D9 / D10 | demo_store heap (re-seeded on upgrade), browser token, `.env` | none / n/a | n/a | heap reset on upgrade, so no legacy data | not applicable (orders growth **known**) |
| V2 / V4 / V5 | `DID_CACHE`, `DID_FAILED`, `OVERRIDES` | `MAX_DID_BYTES` after fetch | `5d856e3` | heap, emptied by every upgrade, so no pre-limit entry survives a deploy of the limit | holds (C) |
| V1, V3, V6-V10 | counters, pending ids, allocator, rng, engine heaps | n/a | n/a | heap | not applicable |
| S2 `candid_parser` | parser that validated stored overrides | pinned `0.4.1` since `9afe403` (`git log -p Cargo.toml`) | n/a | a parser upgrade could make a stored override fail; has not happened | holds (C) |

## Caps bounded by count, not bytes

| Row | Cap | Bytes bounded? | Verdict |
|---|---|---|---|
| LIM5 / E13 / W13 | `MAX_AUDIT_PAGE` 100 | no | **known** (seed) |
| E12 | `get_policy` allowlist, tokens, `pending_run_ids` | no cap at all | **known** |
| LIM32 / W3 / B16 | search `MAX_METHODS` 40 | no; `types` is the transitive closure, and every canister's full `allowed_methods` list is printed even when cut (`src/search.rs:95-143`) | **known** (r2-1: results over 1 MB truncated; admin-sized allowlist returned whole) |
| LIM8 | `MAX_BATCH` 50 | reply yes (LIM7 1.9 MB); request by ingress | holds for reply size; parse amplification **known** |
| LIM9 / LIM10 | 5 searches, 1 script per request | reply by LIM7 | **known** (query path, shared message budget) |
| LIM14 | 16 violations x 512 B | yes, both | holds (marker overrun **known**) |
| LIM15 | log 64 KiB | bytes yes, lines no | **known** (empty lines free) |
| LIM16 | 200k nodes / 4 MiB / 128 | both | holds (py_repr / blob gaps **known**) |
| LIM25 | JS `MAX_PENDING` 64 per round | each arg by LIM16; payload limit not checked | **known** (inter-canister payload trap) |
| LIM26 | Python 256 suspensions | per value LIM16 | **known** (host replies not counted) |
| LIM27 / W11 | Rhai `MAX_REQUESTS` 64 answers kept | per reply only by the IC reply limit and `DECODING_QUOTA` | **known** (r2-6: W11 / Rhai size limits) |
| LIM12 | 8 runs | per run heap unbounded (JS) | **known** (#24, wasm_memory_limit) |
| E20 / W15 | demo_store `list_orders`, `sales_summary` | no | **known** (seed) |
| E16-E19 `icp.candid` | one interface | yes, 512 KiB, except S4-4-1 | see S4-4-1 |

## Other inventories

| Rows | Verdict |
|---|---|
| C1-C15 | C1 covered above; C2 heap only (reset on upgrade); C3-C15 store nothing a later version reads |
| A1-A12 | only A5 writes the durable stores that later checks apply to |
| E1/E2 | `apply` re-validates token hashes it is given (F1) but never revisits D5, D2 or D1 contents | covered by S4-4-1 and known post_upgrade findings |
| E3-E5, E14, E15, E21-E28 | no stored data with a newer write-time check |
| U1-U16 | U10/U11 are the inputs held to `MAX_DID_BYTES`; only U11 is stored (D5) |
| O1-O10 | O3 audit (known), O4 admin stores (above) |
| K1-K12 | K11 upgrade is where a migration would run; none exists for D5 |
| W1-W16, F1-F16, B1-B17, VF1-VF8 | nothing persisted across versions other than the rows above; F3 (decode of old schema) **known** |
| AD1-AD9 | above |
| S1-S10 | S2 parser pin above; rest not applicable |
| CL22 | "Interfaces over 512 KiB are refused" (README:444): false for overrides stored before `5d856e3` (S4-4-1); grid-L16-r1 marked it "holds (untested)" from the write path |
| T1-T12 | `lib.rs` test at :496-503 covers the write-path refusal only; no test reads an over-limit stored override |

## S4-4-1 probe (R, native, deleted after)

Added to a private copy's `src/lib.rs` tests: build a valid 624,310-byte `.did`
(8,552 methods); `super::set_candid_override(p, Some(did))` returns
`Err("interface is 624310 bytes, over the 524288-byte limit")`; then
`crate::policy::set_candid_override(p, Some(did))` (the store write the pre-`5d856e3`
endpoint made after parsing) and `bridge::cached_service(&p)` returns the parsed service
(8,552 methods), and `IcBridge::candid("RYJL3-…")` returns all 624,310 bytes. Output:

```
write path: Err("interface is 624310 bytes, over the 524288-byte limit")
did bytes 624310 (> 524288), methods parsed: 8552
icp.candid returned 624310 bytes
test tests::sweep_r4_4_legacy_override_over_limit_is_served ... ok
```

## Not tested

Whether the live deployment holds an override written before `5d856e3`: there is no
endpoint that lists D5, and this sweep is not the Live phase. The cost of re-parsing a ~2 MiB
legacy override on the query path (5B limit) was not measured; the known finding on query
re-parse of a <= 512 KiB override covers the mechanism.
