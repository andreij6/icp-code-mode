# Coverage grid: L3 Resource bounds, round 4 — hash-collision quadratic work

Run `2026-11-17-295d64b`. Lens L3: engine and host hash tables use public or unkeyed hashes, so
an attacker who chooses keys can force O(n^2) probing. Rows from `inventory.md`. Cells:
**finding** (candidate id below), **holds** (why), **n/a** (why).

Candidates returned by this round: **L3r4-1** (host-side converters, no budget applies),
**L3r4-2** (VM-side set/dict ops, one unpolled native op crosses 40e9 within the policy budget),
**L3r4-3** (root cause: no hash table in either engine has a secret seed — the "random" seed is
the fixed splitmix stream). Already filed and not repeated: bigint `**` and the native-op traps
of grid-L3-r3 (L3r3-1, L3r3-2); host converter cost per node (SW2-1-1).

## The hash functions (all public or unkeyed — confirmed from source)

| Table | Hash | Keyed? | Source |
|---|---|---|---|
| Python `dict`/`set` (`PyHashTable`, hashbrown `HashTable<usize>`) | per-value `py_hash` | — | `monty-1.6.0/src/types/hash_table.rs:118-127` |
| Python `int` key hash | `HashValue::new(i.cast_unsigned())`; `new` only bit-inverts, `raw()` returns `i` | hash == the int itself | `monty-1.6.0/src/value.rs:1189-1190`, `src/hash.rs:70-84` |
| Python `str`/`bytes` key hash | `DefaultHasher::new()` = SipHash-1-3 with key **(0,0)** | unkeyed (fixed zero key) | `monty-1.6.0/src/hash.rs:160-186` |
| Boa object property table | `rustc_hash::FxHashMap` (FxHash) | unkeyed, trivially invertible | `boa_engine-0.22.0/src/object/shape/property_table.rs:5,11,19` |
| Boa `Map.groupBy`/`Object.groupBy` | `BuildHasherDefault<FxHasher>` | unkeyed | `boa_engine-0.22.0/src/builtins/map/mod.rs:707-735` |
| Boa `Map`/`Set` (`OrderedMap`, IndexMap) | std `RandomState` (SipHash) | seed from `getrandom` | `boa_engine-0.22.0/src/builtins/map/ordered_map.rs:5,39` |
| Boa shared-shape transitions | IndexMap with `RandomState` | seed from `getrandom` | `boa_engine-0.22.0/src/object/shape/shared_shape/mod.rs:365-369` |
| ahash tables (monty `AHashSet`, rhai) | ahash | seed from `getrandom` | `monty-1.6.0/src/types/hash_table.rs:31` |

The one seed source, `getrandom`, is the fixed deterministic splitmix64 stream
(`src/lib.rs:433-449`), seeded from the compile-time constant `0x9E3779B97F4A7C15` and never
reseeded. Its own comment says "fine for hash seeding." It is **not** fine against a chosen-key
adversary: the seed is public and constant, so an attacker computes the same SipHash/ahash the
canister will use and precomputes multi-collisions offline. FxHash and SipHash(0,0) have no seed
at all. **No hash table in engine or host has a secret seed (L3r4-3).** The grids were searched
for "collision"/"HashDoS": only the token-prefix cell existed; this is new.

## How the collision bites

- **hashbrown `HashTable`** (Python dict/set) indexes into the control array by `hash & bucket_mask`
  and matches the top 7 bits as the control byte. For keys `i<<32` (i = 0,1,2,...) the raw hash is
  `i<<32`: the low 32 bits are all zero, so `hash & bucket_mask == 0` for every key (bucket_mask is
  far below 2^32). All N keys begin probing at bucket 0; insertion is O(n) per key, O(n^2) total.
  The control-byte (h2) differs, so lookups of *absent* keys stay cheap, but building the table, a
  set/dict intersection, union, `==`, `issuperset`, `symmetric_difference` and `dict.update` each
  re-probe the colliding region. Str keys reach the same state by SipHash(0,0) low-bit collisions.
- **FxHash** (Boa properties) is `state = (state ^ byte) * K`; a chosen key sets any target hash,
  so an attacker fixes `hash & mask` and the top bits alike — strictly easier than hashbrown.

## Grid

