# Variant sweep r1-0

Pattern: "A read path that serves policy-scoped data from a cache or durable override before
the policy check that guards the fetch path; a revocation that clears caches but not the
durable source the read consults first."

Method: code reading at `295d64b` (no build needed; every cell below is a code-path
statement). For each inventory row the question was: does this row read data whose access is
governed by a policy (allowlist, method list, token, limits, controller) from a cache or a
durable store *before* that policy is checked, and does any revocation leave a source that
the read consults first?

Result: no new candidate. Every hit is an already-known finding (L1-1, L1-2/L8-3, L14-6,
known post_upgrade/InitArgs and certified_reads findings).

## Read paths that consult a cache or durable store

| Row | Read path | Cache / durable source consulted | Policy check and order | Verdict |
|---|---|---|---|---|
| E16/E18/E19 `icp.candid` (JS/Python/Rhai all route to `IcBridge::candid`, `js.rs:393`, `python.rs:133`, `rhai_rt.rs:215`) | `bridge.rs:471-486` | D5 override, then V2 `DID_CACHE`, then V4 `DID_FAILED`, all before `reserve` | allowlist checked only on a miss | **known** (L1-1; eviction-race variant also recorded under L1-1/L8-3) |
| E16/E18/E19 `icp.call` | `bridge.rs:510-516` | override, then `DID_CACHE` in `service()` | `reserve` (`check_call`: allowlist, method, cap, budget) runs first | holds; post-await re-check missing is **known** (L1-2/L8-3) |
| E4b query-path search `search_cached` (`lib.rs:111-117`) | `cached_service` (`bridge.rs:207-213`) | override, then `DID_CACHE` | entries come only from `search::scope(.., list_allowed())`, read live each request | holds: a non-allowlisted canister yields no entry even if it has an override or a cache entry |
| E5b update-path search `run_search` (`lib.rs:121-190`) | `cached_service`, `bridge.candid` | override, cache, failed cache | scope from live allowlist at start; mid-search removal not re-checked | holds for the pattern; the mid-search snapshot is **known** (L8-3 / "Allowlist and token checks taken before a suspension") |
| `search::render` method filter (`search.rs:103-110`) | allowed method list | live `e.allowed` from the same request's `list_allowed()` | filter applied to every rendered method | holds |
| V5 `OVERRIDES` parsed cache (`bridge.rs:216-226`) | heap parse cache | D5 is read first and the heap entry is used only when its text equals D5's | `set_candid_override(p, None)` removes D5, so the stale heap parse is never served | holds (revocation of the durable source wins over the cache) |
| V2/V4 eviction by `remove_allowlist` (`lib.rs:300-304`, `bridge.rs:270-275`) | clears V2, V4, V5 | D5 not cleared | | **known** (L1-1, L14-6: overrides outlive `remove_allowlist`) |
| `set_allowlist` narrowing (`lib.rs:295-298`) | no cache keyed by method list exists | `check_call` and `search::render` read the live list | | holds (interface caches are per canister, not per method; `icp.candid` returning methods outside the list is documented: `policy.rs:198` "any allowlisted canister") |
| W11 Rhai `answers` replay cache (`rhai_rt.rs:63-80`) | per-run cache by sequence number | each request is compared with the cached one (divergence = error); a new request reaches the bridge and its policy check | | holds: per run, not shared, no read before a check beyond data the run already obtained |
| JS `pending` / Python `queued` (`js.rs:383-398`, `python.rs:247-262`) | per-round queues, no memoisation | every request reaches the bridge | | holds |

## Credentials and revocation

| Row | Durable / cached source | Read before the check? | Revocation clears it? | Verdict |
|---|---|---|---|---|
| D3 tokens, `verify_token` (`policy.rs:229-232`) | stable map only; no heap cache of tokens or of authenticated callers | E3, E4, E5 call `authenticate` each request (`lib.rs:276-277`, `mcp.rs:345, 368`) | `revoke_token_hash` normalises with the same trim + lowercase as `add_token_hash` (`policy.rs:211, 220`) | holds; run already in flight after revocation is **known** (L1-2/L8-3) |
| Batch in one `http_update` (`mcp.rs:367-397`): token checked once, then up to 5 searches + 1 script awaited in sequence | none | | | **known** (same "checks taken before a suspension" finding) |
| E2 `post_upgrade` re-applying InitArgs | `icp.yaml` `init_args` | | revoked tokens / removed allowlist entries come back | **known** |
| D9 demo page token in `localStorage` | browser store | the server re-checks on every request (401 after revocation) | | holds for the pattern; indefinite retention is **known** |
| D10 / C12 `.env` token, Chrome profiles | disk | server re-checks | | holds for the pattern; Chrome-profile residue is **known** |
| E12-E14 admin queries | none | `guard = "controller"` reads `is_controller` live | | holds |

## Other rows

| Row | Verdict |
|---|---|
| D1 `LIMITS` | not applicable for this pattern (not access-scoped data); snapshot-vs-live inconsistency is **known** |
| LIM4 `certified_reads` / D6 | the certified-data constant that tells the gateway to skip verification is **known** |
| E4a demo page (C5, `mcp.rs:321-330`) | not applicable: static HTML, `cache-control: no-cache`; in-page `cache` object (`demo.html:361-422`) is per page load and holds only public tool definitions and the demo_store id from a search reply (splicing is **known**) |
| V1, V3, V6-V10 | not applicable: no policy-scoped data |
| D4 audit log | not applicable: read only by controllers via guarded queries |
| C2 demo_store (E20-E25) | not applicable: no policy at all (**known**) |
| C3, C4, C6-C11, C13-C15, S1-S10 | not applicable: build/config rows, no read path |
| B1-B17 conversions, W1-W16, F1-F16, LIM1-LIM32 (other than above), VF1-VF8, CL1-CL37, T1-T12 | swept for a cache or override consulted before a policy check: none besides the rows above |

## Not tested

- Whether the IC HTTP gateway / boundary nodes cache `POST /mcp` query responses (which
  carry search results and have no `cache-control` header): would be the same pattern one
  layer out. Needs a mainnet or ic-gateway deployment; not checkable on the local network.
