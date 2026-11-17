# Coverage grid: L5 Cost and payment, round 2 (value-moving flows, live)

Run `2026-11-17-295d64b`. Lens question: who pays, and can someone else make them pay? This
round covers the one flow round 1 left as "needs a controller": a value-moving method
(`icrc1_transfer`, legacy `transfer`) on the local ICP ledger `ryjl3-tyaaa-aaaaa-aaaba-cai`,
driven end to end through the MCP gateway.

Evidence keys: **R** = reproduced live on the project's local icp-cli network (port 8010,
`icp_code_mode` = `4caro-hl777-77775-aaaba-cai`, module hash `ad3ee221...` = the audited
build, not redeployed by this lens); **C** = code path.

## Setup and restoration (Live phase)

- Test funds: 300M test ICP sent from the local `anonymous` identity to the canister's
  default ledger account (blocks 496, 498). Destinations were subaccounts `..01`-`..0c` of
  `anonymous`.
- Allowlist: `icrc1_transfer` added to the ledger entry at 11:06 UTC (later `transfer` as
  well, for the memo test); restored afterwards to the original five read methods (see "Restoration" at the end).
- `set_limits` was changed once to `max_instructions = 25_000_000` for one run (SW3-2 test)
  and immediately set back to the defaults (25 calls, 20_000_000_000, no timeout, no
  certified reads); `get_policy` confirmed the restore.
- Interface cache was evicted with `remove_allowlist` + `set_allowlist` to get cold-cache runs.
- The test token was read from `.env` by a helper script and never printed.
- Interference seen: between 11:08 and 11:09 UTC every update call to `icp_code_mode`
  failed with "Canister exceeded its current Wasm memory limit of 3221225472 bytes" (memory
  size 3.42 GB; a `Trapped` search run against another lens's fault-injection canister
  `4qggx-...` precedes it). Someone else reinstalled the same module (memory back to 95 MB,
  stable state kept). Later all 8 run slots were held by another lens's `hang` runs, which
  blocked scripts ("busy") for minutes. Neither is a finding of this lens; both are noted
  for L3/L18/L4-5.

## Measurements

| # | Script (sent as `tools/call`) | Reply to the client | What moved on the ledger | Audit |
|---|---|---|---|---|
| M1 | JS `amount: 9007199254740993` | `{"Ok":497}`, `isError:false` | **9_007_199_254_740_992** (1 e8s less than written, no warning) | `Started`+`Ok`, methods `icrc1_transfer` |
| M2 | JS `amount: "9007199254740993"` (string) | `{"Ok":499}` | 9_007_199_254_740_993 (exact) | same |
| M3 | Python `"amount": 9007199254740993` (int) | `{"Ok":500}` | 9_007_199_254_740_993 (exact) | same |
| M4 | JS legacy `transfer`, `memo: 12345678901234567` (nat64, e.g. an exchange deposit memo), 1 ICP | `{"Ok":515}`, `isError:false` | 1 ICP with **memo 12_345_678_901_234_568** in block 515 (a different payment identifier; a receiver that routes deposits by memo would not credit it) | `Started`+`Ok`, methods `transfer` |
| M5 | JS: 63 x `await icp.candid(L)` (cached, no message), then `await icp.call(L,"icrc1_transfer",{amount:1e8,...})` | `"script exceeded 64 host round-trips; batch calls with Promise.all"`, `isError:true` | **1 ICP moved**; block index never returned | `Started` (icrc1_transfer) + closing `Error` with that text (entries 919/920) |
| M6 | M5 sent again, byte-identical (a client/gateway retry) | same error | **another 1 ICP moved** (subaccount `..04`: 0 -> 1e8 -> 2e8) | same pair again |
| M7 | Python: 255 x `icp.principal()`, then `icp.call(L,"icrc1_transfer",...)` | `"script exceeded 256 host suspensions"`, `isError:true` | **1 ICP moved** | `Started` + `Error` (938/939) |
| M8 | JS transfer with `created_at_time: Date.now()*1000000`, memo blob, sent twice | `{"Ok":503}`, `{"Ok":504}` | **2 ICP** (no dedup: the sandbox clock is per run) | two `Ok` runs |
| M9 | JS transfer with a constant `created_at_time` (decimal string), memo, sent twice | `{"Ok":505}`, `{"Err":{"Duplicate":{"duplicate_of":505}}}`, both `isError:false` | 1 ICP (ledger dedup works) | **both runs `Ok`**, identical methods: the audit cannot tell the transfer from the refused duplicate |
| M10 | JS transfer with memo, `created_at_time: null`, sent twice | `{"Ok":506}`, `{"Ok":507}` | 2 ICP (memo alone does not dedup, per ICRC-1) | two `Ok` runs |
| M11 | JS cold-cache transfer with `max_instructions` = 25M (reserve at ~19M, send at 32.8M) | `"policy: instruction budget exceeded (36981408 > 25000000)"`, `isError:true` | **1 ICP moved** (subaccount `..0a` 1e8 -> 2e8) | `Started` (canister_metadata, 19.0M) + `Call` (icrc1_transfer, 32.8M) + `PolicyViolation` (1054-1056) |
| M12 | JS args with a 127-deep extra field (whole args become `null`, L7-2) | `"argument 0: expected record object, got null"` | nothing | no message |
| M13 | Revocation race (L1-2): cold-cache transfer, `set_allowlist` without `icrc1_transfer` issued 0.2-1.2 s after the request | all four `Ok` | 4 ICP | Started->Call gap measured at 0.115-0.13 s (one round); every revoke executed after the send, so the race was not won. Not reproduced |

