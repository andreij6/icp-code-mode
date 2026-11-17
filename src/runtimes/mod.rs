//! Language dispatch. Each engine gets a fresh VM per call and shares the
//! same `HostBridge`, so behaviour differs only in syntax.

use crate::bridge::HostBridge;
use serde_json::Value;

#[cfg(feature = "js")]
pub mod js;
#[cfg(feature = "python")]
pub mod python;
#[cfg(feature = "rhai")]
pub mod rhai_rt;

/// Upper bound on script source size.
pub const MAX_CODE_BYTES: usize = 16 * 1024;
/// Concurrent `icp.*` requests in one batch (JS `Promise.all`, Python `asyncio.gather`).
pub const MAX_PENDING: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    JavaScript,
    Python,
    Rhai,
}

impl Language {
    /// The languages compiled into this build; the first is the default.
    pub const ENABLED: &[Language] = &[
        #[cfg(feature = "js")]
        Language::JavaScript,
        #[cfg(feature = "python")]
        Language::Python,
        #[cfg(feature = "rhai")]
        Language::Rhai,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Language::JavaScript => "javascript",
            Language::Python => "python",
            Language::Rhai => "rhai",
        }
    }
}

impl std::str::FromStr for Language {
    type Err = String;
    /// Only languages compiled into this build parse, so a request for another one is
    /// refused when it is validated, before any run slot or audit entry.
    fn from_str(s: &str) -> Result<Self, String> {
        let names = Language::ENABLED.iter().map(|l| l.as_str()).collect::<Vec<_>>().join(", ");
        let l = match s.to_ascii_lowercase().as_str() {
            "javascript" | "js" => Language::JavaScript,
            "python" | "py" => Language::Python,
            "rhai" => Language::Rhai,
            other => return Err(format!("unsupported language `{other}` (expected one of: {names})")),
        };
        if !Language::ENABLED.contains(&l) {
            return Err(format!("language `{}` is not compiled into this canister (available: {names})", l.as_str()));
        }
        Ok(l)
    }
}

#[cfg(test)]
mod tests {
    use super::Language;

    /// 95: a known but uncompiled language is refused when parsed; every enabled one parses.
    #[test]
    fn only_compiled_languages_parse() {
        for name in ["javascript", "python", "rhai"] {
            let r = name.parse::<Language>();
            let enabled = Language::ENABLED.iter().any(|l| l.as_str() == name);
            assert_eq!(r.is_ok(), enabled, "{name}: {r:?}");
            if !enabled {
                assert!(r.unwrap_err().contains("not compiled"));
            }
        }
        assert!("cobol".parse::<Language>().unwrap_err().contains(Language::ENABLED[0].as_str()));
    }
}

pub struct LanguageRouter;

impl LanguageRouter {
    /// Run `code` in a fresh sandbox of the chosen language. Returns the script's
    /// result value; output goes to `bridge.log`.
    pub async fn execute<B: HostBridge>(lang: Language, code: &str, bridge: &B) -> Result<Value, String> {
        if code.len() > MAX_CODE_BYTES {
            return Err(format!("script exceeds {MAX_CODE_BYTES} bytes"));
        }
        let _ = bridge; // silence unused warnings when every engine is compiled out
        let out = match lang {
            #[cfg(feature = "js")]
            Language::JavaScript => js::run(code, bridge).await,
            #[cfg(feature = "python")]
            Language::Python => python::run(code, bridge).await,
            #[cfg(feature = "rhai")]
            Language::Rhai => rhai_rt::run(code, bridge).await,
            #[allow(unreachable_patterns)]
            other => Err(format!("language `{}` is not compiled into this canister", other.as_str())),
        };
        out.map(crate::bridge::big_ints_to_strings)
    }
}

/// Equivalent scripts in all three languages against the same mock bridge.
#[cfg(all(test, feature = "js", feature = "python", feature = "rhai"))]
mod cross_language_tests {
    use super::*;
    use crate::bridge::mock::MockBridge;
    use futures::executor::block_on;
    use serde_json::json;

    const LEDGER: &str = "ryjl3-tyaaa-aaaaa-aaaba-cai";

