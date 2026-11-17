# Variant sweep r5-1: identifier re-serialised from a lossy parse instead of echoed verbatim

Run `2026-11-17-295d64b`, Phase 6, round 5. Pattern under test. The seed is known and not
re-reported: L7-9. serde_json reads JSON-RPC ids as f64, and the server echoes the altered
id.

> An identifier is re-serialised from a lossy parse instead of being echoed verbatim.

Generalised for the sweep: an identifier (request id, token id, principal, method name, field
or variant label, language or tool name, finding key) is parsed, normalised, truncated or
converted into a lossy form. It is then shown to a party, or used as a key, in place of the
original. As a result the shown form no longer round-trips, two identifiers collapse into
one, or one identifier appears as another.

Method: I read the code at `295d64b` (sources exported with `git show`) for every inventory
row. The library code I read: ic_principal 0.1.5 `from_text`, and Boa 0.22
`JsValue::from_json` plus `PropertyKey::from(JsString)`. There were two probes:

- **P1 (native):** a test added to `policy::tests` in a private `git archive` copy. Every
  file was touched so cargo could not reuse stale output. The run was
  `cargo test --lib sw5_probe` with the shared `CARGO_TARGET_DIR`. The copy was deleted
  afterwards.
- **P2 (node):** the audit workflow's own `keyOf`, copied verbatim into a scratch script.

Nothing was deployed, called or reconfigured. **R** = reproduced, **C** = code path.

These findings are known and not re-reported:

- L7-9 (JSON-RPC ids)
- L7-10 (lone surrogates become literal `\uXXXX`, including in method and canister strings)
- L7-14 and the json_to_idl label and variant-tag collisions
- `_n_` alias resolving to a different case
- Search labels printed in a form the converter refuses (`1_000`, `true`/`false`)
- Search `script_sha256` taken from raw spelling
- Search trims `canister_id`
- Search echoes the "not allowlisted" id in full
- Python dict keys as `py_repr`
- Python class named `str` and similar type-name dispatch
- L7-16 and L7-17, and their demo-page `pretty()` rounding (noted in L7-r1 and r4-2)
- Demo page splices the canister id into code
- `demo-setup.sh` parses human-readable output
- Audit `canister.method` labels unescaped
- Interface fetches recorded as `aaaaa-aa`
- Live refutation matched by exact title

## New candidates

| Id | Sev | Title | Ev |
|---|---|---|---|
| SW5-1-1 | Medium | The controller sees each token only as a 16-hex hash prefix, in `get_policy` and in every audit `caller`, but `revoke_api_token_hash` accepts only the full 64-hex hash. The identifier the operator can see does not revoke anything (returns `false`). A token whose full hash was not saved outside the canister cannot be revoked without a reinstall or new code | R (P1) |
| SW5-1-2 | Medium | The audit workflow keys findings by `(location + '|' + root_cause).toLowerCase().slice(0, 160)`. Two different confirmed findings that share a long location, or a 160-character prefix, collapse into one key. The second is dropped without being listed as confirmed or refuted, and the round counts as dry | R (P2) |

### SW5-1-1 detail

- **Where:**
  - `src/policy.rs:221-223`. `list_tokens` returns `e.key()[..16]` and the label.
  - `src/lib.rs:336-345`. `get_policy` exposes that list as `PolicyView.tokens`, and
    `icp_code_mode.did` documents it as "(token hash prefix, label)".
  - `src/lib.rs:122, 222`. The audit `caller` is `caller_hash.get(..16)`.
  - `src/policy.rs:217-219` and `src/lib.rs:317`. `revoke_token_hash` removes only an exact,
    trimmed, lowercased 64-hex key.
  - The full hash is returned nowhere: the .did shows no other method that reads `TOKENS`.
- **Repro (P1):** `add_token_hash(sha256("leaked-token"), "laptop")`, then take
  `list_tokens()`. The output was
  `SW5 shown=d1737c1fb291d3a7 len=16 label=laptop` and then
  `SW5 revoke(shown)=false still_valid=true`.
- **Why it matters:**
  - README.md:459-462 tells operators to treat tokens as revocable credentials and to revoke
    them "when in doubt".
  - The natural trigger for doubt is an audit entry or a `get_policy` row, and both name the
    token only by this prefix.
  - The README's setup flow (README.md:53-54, 144-148) computes the hash inline in a shell
    command and never stores it. The operator therefore has the full hash only while the
    plaintext token is still at hand.
  - For a lost client or a leaked token, nothing the canister returns can be used to revoke
    it. A reinstall would also wipe D1-D5, including the audit log, and `post_upgrade` args
    cannot remove a token (only add one).
  - The canister never shows the prefix anywhere a non-controller can see it, and a hash of
    a 256-bit random token is not sensitive. The truncation therefore protects nothing that
    a controller-only view needs to hide.
