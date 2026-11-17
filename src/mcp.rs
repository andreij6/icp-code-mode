//! MCP (protocol 2025-03-26) over JSON-RPC 2.0, served as Streamable HTTP
//! through the ICP HTTP gateway.
//!
//! `http_request` (query) answers everything that is read-only and asks the
//! gateway to upgrade anything containing `tools/call`; `http_request_update`
//! then runs the script in replicated mode where inter-canister calls work and
//! the audit log can be written.

use crate::policy;
use crate::runtimes::Language;
use crate::search::SearchArgs;
use candid::{CandidType, Deserialize};
use serde_bytes::ByteBuf;
use serde_json::{Value, json};

pub const PROTOCOL_VERSION: &str = "2025-03-26";
pub const TOOL_NAME: &str = "execute_icp_script";
pub const SEARCH_TOOL_NAME: &str = "search_icp_canisters";
/// Budget for the result text *as JSON-escaped on the wire*; logs get a quarter.
pub const MAX_RESULT_BYTES: usize = 1_000_000;
/// The IC rejects replies over 2 MiB (and traps the canister, rolling back the audit
/// entry); anything bigger than this is replaced by a small error.
pub const MAX_REPLY_BYTES: usize = 1_900_000;
/// Messages per JSON-RPC batch. Without a cap, thousands of tiny invalid messages
/// inflate the reply past the limit, and the resulting trap rolls back the audit entry.
pub const MAX_BATCH: usize = 50;

#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: ByteBuf,
    pub certificate_version: Option<u16>,
}

#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct HttpUpdateRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: ByteBuf,
}

#[derive(CandidType, Deserialize, Clone, Debug, PartialEq)]
pub struct HttpResponse {
    pub status_code: u16,
    pub headers: Vec<(String, String)>,
    pub body: ByteBuf,
    pub upgrade: Option<bool>,
}

impl HttpResponse {
    fn new(status_code: u16, body: Vec<u8>, content_type: Option<&str>) -> Self {
        let headers = content_type.map(|c| vec![("content-type".to_string(), c.to_string())]).unwrap_or_default();
        Self { status_code, headers, body: ByteBuf::from(body), upgrade: None }
    }
    fn json(status_code: u16, v: &Value) -> Self {
        Self::new(status_code, serde_json::to_vec(v).unwrap_or_default(), Some("application/json"))
    }
    fn upgrade() -> Self {
        Self { upgrade: Some(true), ..Self::new(200, vec![], None) }
    }
}

const SEARCH_DESCRIPTION: &str = r#"Find what execute_icp_script may call: the allowlisted canisters, their allowed methods with Candid signatures, and the definitions of the types those signatures use. Read-only, fast and free (no consensus round, no audit entry, does not count toward the call cap). Call this first, then write the script.

- No arguments: everything that is allowed (up to 40 methods; a note says how many more matched).
- query: space-separated keywords, all of which must appear in the method name or signature, e.g. "balance" or "transfer account".
- canister_id: restrict to one canister.

Candid to JSON when calling: records are objects, variants are {"Tag": value} (or "Tag" with no payload), opt is a value or null, principals are text, blobs are byte arrays or hex strings, and nat/int are numbers or decimal strings."#;

const TOOL_DESCRIPTION: &str = r#"Run a short script inside the canister's deterministic Wasm sandbox. Use it to batch inter-canister calls, filter/transform data, and return ONE distilled JSON result. Use search_icp_canisters first to find canister ids, allowed methods and signatures. Policy (canister/method allowlist, call cap, instruction budget) is enforced on-chain; every run is audited.

Injected `icp` SDK (identical semantics in every language):
- icp.candid(canister_id) -> string        the target's .did interface text
- icp.call(canister_id, method, args) -> value   args: JSON-like value; a list for multi-arg methods. nat/int come back as numbers when JS-safe, else decimal strings; principals as text; opt as value|null; variants as {"Tag": value}.
- icp.principal() -> string                this canister's principal
- print / console.log output is captured and returned alongside the result.

JavaScript (default): script body runs inside an async function; `return` the result.
  const [a, b] = await Promise.all([
    icp.call("ryjl3-tyaaa-aaaaa-aaaba-cai", "icrc1_balance_of", {owner: "aaaaa-aa"}),
    icp.call("ryjl3-tyaaa-aaaaa-aaaba-cai", "icrc1_decimals", null)]);
  return {balance: a, decimals: b};

