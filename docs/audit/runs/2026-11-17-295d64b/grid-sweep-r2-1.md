# Variant sweep r2-1

Pattern: **"A value budget that counts nodes while the per-node cost scales with the target
type's width; a budget checked before and after work but not during it."** Seed (known, not
re-reported): `json_to_idl` cost is value nodes x variant tags / record fields (L3-3).

Question asked of every inventory row: does this row convert, decode, render or copy a value
under a budget that counts *items* (nodes, values, methods, requests, entries, bytes on the
wire) while the work done *per item* grows with something the budget does not count (type
width, number size, text length, a memo table, a repr); and is any budget (value budget,
decoding quota, instruction budget) checked only before and after that work?

Evidence: code reading at `295d64b` plus native probes in a private copy
(`/private/tmp/claude-501/sweep-r2-1.*`, no `target/`, shared `CARGO_TARGET_DIR`,
`cargo test --release --lib sweep_*`, deleted afterwards). Instruction counts are macOS
`/usr/bin/time -l` "instructions retired" for the whole test process (native aarch64, a proxy
for wasm instructions, not an on-chain count; wasm32 has no native 64x64->128 multiply, so
bignum work is, if anything, dearer on-chain). Nothing deployed, nothing live touched.
R = reproduced, C = code path.

## New candidates

| Id | Sev | Title | Ev |
|---|---|---|---|
| SW2-1-1 | High | Python `to_json` turns big ints and unsupported objects into `py_repr()` text that is never charged to the 4 MiB byte budget and costs time that scales with the value's size, while the node budget counts each as one node; no instruction check runs during the conversion | R |
| SW2-1-2 | High (may merge with L3-3) | A `nat`/`int` argument given as decimal text is parsed with a quadratic algorithm (`BigUint::parse_bytes`), while the value budget charges it once (1 node, linear bytes): one 4 MB string costs ~284B native instructions inside one uninterruptible encode | R |
| SW2-1-3 | Medium | candid's decoding quota charges 1 unit per `opt` value, but every `opt` value clones the decoder's subtype memo (`gamma`), whose size grows with the expected interface's width; a hostile allowlisted target makes one 85 KB reply cost ~75B native instructions in the reply callback | R |

### SW2-1-1 (Python repr bypasses the value byte cap and the budget)

`src/runtimes/python.rs:62-100`: `budget.node()` once per object; `"int"` beyond i64
(`:71`), non-bytes `bytes` nodes (`:84`) and every other type (`:98`) become
`Value::String(o.py_repr())` with no `budget.bytes(..)`; dict keys are repr'd first and
counted after (`:92-93`). Python objects are shared, so `[x] * N` is N references in the VM
and N full reprs out of it. Probes (`run()` with `MockBridge`):

| Script | Outcome | Native instr. |
|---|---|---|
| `s = '1' * 30103` / `[s] * 1000` (control: same text as `str`) | refused: "more than 4194304 bytes of text" | 46M |
| `x = 1 << 100000` / `[x] * 1000` | **Ok**, 30,106,001 B of JSON (7x the cap) | 22.3B |
| same, as `icp.call(L, 'icrc1_balance_of', [x] * 1000)` | bridge receives a 30,106,001 B argument | 22.3B |
| `k = {'a' * 100000: 1}.keys()` / `[k] * 1000` | **Ok**, 100,028,001 B of JSON (24x the cap) | 13.7B |
| `x = 1 << 3000000` / `1` (VM work only) | Ok | 32M |
| `x = 1 << 3000000` / `x` | Ok, 903,092 B | 4.0B |
| `x = 1 << 3000000` / `[x] * 10` | Ok, 9 MB | 39.8B |