- **Refute attempts:**
  - (a) Collision safety of the 64-bit prefix was assessed by L10-r1 and holds. That is a
    different question: it concerns telling tokens apart, not using the id as input.
  - (b) An operator who still has every plaintext can recompute hashes. That fails exactly in
    the leak or loss case the README cites.
  - (c) No prefix-match path exists: `revoke_token_hash` uses `remove(&key)` on a
    `StableBTreeMap`, exact match only.
- **Fix:** one of:
  - return the full 64-hex hash in `PolicyView.tokens` (and optionally in the audit
    `caller`);
  - accept a unique prefix of 16 or more hex characters in `revoke_api_token_hash`, refusing
    an ambiguous prefix;
  - add a `revoke_api_token_label`.

  Then document the chosen form in the .did. Add a test that revokes using exactly what
  `get_policy` lists.

### SW5-1-2 detail

- **Where:** `.claude/workflows/audit.js:177` (`keyOf`), used for deduplication at `:213-216`
  (`absorb`) and at `:291` (Live). In `absorb`, a confirmed verdict whose key is already in
  `seen` takes `continue`. It is not added to `confirmed` or to `refuted`, and nothing is
  logged.
- **Repro (P2):** I copied `keyOf` and the `absorb` loop verbatim and ran four confirmed
  verdicts through them:
  - A and B: same 172-character location (`src/bridge.rs:470-506 (IcBridge::candid), ...
    README.md:441`), different root causes;
  - C and D: same short location, and root causes that share their first 151 characters.

  The output was `confirmed: A,C`. B and D vanish.
- **Effect:**
  - Once the location string alone is 159 characters or more, the root cause is ignored
    entirely. Every later finding at that location (one multi-site location or a long list
    of `file:line`) is dropped.
  - `fresh` is computed from the same check, so a round whose only new finding collapsed
    counts toward the two-dry-rounds stopping rule (PLAN.md section 9).
  - The `seen` keys handed to later reviewers ("Already reported") are the same truncated
    strings, so the dropped finding is also hidden from the people who might find it again.
  - This is the same family as known SW4-6-2: the report silently loses a finding and the
    stopping rule is advanced. The root is different: SW4-6-2 is a missing agent output,
    while this is a lossy key derived from free text instead of a stable identifier.
- **Refute attempts:**
  - (a) Verifiers merge duplicates. However, two verifiers in different lenses or sweeps never
    see each other's output, so cross-agent duplicates are exactly what `keyOf` must
    separate, and it cannot tell them apart from distinct findings.
  - (b) Lower-casing is harmless; the truncation is the cause.
  - (c) r4-6 noted this site "for the critic" but returned no candidate.
- **Fix:**
  - Key on a stable identifier: the verifier's own id, or a hash of the full untruncated
    `location|root_cause`.
  - Log, rather than drop, a verdict whose key matches. Record it as "merged into <title>"
    so a near-duplicate stays visible in the report.

## Grid