Python: icp.call looks synchronous; `await asyncio.gather(icp.call_async(...), ...)` batches. The value of the last expression is the result.
  did = icp.candid("ryjl3-tyaaa-aaaaa-aaaba-cai")
  bal = icp.call("ryjl3-tyaaa-aaaaa-aaaba-cai", "icrc1_balance_of", {"owner": "aaaaa-aa"})
  print("methods:", did.count("->"))
  {"balance": bal}

Rhai: module syntax (`call` is a reserved word, so it is icp::call_canister); the last expression is the result.
  let bal = icp::call_canister("ryjl3-tyaaa-aaaaa-aaaba-cai", "icrc1_balance_of", #{owner: "aaaaa-aa"});
  #{balance: bal, me: icp::principal()}

No network, filesystem, clock, or randomness (Date/time are fixed to the block time; Math.random is removed)."#;

pub fn tools_list() -> Value {
    json!({ "tools": [{
        "name": SEARCH_TOOL_NAME,
        "description": SEARCH_DESCRIPTION,
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Keywords to match against method names and signatures." },
                "canister_id": { "type": "string", "description": "Only this canister (principal text)." }
            },
            "additionalProperties": false
        },
        "annotations": { "readOnlyHint": true, "idempotentHint": true, "openWorldHint": false }
    }, {
        "name": TOOL_NAME,
        "description": TOOL_DESCRIPTION,
        "inputSchema": {
            "type": "object",
            "properties": {
                "code": { "type": "string", "description": "Source code to run inside the sandbox." },
                "language": {
                    "type": "string",
                    "enum": ["javascript", "python", "rhai"],
                    "default": "javascript",
                    "description": "Runtime language."
                }
            },
            "required": ["code"],
            "additionalProperties": false
        }
    }]})
}

fn initialize_result() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "icp-code-mode", "version": env!("CARGO_PKG_VERSION") },
        "instructions": "Call search_icp_canisters to see which canisters and methods are allowed, then use execute_icp_script to query them with a single script instead of many tool calls."
    })
}

fn rpc_error(id: Value, code: i64, message: impl Into<String>) -> Value {
    let mut message = message.into(); // may echo client input (method, tool, language names)
    crate::bridge::truncate(&mut message, 300);
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn rpc_ok(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

/// Build an MCP `CallToolResult` from a script outcome plus captured output.
pub fn tool_result(outcome: Result<Value, String>, logs: Vec<String>) -> Value {
    let (mut text, is_error) = match outcome {
        Ok(Value::String(s)) => (s, false),
        Ok(v) => (serde_json::to_string(&v).unwrap_or_default(), false),
        Err(e) => (e, true), // engines already prefix their error type
    };
    truncate_escaped(&mut text, MAX_RESULT_BYTES);
    let mut content = vec![json!({ "type": "text", "text": text })];
    if !logs.is_empty() {
        let mut out = logs.join("\n");
        truncate_escaped(&mut out, MAX_RESULT_BYTES / 4);
        content.push(json!({ "type": "text", "text": format!("[console output]\n{out}") }));
    }
    json!({ "content": content, "isError": is_error })
}

/// Truncate so the *JSON-escaped* form fits in `max` bytes (control characters
/// expand to `\u00XX`, six bytes each).
fn truncate_escaped(s: &mut String, max: usize) {
    let mut used = 0;
    for (i, c) in s.char_indices() {
        used += match c {
            '"' | '\\' | '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2,
            c if (c as u32) < 0x20 => 6,
            c => c.len_utf8(),
        };
        if used > max {
            s.truncate(i);
            s.push_str("…[truncated]");
            return;
        }
    }
}

/// A validated `execute_icp_script` call.
pub struct ToolCall {
    pub language: Language,
    pub code: String,
}

/// A validated `tools/call`.
pub enum ToolRequest {
    Execute(ToolCall),
    Search(SearchArgs),
}

fn opt_string(args: &Value, key: &str) -> Result<Option<String>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(format!("`{key}` must be a string")),
    }
}