    fn run(lang: Language, code: &str) -> (Result<Value, String>, MockBridge) {
        let b = MockBridge::default();
        let out = block_on(LanguageRouter::execute(lang, code, &b));
        (out, b)
    }

    #[test]
    fn same_script_three_languages() {
        let js = format!(
            r#"const did = await icp.candid("{LEDGER}");
               const [a, b] = await Promise.all([
                 icp.call("{LEDGER}", "icrc1_balance_of", {{owner: "aaaaa-aa"}}),
                 icp.call("{LEDGER}", "icrc1_balance_of", {{owner: "2vxsx-fae"}})]);
               console.log("me", icp.principal());
               return {{total: a.balance + b.balance, owners: [a.owner, b.owner], has_did: did.includes("icrc1_balance_of")}};"#
        );
        let py = format!(
            r#"did = icp.candid("{LEDGER}")
a = icp.call("{LEDGER}", "icrc1_balance_of", {{"owner": "aaaaa-aa"}})
b = icp.call("{LEDGER}", "icrc1_balance_of", {{"owner": "2vxsx-fae"}})
print("me", icp.principal())
{{"total": a["balance"] + b["balance"], "owners": [a["owner"], b["owner"]], "has_did": "icrc1_balance_of" in did}}"#
        );
        let rhai = format!(
            r#"let did = icp::candid("{LEDGER}");
               let a = icp::call_canister("{LEDGER}", "icrc1_balance_of", #{{owner: "aaaaa-aa"}});
               let b = icp::call_canister("{LEDGER}", "icrc1_balance_of", #{{owner: "2vxsx-fae"}});
               print("me " + icp::principal());
               #{{total: a.balance + b.balance, owners: [a.owner, b.owner], has_did: did.contains("icrc1_balance_of")}}"#
        );
        let expected = json!({ "total": 300, "owners": ["aaaaa-aa", "2vxsx-fae"], "has_did": true });
        for (lang, code) in [(Language::JavaScript, js), (Language::Python, py), (Language::Rhai, rhai)] {
            let (out, b) = run(lang, &code);
            assert_eq!(out, Ok(expected.clone()), "{lang:?}");
            assert_eq!(b.calls.borrow().len(), 2, "{lang:?} must issue exactly two calls");
            assert_eq!(b.calls.borrow()[1].2, json!({ "owner": "2vxsx-fae" }), "{lang:?}");
            let logs = b.logs.take();
            assert_eq!(logs.len(), 1, "{lang:?}: {logs:?}");
            assert!(logs[0].contains("aaaaa-aa"), "{lang:?}: {logs:?}");
        }
    }

    #[test]
    fn policy_errors_surface_and_are_catchable() {
        let cases = [
            (Language::JavaScript, r#"try { await icp.call("aaaaa-aa", "raw_rand", null); return "no"; } catch (e) { return "caught: " + e.message; }"#),
            (Language::Python, "try:\n    icp.call(\"aaaaa-aa\", \"raw_rand\", None)\n    r = \"no\"\nexcept Exception as e:\n    r = \"caught: \" + str(e)\nr"),
            (Language::Rhai, r#"let r = (); try { icp::call_canister("aaaaa-aa", "raw_rand", ()); r = "no"; } catch (e) { r = "caught: " + e; } r"#),
        ];
        for (lang, code) in cases {
            let out = run(lang, code).0.unwrap();
            let s = out.as_str().unwrap_or_default();
            assert!(s.starts_with("caught: ") && s.contains("not allowlisted"), "{lang:?}: {out}");
        }
        // Uncaught: becomes an Err for the tool result.
        for (lang, code) in [
            (Language::JavaScript, r#"await icp.call("aaaaa-aa", "raw_rand", null)"#),
            (Language::Python, r#"icp.call("aaaaa-aa", "raw_rand", None)"#),
            (Language::Rhai, r#"icp::call_canister("aaaaa-aa", "raw_rand", ())"#),
        ] {
            assert!(run(lang, code).0.unwrap_err().contains("not allowlisted"), "{lang:?}");
        }
    }

    #[test]
    fn syntax_errors_and_runaway_loops_are_contained() {
        for (lang, code) in [(Language::JavaScript, "return (;"), (Language::Python, "def (:"), (Language::Rhai, "let = ;")] {
            assert!(run(lang, code).0.is_err(), "{lang:?}");
        }
        for (lang, code) in [
            (Language::JavaScript, "while (true) {}"),
            (Language::Python, "while True:\n    pass"),
            (Language::Rhai, "loop {}"),
        ] {
            assert!(run(lang, code).0.is_err(), "{lang:?} infinite loop must be stopped");
        }
        for (lang, code) in [
            (Language::JavaScript, "function f(n) { return f(n + 1); } return f(0);"),
            (Language::Python, "def f(n):\n    return f(n + 1)\nf(0)"),
            (Language::Rhai, "fn f(n) { f(n + 1) } f(0)"),
        ] {
            assert!(run(lang, code).0.is_err(), "{lang:?} unbounded recursion must be stopped");
        }
    }

    /// Requests interleave across awaits on-chain. Runs of every engine, suspended
    /// mid-call while others start, finish and collect garbage, must not disturb each other.
    #[test]
    fn interleaved_runs_are_isolated() {
        use crate::bridge::mock::YieldBridge;
        let js = |tag: &str| format!(
            r#"globalThis.leak = "{tag}"; const junk = []; for (let i = 0; i < 20000; i++) junk.push({{i, s: "x" + i}});
               const a = await icp.call("{LEDGER}", "icrc1_balance_of", {{owner: "{tag}"}});
               const more = []; for (let i = 0; i < 20000; i++) more.push([i]);
               const b = await icp.call("{LEDGER}", "icrc1_balance_of", {{owner: "{tag}"}});
               return [globalThis.leak, a.owner, b.owner, junk.length + more.length];"#);
        let py = |tag: &str| format!("x = ['{tag}'] * 1000\na = icp.call('{LEDGER}', 'icrc1_balance_of', {{'owner': '{tag}'}})\n[x[0], a['owner'], len(x)]");
        let rh = |tag: &str| format!(r#"let a = icp::call_canister("{LEDGER}", "icrc1_balance_of", #{{owner: "{tag}"}}); let b = icp::call_canister("{LEDGER}", "icrc1_balance_of", #{{owner: "{tag}"}}); [a.owner, b.owner]"#);
        let bridges: Vec<YieldBridge> = (0..7).map(|_| YieldBridge(MockBridge::default())).collect();
        let (j1, j2, j3, p1, p2, r1, r2) = (js("one"), js("two"), js("three"), py("p1"), py("p2"), rh("r1"), rh("r2"));
        let out = block_on(futures::future::join_all(vec![
            Box::pin(LanguageRouter::execute(Language::JavaScript, &j1, &bridges[0])) as std::pin::Pin<Box<dyn std::future::Future<Output = _>>>,
            Box::pin(LanguageRouter::execute(Language::Python, &p1, &bridges[1])),
            Box::pin(LanguageRouter::execute(Language::JavaScript, &j2, &bridges[2])),
            Box::pin(LanguageRouter::execute(Language::Rhai, &r1, &bridges[3])),
            Box::pin(LanguageRouter::execute(Language::JavaScript, &j3, &bridges[4])),
            Box::pin(LanguageRouter::execute(Language::Python, &p2, &bridges[5])),
            Box::pin(LanguageRouter::execute(Language::Rhai, &r2, &bridges[6])),
        ]));
        assert_eq!(out[0], Ok(json!(["one", "one", "one", 40000])));
        assert_eq!(out[2], Ok(json!(["two", "two", "two", 40000])));
        assert_eq!(out[4], Ok(json!(["three", "three", "three", 40000])));
        assert_eq!(out[1], Ok(json!(["p1", "p1", 1000])));
        assert_eq!(out[5], Ok(json!(["p2", "p2", 1000])));
        assert_eq!(out[3], Ok(json!(["r1", "r1"])));
        assert_eq!(out[6], Ok(json!(["r2", "r2"])));
    }

    #[test]
    fn unknown_language_and_oversized_code() {
        assert!("cobol".parse::<Language>().is_err());
        let big = "1;".repeat(MAX_CODE_BYTES);
        assert!(run(Language::Rhai, &big).0.unwrap_err().contains("exceeds"));
    }

    const JS: Language = Language::JavaScript;
    const PY: Language = Language::Python;
    const RH: Language = Language::Rhai;

    /// 110: with the off-chain instruction counter, a deadline stops a running loop in
    /// every runtime (Python's VM budget follows the deadline natively too).
    #[test]
    fn instruction_deadline_stops_every_runtime() {
        for (lang, code) in [(JS, "while (true) {}"), (PY, "while True:\n    pass"), (RH, "loop {}")] {
            let out = block_on(LanguageRouter::execute(lang, code, &MockBridge::with_budget(20_000)));
            let e = out.unwrap_err();
            let python_vm = lang == PY && e.contains("TimeoutError") && !e.contains("2s");
            assert!(e.contains("instruction budget exceeded") || python_vm, "{lang:?}: {e}");
        }
        // Nothing is sent once it is spent.
        for (lang, code) in [(JS, format!("await icp.call('{LEDGER}', 'm', null)")), (PY, format!("icp.call('{LEDGER}', 'm')")),
                             (RH, format!("icp::call_canister(\"{LEDGER}\", \"m\")"))] {
            let b = MockBridge::with_budget(3);
            let e = block_on(LanguageRouter::execute(lang, &code, &b)).unwrap_err();
            assert!(e.contains("budget") || e.contains("TimeoutError"), "{lang:?}: {e}");
            assert!(b.calls.borrow().is_empty(), "{lang:?}");
        }
    }

    /// 114: integers beyond 2^53 leave every runtime as exact decimal strings.
    #[test]
    fn big_integers_leave_as_strings() {
        let expected = json!(["1152921504606846976", 9007199254740991u64, "-9007199254740992", 3]);
        for (lang, code) in [
            (PY, "[2**60, 2**53 - 1, -(2**53), 3]"),
            (RH, "[1152921504606846976, 9007199254740991, -9007199254740992, 3]"),
        ] {
            assert_eq!(run(lang, code).0, Ok(expected.clone()), "{lang:?}");
        }
        // JS numbers are doubles: integral ones up to 2^53 are integers, the rest floats.
        assert_eq!(run(JS, "return [2**53 - 1, 3]").0, Ok(json!([9007199254740991u64, 3])));
        assert!(run(JS, "return 2**60").0.unwrap().is_f64());
    }

    /// 155: values JSON has no rule for leave every runtime the same way.
    #[test]
    fn values_without_a_json_form_follow_one_rule() {
        let common = json!(["NaN", "inf", "-inf", -0.0, [1, 2]]);
        for (lang, code) in [
            (JS, "return [NaN, Infinity, -Infinity, -0, new Uint8Array([1, 2])]"),
            (PY, "[float('nan'), float('inf'), float('-inf'), -0.0, b'\\x01\\x02']"),
            (RH, "let b = blob(); b.push(1); b.push(2); [parse_float(\"NaN\"), parse_float(\"inf\"), parse_float(\"-inf\"), -0.0, b]"),
        ] {
            let v = run(lang, code).0.unwrap();
            assert_eq!(v, common, "{lang:?}");
            assert!(v[3].as_f64().unwrap().is_sign_negative(), "{lang:?}: -0 keeps its sign");
        }
        // Bytes from other containers, and lone surrogates (U+FFFD, as Python's lexer does).
        let v = run(JS, "return [new Uint8Array([1, 2, 3]).buffer, new DataView(new Uint8Array([4, 5]).buffer), new Int16Array([-1]), '\\ud800x', {['\\udc00']: 1}]").0;
        assert_eq!(v, Ok(json!([[1, 2, 3], [4, 5], [-1], "\u{fffd}x", { "\u{fffd}": 1 }])));
        assert_eq!(run(PY, "'\\ud800x'").0, Ok(json!("\u{fffd}x")));
        // JS errors leave as their text, also inside `allSettled` results.
        let v = run(JS, "const [r] = await Promise.allSettled([Promise.reject(new TypeError('boom'))]); return r").0;
        assert_eq!(v, Ok(json!({ "status": "rejected", "reason": "TypeError: boom" })));
        // Inside the script, JSON.stringify keeps JSON's own rules.
        assert_eq!(run(JS, "return JSON.stringify([NaN, -0, new Error('x'), new Uint8Array([1])])").0, Ok(json!("[null,0,{},{\"0\":1}]")));
    }

    /// 156: at most `MAX_DEPTH` levels of arrays, whatever the innermost value, in every runtime.
    #[test]
    fn nesting_depth_is_the_same_everywhere() {
        let max = crate::candid_bridge::MAX_DEPTH;
        for n in [max, max + 1, max + 2] {
            for inner in ["[]", "[1]"] {
                for (lang, code) in [
                    (JS, format!("let a = {inner}; for (let i = 1; i < {n}; i++) a = [a]; return a")),
                    (PY, format!("a = {inner}\nfor _ in range({n} - 1):\n    a = [a]\na")),
                    (RH, format!("let a = {inner}; for i in 1..{n} {{ a = [a]; }} a")),
                ] {
                    let out = run(lang, &code).0;
                    assert_eq!(out.is_ok(), n == max, "{lang:?} {n} {inner}: {out:?}");
                }
            }
        }
    }

    /// 175 / 176: captured output is capped at `MAX_LOG_BYTES` as sent (escaped, a newline
    /// per line), a cut line keeps its prefix, and the transport never cuts it again.
    #[test]
    fn output_caps_are_measured_alike() {
        use crate::bridge::{CUT_MARK, MAX_LOG_BYTES, log_cost};
        let cases: [(&str, [String; 3]); 4] = [
            ("empty", ["for (let i = 0; i < 100000; i++) console.log(); return 1".into(), "for _ in range(100000):\n    print()\n1".into(), "for i in 0..100000 { print(\"\"); } 1".into()]),
            ("control", ["console.log('\\x01'.repeat(60000)); return 1".into(), "print('\\x01' * 60000)\n1".into(), "let s = \"\"; s.pad(60000, '\\x01'); print(s); 1".into()]),
            ("long", ["console.log('x'.repeat(70000)); return 1".into(), "print('x' * 70000)\n1".into(), "let s = \"\"; s.pad(70000, 'x'); print(s); 1".into()]),
            ("lines", ["for (let i = 0; i < 10; i++) console.log('x'.repeat(7000)); return 1".into(), "for _ in range(10):\n    print('x' * 7000)\n1".into(), "let s = \"\"; s.pad(7000, 'x'); for i in 0..10 { print(s); } 1".into()]),
        ];
        for (name, codes) in cases {
            for (lang, code) in [JS, PY, RH].into_iter().zip(codes) {
                let (out, b) = run(lang, &code);
                assert_eq!(out, Ok(json!(1)), "{lang:?} {name}");
                let logs = b.logs.take();
                let cost: usize = logs.iter().map(|l| log_cost(l)).sum();
                assert!(cost <= MAX_LOG_BYTES && cost > MAX_LOG_BYTES - 64, "{lang:?} {name}: {cost}");
                let last = logs.last().unwrap();
                assert!(last.ends_with(CUT_MARK), "{lang:?} {name}");
                if name != "empty" {
                    assert!(!last.starts_with(CUT_MARK), "{lang:?} {name}: the cut line keeps its prefix");
                }
                let r = crate::mcp::tool_result(Ok(json!(1)), logs);
                assert_eq!(r["content"][1]["text"].as_str().unwrap().matches(CUT_MARK).count(), 1, "{lang:?} {name}");
            }
        }
    }

    #[test]
    fn language_parsing_aliases_and_names() {
        assert_eq!("js".parse::<Language>().unwrap().as_str(), "javascript");
        assert_eq!("PY".parse::<Language>().unwrap().as_str(), "python");
        assert_eq!("python".parse::<Language>().unwrap().as_str(), "python");
        assert_eq!("RHAI".parse::<Language>().unwrap().as_str(), "rhai");
    }
}
