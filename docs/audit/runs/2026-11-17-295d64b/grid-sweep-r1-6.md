# Variant sweep r1-6

Pattern: "A record names the transport instead of the subject of the effect, and a
documented label format that the code never produces."

Source finding (known, not re-reported): interface-fetch audit entries are labelled
`aaaaa-aa.canister_metadata` with `canisters = ["aaaaa-aa"]`, so the canister whose interface
was read is not recorded, while the `.did` / `policy.rs:94` doc promises
`canister.<candid interface>`.

Method: code reading at `295d64b`. Nothing was built, deployed or called. Every place that
writes a record or a label (audit entries, violation and error strings kept in the audit,
`PolicyView`, log-line prefixes, search output, demo_store orders) was checked for two
things: (a) does the record name the party that carried the message rather than the thing
the effect was about, and (b) does a doc describe a label/value format the code never emits.

Result: two new candidates (SW6-1, SW6-2). Everything else is either a known finding or holds.

## Candidates

| Id | Sev | Title | Evidence |
|---|---|---|---|
| SW6-1 | Medium | Script calls to the management canister are allowlisted, audited and reported by the recipient `aaaaa-aa` only; the canister actually acted on (the `canister_id` argument) is neither checked by policy nor recorded anywhere in the log | code path |
| SW6-2 | Low | Search audit entries put undocumented values in `language` (`"search"`) and `script_sha256` (SHA-256 of an internal `search:<query>:<canister_id>` string built from the caller's raw spelling), where the README documents the field as the script's SHA-256 | code path |

### SW6-1 detail

- Label: `IcBridge::sent` (`src/bridge.rs:387-389`) builds every audit label as
  `format!("{}.{method}", p.to_text())` and `record_message` (`:395-419`) puts `p` in
  `canisters` and `touched`. `p` is the message recipient.
- Policy: `check_call(p, method)` (`src/policy.rs:193-196`) keys the allowlist by the
  recipient. For `p = aaaaa-aa` the recipient is a proxy: the canister that is stopped,
  deleted, re-configured, uninstalled or read (`canister_status`) is the `canister_id` field
  of the arguments, which neither policy nor the audit look at.
- Reachable configuration: `set_allowlist(aaaaa-aa, vec {"canister_status"})` (any method list
  is accepted, `src/lib.rs:295-298`) plus `set_candid_override(aaaaa-aa, <ic.did>)` (accepted
  for any principal, `src/lib.rs:323-333`; without it the fetch of `aaaaa-aa`'s interface
  fails). A script `icp.call("aaaaa-aa", "canister_status", {canister_id: X})` then passes
  `reserve`, encodes via the override, is sent with `Call::unbounded_wait(aaaaa-aa, ..)`
  (`src/bridge.rs:524-528`), and writes `Started` with `canisters = ["aaaaa-aa"]`,
  `methods = ["aaaaa-aa.canister_status"]`; the closing entry is the same. Only the script's
  SHA-256 is stored (`src/lib.rs:223`), so `X` cannot be recovered from the log. The same
  grant admits `canister_status` on every canister this canister controls, not one chosen
  canister; the same holds for `stop_canister`, `update_settings`, `uninstall_code`,
  `delete_canister`. Error text on failure is also `call aaaaa-aa.<method> failed`
  (`src/bridge.rs:530`).
- Doc: README.md:387-389 "Each outbound message gets its own entry, naming the target and
  method in `methods`". For a management call the "target" recorded is the transport.
- Distinct from known L1-3 (acceptance of `aaaaa-aa` with `"*"`): this is about a narrow
  method grant and about what the record names; the fix for the known fetch-label finding
  (special-casing `fetch_did`) would not change it.
- Refute attempts: (1) does anything else record the subject? `violations` only on refusal;
  `error` only on failure; `run_id`/`script_sha256` don't carry arguments. No. (2) Is
  management access documented as unsupported? No: README never mentions it, and the code
  accepts it. (3) Could the IC route the call so the recipient is `X`? The call is made to
  `aaaaa-aa`; the label uses the `p` passed to `Call::*`, which is `aaaaa-aa`.
- Not reproduced: the production `IcBridge` cannot run natively (register #58), and changing
  the allowlist on the live network is outside this phase.

### SW6-2 detail

- `src/lib.rs:122-125, 175-177`: `key = format!("search:{}:{}", query, canister_id)` using
  `args.canister_id` as typed; `script_sha256 = sha256(key)`; `language: "search"`.
  `search::scope` trims and canonicalises the id (`src/search.rs:39`), the hash does not, so
  `ryjl3-…`, `RYJL3-…` and ` ryjl3-…` searches of the same canister record three different
  hashes. Neither the key format nor the `"search"` language value is documented anywhere
  (README.md:374 lists "language, script SHA-256"; `grep "search:"` finds only `lib.rs:123`).
- Impact: an auditor cannot map a search entry back to the query without knowing the
  internal format, and equal searches do not hash equal. The subject canister is also
  missing from `canisters` when `canister_metadata` answers (that part is the known finding).

## Grid

| Row(s) | Record / label | Names the subject? | Doc format matches? | Verdict |
|---|---|---|---|---|
| O2 / K5 interface fetch, `AuditEntry.methods`/`canisters` | `aaaaa-aa.canister_metadata`, `["aaaaa-aa"]` | no | no (`canister.<candid interface>`) | **known** (source finding) |
| O2 fallback `__get_candid_interface_tmp_hack` | `<target>.__get_candid_interface_tmp_hack` | yes | no (same doc) | **known** |
| O1 script call to an ordinary target | `<target>.<method>` | yes (the recipient is the subject) | yes (README:387) | holds |
| O1 script call to `aaaaa-aa` (A7, AD2) | `aaaaa-aa.<method>` | no: subject is the `canister_id` argument | README "target" = recipient | **SW6-1** |
| O1 self-call to `http_request_update` (AD2) | outer `<self>.http_request_update`; inner run has its own `run_id`, unlinked | no link to inner run | n/a | **known** (L1-3 states the unlinked `run_id`) |
| VF4 ledger transfers (`icrc1_transfer`) | `<ledger>.icrc1_transfer` | yes (ledger is the recipient and the subject); amount/recipient are arguments, which the docs never promise to record | n/a | holds for the pattern (arguments are not a documented field) |
| E5d / K7 `Trapped` entry | `canisters`/`methods` from `touched`/`messages` | same labels as above | `error` text says "the script's result was discarded" also on a search run | holds (wording only); instructions/`…and N more` gaps **known** |
| `AuditEntry.caller` (B3, A3, A8) | first 16 hex of token SHA-256 | yes for the documented subject (the token); a canister calling `http_request_update` directly is recorded by token, as documented | yes (`.did:20`) | holds |
| `PolicyView.tokens` (E12) | `key[..16]`, label | yes; same 16-hex prefix as `caller`, so they join | yes (`.did:84`) | holds |
| `AuditEntry.language` for scripts | `javascript`/`python`/`rhai` (`runtimes/mod.rs:25-31`) | yes | yes (matches the tool's enum) | holds |
| `AuditEntry.language`/`script_sha256` for searches (E5b) | `"search"`, sha256 of `search:<q>:<raw id>` | no (hash of request text in caller's spelling) | undocumented | **SW6-2** |
| `AuditEntry.run_id` | `Started` id or own id (`policy.rs:251`) | yes | yes (`.did:9-12`) | holds |
| `AuditEntry.calls` | `messages.len()` | yes | yes (README:409, register #53) | holds; cap-vs-messages gap **known** |
| `AuditEntry.violations` (LIM14) | `policy: canister X is not allowlisted` / `method m on X` / call limit / budget | names the subject `X` (canonical text), including for refused interface fetches | n/a | holds; size overflow **known** |
| `AuditEntry.error` / V4 `DID_FAILED` | `could not fetch candid for X (<fallback error>)` | names the subject; the `canister_metadata` failure is dropped and only the fallback's error kept | n/a | holds for the pattern (subject named); dropped primary cause is an L9 detail, not this pattern |
| `AuditStatus` doc comments (`policy.rs:57-73`, `.did:24-40`) | Started/Call/Trapped | n/a | written for searches too; otherwise produced as described | holds; "two kinds write no entries" gap **known** |
| V3 `pending_run_ids` | run ids incl. searches | yes | "runs" covers searches | holds; searches outside run slots **known** |
| V1 `active_runs` | scripts only | yes | yes | holds |
| Log lines (E17, E18, E19, B11) | JS `[level] …` (not for `log`), Rhai `[debug <pos>] …`, Python plain; `[console output]` header | n/a (not a record of an effect) | no format promised | holds; Python cap difference **known** |
| Script errors for calls (U9) | `call <p>.<method> failed: …` | recipient; for `aaaaa-aa` see SW6-1 | n/a | holds / SW6-1 |
| E11 / D5 override doc (`lib.rs:321-322`, `.did:103-104`) | "for a target that does not expose `__get_candid_interface_tmp_hack`" | names the fallback transport, not when overrides apply | no (used first) | **known** (README third-source / L16-5) |
| README:494 / bridge.rs:421 "`canister_metadata` not on every replica" | | | | **known** (L16-11) |
| B16 search output | `canister_id` canonical (`p.to_text()`), `allowed_methods`, signatures | yes | yes (README:279 example) | holds |
| E21 / D8 demo_store `Order.customer` | argument, not `msg_caller` | no | n/a | **known** (demo_store has no access control / orders for any customer) |
| D6 certified data, B15 certificate header | constant skip-certification tree | n/a | n/a | **known** (skip-certification finding) |
| D9, D10, C12, U16, O10 browser/dev tooling | no records of effects | | | n/a |
| E1/E2 `apply`: init tokens labelled `"init"` | label fixed | n/a | undocumented but harmless | holds |
| E3, E4, E4a, E4b, E6-E10, E12-E15, E20, E22-E28 | write no labelled record of an outbound effect, or admin state only | | | n/a |
| C1-C15, S1-S10, LIM1-LIM32, W1-W16, F1-F16, B1-B2, B4-B10, B12-B14, B17, VF1-VF3, VF5-VF8, AD1, AD3-AD9, T1-T12, CL1-CL37 (other than rows above) | swept: no other record or label of an outbound effect; claims rows were checked only for label-format statements, all listed above | | | n/a / covered above |

## Not tested

- SW6-1 and SW6-2 by reproduction: both live in the wasm-only `IcBridge` / `run_search`
  path, which has no native harness (register #58), and exercising SW6-1 live needs an
  allowlist and override change outside this phase. A Live-phase check: allowlist
  `aaaaa-aa` for `canister_status`, set the management `.did` as its override, run
  `icp.call("aaaaa-aa","canister_status",{canister_id: <demo_store>})`, read the two new
  audit entries, then restore the allowlist and remove the override.