fn parse_tool_call(params: &Value) -> Result<ToolRequest, String> {
    let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
    let args = params.get("arguments").cloned().unwrap_or(Value::Null);
    match name {
        TOOL_NAME => {
            let code = args.get("code").and_then(Value::as_str).ok_or("missing string argument `code`")?;
            let language = match opt_string(&args, "language")? {
                None => Language::JavaScript,
                Some(s) => s.parse()?,
            };
            Ok(ToolRequest::Execute(ToolCall { language, code: code.to_string() }))
        }
        SEARCH_TOOL_NAME => Ok(ToolRequest::Search(SearchArgs {
            query: opt_string(&args, "query")?,
            canister_id: opt_string(&args, "canister_id")?,
        })),
        _ => Err(format!("unknown tool `{name}`")),
    }
}

enum Msg {
    /// Already answered (or nothing to answer for a notification).
    Done(Option<Value>),
    ToolCall(Value, Result<ToolRequest, String>),
}

fn classify(msg: &Value) -> Msg {
    let id = msg.get("id").cloned();
    let method = msg.get("method").and_then(Value::as_str);
    // MCP: request ids are strings or numbers, never null.
    let valid = msg.get("jsonrpc").and_then(Value::as_str) == Some("2.0")
        && method.is_some()
        && id.as_ref().is_none_or(|i| matches!(i, Value::String(_) | Value::Number(_)));
    if !valid {
        // Responses from the client (e.g. to server requests) carry no method: ignore them.
        if msg.get("result").is_some() || msg.get("error").is_some() {
            return Msg::Done(None);
        }
        // Echo the id only if it is a valid one (JSON-RPC: null when undeterminable).
        let id = id.filter(|i| i.is_string() || i.is_number()).unwrap_or(Value::Null);
        return Msg::Done(Some(rpc_error(id, -32600, "invalid JSON-RPC 2.0 request")));
    }
    let Some(id) = id else {
        return Msg::Done(None); // notification, e.g. notifications/initialized
    };
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    Msg::Done(Some(match method.unwrap() {
        "initialize" => rpc_ok(id, initialize_result()),
        "ping" => rpc_ok(id, json!({})),
        "tools/list" => rpc_ok(id, tools_list()),
        "tools/call" => return Msg::ToolCall(id, parse_tool_call(&params)),
        m => rpc_error(id, -32601, format!("method not found: {m}")),
    }))
}

/// Check method/path/bearer token. Returns the token's SHA-256 on success.
pub fn authenticate(method: &str, url: &str, headers: &[(String, String)]) -> Result<String, HttpResponse> {
    let path = url.split(['?', '#']).next().unwrap_or("");
    if path.trim_end_matches('/') != "/mcp" {
        return Err(HttpResponse::json(404, &json!({ "error": "not found; the MCP endpoint is POST /mcp" })));
    }
    if !method.eq_ignore_ascii_case("POST") {
        let mut r = HttpResponse::json(405, &json!({ "error": "only POST is supported (no SSE stream)" }));
        r.headers.push(("allow".into(), "POST".into()));
        return Err(r);
    }
    let token = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("authorization"))
        .and_then(|(_, v)| v.split_once(' '))
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
        .map(|(_, t)| t.trim());
    match token.and_then(policy::verify_token) {
        Some(hash) => Ok(hash),
        None => {
            let mut r = HttpResponse::json(401, &json!({ "error": "missing or invalid bearer token" }));
            r.headers.push(("www-authenticate".into(), "Bearer".into()));
            Err(r)
        }
    }
}

/// Parse the body into individual messages; `bool` = it was a batch.
fn parse_body(body: &[u8]) -> Result<(Vec<Value>, bool), HttpResponse> {
    match serde_json::from_slice::<Value>(body) {
        Ok(Value::Array(a)) if a.len() > MAX_BATCH => Err(HttpResponse::json(
            400,
            &rpc_error(Value::Null, -32600, format!("batch of {} messages exceeds the limit of {MAX_BATCH}", a.len())),
        )),
        Ok(Value::Array(a)) if !a.is_empty() => Ok((a, true)),
        Ok(Value::Array(_)) => Err(HttpResponse::json(400, &rpc_error(Value::Null, -32600, "empty batch"))),
        Ok(v) => Ok((vec![v], false)),
        Err(e) => Err(HttpResponse::json(400, &rpc_error(Value::Null, -32700, format!("parse error: {e}")))),
    }
}