| Row | Cell | Reason |
|---|---|---|
| U6 Script source (program) | **finding L3r4-2** | A token holder writes a script that builds a colliding `set`/`dict` and runs one set/dict operation over it. Construction via `set(range(0,N<<32,1<<32))` is polled (interrupted at the native-VM wall budget), but a single collection operation — `keys()&set`, `set==set`, `issuperset`, `symmetric_difference`, `dict.update(colliding_dict)` — runs to completion inside one native op with no budget poll. Native (R, private copy, release): N=20,000 colliding ints, `d={k:1 for k in s}; len(d.keys()&s)` ran 3.22 s against the 2 s native budget, ~1.2 s unpolled inside one op, vs 0.001 s for `set(range(N))`; 580x. On chain the same unpolled op reaches the 40e9 per-message limit within the default 20e9 policy budget → trap (same class as L3r3-1, F14). JS object-property and `Object.fromEntries`/`new Map`/`new Set` with FxHash-colliding keys are the Boa equivalent. |
| U8 Candid reply bytes from targets | **finding L3r4-1** | The decoded reply of a hostile allowlisted target is turned into engine values host-side: `python::from_json` (`src/runtimes/python.rs:104-115`) builds `MontyObject::dict` keyed by SipHash(0,0); `JsValue::from_json` (`src/runtimes/js.rs:403`) builds a JS object keyed by FxHash. Field names come from the target-authored `.did` (up to `MAX_DID_BYTES` = 512 KiB, `src/bridge.rs:186`): `idl_to_json` inserts each record field under `label_name` (`src/candid_bridge.rs:345-349`). A target whose return record has thousands of field names chosen to collide forces O(n^2) insertion. Crucially this runs **between** budget checks (JS: after `check_budget` at `js.rs:376`, before the next round; Python: inside the host-reply conversion), as straight-line Rust with no instruction poll, so **no value of `max_instructions` bounds it** — it runs until the 40e9 IC message limit traps, with no catchable error and no audit entry (register #11). |
| W8 JS parse/snapshot/`JsValue::from_json` of replies | **finding L3r4-1** | `JsValue::from_json(&j, ctx)` at `src/runtimes/js.rs:403` stores reply object keys into the Boa property table (unkeyed FxHash). Host-side, unpolled — see U8. |
| W10 Python parse / `to_json` / `from_json` of replies | **finding L3r4-1** | `from_json` (`python.rs:115`) builds a dict keyed by SipHash(0,0) from reply object keys. Host-side, unpolled — see U8. `to_json` repr cost is SW2-1-1 (not repeated). |
| B8 JS value<->JSON | **finding L3r4-1, L3r4-2** | JSON->JS (`JsValue::from_json`, host-side reply path, L3r4-1) and script-visible `JSON.parse`, `Object.fromEntries`, object literals, `new Map`/`new Set` (L3r4-2) all land keys in FxHash property tables or RandomState IndexMaps whose seed is the predictable splitmix stream. |
| B9 Python MontyObject<->JSON | **finding L3r4-1, L3r4-2** | JSON->MontyObject `from_json` dict keyed by SipHash(0,0) (host-side, L3r4-1); script-side `dict`/`set`/`json.loads` of a colliding object (L3r4-2). |
| V7 getrandom splitmix `STATE` (shared seed) | **finding L3r4-3** | This is the single seed for every ostensibly-randomised table (Boa Map/Set RandomState, Boa shape transitions, ahash). It is a fixed compile-time-seeded splitmix64 stream, so the seed is public and constant across replicas and runs. HashDoS mitigation that relies on an unpredictable seed is defeated. (The determinism itself is required for replica agreement, so the fix is not "make it random" but "key the untrusted-input tables with a value that is secret to the attacker, or bound the work" — see fixes.) |
| LIM2 `max_instructions` (default 20e9, settable u64) | **finding L3r4-1 (does not bound host path); L3r4-2** | For L3r4-2 the budget is the trap trigger: one unpolled colliding op added to a nearly-spent budget crosses 40e9 (as in L3r3-2). For L3r4-1 the budget is irrelevant — the host converter is not polled, so raising or lowering `max_instructions` neither prevents nor is honoured by the trap. A high `max_instructions` additionally lets a VM-side colliding op (L3r4-2) run further before the final post-run check, so it traps at 40e9 rather than ending with a clean `policy:` error. |
| LIM26 Python VM budget / turn <= 30e9 / 64 MiB | **finding L3r4-2** | The VM budget is polled between bytecodes and inside some loops (`LOOP_CHECK_INTERVAL`), but a single native set/dict operation over colliding keys is not polled inside its probe loop. Same mechanism as L3r3-1; the collision just makes a cheap-looking op O(n^2). The 64 MiB memory limit does not help: the colliding collection is small in bytes (N 16-byte ints = 320 KB at N=20,000). |
| F14 40e9 per-message IC trap | **finding L3r4-1, L3r4-2** | Both candidates reach 40e9 in one message: L3r4-2 from a script with a token (budget-bounded path still traps because the op is unpolled); L3r4-1 from a hostile target's reply with no budget at all. Trap = HTTP 503, no audit entry (nothing sent after the trap; register #11). |

## Not tested (handed forward)

- **On-chain reproduction of this round's two candidates.** The native 580x / 1.2 s-unpolled
  measurement (L3r4-2) and the FxHash/SipHash(0,0) code paths (L3r4-1) are first-hand; the 40e9
  **trap** on chain is asserted from the grid-L3-r3 result that unpolled native ops of this cost
  trap at 40e9, not re-measured here for the specific ops (`keys()&set`, `dict.update`,
  `set.__eq__`, `issuperset`, `symmetric_difference`). To reproduce: on the port-8010 network,
  scale N for each op until HTTP 503, and for L3r4-1 deploy a target whose `.did` return record has
  ~20k-40k field names colliding under FxHash32 (JS) and SipHash(0,0) (Python) and call it.
- **Exact collision-set construction for the host path.** FxHash multi-collisions are trivial to
  generate (invert the multiply); SipHash(0,0) low-bit multi-collisions are a known offline search.
  Not generated here. Feasibility is not in doubt (seeds are public/zero), but the concrete 512 KiB
  `.did` that maximises O(n^2) within the IC decode quota was not built.
- **Whether `serde_json::Map` (preserve_order = IndexMap) already pays O(n^2) at `idl_to_json`**
  before the engine converter, i.e. one extra colliding insert pass on the same keys. Not measured.
- **rhai.** Rhai object maps use ahash (predictable seed) but rhai has no host-reply converter on
  the colliding-key path and its own small-size limits; left for a rhai-specific pass.
