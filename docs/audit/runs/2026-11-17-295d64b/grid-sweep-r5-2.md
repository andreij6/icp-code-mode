# Variant sweep r5-2: unrepresentable value replaced by a look-alike

Run `2026-11-17-295d64b`, Phase 6, round 5. Pattern (shared root of L7-4 / L7-6, not
re-reported):

> A value the target format cannot hold is replaced by an escaped or substitute look-alike
> (null, a repr string, literal escape text, an empty or index-keyed object) instead of an
> error.

Prior sweeps r4-2, r4-8 and r4-10 covered this root. This round looked for value classes and
rows they did not name. Known and not re-reported: L7-1, L7-2, L7-4, L7-5, L7-6, L7-7, L7-9,
L7-10, L7-11, L7-12, L7-14, L7-16, L7-16b, L7-17, L6-9, S10-1, S10-2, SW4-8-1, SW4-8-2,
SW4-2-1/2/3, SW3-9-1, the boxed-primitive and README:369 placeholder findings, demo_store
`restock` saturation, and the `reserved` wrap.

Evidence: **R** = reproduced natively. The copy was a `git archive 295d64b` with no
`target/` and none of the uncommitted test additions in the working tree. Its package
version was set to `0.1.0-sweepr52`. It was built with the shared
`CARGO_TARGET_DIR=/private/tmp/claude-501/audit-target`, the probe ran as a throw-away test
module `runtimes::sweep_r5_2` with MockBridge, and the copy was deleted afterwards. **C** =
the code path is unambiguous. No live canister was touched.

Note on the tree: the working tree has uncommitted test-only additions in `src/` and
`demo_store/` (insertions inside `mod tests` / `mod mock`, no deletions). Non-test line
numbers match `295d64b`, so every reference below is valid for the audited commit.

## Candidate

### SW5-2-1 (Low). JS byte containers have no blob form: Uint8Array leaves as an index-keyed object, ArrayBuffer/DataView as `{}`, and the run reports Ok

**Where.** `SNAPSHOT_JS` (`src/runtimes/js.rs:68-82`) recognises only `Array.isArray` values
as arrays. A typed array passes `typeof x === "object"`, fails `isArray`, and is walked as a
plain object. Its own enumerable keys are its indices, so `Uint8Array [1,2]` becomes
`{"0":1,"1":2}`. `ArrayBuffer` and `DataView` have no own enumerable keys, so they become
`{}` and their bytes are dropped. The blob rule elsewhere in the system is "array of
numbers": Candid blob replies come back as arrays (`candid_bridge.rs:338`), Python `bytes`
and Rhai blobs are exported as arrays (`python.rs:79-83`, `rhai_rt.rs:93-96`), and the tool
description says "blobs are byte arrays or hex strings" (`src/mcp.rs:75`).

**R (native, MockBridge):**

| Script | Result |
|---|---|
| JS `return new Uint8Array([1,2,255])` | `Ok({"0":1,"1":2,"2":255})` |
| JS `return new Uint8Array([1,2]).buffer` | `Ok({})`: the bytes are gone |
| JS `return new DataView(new ArrayBuffer(4))` | `Ok({})` |
| JS `return {hash: new Uint8Array([7,8])}` | `Ok({"hash":{"0":7,"1":8}})` |
| JS `return Array.from(new Uint8Array([7,8]))` (control) | `Ok([7,8])` |
| JS `icp.call(L,'icrc1_balance_of',{owner:'aaaaa-aa', subaccount: new Uint8Array(2)})` | the mock records `{"subaccount":{"0":0,"1":0}}`; `encode_args` against `record { owner : principal; subaccount : opt blob }` gives `argument 0: .subaccount: expected array, got object` (loud) |
| same with `new Uint8Array(2).buffer` | the mock records `{"subaccount":{}}`; encode gives the same error (loud) |
| Python `icp.call(..., {'owner':'aaaaa-aa','subaccount':bytes(2)})` | sends `[0,0]`, encodes Ok |
| Rhai `let b = blob(3, 1); b` | `Ok([1,1,1])` |

**Impact.** Results are where the substitution is silent. A JS script that returns bytes it
built (a typed array, or its `.buffer`) hands the model an index-keyed object, or `{}` with
the data lost, while the run is audited Ok. The same script in Python or Rhai returns an
array, against "identical semantics in every language" (`src/mcp.rs:79`, shown to the AI).
In call arguments the result is a loud refusal, but its text, "expected array, got object",
contradicts the description the AI was given ("blobs are byte arrays"), because
`Uint8Array` is JavaScript's byte array.