So a ~30-byte script reaches the 40B per-message limit in host code (`[x] * 20`), outside
Monty's budget (which stops at the VM boundary) and with no `check_budget` in `to_json`.
If the script made no call, the trap rolls back the whole request: no audit entry (K8).
Second symptom, same root cause (one node charged for a value whose expansion scales with
its width): a `bytes` value is one node and expands to `len` JSON elements, so
`b'a' * 4000000` leaves as a 4,000,000-element array (20x the 200k element cap, 155 MB RSS)
(R). JS (`SNAPSHOT_JS` counts every element and every string's length) and Rhai (blobs
count against `max_array_size` 10,000; `rhai-1.27.0/src/eval/data_check.rs:33,76,108`)
hold. Violates CL10 (README.md:369) and invariant 4. Relation to known findings: L7-6
(repr instead of an error, fidelity) and "Python's VM budget is not checked inside one
native operation (bigint power, repr)" are the VM side; this is the host conversion, and
the byte-cap bypass is not in either.

### SW2-1-2 (quadratic bignum text parse in `json_to_idl`)

`src/candid_bridge.rs:206-207` -> `int::<Nat>` / `int::<Int>` -> `num_text` (`:160`, copies
with `replace('_', "")`) -> `Nat::from_str` -> `BigUint::parse_bytes(v, 10)`
(`candid-0.10.40/src/types/number.rs:57-67`; num-bigint 0.4.8 `from_radix_digits_be`,
`convert.rs:91`, multiply-accumulate per chunk, O(n^2)). The value budget charges the
string one node and its length in bytes (linear); the encode is one uninterruptible step
(`bridge.rs:516`, no budget check between `reserve` and send). Probe
`Service::parse("service : { f : (nat) -> () }").encode_args("f", "7" x d)`:

| digits | encoded bytes | native instr. |
|---|---|---|
| 10 | 12 | 39M (process) |
| 1,000,000 | 474,569 | 17.9B |
| 4,000,000 (inside the 4 MiB value cap) | 1,898,252 (fits a 2 MiB message) | 284.5B |

Reachable by any token holder with one string (JS `'7'.repeat(4e6)`, Python `'7' * 4000000`)
against any allowlisted method with a `nat` or `int` anywhere in its arguments (e.g. ICRC-1
`icrc1_transfer.amount`); ~1.5M digits already pass 40B. With the interface cached the
encode runs in the first execution, so the trap leaves no audit entry. demo_store's
arguments have no `nat`/`int` (only `nat64`/`nat32`, parsed by `u64`/`u32::from_str`,
linear), so the demo deployment is not exposed. Same function as L3-3 but a different cost
driver (number width, not type width); a fix for L3-3 (tag index, per-node budget check)
does not bound one `parse_bytes` call. Proposed fix: cap decimal text for `nat`/`int`
(e.g. 2^14 digits, far beyond any token amount) before parsing.

### SW2-1-3 (candid decoder: per-`opt` clone of the subtype memo)