fn finish(replies: Vec<Value>, batch: bool) -> HttpResponse {
    let r = match (replies.len(), batch) {
        (0, _) => HttpResponse::new(202, vec![], None), // only notifications
        (_, true) => HttpResponse::json(200, &Value::Array(replies)),
        _ => HttpResponse::json(200, &replies[0]),
    };
    // Last line of defence: the reply must always fit, whatever it echoes (e.g. huge ids),
    // because a trap here would discard the audit entry of a script that already ran.
    if r.body.len() > MAX_REPLY_BYTES {
        return HttpResponse::json(200, &rpc_error(Value::Null, -32603, format!("reply exceeded {MAX_REPLY_BYTES} bytes")));
    }
    r
}

/// Demo UI (no secrets inside; it asks for the token and calls `POST /mcp`).
const DEMO_HTML: &str = include_str!("demo.html");

fn demo_page(method: &str, url: &str) -> Option<HttpResponse> {
    let path = url.split(['?', '#']).next().unwrap_or("");
    (method.eq_ignore_ascii_case("GET") && matches!(path, "/" | "/demo")).then(|| {
        let mut r = HttpResponse::new(200, DEMO_HTML.as_bytes().to_vec(), Some("text/html; charset=utf-8"));
        r.headers.push(("cache-control".into(), "no-cache".into()));
        r
    })
}

/// Query path: answer read-only methods. `search(args)` answers a search from cached
/// state, or returns `None` when an interface must be fetched first. Anything that
/// needs an update (scripts, such searches) upgrades the whole request.
///
/// Query responses are not certified, so a single malicious node could alter them.
/// With `certified` (policy `certified_reads`), every authenticated request is upgraded
/// to an update call instead, and the demo page (uncertified HTML that asks for the
/// token) is switched off.
pub fn http_query(req: &HttpRequest, certified: bool, search: impl Fn(&SearchArgs) -> Option<Value>) -> HttpResponse {
    if let Some(page) = demo_page(&req.method, &req.url) {
        if certified {
            return HttpResponse::json(404, &json!({ "error": "the demo page is disabled while certified_reads is on" }));
        }
        return page;
    }
    if let Err(r) = authenticate(&req.method, &req.url, &req.headers) {
        return r;
    }
    if certified {
        return HttpResponse::upgrade();
    }
    let (msgs, batch) = match parse_body(&req.body) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let mut replies = vec![];
    for m in &msgs {
        match classify(m) {
            Msg::Done(r) => replies.extend(r),
            Msg::ToolCall(id, Err(e)) => replies.push(rpc_error(id, -32602, e)),
            Msg::ToolCall(id, Ok(ToolRequest::Search(a))) => match search(&a) {
                Some(v) => replies.push(rpc_ok(id, tool_result(Ok(v), vec![]))),
                None => return HttpResponse::upgrade(),
            },
            Msg::ToolCall(_, Ok(ToolRequest::Execute(_))) => return HttpResponse::upgrade(),
        }
    }
    finish(replies, batch)
}