**Refutation tried.** Is this just `JSON.stringify` semantics, already accepted for `Map ->
{}` and grouped under L6-9? For `ArrayBuffer`/`DataView` -> `{}`, yes: it is the same drop
as `Map`, and the verifier may merge it into L6-9. The Uint8Array case is a different value
class from L6-9's examples (functions, undefined, Map). The system has its own
representation for exactly this value (the blob array), and the sibling runtimes use it.
No known finding names byte containers. Args are not silent, so this is graded Low.

**Fix.** In the snapshot, convert `ArrayBuffer.isView(x)` (typed arrays; DataView by its
bytes) and `ArrayBuffer` values to plain arrays of their bytes before the object rule,
counting each element against `MAXN`. Alternatively, refuse them with a TypeError that says
to use `Array.from`. Mention typed arrays in the tool description's blob sentence.
Regression test: JS `return new Uint8Array([1,2])` returns `[1,2]` (or fails), and the
same value as an `opt blob` argument encodes.

## Grid (every inventory row against the pattern)

| Row(s) | Verdict | Reason |
|---|---|---|
| C1 runtimes | **SW5-2-1** (JS byte containers); rest known | JS snapshot is kind-based except for typed arrays/buffers; Python and Rhai byte types map to arrays |
| C1a feature subsets | n/a | Per-runtime conversion code is identical in every subset |
| C2 demo_store | holds / known | Candid-typed; `restock` saturation and `reserved` wrap are known; `cancel_order` `saturating_sub` (`demo_store/src/lib.rs:324`) can clamp only after the known wrap or oversell, so it is the same root |
| C3 web-time shim | holds (C) | `checked_add` / `checked_sub` return `None`, `Add` panics (F9, other pattern); `saturating_duration_since` clamps negative spans to 0, which cannot occur on a monotonic counter |
| C4 vendored chrono | n/a | No conversion owned by the app |
| C5 demo page | known | `pretty()` re-parses results and rounds integers above 2^53 (the display end of L7-17); `esc(..).slice(0,40)` can cut an entity (display only, L17 grid) |
| C6 demo-setup.sh | known | Parses human-readable icp-cli output (known finding) |
| C7-C11, C13-C15 | n/a | Build, config, interface and audit tooling: no value conversion |
| C12 video | n/a | Dev tooling; passes files through |
| A1-A12 | n/a | Actors; their values are the U rows |
| E1/E2 `apply` | holds | A bad token hash traps (F1); opt-arg decoding to null is known |
| E3 inspect_message | holds | Decode failure rejects |
| E4/E4b, E5 routing | holds / known | JSON `\uD800` escapes refuse the whole body with -32700 (serde_json validates surrogates for strings, `read.rs:962-1009`); non-string `params.name` becomes `""` and is refused as "unknown tool" (loud; the message names the tool as empty); id echo is L7-9 |
| E5a run_script / E5b run_search | holds | The audit `script_sha256` key `search:{query}:{canister_id}` (`lib.rs:123`) maps absent to `""` and joins with an unescaped `:`. The colliding spellings either mean the same search or name a canister id that is not a principal; such a search has no entries and writes no audit entry. So no two audited searches can share a hash through this path (C) |
| E5c/E5d | n/a | Fixed text |
| E6-E11 admin | holds | Candid-typed; token hash trimmed and case-folded, then validated to 64 hex; the override is parsed before it is stored |
| E12-E14 | holds | Typed replies; token key prefix `[..16]` of validated hex |
| E15 exports | n/a | |
| E16 JS `icp.call` / `icp.candid` | **SW5-2-1** for byte containers in args (loud) | Other cases known: L7-2, L7-4, L7-10, L6-9, boxed primitives. `arg_string` refuses non-strings, including `String` objects (C, `js.rs:195-200`) |
| E17 JS console | known | SW4-8-1 / SW4-2-1 |
| E18 Python | known | L7-6 covers `Decimal`, `datetime`, `namedtuple`, exceptions, `complex` and `range` (all `_ => py_repr`, `python.rs:98`; Monty type names at `monty-types graph.rs:546-580`); S10-1 covers class-name dispatch; SW3-9-1 covers `Repr`/`Cycle` nodes; `bytes` -> array (R) |
| E19 Rhai | holds | Closure-shared values convert intact (`Dynamic::is_*` / `into_*` read through `Shared`, `rhai-1.27.0 types/dynamic.rs:2221-2310, 2735-2865`; R in r4-2); blob -> array (R); char -> string, unit -> null; FnPtr and ranges refused |
| E20-E25 demo_store | holds | Candid-typed |
| E26-E28 | see C5, C6, C12 | |
| U1-U5 | holds / known | See E4/E5 |
| U6 script source | known | Lone-surrogate literals: L7-10 (JS), SW4-8-2/SW4-2-3 (Python), Rhai refuses |
| U7 values leaving scripts | **SW5-2-1**; rest known | |
| U8 reply bytes | holds / known | candid text decoding is strict UTF-8 (`candid-0.10.40 binary_parser.rs:101`, `de.rs:1133`); non-finite floats S10-2; label collisions L7-14 |
| U9 reject text | holds | The IC requires UTF-8 reject messages; carried as `String` on the error channel |
| U10 interface text | holds | Non-UTF-8 `candid:service` refuses (`bridge.rs:431`); candid_parser's grammar refuses non-UTF-8 byte escapes (`grammar.lalrpop:95`) |
| U11-U16 | holds / n/a | Typed admin input; platform values; client-side data |
| O1/O2 | see U7 | |
| O3 audit entries | known | Error text carries SW4-8-1 substitutes; `methods` carries L7-10 escapes |
| O4-O10 | n/a | No converted value |
| D1-D5 stable records | holds / known | `Decode!` failure traps (F3); unknown status on rollback traps (known); opt-null coercion of args is known |
| D6-D10 | n/a | |
| V1-V11 | n/a | Counters and caches |
| K1-K12 | n/a | Commit points |
| LIM1-LIM32 | n/a | LIM22 drives B6 (documented decimal strings) |
| W1-W16 | n/a | Cost rows |
| F1-F16 | holds / known | F11 is L7-5; F16 is known |
| B1 | holds | See E4 |
| B2 | holds | |
| B3 | holds | `get(..16).unwrap_or(whole)` keeps the hash |
| B4 | holds / known | `from_text` errors; search's trim is known |
| B5 JSON -> Candid | holds / known | Byte-container args refuse loudly (SW5-2-1 context); other cases known |
| B6 Candid -> JSON | holds / known | Every `IDLValue` arm has a lossless rule except the known S10-2 and L7-14 |
| B7 .did parse | holds | Errors are errors |
| B8 JS <-> JSON | **SW5-2-1**; rest known | |
| B9 Python <-> JSON | known | |
| B10 Rhai <-> JSON | holds | |
| B11 tool result | known | L7-1, L7-16 |
| B12 stable Candid | holds | |
| B13 language | holds | Unknown names refuse; aliases documented |
| B14 time | known | L7-16b; JS millisecond truncation of block-time nanoseconds matches JS `Date` precision (holds) |
| B15 certificate header | n/a | Constant tree |
| B16 search JSON | holds | Method names are emitted raw; signatures and type texts come from candid's pretty-printer, which quotes non-identifier labels with `escape_debug` (`candid-0.10.40 pretty/candid.rs:67-69`), and those are Candid escapes. Rust's `\0` for NUL is not a Candid escape, but a label containing NUL is contrived: a display artefact, not a converted value |
| B17 demo page | known | See C5 |
| VF1-VF8 | n/a | |
| AD1-AD9 | holds | Typed, validated, or known (opt-null, set_limits) |
| S1-S10 | n/a | Dependency conversions examined under B5-B10 |
| CL10 / CL29 | **SW5-2-1** contradicts CL29 ("identical semantics in every language") and the `mcp.rs:75` blob sentence | |
| CL other | n/a | |
| T1-T12 | gap | No test returns or passes a typed array, ArrayBuffer or DataView from JS |
| History | known | #39 is the origin of S10-2 |

## Not tested

- The wasm build and the live canister: the snapshot is pure Boa code with the same
  behaviour on wasm32. The probe used MockBridge plus `encode_args`, which is the shipped
  encoder. A live repro needs only an ordinary token and the demo_store allowlist; it was
  not run, because this phase does not deploy.
- Boa's `structuredClone`-like or `SharedArrayBuffer` paths: `SharedArrayBuffer` is deleted
  from the global (`js.rs:314`), so it cannot be reached.
