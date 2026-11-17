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
pub const MAX_CODE_BYTES: usize = 100 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    JavaScript,
    Python,
    Rhai,
}

impl Language {
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
    fn from_str(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "javascript" | "js" => Ok(Language::JavaScript),
            "python" | "py" => Ok(Language::Python),
            "rhai" => Ok(Language::Rhai),
            other => Err(format!("unsupported language `{other}` (expected javascript, python or rhai)")),
        }
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
        match lang {
            #[cfg(feature = "js")]
            Language::JavaScript => js::run(code, bridge).await,
            #[cfg(feature = "python")]
            Language::Python => python::run(code, bridge).await,
            #[cfg(feature = "rhai")]
            Language::Rhai => rhai_rt::run(code, bridge).await,
            #[allow(unreachable_patterns)]
            other => Err(format!("language `{}` is not compiled into this canister", other.as_str())),
        }
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
}