/// Update path: everything, including tool execution via `exec(caller_hash, request)`,
/// which must return a `CallToolResult` value.
pub async fn http_update(req: &HttpUpdateRequest, exec: impl AsyncFn(&str, ToolRequest) -> Value) -> HttpResponse {
    let caller = match authenticate(&req.method, &req.url, &req.headers) {
        Ok(h) => h,
        Err(r) => return r,
    };
    let (msgs, batch) = match parse_body(&req.body) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let mut replies = vec![];
    let mut ran_script = false;
    for m in &msgs {
        match classify(m) {
            Msg::Done(r) => replies.extend(r),
            Msg::ToolCall(id, Err(e)) => replies.push(rpc_error(id, -32602, e)),
            // One script per request: each gets its own message budget, and the reply
            // size stays bounded. Searches are cheap and size-capped, so they are exempt.
            Msg::ToolCall(id, Ok(ToolRequest::Execute(_))) if ran_script => {
                replies.push(rpc_error(id, -32600, "only one execute_icp_script call per HTTP request"))
            }
            Msg::ToolCall(id, Ok(req)) => {
                ran_script |= matches!(req, ToolRequest::Execute(_));
                replies.push(rpc_ok(id, exec(&caller, req).await));
            }
        }
    }
    let size: usize = replies.iter().map(|r| serde_json::to_vec(r).map_or(0, |v| v.len())).sum();
    if size > MAX_REPLY_BYTES {
        for r in replies.iter_mut().filter(|r| r.pointer("/result/content").is_some()) {
            r["result"] = tool_result(Err(format!("reply exceeded {MAX_REPLY_BYTES} bytes")), vec![]);
        }
    }
    finish(replies, batch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::mock::MockBridge;
    use crate::runtimes::LanguageRouter;
    use futures::executor::block_on;

    const TOKEN: &str = "test-token";

    fn setup() {
        policy::add_token_hash(&policy::sha256_hex(TOKEN.as_bytes()), "test".into()).unwrap();
    }

    fn headers(token: &str) -> Vec<(String, String)> {
        vec![
            ("Content-Type".into(), "application/json".into()),
            ("Authorization".into(), format!("Bearer {token}")),
        ]
    }

    fn query(body: Value) -> HttpResponse {
        http_query(
            &HttpRequest {
                method: "POST".into(),
                url: "/mcp".into(),
                headers: headers(TOKEN),
                body: ByteBuf::from(serde_json::to_vec(&body).unwrap()),
                certificate_version: Some(2),
            },
            false,
            mock_search,
        )
    }

    fn update(body: Value) -> HttpResponse {
        let req = HttpUpdateRequest {
            method: "POST".into(),
            url: "/mcp".into(),
            headers: headers(TOKEN),
            body: ByteBuf::from(serde_json::to_vec(&body).unwrap()),
        };
        block_on(http_update(&req, async |_caller: &str, req: ToolRequest| match req {
            ToolRequest::Execute(call) => {
                let bridge = MockBridge::default();
                let out = LanguageRouter::execute(call.language, &call.code, &bridge).await;
                tool_result(out, bridge.logs.take())
            }
            ToolRequest::Search(a) => tool_result(Ok(json!({ "searched_in_update": a.query })), vec![]),
        }))
    }

    /// Pretend the interface for "ledger" queries is cached and anything else is not.
    fn mock_search(a: &SearchArgs) -> Option<Value> {
        (a.query.as_deref() != Some("uncached")).then(|| json!({ "canisters": [], "q": a.query }))
    }

    fn body(r: &HttpResponse) -> Value {
        serde_json::from_slice(&r.body).unwrap()
    }

    #[test]
    fn auth_and_routing() {
        setup();
        let mk = |method: &str, url: &str, token: &str| HttpRequest {
            method: method.into(),
            url: url.into(),
            headers: headers(token),
            body: ByteBuf::from(b"{}".to_vec()),
            certificate_version: None,
        };
        assert_eq!(http_query(&mk("POST", "/mcp", "wrong"), false, mock_search).status_code, 401);
        assert_eq!(http_query(&mk("POST", "/other", TOKEN), false, mock_search).status_code, 404);
        assert_eq!(http_query(&mk("GET", "/mcp", TOKEN), false, mock_search).status_code, 405);
        let r = http_query(&mk("POST", "/mcp?x=1", TOKEN), false, mock_search);
        assert_eq!(body(&r)["error"]["code"], -32600);
    }

    #[test]
    fn initialize_and_list_in_query() {
        setup();
        let r = query(json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": { "name": "t", "version": "1" } } }));
        assert_eq!(r.status_code, 200);
        let b = body(&r);
        assert_eq!(b["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(b["result"]["capabilities"], json!({ "tools": {} }));

        let r = query(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
        assert_eq!((r.status_code, r.body.len()), (202, 0));

        let b = body(&query(json!({ "jsonrpc": "2.0", "id": "a", "method": "tools/list" })));
        let tools = b["result"]["tools"].as_array().unwrap();
        let names: Vec<_> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, [SEARCH_TOOL_NAME, TOOL_NAME]);
        assert_eq!(tools[0]["annotations"]["readOnlyHint"], true);
        assert_eq!(tools[1]["inputSchema"]["required"], json!(["code"]));
        assert_eq!(tools[1]["inputSchema"]["properties"]["language"]["enum"], json!(["javascript", "python", "rhai"]));

        let b = body(&query(json!({ "jsonrpc": "2.0", "id": 2, "method": "nope" })));
        assert_eq!(b["error"]["code"], -32601);
        let r = http_query(&HttpRequest { body: ByteBuf::from(b"{oops".to_vec()), ..mk_req() }, false, mock_search);
        assert_eq!(body(&r)["error"]["code"], -32700);
    }

    fn mk_req() -> HttpRequest {
        HttpRequest { method: "POST".into(), url: "/mcp".into(), headers: headers(TOKEN), body: ByteBuf::new(), certificate_version: None }
    }

    #[test]
    fn tools_call_upgrades_from_query() {
        setup();
        let call = json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": { "name": TOOL_NAME, "arguments": { "code": "return 1" } } });
        assert_eq!(query(call.clone()).upgrade, Some(true));
        // Batches containing a tools/call upgrade as a whole.
        let batch = json!([{ "jsonrpc": "2.0", "id": 1, "method": "ping" }, call]);
        assert_eq!(query(batch).upgrade, Some(true));
    }

    #[cfg(feature = "js")]
    #[test]
    fn tools_call_executes_in_update() {
        setup();
        let b = body(&update(json!({ "jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {
            "name": TOOL_NAME, "arguments": { "language": "javascript", "code":
            "const r = await icp.call('ryjl3-tyaaa-aaaaa-aaaba-cai', 'icrc1_balance_of', {owner: 'aaaaa-aa'}); console.log('got', r.balance); return {b: r.balance};" } } })));
        assert_eq!(b["id"], 7);
        assert_eq!(b["result"]["isError"], false);
        assert_eq!(b["result"]["content"][0], json!({ "type": "text", "text": "{\"b\":100}" }));
        assert_eq!(b["result"]["content"][1]["text"], "[console output]\ngot 100");

        // Script errors are tool errors (isError), not protocol errors.
        let b = body(&update(json!({ "jsonrpc": "2.0", "id": 8, "method": "tools/call", "params": {
            "name": TOOL_NAME, "arguments": { "code": "await icp.call('aaaaa-aa', 'raw_rand', null)" } } })));
        assert_eq!(b["result"]["isError"], true);
        assert!(b["result"]["content"][0]["text"].as_str().unwrap().contains("not allowlisted"));
    }

    #[test]
    fn tools_call_bad_params_are_protocol_errors() {
        setup();
        let call = |args: Value, name: &str| body(&update(json!({ "jsonrpc": "2.0", "id": 9, "method": "tools/call",
            "params": { "name": name, "arguments": args } })));
        assert_eq!(call(json!({ "code": "1" }), "other")["error"]["code"], -32602);
        assert_eq!(call(json!({}), TOOL_NAME)["error"]["code"], -32602);
        assert_eq!(call(json!({ "code": "1", "language": "cobol" }), TOOL_NAME)["error"]["code"], -32602);
    }

    #[test]
    fn batch_in_update() {
        setup();
        let r = update(json!([
            { "jsonrpc": "2.0", "id": 1, "method": "ping" },
            { "jsonrpc": "2.0", "method": "notifications/initialized" },
            { "jsonrpc": "2.0", "id": 2, "method": "tools/list" }
        ]));
        let b = body(&r);
        assert_eq!(b.as_array().unwrap().len(), 2);
        assert_eq!(b[0]["result"], json!({}));
    }

    #[test]
    fn search_is_answered_in_query_unless_a_fetch_is_needed() {
        setup();
        let search = |q: &str| json!({ "jsonrpc": "2.0", "id": 5, "method": "tools/call",
            "params": { "name": SEARCH_TOOL_NAME, "arguments": { "query": q } } });
        let r = query(search("balance"));
        assert_eq!(r.upgrade, None);
        let b = body(&r);
        assert_eq!(b["result"]["isError"], false);
        assert_eq!(b["result"]["content"][0]["text"], "{\"canisters\":[],\"q\":\"balance\"}");
        assert_eq!(query(search("uncached")).upgrade, Some(true));
        // In update it goes to exec.
        let b = body(&update(search("uncached")));
        assert_eq!(b["result"]["content"][0]["text"], "{\"searched_in_update\":\"uncached\"}");
        // Bad argument types are protocol errors, answered without an upgrade.
        let bad = json!({ "jsonrpc": "2.0", "id": 6, "method": "tools/call",
            "params": { "name": SEARCH_TOOL_NAME, "arguments": { "query": 7 } } });
        let r = query(bad);
        assert_eq!((r.upgrade, body(&r)["error"]["code"].clone()), (None, json!(-32602)));
    }

    #[test]
    fn demo_page_is_served_without_auth() {
        let get = |url: &str| http_query(&HttpRequest { method: "GET".into(), url: url.into(), headers: vec![], ..mk_req() }, false, mock_search);
        let r = get("/");
        assert_eq!(r.status_code, 200);
        assert!(r.headers.contains(&("content-type".into(), "text/html; charset=utf-8".into())));
        assert!(std::str::from_utf8(&r.body).unwrap().contains("search_icp_canisters"));
        assert_eq!(get("/demo?x").status_code, 200);
        assert_eq!(get("/mcp").status_code, 405); // MCP endpoint itself is unchanged
    }

    /// Reported audit-suppression vector: pad a batch so the reply exceeds the limit and the
    /// canister traps while replying, which rolls back the script's audit entry.
    #[test]
    fn replies_always_fit() {
        setup();
        // 30,000 tiny invalid messages: refused outright, in both paths.
        let padded = Value::Array(vec![json!({}); 30_000]);
        let (q, u) = (query(padded.clone()), update(padded));
        for r in [&q, &u] {
            assert_eq!(r.status_code, 400);
            assert!(body(r)["error"]["message"].as_str().unwrap().contains("exceeds the limit of 50"));
        }
        // Inside the cap, replies that echo client input still cannot exceed the limit.
        let big_id = "i".repeat(MAX_REPLY_BYTES);
        let r = update(json!({ "jsonrpc": "2.0", "id": big_id, "method": "ping" }));
        assert!(r.body.len() < 1000);
        assert_eq!(body(&r)["error"]["code"], -32603);
        let r = update(json!({ "jsonrpc": "2.0", "id": 1, "method": "m".repeat(500_000) }));
        assert!(r.body.len() < 1000, "error messages do not echo unbounded input");
    }

    #[test]
    fn certified_mode_upgrades_everything_and_disables_the_demo_page() {
        setup();
        let post = |token: &str| HttpRequest {
            body: ByteBuf::from(br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#.to_vec()),
            headers: headers(token),
            ..mk_req()
        };
        assert_eq!(http_query(&post(TOKEN), true, mock_search).upgrade, Some(true));
        assert_eq!(http_query(&post(TOKEN), false, mock_search).upgrade, None);
        // Bad tokens are still refused in the query, so they never reach an update call.
        assert_eq!(http_query(&post("wrong"), true, mock_search).status_code, 401);
        let page = HttpRequest { method: "GET".into(), url: "/".into(), headers: vec![], ..mk_req() };
        assert_eq!(http_query(&page, true, mock_search).status_code, 404);
        assert_eq!(http_query(&page, false, mock_search).status_code, 200);
    }

    #[test]
    fn ids_must_be_string_or_number() {
        setup();
        let b = body(&query(json!({ "jsonrpc": "2.0", "id": null, "method": "ping" })));
        assert_eq!((b["id"].clone(), b["error"]["code"].clone()), (Value::Null, json!(-32600)));
        let b = body(&query(json!({ "jsonrpc": "2.0", "id": { "a": 1 }, "method": "ping" })));
        assert_eq!(b["id"], Value::Null);
    }

    #[test]
    fn result_size_is_bounded_after_escaping() {
        let r = tool_result(Ok(Value::String("\u{1}".repeat(900_000))), vec!["\u{1}".repeat(900_000)]);
        let wire = serde_json::to_vec(&r).unwrap().len();
        assert!(wire < MAX_REPLY_BYTES, "{wire}");
        assert!(r["content"][0]["text"].as_str().unwrap().ends_with("[truncated]"));
    }

    #[cfg(feature = "rhai")]
    #[test]
    fn one_tool_call_per_request() {
        setup();
        let call = |id: u32| json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": { "name": TOOL_NAME, "arguments": { "language": "rhai", "code": "40 + 2" } } });
        let search = |id: u32| json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": { "name": SEARCH_TOOL_NAME, "arguments": {} } });
        let b = body(&update(json!([search(0), call(1), search(2), call(3)])));
        assert!(b[0]["result"].is_object(), "search before the script still runs");
        assert_eq!(b[1]["result"]["content"][0]["text"], "42");
        assert!(b[2]["result"].is_object(), "search after the script still runs");
        assert_eq!(b[3]["error"]["code"], -32600);
    }
}
