//! Runs the release Wasm under PocketIC: scripts that can only fail on wasm32 (memory
//! exhaustion, stack depth, huge allocations) must come back as a JSON-RPC reply or a
//! clean 503, never a trap, and the canister must still answer afterwards.
//! Needs the default (all-language) build; WASM overrides its path.

use candid::{CandidType, Deserialize, Principal, decode_one, encode_one};
use pocket_ic::PocketIc;
use serde_json::{Value, json};

const TOKEN: &str = "harness-token";
// echo -n harness-token | shasum -a 256
const TOKEN_SHA256: &str = "c56fdc11770ecd8117b413923b08863ffa0a234046fe61623ac2e24eac4231f4";

#[derive(CandidType)]
struct Limits {
    call_timeout_seconds: Option<u32>,
    tool_calls_per_minute: Option<u32>,
    max_calls_per_script: u32,
    max_instructions: u64,
    certified_reads: Option<bool>,
}

#[derive(CandidType)]
struct InitArgs {
    token_sha256: Vec<String>,
    allowlist: Vec<(Principal, Vec<String>)>,
    limits: Option<Limits>,
}

#[derive(CandidType)]
struct HttpUpdateRequest {
    url: String,
    method: String,
    body: Vec<u8>,
    headers: Vec<(String, String)>,
}

#[derive(Deserialize, CandidType)]
struct HttpResponse {
    body: Vec<u8>,
    status_code: u16,
}

// Each case is (language, code). Every one used to trap on wasm32, or is the kind that can.
const CORPUS: &[(&str, &str)] = &[
    ("python", "import json\njson.dumps(1, indent=2**31)"),
    ("python", "def f(n):\n    return f(n + 1)\nf(0)"),
    ("python", "x = [0] * 10**10"),
    ("python", "s = 'x' * 2**40"),
    ("javascript", "function f(n) { return f(n + 1) } f(0)"),
    ("javascript", "const a = []; while (true) a.push('x'.repeat(1 << 20))"),
    ("javascript", "'x'.repeat(2 ** 31)"),
    ("javascript", "new Array(2 ** 32 - 1).fill(0).join(',')"),
    ("javascript", "JSON.stringify(Array.from({ length: 1e7 }, (_, i) => ({ i })))"),
    ("rhai", "fn f(n) { f(n + 1) } f(0)"),
    ("rhai", "let s = \"x\"; loop { s += s; }"),
    ("rhai", "loop {}"),
];

fn call(pic: &PocketIc, id: Principal, language: &str, code: &str) -> Result<(u16, Value), String> {
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": "execute_icp_script", "arguments": { "language": language, "code": code } } });
    let req = HttpUpdateRequest {
        url: "/mcp".into(),
        method: "POST".into(),
        body: serde_json::to_vec(&body).unwrap(),
        headers: vec![
            ("content-type".into(), "application/json".into()),
            ("authorization".into(), format!("Bearer {TOKEN}")),
        ],
    };
    let reply = pic
        .update_call(id, Principal::anonymous(), "http_request_update", encode_one(req).unwrap())
        .map_err(|e| format!("rejected (trap?): {e:?}"))?;
    let r: HttpResponse = decode_one(&reply).map_err(|e| e.to_string())?;
    if r.status_code == 503 {
        return Ok((503, Value::Null)); // a clean refusal (out of memory, busy); any body
    }
    let v = serde_json::from_slice(&r.body).map_err(|e| format!("status {} with a non-JSON body: {e}", r.status_code))?;
    Ok((r.status_code, v))
}

#[test]
fn corpus_never_traps() {
    let path = std::env::var("WASM").unwrap_or_else(|_| {
        concat!(env!("CARGO_MANIFEST_DIR"), "/../target/wasm32-unknown-unknown/release/icp_code_mode.wasm").into()
    });
    let wasm = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}; build the release Wasm first"));
    let pic = PocketIc::new();
    let id = pic.create_canister();
    pic.add_cycles(id, 100_000_000_000_000);
    let init = Some(InitArgs { token_sha256: vec![TOKEN_SHA256.into()], allowlist: vec![], limits: None });
    pic.install_canister(id, wasm, encode_one(init).unwrap(), None);

    let mut failures = vec![];
    for (lang, code) in CORPUS {
        match call(&pic, id, lang, code) {
            Ok((503, _)) => {}
            Ok((200, v)) if v["jsonrpc"] == "2.0" && (v.get("result").is_some() || v.get("error").is_some()) => {}
            Ok((status, v)) => failures.push(format!("{lang} {code:?}: status {status}, body {v}")),
            Err(e) => failures.push(format!("{lang} {code:?}: {e}")),
        }
    }
    // The canister still answers, and correctly.
    match call(&pic, id, "rhai", "40 + 2") {
        Ok((200, v)) if v["result"]["content"][0]["text"] == "42" => {}
        other => failures.push(format!("follow-up call: {other:?}")),
    }
    assert!(failures.is_empty(), "{} failure(s):\n{}", failures.len(), failures.join("\n"));
}