## Cells

| Row | Verdict | Reason / evidence |
|---|---|---|
| VF4 assets held by C1's principal | **finding** L5r2-1, L5r2-2, L5r2-3 | Round 1 left ledger funds "n.a. until a controller allowlists a transfer". With `icrc1_transfer` allowlisted (the README's own example of a value method), every token holder moves the canister's funds (register #49, accepted) and: a transfer is executed while the run is reported failed (M5, M7, M11), a retry repeats it (M6, M8, M10), the amount written as a JS number is changed (M1), and the audit cannot tell what moved (M5, M9, M11). No cycles can be moved: no `with_cycles`/`msg_cycles_*` (C), holds |
| O1 outbound call | **finding** L5r2-1 (send after a reported failure), L5r2-2 (send past the budget); L1-2 combination not reproduced | M5/M7: the message is sent in round 64 / suspension 256 and its reply is dropped by the loop bound (`js.rs:374-409`, `python.rs:239-277`). M11: `reserve` checks the budget at ~19M, the fetch reply, parse and encode run, and the transfer goes out at 32.8M, past the 25M budget; nothing re-checks before `self.sent` (`bridge.rs:515-528`). L1-2 (no allowlist re-check after the fetch await): the window is one fetch round trip, measured 0.115-0.13 s locally; 4 attempts did not land a revoke inside it (M13). Note: `set_allowlist` narrowing does not evict the cache, so warm runs have no window; `remove_allowlist` evicts, so the next run is cold. No cycles attached: holds |
| B5 JSON -> Candid | **finding** L5r2-4 (live consequence of L7-3) | M1: an unsafe JS integer is accepted as an exact `nat` and the ledger moves a different amount than the script text says, reported `Ok`. M2/M3: strings and Python ints are exact, so the defect is the silent acceptance of non-safe floats in `num_text` (`candid_bridge.rs:151-163`). The money impact on an amount is at most one ulp (relative 2^-53); the impact on identifiers carried as `nat64` (legacy ledger `memo`, NNS neuron ids, `created_at_time`) is a different identifier: see M4. Opt/null handling of `created_at_time: null`, blob subaccounts as `0x` hex: hold (M2-M10) |
| E16 JS host functions | **finding** L5r2-1 (M5, M6), L5r2-4 (M1); L7-2 holds for value methods | L7-2 deep args become `null`, but `icrc1_transfer`/`transfer` take a record, so `null` fails to encode and nothing is sent (M12, R). A method whose single argument is `opt` would receive `null` (C, not on this ledger) |
| E17 console/print | not applicable | Moves no value; output cap is L3/L6's |
| E18 Python host functions | **finding** L5r2-1 (M7); amounts hold (M3) | Python ints convert exactly; a Python float that happens to be integral (e.g. `balance * 1.0`) goes through the same `num_text` path as JS (C, L7-3 root) |
| E19 Rhai host functions | holds (C + native, not live) | Rhai counts `answers.len() >= MAX_REQUESTS` before sending (`rhai_rt.rs:211`), so a 64th request that is sent is also completed (SW5-1 native table: 64 -> Ok); replay issues each call once (`rhai_rt::replay_issues_each_call_once_and_keeps_final_logs`), so a transfer is not re-sent by a replay. Rhai integers are i64; floats would hit L7-3. Not run live: all 8 run slots were held by another lens's `hang` runs during this window |
| grid-L4 "Retries / replays of a whole request" | **finding** L5r2-1 (was "not applicable") | The L4 cell reasoned no promise was made. But the service itself provokes the retry: M5/M7/M11 tell the client the script failed (M5's text even says to restructure and run again) after the transfer executed, and M6 shows a byte-identical retry (same JSON-RPC id 6) moves value again; nothing dedups by request id or script hash. Ledger-side dedup protects only a script that hard-codes `created_at_time` (M9); the natural `Date.now()`-derived value differs per run because the sandbox clock is `FixedClock(now_ms())` per run (`js.rs:278`) (M8), memo alone does not dedup (M10), and neither the tool description (`mcp.rs:77-101`) nor the README mentions `created_at_time` or idempotency. `execute_icp_script` has no annotations (`mcp.rs:116-133`), so MCP's default `idempotentHint:false` applies, which tells a client not to assume retries are safe, but the client is retrying a call it was told failed |
| Audit of value moves (D4, CL15, README "How to read the audit log") | **finding** L5r2-3 | An entry records only `canister.method`; no argument hash, amount, recipient or reply. M9: a transfer and its refused duplicate both close `Ok` with identical entries. M5/M7: the run that moved 1 ICP closes `Error` "script exceeded 64 host round-trips"; README:404-406 defines `Error` as "the script failed" and README:397-398 says "Whether the target executed it is in the script's result or error", but here the ledger's reply (block 501/502) reached the canister and was discarded, so neither the client, the result, nor the audit has it. M11: closes `PolicyViolation`, which README:406 describes as "A call was refused by policy, or the instruction budget ran out", while the call was not refused |
| I2 policy precedes effect | violated (R, M11) | Budget checked before the await only (SW3-2). Allowlist re-check after the await: C (L1-2), not reproduced live |
| I3 records match reality | violated (R, M5, M7, M9, M11) | See L5r2-3 |
| I8 fidelity | violated (R, M1, M4) | See L5r2-4 |

## Candidates

- **L5r2-1** (Critical per the plan's rule "grade by the worst outcome demonstrated": loss of
  value; narrow trigger; root cause SW5-1, this is its value consequence and a regrade
  request). A transfer that is the last request of a JS run reaching round 64 (or a Python
  run reaching suspension 256) is executed, its reply is discarded, and the client is told
  the script failed; a retry of the same `tools/call` executes it again. R: M5+M6 moved
  2 ICP for two `isError:true` replies; M7 for Python. Ledger dedup does not help unless the
  script hard-codes `created_at_time` (M8-M10). Fix: the SW5-1 fix (re-check the promise /
  `progress` after the loop or count before dispatch, as Rhai does); plus, for any run that
  sent messages, never report a bare failure: include the sent-message list and say the
  effects may have happened.
- **L5r2-2** (High: value moved while the run is reported as a policy refusal; root cause
  SW3-2, value consequence). With a cold interface cache the instruction budget is checked
  before the fetch and not again before the send: M11 moved 1 ICP at 32.8M instructions
  with a 25M budget and reported `policy: instruction budget exceeded`. At the default 20B
  budget the overshoot window is the fetch + parse + encode (about 14M instructions for the
  ledger interface), i.e. only scripts that spent ~19.98B first. Fix: re-check budget and
  allowlist in `call` right before `self.sent` (also closes L1-2).
- **L5r2-3** (Medium: an operator is misled). The audit log cannot show whether or how much
  value moved: no args/amount/recipient/reply in entries; a ledger-refused duplicate and a
  real transfer both close `Ok` (M9); runs that moved value close `Error` (M5, M7) or
  `PolicyViolation` (M11), contrary to README:397-398, 404-406. Fix: record a hash of the
  encoded args and the reply status (or the first bytes of the reply) on the message entry
  when the reply arrives, and give the closing entry a status that distinguishes "script
  failed after N messages were sent" from "nothing was sent".
- **L5r2-4** (High per the scale "return wrong data as if it were right", for identifiers;
  root cause L7-3, value consequence and a regrade request). R: M1 moved 9_007_199_254_740_992
  for a script that wrote 9007199254740993, reported `Ok`. The amount error is at most one
  ulp; M4 shows the identifier case: a legacy `transfer` with memo 12345678901234567 was
  recorded in block 515 with memo 12345678901234568, reported `Ok`. Fix: refuse JS/Python numbers
  that are not safe integers where a `nat`/`int` is expected ("pass it as a string"), as the
  output side already does.

## Restoration

- 11:17 UTC: the canister's remaining test ICP (2_978_400_735_587_022 e8s) returned to the
  `anonymous` default account by a script (block 516); the canister's ledger balance is 0.
  Test ICP sent to `anonymous` subaccounts `..01`-`..0c` stays with `anonymous`.
- Ledger allowlist entry set back to the original five methods (`icrc1_balance_of`,
  `icrc1_symbol`, `icrc1_decimals`, `icrc1_metadata`, `icrc1_total_supply`); `get_policy`
  diffed against the snapshot taken before: identical except two allowlist entries added by
  another lens (`4qggx-...`, `4xhad-...`). Limits at defaults. Network left running.

## Not tested

- L1-2 race live: the window is one round (~0.12 s locally); four timed attempts missed it.
  A deterministic test needs a hook to hold the fetch reply (e.g. a target whose
  `canister_metadata` is slow, or a PocketIC test that controls rounds).
- Gateway-initiated retries: whether the HTTP gateway (local or boundary node) re-submits a
  timed-out `http_request_update` was not observed; only client retries (M6) were. A script
  waiting on a slow target past the gateway timeout would make the client see a failure
  while the run continues: not tested (needs a slow target and the gateway's timeout).
- 18-decimal ICRC tokens (ckETH-like), where every amount above ~0.009 token exceeds 2^53:
  no such ledger on the local network, and installing one means downloading a ledger Wasm.
- Rhai transfer at request 64: not run live (slots held by other runs); native result cited.
- ICRC-2 `icrc2_approve`/`icrc2_transfer_from` (allowance amounts and expiry are nat/nat64):
  not allowlisted or tested; same converter, same expectation as M1.