`candid-0.10.40/src/de.rs:801-842` `recoverable_visit_some` does `let self_clone =
self.clone()` for every `opt` value whose wire and expected types are both `opt`
(`:1158-1166`); `Deserializer` (`:309-336`) holds `gamma: HashSet<(Type, Type)>` by value.
`check_subtype` (`:515-528`, called for every `func`/`service` value, `:728, :740`) leaves
every proved `(Var, _)` pair in `gamma` (`types/subtype.rs:44-50, 660-683`; only failures
are retracted). The quota charges `add_cost(1)` per option (`:1155`, x50 untyped) and
nothing for the clone. Expected type (the target's own interface, `get : () -> (func () ->
(record { 0 : a0; ...; m-1 : a{m-1} }) query, vec opt null) query` with `type a{i} =
reserved`), reply = one `func` value (3 wire table entries) + `vec opt null` of n:

| m (interface bytes) | n (reply bytes) | decode | native instr. (decode only) |
|---|---|---|---|
| 1 (83) | 70,000 (70 KB) | Ok, 16 ms | 0.6B (whole process) |
| 1,000 (28 KB) | 70,000 (73 KB) | Ok, 1.5 s | n/m |
| 5,000 (152 KB) | 70,000 (85 KB) | Ok, 7.0 s | ~74.8B (78.4B - 3.6B setup) |
| 15,000 (477 KB, under `MAX_DID_BYTES`) | 70,000 (115 KB) | Ok, 23.3 s | ~227.5B (258.4B - 30.8B setup) |

n = 80,000 runs out of quota, but only after the same time is spent (8.1 s at m = 5,000).
`decode_reply` runs in the reply callback (`bridge.rs:532`) with no budget check, so on
chain the callback exceeds 40B and traps (Trapped entry; repeated on every call to that
method). The reply is 85 KB, far under the ~400 KB the quota otherwise admits. Needs a
hostile allowlisted target (it controls both the interface and the reply); the deep-reply
stack overflow and the candid_parser cost findings are other routes for the same actor, but
this one has a different root cause (quota does not charge the memo clone) and is not fixed
by bounding depth or interface size. Proposed fix: decode untyped (`IDLArgs::from_bytes`)
and coerce with app code, or report upstream; app-side, a reply-callback budget check cannot
interrupt it.

## Coverage grid (rows from inventory.md)

| Row | Item budget vs per-item cost | Checked during? | Cell |
|---|---|---|---|
| U7 / LIM16 values leaving JS (`SNAPSHOT_JS`, `js.rs:45-94`) | every element and string length counted; keys counted by length | snapshot runs in Boa under the loop limit; user `toJSON`/replacer per node is the known sync-JS budget finding | holds for this pattern |
| U7 / LIM16 values leaving Python (`python.rs:62-100`) | 1 node per object; repr text uncounted; bytes -> len elements | no | **SW2-1-1** |
| U7 / LIM16 values leaving Rhai (`rhai_rt.rs:88-117`) | 1 node per value, strings/keys/blobs by length; Rhai's own data-size limits cap arrays/blobs at 10,000 and the per-level `clone()` at depth x nodes | per op (`on_progress`) | holds |
| B5 / W5 `json_to_idl` variant tags, record fields | known L3-3 | no | known |
| B5 `json_to_idl` `nat`/`int` text | 1 node, linear bytes; quadratic parse | no | **SW2-1-2** |
| B5 `json_to_idl` `nat64`.. `int8`, float, hex blob, principal text | `from_str` / `hex::decode` / base32: linear | n/a | holds |
| B5 defaulted record fields (`field_value(..).unwrap_or(Null)`, `:270`) | `{}` expands to one IDL field per declared field (null/reserved/opt) | no | same root cause as L3-3 (cost scales with record width); not re-reported |
| B5 `IDLArgs::to_bytes_with_types` -> `annotate_type` (`candid value.rs:88-103`, `ser.rs:49-61`) | second full copy of the value, typed per node | no | part of L3-3's "one uninterruptible encode"; not re-reported |
| U8 / W6 `decode_reply`: candid quota vs `opt` values | 1 unit per option, clone of `gamma` per option | no | **SW2-1-3** |
| U8 / W6 candid quota vs records with absent `opt` fields | `next_key_seed` charges 4 per expected field (`de.rs:1800`), `deserialize_option` 1 | quota | holds |
| U8 / W6 candid quota vs variants | `deserialize_enum` linear `find` over expected tags (`de.rs:1470`), charged 1 + 4; expected tags bounded by the 512 KiB interface | quota | not reproduced; same actor and cost class as SW2-1-3, listed as unverified |
| U8 / W6 candid `nat`/`int` LEB128 decode | linear (`number.rs:232-262`, radix-128 single pass) | quota (x50) | holds |
| U8 `idl_to_json` `Nat`/`Int` -> decimal (`candid_bridge.rs:355-356`) | `to_str_radix(10)`, superlinear; quota caps one nat at ~390 KB on the wire | no | holds: 390,007-byte reply -> 821,813-digit string, 3.58B native (process); 450,007 bytes refused by quota (R) |
| U8 `idl_to_json` records/variants (`label_name` per field) | linear in decoded fields, which the quota charges | n/a | holds |
| W10 Python `from_json` replies (`python.rs:104-117`) | linear, big numbers stay strings | n/a | holds |
| W8 JS `JsValue::from_json` replies | linear | between rounds | holds |
| W11 Rhai `to_dynamic` of cached answers per replay | one answer per op | per op | holds (replay O(n^2) is register-accepted) |
| E17 JS `console.log` (`js.rs:227-248`) | budget checked before, cap checked before formatting, line formatted in full then truncated; `BigInt` text via Boa `to_string` | no | the BigInt formatting cost is register #23 (one Boa built-in); full formatting before the cap is known (printed values formatted in full) |
| E18 Python `print` | known (formatted in full before the cap) | | known |
| E19 Rhai `print`/`debug` | text built by Rhai under its data limits, truncated by `RunState::log` | per op | holds |
| W3 / LIM32 search `MAX_METHODS` (40 methods) vs rendered size per method (signature + referenced types) | count, not bytes; result goes through `tool_result` 1 MB truncation | render checked after | size: known (results over 1 MB truncated); cost: known (search render cost after render) |
| LIM9 `MAX_SEARCHES` x keywords | known | | known |
| LIM1 call cap counts slots, a fetch slot can send 2 messages | known | | known |
| LIM5 `MAX_AUDIT_PAGE` counts entries, entry size scales | known | | known |
| LIM14 violations 16 x 512 B | count and bytes | n/a | holds (marker overshoot known) |
| LIM15 / LogBuf (`bridge.rs:98-123`) | bytes | before each push | holds (marker overshoot known) |
| LIM8 `MAX_BATCH` 50 messages | per-message cost is a budgeted script or a search (known) | | holds for this pattern |
| LIM25 JS `MAX_PENDING` x args size | each pending request holds its args JSON (value-capped) | | JS memory is register #24 (accepted); not re-reported |
| LIM25 JS `MAX_PROMISE_JOBS`, budget every 256 jobs | job cost unbounded per job | every 256 jobs | known (sync JS budget) |
| LIM26 Python VM budget vs host work between suspensions | | | known (SW1-1) |
| LIM27 Rhai 1M operations vs per-op cost | `on_progress` checks the instruction budget every op; single op bounded by Rhai's data limits | per op | holds |
| W7 `Service::parse` | known (candid_parser cost and stack) | after | known |
| W1 `parse_body` / W2 `inspect_message` | ingress-bounded, linear | | holds / known (inspect quota) |
| W12 `truncate_escaped`, result `to_string`, `normalize_numbers` | linear in an already-capped value (except SW2-1-1's oversize values) | | holds |
| W13 `audit_page`, W14 log growth, W15/W16 demo_store and demo page | not this pattern | | n/a (known elsewhere) |
| B8 JS stringify wrapper (`JSON.stringify` -> snapshot) | same counting as results | | holds (array replacer order known) |
| B9 Python conversions | | | **SW2-1-1** |
| B10 Rhai conversions | | | holds |
| B11 `tool_result` | | | holds |
| B12 stable Storable encode/decode | admin-sized or audit entries (capped fields) | | n/a |
| B16 search JSON | | | known |
| VF1 / VF5 cycles per run | instruction budget is the only per-run meter; SW2-1-1..3 spend up to the 40B message limit outside it | | candidates above |
| E1-E14, AD*, D*, V*, K*, S*, C*, A*, CL* other than CL10 | no item-counted conversion | | n/a |
| CL10 "200k elements and 4 MiB of text, in all three languages" | false for Python | | **SW2-1-1** |
| README.md:456 "every other path stops with a clean error" | false for SW2-1-1/2 (any token holder) and SW2-1-3 (hostile target) | | candidates above |

## Not tested

- On-chain instruction counts: all three candidates were measured natively (aarch64
  "instructions retired"); none was replayed on the local network (not this phase).
- Variant-tag linear search in candid's `deserialize_enum` (`de.rs:1470`): reasoned, not
  measured.
- Whether Monty's `py_repr` uses num-bigint's divide-and-conquer `to_radix_digits_le`; the
  cost was measured, not attributed.