| Row(s) | Identifier / site | Result | Reason |
|---|---|---|---|
| U4 / B1 | JSON-RPC `id` | known (L7-9) | One `classify` path for both routes. String ids are echoed verbatim; numbers are re-serialised from serde_json's f64 |
| U4 | JSON-RPC `method` in "method not found" | holds | Echoed verbatim (`mcp.rs:261`), cut at 300 B by the documented LIM11 bound with a visible `…[truncated]` marker |
| U5 | `params.name` (tool) | holds (note) | A non-string name is echoed as an empty string, "unknown tool ``" (`mcp.rs:211`, `unwrap_or_default`). It is a refusal either way and the client still has its own request. Cosmetic |
| U5 / B13 | `arguments.language` | holds (note) | The error echoes the lowercased spelling (`runtimes/mod.rs:38-42`): `JS2` is shown as `js2`. Refusal text only, no other effect. The audit records the canonical `Language::as_str()`, which is the intended identity |
| U5 | `arguments.canister_id` (search) | known | Trimmed before parsing (known); output uses `Principal::to_text()`, which is canonical and lossless (ic_principal `from_text` parses case-insensitively, then requires that the text printed back equals the input in lowercase) |
| U2 / B3 | bearer token -> hash -> `caller` | **SW5-1-1** | The audit and `get_policy` show 16-hex prefixes; revoke needs 64 hex |
| E9 / E10 / AD5 / AD6 | token hash input | holds | `add` and `revoke` both `trim().to_ascii_lowercase()` (`policy.rs:209, 218`): one canonical form on both sides |
| E12 | `PolicyView.tokens` | **SW5-1-1** | Lossy (prefix) view of D3 |
| E12 | `PolicyView.allowlist` | holds | `Principal` values plus verbatim method strings: the same values `set_allowlist` and `remove_allowlist` take |
| E11 / D5 / V5 | override key | holds | `canister.to_text()` on both write and read (`policy.rs:233-242`, `bridge.rs:207-216, 472`) |
| E6 / D2 | allowlist method names | holds | Stored verbatim and compared verbatim with the script's method string (`policy.rs:169-190`). The message is sent with the same verbatim string (`bridge.rs:526-529`) |
| U7 / B4 | script canister ids (JS/Python/Rhai) | holds / known | `parse_principal` returns canonical text, and every later use (policy, cache, audit `canisters`, errors) uses `p.to_text()`. The JS lone-surrogate escape is L7-10 |
| U7 | script method names | holds / known | The same string is used for `check_call`, `Service::method` lookup, `Call::*_wait(p, method)` and the audit label. The JS escape is L7-10; the unescaped label is L10r4-1 |
| U8 / B6 | principal, service and func values in replies | holds | `p.to_text()` is lossless (bytes <-> text is a bijection). `func` is returned as `{principal, method}` with the method verbatim, and `json_to_idl` accepts that shape back (`candid_bridge.rs:217-227, 349`) |
| U8 / B6 | nat64 / nat / int ids in replies | holds | `big()`: numbers up to 2^53-1, decimal strings above, both accepted back exactly by `int()` (`candid_bridge.rs:303-310, 172-188`) |
| U8 / B6 | record field and variant labels | known | Named labels come from the typed decode (the env's own names). Id labels are printed as decimal and accepted back as decimal or `_n_`. The quoted-number/id collisions and the `_n_` variant alias are known |
| B8 | JSON -> JS objects (`JsValue::from_json`) | holds | Keys are inserted as own data properties (`boa_engine serde_json.rs:68-79`), so a `__proto__` label stays a field. Integer-like keys become `PropertyKey::Index` through the same conversion that property lookup uses, so numeric labels stay reachable |
| B9 / B10 | JSON -> Python / Rhai | holds | Reply ints are at most 2^53, so `as_i64` and Rhai `INT` are exact. Map keys are verbatim strings |
| B9 | Python -> JSON keys | known | `py_repr` keys are L7-7 |
| W11 | Rhai replay request identity | holds | `Request` equality on the verbatim canister, method and args `Value`, deterministic per replay (`rhai_rt.rs:70-76`). Two spellings of one principal are different requests but always in the same order |
| U10 / V2 | interface text | holds | Strict `String::from_utf8` (not `_lossy`) for `canister_metadata` (`bridge.rs:430`); `DID_CACHE` and overrides keep the verbatim text that `icp.candid` returns |
| B16 / W3 | search `methods[].name`, `types` | holds / known | `name` is the env's method name (the wire name, quoted escapes resolved by candid_parser); signatures are re-printed (the label-form issues are known) |
| E5b | search `script_sha256` key `search:{q}:{id}` | holds (beyond known) | The `:` join is ambiguous only when `canister_id` is empty or invalid. Such a search has no entries, so no audit entry is written. A valid principal contains no `:`. Raw spelling is known |
| D4 / O3 | audit `language`, `canisters`, `run_id`, `id` | holds | Enum name, canonical text, u64 |
| E13 / B12 | audit Candid round trip | holds | Candid text is length-prefixed and lossless |
| B11 | tool result text | known | Raw string results are L7-16 |
| B17 / C5 | demo page `pretty()` and the raw panel | known | Rounding of ints above 2^53 is L7-17's symptom (L7-r1). The raw panel re-serialises only the envelope (page-generated ids, strings) |
| C6 | `demo-setup.sh` canister id | known | Parses human-readable output |
| C12 | `.env` `MCP_TOKEN` parse (`capture-demo.mjs:7`) vs `GEMINI_API_KEY` (`tts.mjs:11`, strips quotes) | holds (note) | Sibling parsers disagree on quotes, but a quoted token fails loudly (401) rather than being silently altered; dev tooling. The README writes the value unquoted (README.md:53) |
| C13 | audit workflow `keyOf` | **SW5-1-2** | Lossy key from free text |
| C13 | Live matching by exact `title` | known | Retitled live refutation is dropped |
| C13 | pattern dedupe `new Set(patterns)` | holds | Exact-string identity only causes near-duplicate patterns to be swept twice (cost), never a loss |
| E20-E25 / U13 | demo_store ids (product, order, customer principal) | holds | Candid-typed, echoed as `nat64` and `principal` |
| E1/E2 / AD1 | `InitArgs.token_sha256` | holds | Same normalisation as `add_token_hash` (`lib.rs:38-49`) |
| C1a, C3, C4, C7-C11, C14, C15, S* | build, config and docs rows | n/a | No identifier is parsed and echoed at run time |
| F*, LIM*, K*, VF* | | n/a | Not identifier-carrying rows for this pattern |
