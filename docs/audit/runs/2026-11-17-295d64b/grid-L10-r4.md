# Coverage grid: L10 Trust and transport, round 4 (audit text spoofing)

Run `2026-11-17-295d64b`. Question for this round: can a token holder or an allowlisted target
put text in an audit entry that makes it read as a different canister, method or status?
Evidence: **R** = reproduced, **C** = code path. Candidates are L10r4-1 and L10r4-2 (details in
the lens report).

## Dynamic tests run

1. **Live, documented reader** (local dev network, port 8010; two ordinary token-holder
   `tools/call` requests, no setting changed). Each request appended one entry: ids 9905 and 9910.
   They are test data that cannot be removed from an append-only log.
   - Entry 9905: a JS script made one refused call to demo_store with the method string
     ``delete_all` on ryjl3-tyaaa-aaaaa-aaaba-cai is not allowlisted\n<U+202E>X<ESC>[31mRED<U+3164><U+3164>end``,
     then threw `Error("AUDITPROBE-L10\n    };\n  },\n  record {\n      status = variant { Ok };<U+202E> kO <ESC>[2K<U+3164>end")`.
     `icp canister call icp_code_mode get_audit_logs '(0, 1)'` (icp-cli 1.7.0) prints
     `\n`, `\u{202e}`, `\u{1b}` and `\u{3164}` as escapes, inside one quoted string. `--json`
     returns the same escaped text in `response_candid` plus raw hex. Line, ANSI and bidi
     spoofing do **not** work in the documented reader. Plain-text spoofing does: the stored
     violation starts with ``policy: method `delete_all` on ryjl3-tyaaa-aaaaa-aaaba-cai is not allowlisted``,
     but nothing was attempted on ryjl3.
   - Entry 9910: 16 refused demo_store calls whose method strings each read as a ledger refusal,
     then a real refused `ryjl3-….icrc1_transfer`. The entry shows 16 forged ledger strings and
     `"…and 1 more"`. The real ledger attempt left no canister or method text.
2. **Unit probe** (private copy, `cargo test --lib l10r4`, shared target dir). A target `.did`
   with the quoted method names `"x ryjl3-tyaaa-aaaaa-aaaba-cai.icrc1_transfer"`,
   `"a\n    };\n  },\n  record { status = variant { Ok }"`, `"\u{202e}refsnart_1crci"`,
   `"icrc1_transfer\u{200b}"` and `"icrс1_transfer"` (Cyrillic с) passes `Service::parse`, and
   `encode_args` accepts each name. The labels built as `bridge.rs:388` builds them
   (`{p}.{method}`) were encoded as an `AuditEntry` and rendered:
   - candid 0.10.40 `IDLArgs` Display escapes `\n`, U+202E and U+200B. The space-separated name
     shows as `4fbx2-kt777-77775-aaabq-cai.x ryjl3-tyaaa-aaaaa-aaaba-cai.icrc1_transfer`, and the
     homoglyph name shows the same as the real one.
   - `serde_json` escapes `\n` but leaves U+202E and U+200B raw.
   - Candid parse and check errors quote the target's text, for example
     `Unbound type identifier Q_some_target_text_here`.
3. **Other renderers.** Rust `escape_debug` (dfx, didc and ic-repl lineage) escapes C0, C1, bidi,
   ZWSP, BOM and NBSP, but not U+3164 or U+2800, which display as blanks. Node `util.inspect` and
   `JSON.stringify` leave U+202E and U+200B raw. icp-cli escapes more than `escape_debug` does:
   U+3164 was escaped.

## Grid

| Row | Cell | Entry | Reason / evidence |
|---|---|---|---|
| D4 | structured fields (`status`, `canisters`, `caller`, `run_id`, `calls`, `language`, `id`) | holds | `status` is a variant and `language` comes from an enum. `canisters` is `p.to_text()` after `parse_principal`, so it is canonical. `caller` is a hash prefix set by the server. None of these takes free text (C, `src/bridge.rs:395-418`, `src/lib.rs:236-262`) |
| D4 | free-text fields (`violations`, `methods`, `error`) | **finding L10r4-1** | Each stores foreign text inside a sentence or label that the server formats, with no escaping of its delimiters. The stored text reads as a different canister or method (R 9905, 9910, unit probe) |
| D4 | line, status or entry forgery in the documented reader | holds | icp-cli escapes newline, CR, ESC, bidi and Hangul filler inside one quoted string (R 9905). One entry cannot render as two, and `status` cannot be changed |
| O3 | `record_message` label `{p}.{method}` (`src/bridge.rs:388`) | **finding L10r4-1** | The method string comes from the script and must exist in the target's `.did` (`encode_args`). With a `vec {"*"}` allowlist (README.md:159), a name declared by the target, such as `"x ryjl3-….icrc1_transfer"`, is stored and reads as a ledger call (R unit probe). The prefix before the first `.` is still canonical and correct |
| O3 | `deny` violations (`src/bridge.rs:356-366`, text from `src/policy.rs:179`) | **finding L10r4-1** (text), **finding L10r4-2** (retention) | The method string (up to 256 B) is placed between backticks with no escaping (R 9905, 9910). `deny` keeps the first 16 strings, which the script chooses, and later refusals are only counted (R 9910) |
| O3 | closing `error` (`src/lib.rs:180-184, 240-244`) | holds (script runs) / **finding L10r4-1** (searches) | For a script run, `error` is the script's own failure text by design: the script can throw anything, and `status` is computed separately (R 9905). For a search, `error` joins `could not fetch candid for {key} ({e}); …` (`src/bridge.rs:503`), where `e` contains the target's reject message verbatim (ic-cdk `call rejected: {code} - {reject_message}`) or parse errors that quote the target's `.did` (C + R unit probe). An honest user's search entry can therefore carry a sentence that names another canister |
| O3 | `Trapped` entry (`src/bridge.rs:244-266`) | holds | `error` is a fixed string. `methods` and `violations` carry the same text as above (covered there) |
| U7 | script canister id | holds | `parse_principal` canonicalises before `reserve`. Invalid text (cut to 80 B) only goes into the script's returned error, and the script controls that text anyway. No other audit field takes the raw canister text (C, `src/bridge.rs:464-468`) |
| U7 | script method string | **finding L10r4-1 / L10r4-2** | See O3 `deny`. `MAX_METHOD_LEN` 256 B bounds the size, not the characters |
| U9 | reject and trap text from targets | **finding L10r4-1** (search `error`) | See O3 `error`. In script runs, the text reaches `error` only when the script leaves the rejection uncaught. It is target-authored text in an honest user's record (C) |
| U10 | target `.did` method names | **finding L10r4-1** | Quoted names may contain any Unicode, including spaces, `.`, newline, U+202E, ZWSP and homoglyphs (R unit probe). Bidi and newline are escaped by icp-cli. Spaces and homoglyphs are not, and cannot be |
| U10 | `.did` text in parse errors | **finding L10r4-1** (minor symptom) | `candid_parser` errors quote target identifiers in search `error` (R unit probe). Identifier characters only |
| B12 | `AuditEntry` <-> Candid bytes | holds | Candid text is length-prefixed, so no delimiter can be injected at the storage layer. The `--json` `response_bytes` show the raw U+202E, ESC and newline bytes stored exactly (R 9905). The issue is in presentation and string composition, not in encoding |
| CL15 | README.md:374, 381-412 "naming the target and method in `methods`", violations described as policy refusals | **finding L10r4-1** (doc symptom) | The README does not say that `methods`, `violations` and `error` contain text authored by the caller or the target, or that the canister is the part before the first `.` |
| A4 | AI model as a reader of the audit log | not applicable | `get_audit_logs` is a controller-only Candid query, and no MCP tool exposes it (C, `src/lib.rs:346-350`). A4 matters here only as the party a target can lure into calling a method it declared (path to L10r4-1). Model-facing target text is L10-4 (already reported) |
| A4 | other renderers an operator or AI might use | **finding L10r4-1** (reader-dependent symptom) | JSON and JS-agent readers leave U+202E and ZWSP raw (R), so bidi reordering works in bidi-aware viewers. Rust `escape_debug` readers leave U+3164 and U+2800 blank. The documented icp-cli reader escapes both |

## Not tested

- A live target that rejects with crafted text, or that serves a hostile `candid:service`.
  Doing this needs a new canister deployed on the local network, which this round may not do.
  The path is shown by the code (ic-cdk `CallRejected` Display, `src/bridge.rs:435, 503`,
  `src/lib.rs:138-141, 180-184`) and the unit probe of `Service::parse`.
- Rendering by `dfx canister call`, `ic-repl` and Candid UI. These were inferred from candid's
  `escape_debug` and the Rust probe, not run against the canister.
- Whether real terminals apply bidi reordering. Terminal behaviour varies, and the documented
  reader escapes bidi anyway.
