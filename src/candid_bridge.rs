//! Candid interface parsing plus type-directed JSON <-> Candid conversion.
//!
//! Scripts speak JSON; target canisters speak Candid. The target's `.did` is the
//! source of truth: arguments are coerced against the method's declared argument
//! types, and replies are decoded against its declared return types so record
//! and variant fields come back with their real names.

use crate::bridge::{MAX_METHOD_LEN, ValueBudget, float_json as f64_json, truncate};
use candid::types::value::{IDLField, VariantValue};
use candid::types::{Field, FuncMode, Function, Label, Type, TypeInner};
use candid::{IDLArgs, IDLValue, Int, Nat, Principal, TypeEnv};
use serde_json::{Map, Number, Value};
use std::collections::BTreeMap;

/// JSON integers beyond this magnitude are emitted as strings (JS `Number` safety).
pub const MAX_SAFE_INT: u64 = (1 << 53) - 1;
/// Candid decoding work budget per reply (see `DecoderConfig::set_decoding_quota`). Candid
/// charges 50x per byte when decoding to `IDLValue`, so this admits a full 2 MiB reply.
const DECODING_QUOTA: usize = 50 * (2 << 20) + 5_000_000;
/// Deepest JSON nesting in either direction (`opt` adds no level). Runtimes accept values
/// at least this deep (see `runtimes::js::MAX_VALUE_DEPTH`). Candid's own recursion while
/// decoding is bounded separately, in the vendored crate (vendor/candid).
pub const MAX_DEPTH: usize = 128;

/// Deepest type nesting accepted in one definition. The parser and type checker recurse
/// once per level, and about 5,000 levels overflow the 8 MiB Wasm stack: a trap on every
/// fetch of a hostile interface.
pub const MAX_DID_NESTING: usize = 256;
/// Bound on the nesting summed over all definitions, each counting at least 1 (so it also
/// bounds their number). Candid's encoder recurses through named types (`type A1 = opt A2;
/// type A2 = ...`), once per name, and a chain of 20,000 overflows the stack. A path
/// visits each name once, so this sum bounds its depth.
pub const MAX_DID_NESTING_SUM: usize = 2_000;
/// Most fields in one record or variant (or methods in one service). Candid scans a type's
/// fields for every value it encodes or decodes, and a JavaScript object with thousands of
/// hash-colliding keys (from a hostile interface) is quadratic to build.
pub const MAX_FIELDS: usize = 1_024;
/// Longest chain of plain aliases (`type A = B;`). candid_parser's cycle check walks the
/// chain from every definition: quadratic, and 24,000 aliases took 93 s natively.
pub const MAX_ALIAS_CHAIN: usize = 64;

/// Checks on the interface text that must run before candid_parser sees it, from its
/// tokens: nesting (each `vec`/`opt` prefix and each open bracket is a level), fields per
/// brace, record field ids (the parser computes `id + 1` unchecked for records, which overflows past
/// `u32::MAX`), and alias chains. Lexical errors end the scan; the parser reports them.
fn check_shape(did: &str) -> Result<(), String> {
    use candid_parser::token::{Token, Tokenizer};
    /// One open bracket.
    #[derive(Default)]
    struct Open {
        level: usize,
        brace: bool,
        // opened by `record`: only records number their fields (`id + 1`); variants sort
        record: bool,
        fields: usize,
        next_id: u64,
        // the current field: tokens so far, and whether it starts with `label :`
        toks: usize,
        labeled: bool,
    }
    fn end_field(f: &mut Open) -> Result<(), String> {
        if f.brace && f.toks > 0 {
            let id = if f.labeled { f.next_id - 1 } else { f.next_id };
            f.next_id = id + 1;
            f.fields += 1;
            if f.fields > MAX_FIELDS {
                return Err(format!("more than {MAX_FIELDS} fields in one record, variant or service"));
            }
            if f.record && id >= u64::from(u32::MAX) {
                return Err(format!("field id {id} is out of range"));
            }
        }
        (f.toks, f.labeled) = (0, false);
        Ok(())
    }
    let mut open = vec![Open::default()];
    let (mut run, mut stmt, mut sum) = (0usize, 0usize, 0usize);
    let mut prev = Token::Semi;
    // The current top-level definition: its first tokens and its length, to spot aliases.
    let (mut def, mut def_len) = (Vec::new(), 0usize);
    let mut aliases = BTreeMap::new();
    let tokens = Tokenizer::new(did).map_while(|t| t.ok().map(|(_, t, _)| t));
    // The trailing `;` ends the last definition.
    for tok in tokens.chain([Token::Semi]) {
        let top = open.len() == 1;
        if !matches!(tok, Token::Semi | Token::RParen | Token::RBrace) {
            open.last_mut().unwrap().toks += 1;
        }
        let base = open.last().unwrap().level;
        match &tok {
            // A top-level `;` or `type` ends a definition.
            Token::Semi | Token::Type if top => {
                if def_len > 0 {
                    sum += stmt.max(1);
                }
                if let (4, [Token::Type, Token::Id(a), Token::Equals, Token::Id(b)]) = (def_len, def.as_slice()) {
                    aliases.insert(a.clone(), b.clone());
                }
                (stmt, run, def_len) = (0, 0, 0);
                def.clear();
            }
            Token::Semi => {
                end_field(open.last_mut().unwrap())?;
                run = 0;
            }
            Token::Vec | Token::Opt => run += 1,
            // keywords that come before the bracket they open
            Token::Record | Token::Variant | Token::Func | Token::Service | Token::Arrow => {}
            Token::LParen | Token::LBrace => {
                open.push(Open { level: base + run + 1, brace: tok == Token::LBrace, record: prev == Token::Record, ..Default::default() });
                run = 0;
            }
            Token::RParen | Token::RBrace => {
                if open.len() > 1 {
                    end_field(open.last_mut().unwrap())?;
                    open.pop();
                }
                run = 0;
            }
            Token::Colon => {
                let f = open.last_mut().unwrap();
                let label = match &prev {
                    Token::Id(s) | Token::Text(s) => Some(candid::idl_hash(s)),
                    Token::Decimal(s) => s.parse().ok(),
                    Token::Hex(s) => u32::from_str_radix(s, 16).ok(),
                    _ => None,
                };
                if let (true, 2, Some(id)) = (f.brace, f.toks, label) {
                    (f.labeled, f.next_id) = (true, u64::from(id) + 1);
                }
                run = 0;
            }
            _ => run = 0,
        }
        if !(top && tok == Token::Semi) {
            def_len += 1;
            if def.len() < 4 {
                def.push(tok.clone());
            }
        }
        stmt = stmt.max(open.last().unwrap().level + run);
        if stmt > MAX_DID_NESTING {
            return Err(format!("types nested {stmt} levels deep (limit {MAX_DID_NESTING})"));
        }
        if sum > MAX_DID_NESTING_SUM {
            return Err(format!("type definitions nested {sum} levels in total (limit {MAX_DID_NESTING_SUM})"));
        }
        prev = tok;
    }
    for start in aliases.keys() {
        let (mut cur, mut n) = (start, 0);
        while let Some(next) = aliases.get(cur) {
            n += 1;
            if n > MAX_ALIAS_CHAIN {
                return Err(format!("type `{start}` is an alias chain longer than {MAX_ALIAS_CHAIN}"));
            }
            cur = next;
        }
    }
    Ok(())
}

/// A parsed `.did` service.
pub struct Service {
    pub env: TypeEnv,
    pub actor: Type,
    /// Per method: its doc comments and argument/result names, which the types alone lose.
    /// Search matches against them.
    pub docs: BTreeMap<String, String>,
}

/// Argument bytes plus everything needed to decode the reply.
pub struct EncodedCall {
    pub bytes: Vec<u8>,
    pub func: Function,
}

/// Why a reply could not be read.
#[derive(Debug)]
pub struct ReplyError {
    pub msg: String,
    /// The reply did not fit our copy of the target's interface, which has probably
    /// changed since it was fetched. Otherwise it broke a size or depth limit.
    pub stale: bool,
}

impl Service {
    pub fn parse(did: &str) -> Result<Self, String> {
        // The parser's error quotes the offending text, which can be most of the interface.
        let invalid = |e: String| {
            let mut m = format!("invalid candid interface: {e}");
            truncate(&mut m, 300);
            m
        };
        check_shape(did).map_err(invalid)?;
        let prog: candid_parser::IDLProg = did.parse().map_err(|e: candid_parser::Error| invalid(e.to_string()))?;
        // check_prog skips imports: refuse them rather than silently miss their types.
        if prog.decs.iter().any(|d| !matches!(d, candid_parser::syntax::Dec::TypD(_))) {
            return Err(invalid("import declarations are not supported".into()));
        }
        let mut env = TypeEnv::new();
        let actor = candid_parser::check_prog(&mut env, &prog).map_err(|e| invalid(e.to_string()))?;
        let actor = actor.ok_or("candid interface declares no service")?;
        // `service : (init) -> { .. }` is a class; we only call methods.
        let actor = match actor.as_ref() {
            TypeInner::Class(_, t) => t.clone(),
            _ => actor,
        };
        Ok(Self { env, actor, docs: method_docs(&prog) })
    }

    pub fn method<'a>(&'a self, name: &'a str) -> Result<&'a Function, String> {
        self.env
            .get_method(&self.actor, name)
            .map_err(|_| format!("method `{name}` is not declared in the target's candid interface"))
    }

    /// Why `icp.call` can never call this method, if it can't. Search lists only callable ones.
    pub fn uncallable(&self, name: &str, func: &Function) -> Option<String> {
        if name.len() > MAX_METHOD_LEN {
            return Some(format!("method name longer than {MAX_METHOD_LEN} bytes"));
        }
        if func.modes.contains(&FuncMode::Oneway) {
            return Some(format!("method `{name}` is oneway; it returns no reply"));
        }
        if func.modes.contains(&FuncMode::CompositeQuery) {
            return Some(format!("`{name}` is a composite_query; it cannot be called from an update"));
        }
        let empty = |t: &Type| matches!(self.env.trace_type(t).as_deref(), Ok(TypeInner::Empty));
        func.args.iter().any(empty).then(|| format!("method `{name}` takes an argument of type `empty`, which has no values"))
    }

    /// Coerce `args` into the method's argument list and serialize to Candid bytes.
    ///
    /// Accepted shapes: `null` (zero args), a JSON array (positional args), or — for
    /// single-argument methods — the bare argument value. When that argument is itself an
    /// array (a vec or tuple), an array is first read as the argument, and `[x]` is a
    /// positional wrapper only when that reading fails: so a decoded reply passed back
    /// encodes to the same bytes.
    pub fn encode_args(&self, method: &str, args: &Value) -> Result<EncodedCall, String> {
        let func = self.method(method)?.clone();
        if let Some(e) = self.uncallable(method, &func) {
            return Err(e);
        }
        let n = func.args.len();
        let arg0 = |v: &Value| json_to_idl(&self.env, &func.args[0], v, 0).map_err(|e| format!("argument 0: {e}"));
        let values = match args {
            Value::Array(a) if n == 1 && is_array_like(&self.env, &func.args[0]) => match arg0(args) {
                Ok(v) => vec![v],
                // The wrapper reading's error only when it was plausible (`[[...]]`).
                Err(e) if a.len() == 1 => vec![arg0(&a[0]).map_err(|w| if a[0].is_array() { w } else { e })?],
                Err(e) => return Err(e),
            },
            Value::Array(a) if n == 1 && a.len() == 1 => vec![arg0(&a[0])?],
            Value::Null => self.positional(method, &func, &[])?,
            Value::Object(o) if n == 0 && o.is_empty() => vec![],
            Value::Array(a) if n != 1 => self.positional(method, &func, a)?,
            v if n == 1 => vec![arg0(v)?],
            _ => return Err(format!("method `{method}` takes {n} arguments; pass them as a JSON array")),
        };
        let bytes = IDLArgs::new(&values)
            .to_bytes_with_types(&self.env, &func.args)
            .map_err(|e| format!("candid encoding failed: {e}"))?;
        Ok(EncodedCall { bytes, func })
    }

    fn positional(&self, method: &str, func: &Function, args: &[Value]) -> Result<Vec<IDLValue>, String> {
        if args.len() > func.args.len() {
            return Err(format!("method `{method}` takes {} arguments, got {}", func.args.len(), args.len()));
        }
        let missing = std::iter::repeat(&Value::Null);
        func.args
            .iter()
            .zip(args.iter().chain(missing))
            .enumerate()
            .map(|(i, (ty, v))| json_to_idl(&self.env, ty, v, 0).map_err(|e| format!("argument {i}: {e}")))
            .collect()
    }

    /// Decode reply bytes against the method's return types. Single returns are
    /// unwrapped; zero returns become `null`; several become an array.
    pub fn decode_reply(&self, func: &Function, bytes: &[u8]) -> Result<Value, ReplyError> {
        // Quotas bound the work a hostile reply can force on us.
        let mut cfg = candid::DecoderConfig::new();
        // Skipping covers reply fields newer than our copy of the interface.
        cfg.set_decoding_quota(DECODING_QUOTA)
            .set_skipping_quota(DECODING_QUOTA)
            .set_full_error_message(false); // otherwise errors embed the whole reply in hex
        let decoded = IDLArgs::from_bytes_with_types_with_config(bytes, &self.env, &func.rets, &cfg)
            // `:#` shows the cause ("Decoding cost exceeds the limit"), not only the context.
            .map_err(|e| format!("candid decoding failed: {e:#}"))
            .and_then(|args| self.check_reply_types(func, bytes).map(|()| args));
        let args = decoded.map_err(|msg| {
            let limit = ["cost exceeds", "Recursion limit"].iter().any(|s| msg.contains(s));
            ReplyError { msg, stale: !limit }
        })?;
        let budget = &mut ValueBudget::default();
        let mut out: Vec<Value> = args
            .args
            .iter()
            .map(|v| idl_to_json(v, 0, budget))
            .collect::<Result<_, _>>()
            .map_err(|msg| ReplyError { msg: format!("reply: {msg}"), stale: false })?;
        Ok(match out.len() {
            0 => Value::Null,
            1 => out.pop().unwrap(),
            _ => Value::Array(out),
        })
    }
}

/// Doc comments and argument/result names per method of the service in `prog`.
fn method_docs(prog: &candid_parser::IDLProg) -> BTreeMap<String, String> {
    use candid_parser::syntax::IDLType;
    let mut t = prog.actor.as_ref().map(|a| &a.typ);
    if let Some(IDLType::ClassT(_, s)) = t {
        t = Some(s);
    }
    let Some(IDLType::ServT(methods)) = t else { return BTreeMap::new() };
    let mut docs = BTreeMap::new();
    for m in methods {
        let mut text = m.docs.join("\n");
        if let IDLType::FuncT(f) = &m.typ {
            let names: Vec<_> = f.args.iter().chain(&f.rets).filter_map(|a| a.name.as_deref()).collect();
            if !names.is_empty() {
                text = format!("{text}{}names: {}", if text.is_empty() { "" } else { "\n" }, names.join(", "));
            }
        }
        if !text.is_empty() {
            docs.insert(m.id.clone(), text);
        }
    }
    docs
}

impl Service {
    /// Candid's "special opt rule" decodes an `opt` value whose type does not match ours
    /// as `null`, silently. That happens when the target was upgraded and our copy of its
    /// interface is stale, so a mismatch is an error here instead. Called after a
    /// successful decode, so the type table already passed the decoder's size limits.
    fn check_reply_types(&self, func: &Function, bytes: &[u8]) -> Result<(), String> {
        use binrw::BinRead;
        use candid::types::subtype::{Gamma, OptReport, subtype_with_config};
        let header = candid::binary_parser::Header::read_le_args(&mut std::io::Cursor::new(bytes), (None,))
            .map_err(|e| format!("candid decoding failed: {e}"))?;
        let (wire_env, wire) = header.to_types().map_err(|e| format!("candid decoding failed: {e}"))?;
        let tuple = |ts: Vec<Type>| -> Type {
            let fields = ts.into_iter().enumerate().map(|(i, ty)| Field { id: Label::Unnamed(i as u32).into(), ty });
            TypeInner::Record(fields.collect()).into()
        };
        let mut env = self.env.clone();
        let wire = env.merge_type(wire_env, tuple(wire));
        subtype_with_config(OptReport::Error, &mut Gamma::default(), &env, &wire, &tuple(func.rets.clone())).map_err(|e| {
            let mut why = format!("{e:#}");
            truncate(&mut why, 300);
            format!(
                "reply types do not match the interface ({why}); an optional value would have been dropped, \
                 so the target's interface has probably changed"
            )
        })
    }
}

/// `true` when the type is itself encoded as a JSON array (vec / tuple record),
/// which makes `[x]` ambiguous for single-argument methods.
fn is_array_like(env: &TypeEnv, ty: &Type) -> bool {
    // A loop with a bound, not recursion: `type A = opt A` is a legal type.
    let mut ty = ty.clone();
    for _ in 0..MAX_DEPTH {
        match env.trace_type(&ty).as_deref() {
            Ok(TypeInner::Vec(_)) => return true,
            Ok(TypeInner::Record(fs)) => return is_tuple(fs),
            Ok(TypeInner::Opt(inner)) => ty = inner.clone(),
            _ => return false,
        }
    }
    false
}

fn is_tuple(fields: &[Field]) -> bool {
    !fields.is_empty() && fields.iter().enumerate().all(|(i, f)| f.id.get_id() == i as u32 && !matches!(*f.id, Label::Named(_)))
}

/// `s` without digit separators, if every `_` sits between two digits (`1_000`; not `_5_`,
/// `5_` or `1__0`).
fn strip_separators(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let between_digits = |i: usize| i > 0 && i + 1 < b.len() && b[i - 1].is_ascii_digit() && b[i + 1].is_ascii_digit();
    b.iter().enumerate().all(|(i, &c)| c != b'_' || between_digits(i)).then(|| s.replace('_', ""))
}

/// A JSON key as a numeric field id: `_<id>_`, or the id in canonical decimal (also as
/// candid prints it, `1_000`). Not `05` or `+5`: one id, one spelling.
fn label_number(k: &str) -> Option<u32> {
    let d = strip_separators(k.strip_prefix('_').and_then(|k| k.strip_suffix('_')).unwrap_or(k))?;
    let canonical = !d.is_empty() && d.bytes().all(|c| c.is_ascii_digit()) && (d == "0" || !d.starts_with('0'));
    canonical.then(|| d.parse().ok()).flatten()
}

/// The JSON key for a label: unique within a record or variant, and `field_index` reads it
/// back. A name that reads as a number (`"1"`, `"_5_"`) is written as `_<its id>_`.
fn label_key(l: &Label) -> String {
    match l {
        Label::Named(s) if label_number(s).is_some() => format!("_{}_", l.get_id()),
        Label::Named(s) => s.clone(),
        Label::Id(n) | Label::Unnamed(n) => n.to_string(),
    }
}

/// The field a JSON key or variant tag names, by binary search (candid sorts fields by
/// id): a numeric key by id, anything else by name, whose hash is its id. A number that
/// is no field's id may still be a name: `record { "5" : nat }` takes key `5`.
fn field_index(fields: &[Field], key: &str) -> Option<usize> {
    let find = |id: u32| fields.binary_search_by_key(&id, |f| f.id.get_id()).ok();
    label_number(key).and_then(find).or_else(|| find(candid::idl_hash(key)))
}

fn shown(k: &str) -> String {
    let mut k = k.to_string();
    truncate(&mut k, 64);
    k
}

fn num_text(v: &Value) -> Option<String> {
    match v {
        // Integral floats (JS numbers) are accepted as integers: 3e9 -> "3000000000".
        Value::Number(n) if n.is_f64() => match n.as_f64() {
            // Beyond 2^53 a float is already rounded: refuse it rather than send a nearby
            // number. Exact big values come as decimal strings.
            Some(f) if f.abs() > MAX_SAFE_INT as f64 => Some(format!("{f} (beyond 2^53, so rounded; pass it as a decimal string)")),
            // `+ 0.0` turns JS's -0 into 0, which would otherwise print as "-0".
            Some(f) if f.fract() == 0.0 => Some(format!("{:.0}", f + 0.0)),
            _ => Some(n.to_string()),
        },
        Value::Number(n) => Some(n.to_string()),
        // Digit separators as in candid text (`1_000`); misplaced ones fail to parse.
        Value::String(s) => Some(strip_separators(s).unwrap_or_else(|| format!("{s} (misplaced digit separator)"))),
        _ => None,
    }
}

/// Largest integer in a reply converted to text (about 30,000 digits).
const MAX_INT_BITS: u64 = 100_000;
/// Longest decimal text accepted for an integer. Parsing big integers is quadratic, and
/// one parse cannot be interrupted; 1,000 digits is far beyond any amount.
const MAX_INT_DIGITS: usize = 1_000;

fn int<T: std::str::FromStr>(v: &Value, what: &str) -> Result<T, String> {
    let s = num_text(v).ok_or_else(|| format!("expected {what}, got {}", kind(v)))?;
    if s.len() > MAX_INT_DIGITS {
        return Err(format!("{what} has more than {MAX_INT_DIGITS} digits"));
    }
    s.parse::<T>().map_err(|_| format!("`{s}` is not a valid {what}"))
}

/// Infinities and NaN only from the exact strings `f64_json` writes, never from overflow.
fn finite_or_named<F>(f: F, finite: bool, v: &Value, what: &str) -> Result<F, String> {
    match (finite, v.as_str()) {
        (true, _) | (false, Some("NaN" | "inf" | "-inf")) => Ok(f),
        _ => Err(format!("{v} is out of range for {what}")),
    }
}

fn float64(v: &Value) -> Result<f64, String> {
    let f = match v {
        Value::Number(n) => n.as_f64().ok_or("invalid float")?,
        Value::String(s) => s.parse().map_err(|_| format!("`{}` is not a valid float", shown(s)))?,
        _ => return Err(format!("expected float, got {}", kind(v))),
    };
    finite_or_named(f, f.is_finite(), v, "float64")
}

/// Rounded once, straight to f32: through f64 a value can round twice.
fn float32(v: &Value) -> Result<f32, String> {
    let f = match v {
        Value::Number(n) => match (n.as_u64(), n.as_i64()) {
            (Some(u), _) => u as f32,
            (_, Some(i)) => i as f32,
            // serde_json already rounded a float literal to f64
            _ => n.as_f64().ok_or("invalid float")? as f32,
        },
        Value::String(s) => s.parse().map_err(|_| format!("`{}` is not a valid float", shown(s)))?,
        _ => return Err(format!("expected float, got {}", kind(v))),
    };
    finite_or_named(f, f.is_finite(), v, "float32")
}

fn kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn principal(v: &Value) -> Result<Principal, String> {
    let s = v.as_str().ok_or_else(|| format!("expected principal text, got {}", kind(v)))?;
    Principal::from_text(s).map_err(|e| format!("invalid principal `{}`: {e}", shown(s)))
}

/// Convert JSON into a Candid value of type `ty`.
pub fn json_to_idl(env: &TypeEnv, ty: &Type, v: &Value, depth: usize) -> Result<IDLValue, String> {
    if depth > MAX_DEPTH {
        return Err("value nested too deeply".into());
    }
    let ty = env.trace_type(ty).map_err(|e| e.to_string())?;
    let d = depth + 1;
    Ok(match ty.as_ref() {
        TypeInner::Null if v.is_null() => IDLValue::Null,
        TypeInner::Null => return Err(format!("expected null, got {}", kind(v))),
        TypeInner::Reserved => IDLValue::Reserved,
        TypeInner::Bool => IDLValue::Bool(v.as_bool().ok_or_else(|| format!("expected bool, got {}", kind(v)))?),
        TypeInner::Text => IDLValue::Text(v.as_str().ok_or_else(|| format!("expected text, got {}", kind(v)))?.to_owned()),
        TypeInner::Nat => IDLValue::Nat(int::<Nat>(v, "nat")?),
        TypeInner::Int => IDLValue::Int(int::<Int>(v, "int")?),
        TypeInner::Nat8 => IDLValue::Nat8(int(v, "nat8")?),
        TypeInner::Nat16 => IDLValue::Nat16(int(v, "nat16")?),
        TypeInner::Nat32 => IDLValue::Nat32(int(v, "nat32")?),
        TypeInner::Nat64 => IDLValue::Nat64(int(v, "nat64")?),
        TypeInner::Int8 => IDLValue::Int8(int(v, "int8")?),
        TypeInner::Int16 => IDLValue::Int16(int(v, "int16")?),
        TypeInner::Int32 => IDLValue::Int32(int(v, "int32")?),
        TypeInner::Int64 => IDLValue::Int64(int(v, "int64")?),
        TypeInner::Float32 => IDLValue::Float32(float32(v)?),
        TypeInner::Float64 => IDLValue::Float64(float64(v)?),
        TypeInner::Principal => IDLValue::Principal(principal(v)?),
        TypeInner::Service(_) => IDLValue::Service(principal(v)?),
        TypeInner::Func(_) => {
            let (p, m) = match v {
                Value::Array(a) if a.len() == 2 => (&a[0], &a[1]),
                Value::Object(o) if o.len() == 2 => (
                    o.get("principal").unwrap_or(&Value::Null),
                    o.get("method").unwrap_or(&Value::Null),
                ),
                _ => return Err("expected func as {\"principal\", \"method\"}".into()),
            };
            IDLValue::Func(principal(p)?, m.as_str().ok_or("func method must be text")?.to_owned())
        }
        TypeInner::Opt(_) if v.is_null() => IDLValue::None,
        TypeInner::Opt(inner) => {
            // `opt` adds no JSON level, so it does not count toward the depth. Peeled in a
            // bounded loop: `type R = opt R` is legal, and recursing would never stop.
            let (mut t, mut layers) = (inner.clone(), 1);
            while let TypeInner::Opt(i) = env.trace_type(&t).map_err(|e| e.to_string())?.as_ref() {
                layers += 1;
                if layers > MAX_DEPTH {
                    return Err("value nested too deeply".into());
                }
                t = i.clone();
            }
            let mut val = json_to_idl(env, &t, v, depth)?;
            for _ in 0..layers {
                val = IDLValue::Opt(Box::new(val));
            }
            val
        }
        TypeInner::Vec(inner) => {
            let is_blob = matches!(env.trace_type(inner).as_deref(), Ok(TypeInner::Nat8));
            match v {
                Value::Array(items) if is_blob => IDLValue::Blob(
                    items.iter().map(|x| int::<u8>(x, "nat8")).collect::<Result<_, _>>()?,
                ),
                Value::Array(items) => IDLValue::Vec(
                    items.iter().enumerate()
                        .map(|(i, x)| json_to_idl(env, inner, x, d).map_err(|e| format!("[{i}]: {e}")))
                        .collect::<Result<_, _>>()?,
                ),
                // Hex string for blobs: "0x0a0b..." or plain hex.
                Value::String(s) if is_blob => IDLValue::Blob(
                    hex::decode(s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")).unwrap_or(s))
                        .map_err(|e| format!("blob hex: {e}"))?,
                ),
                _ => return Err(format!("expected array, got {}", kind(v))),
            }
        }
        TypeInner::Record(fields) => {
            let mut out = Vec::with_capacity(fields.len());
            match v {
                Value::Array(items) if is_tuple(fields) => {
                    if items.len() != fields.len() {
                        return Err(format!("expected tuple of {} elements, got {}", fields.len(), items.len()));
                    }
                    for (f, x) in fields.iter().zip(items) {
                        out.push(IDLField { id: (*f.id).clone(), val: json_to_idl(env, &f.ty, x, d)? });
                    }
                }
                Value::Object(obj) => {
                    // Each key names one field, and no field twice. A typo in an optional
                    // field must not silently become `null`.
                    let mut given: Vec<Option<(&str, &Value)>> = vec![None; fields.len()];
                    for (k, x) in obj {
                        let i = field_index(fields, k).ok_or_else(|| format!("unknown record field `{}`", shown(k)))?;
                        if let Some((k0, _)) = given[i].replace((k, x)) {
                            return Err(format!("keys `{}` and `{}` name the same field", shown(k0), shown(k)));
                        }
                    }
                    for (f, x) in fields.iter().zip(given) {
                        let x = x.map_or(&Value::Null, |(_, x)| x);
                        let val = json_to_idl(env, &f.ty, x, d).map_err(|e| format!(".{}: {e}", label_key(&f.id)))?;
                        out.push(IDLField { id: (*f.id).clone(), val });
                    }
                }
                Value::Null if fields.is_empty() => {}
                _ => return Err(format!("expected record object, got {}", kind(v))),
            }
            IDLValue::Record(out)
        }
        TypeInner::Variant(fields) => {
            // {"Tag": payload} or, for null/reserved payloads, just "Tag".
            let (tag, payload) = match v {
                Value::String(s) => (s.as_str(), &Value::Null),
                Value::Object(o) if o.len() == 1 => {
                    let (k, x) = o.iter().next().unwrap();
                    (k.as_str(), x)
                }
                _ => return Err("expected variant as {\"Tag\": value} or \"Tag\"".into()),
            };
            let idx = field_index(fields, tag).ok_or_else(|| format!("unknown variant tag `{}`", shown(tag)))?;
            let f = &fields[idx];
            let val = json_to_idl(env, &f.ty, payload, d).map_err(|e| format!("{}: {e}", shown(tag)))?;
            IDLValue::Variant(VariantValue(Box::new(IDLField { id: (*f.id).clone(), val }), idx as u64))
        }
        TypeInner::Empty => return Err("type `empty` has no values".into()),
        other => return Err(format!("unsupported candid type {other:?}")),
    })
}

fn big(s: String) -> Value {
    // Safe integers stay numbers; everything larger is a decimal string.
    let mag = s.trim_start_matches('-');
    match mag.parse::<u64>() {
        Ok(m) if m <= MAX_SAFE_INT => Value::Number(s.parse::<i64>().map(Number::from).unwrap_or_else(|_| Number::from(m))),
        _ => Value::String(s),
    }
}

/// Convert a decoded Candid value into JSON, within the same value budget as values coming
/// out of scripts: a runtime converts the result again, between budget checks.
pub fn idl_to_json(v: &IDLValue, depth: usize, budget: &mut ValueBudget) -> Result<Value, String> {
    if depth > MAX_DEPTH {
        return Err(format!("reply nested deeper than {MAX_DEPTH} levels"));
    }
    budget.node()?;
    let d = depth + 1;
    let text = |s: String, budget: &mut ValueBudget| budget.bytes(s.len()).map(|()| s);
    Ok(match v {
        IDLValue::Bool(b) => Value::Bool(*b),
        IDLValue::Null | IDLValue::None | IDLValue::Reserved => Value::Null,
        IDLValue::Text(s) => Value::String(text(s.clone(), budget)?),
        IDLValue::Number(s) => big(text(s.replace('_', ""), budget)?),
        IDLValue::Float64(f) => f64_json(*f),
        IDLValue::Float32(f) => f64_json(*f as f64),
        IDLValue::Opt(inner) => idl_to_json(inner, depth, budget)?,
        IDLValue::Vec(items) => Value::Array(items.iter().map(|x| idl_to_json(x, d, budget)).collect::<Result<_, _>>()?),
        // One node: an array of small numbers costs no hashing, only its bytes.
        IDLValue::Blob(bytes) => {
            budget.bytes(bytes.len())?;
            Value::Array(bytes.iter().map(|b| Value::from(*b)).collect())
        }
        IDLValue::Record(fields) => {
            let tuple = !fields.is_empty()
                && fields.iter().enumerate().all(|(i, f)| matches!(f.id, Label::Unnamed(n) | Label::Id(n) if n == i as u32));
            if tuple {
                Value::Array(fields.iter().map(|f| idl_to_json(&f.val, d, budget)).collect::<Result<_, _>>()?)
            } else {
                budget.object(fields.len())?;
                let mut m = Map::new();
                for f in fields {
                    m.insert(text(label_key(&f.id), budget)?, idl_to_json(&f.val, d, budget)?);
                }
                Value::Object(m)
            }
        }
        IDLValue::Variant(VariantValue(f, _)) => {
            Value::Object(Map::from_iter([(text(label_key(&f.id), budget)?, idl_to_json(&f.val, d, budget)?)]))
        }
        IDLValue::Principal(p) | IDLValue::Service(p) => Value::String(p.to_text()),
        IDLValue::Func(p, m) => serde_json::json!({ "principal": p.to_text(), "method": text(m.clone(), budget)? }),
        // Decimal text of a huge number costs time quadratic in its size.
        IDLValue::Nat(n) if n.0.bits() > MAX_INT_BITS => return Err(format!("reply has a nat of {} bits", n.0.bits())),
        IDLValue::Int(n) if n.0.bits() > MAX_INT_BITS => return Err(format!("reply has an int of {} bits", n.0.bits())),
        IDLValue::Nat(n) => big(text(n.0.to_str_radix(10), budget)?),
        IDLValue::Int(n) => big(text(n.0.to_str_radix(10), budget)?),
        IDLValue::Nat8(n) => Value::from(*n),
        IDLValue::Nat16(n) => Value::from(*n),
        IDLValue::Nat32(n) => Value::from(*n),
        IDLValue::Nat64(n) => big(n.to_string()),
        IDLValue::Int8(n) => Value::from(*n),
        IDLValue::Int16(n) => Value::from(*n),
        IDLValue::Int32(n) => Value::from(*n),
        IDLValue::Int64(n) => big(n.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const DID: &str = r#"
        type Account = record { owner : principal; subaccount : opt blob };
        type Status = variant { Active; Frozen : record { reason : text }; Count : nat };
        type Tree = variant { leaf : int; node : vec Tree };
        service : {
          balance_of : (Account) -> (nat) query;
          set_status : (principal, Status) -> (Status);
          pair : (record { nat64; text }) -> (record { nat64; text }) query;
          echo_tree : (Tree) -> (Tree);
          many : (nat8, opt text) -> (int, opt text);
          nothing : () -> ();
          fire : () -> () oneway;
        }
    "#;

    /// Encode with the method's arg types, then decode the same bytes as if they
    /// were the reply (all test methods use identical arg/ret types).
    fn roundtrip(method: &str, args: Value) -> Value {
        let s = Service::parse(DID).unwrap();
        let enc = s.encode_args(method, &args).unwrap();
        let as_reply = Function { modes: vec![], args: vec![], rets: enc.func.args.clone() };
        s.decode_reply(&as_reply, &enc.bytes).unwrap()
    }

    #[test]
    fn principal_opt_blob_record() {
        let out = roundtrip("balance_of", json!({ "owner": "aaaaa-aa", "subaccount": [1, 2, 255] }));
        assert_eq!(out, json!({ "owner": "aaaaa-aa", "subaccount": [1, 2, 255] }));
        let out = roundtrip("balance_of", json!([{ "owner": "aaaaa-aa", "subaccount": null }]));
        assert_eq!(out, json!({ "owner": "aaaaa-aa", "subaccount": null }));
        let out = roundtrip("balance_of", json!({ "owner": "aaaaa-aa", "subaccount": "0x0aff" }));
        assert_eq!(out["subaccount"], json!([10, 255]));
    }

    #[test]
    fn variants_and_positional_args() {
        let out = roundtrip("set_status", json!(["ryjl3-tyaaa-aaaaa-aaaba-cai", { "Frozen": { "reason": "audit" } }]));
        assert_eq!(out, json!(["ryjl3-tyaaa-aaaaa-aaaba-cai", { "Frozen": { "reason": "audit" } }]));
        let out = roundtrip("set_status", json!(["aaaaa-aa", "Active"]));
        assert_eq!(out[1], json!({ "Active": null }));
    }

    #[test]
    fn nat_int_precision() {
        let big = "340282366920938463463374607431768211456"; // 2^128
        let out = roundtrip("set_status", json!(["aaaaa-aa", { "Count": big }]));
        assert_eq!(out[1], json!({ "Count": big }));
        let out = roundtrip("set_status", json!(["aaaaa-aa", { "Count": 42 }]));
        assert_eq!(out[1], json!({ "Count": 42 }));
        // Unsafe-for-JS nat64 comes back as a string; safe one as a number.
        let out = roundtrip("pair", json!([[ "18446744073709551615", "x" ]]));
        assert_eq!(out, json!(["18446744073709551615", "x"]));
        let out = roundtrip("pair", json!([7, "x"]));
        assert_eq!(out, json!([7, "x"]));
        let out = roundtrip("many", json!([255, null]));
        assert_eq!(out, json!([255, null]));
    }

    #[test]
    fn recursive_types() {
        let tree = json!({ "node": [{ "leaf": -1 }, { "node": [{ "leaf": 2 }] }] });
        assert_eq!(roundtrip("echo_tree", tree.clone()), tree);
    }

    #[test]
    fn errors_are_descriptive() {
        let s = Service::parse(DID).unwrap();
        let e = |m: &str, v: Value| s.encode_args(m, &v).err().unwrap();
        assert!(e("balance_of", json!({ "owner": "not a principal" })).contains("invalid principal"));
        assert!(e("many", json!([256, null])).contains("nat8"));
        assert!(e("set_status", json!(["aaaaa-aa", "Nope"])).contains("unknown variant"));
        assert!(e("missing", json!(null)).contains("not declared"));
        assert!(e("fire", json!(null)).contains("oneway"));
        assert!(e("many", json!([1, null, 3])).contains("takes 2 arguments"));
        assert!(s.encode_args("nothing", &json!(null)).is_ok());
        assert!(Service::parse("service : { broken").is_err());
        assert!(e("balance_of", json!({ "owner": "aaaaa-aa", "subacount": [1] })).contains("unknown record field `subacount`"));
        assert!(s.encode_args("nothing", &json!({})).is_ok());
    }

    #[test]
    fn js_style_floats_and_non_finite() {
        // JS hands integers > 2^31 over as floats.
        let out = roundtrip("set_status", json!(["aaaaa-aa", { "Count": 3_000_000_000.0 }]));
        assert_eq!(out[1], json!({ "Count": 3_000_000_000u64 }));
        assert!(Service::parse(DID).unwrap().encode_args("many", &json!([-0.0, null])).is_ok(), "-0 is a valid nat8");
        assert_eq!(f64_json(f64::NAN), json!("NaN"));
        assert_eq!(float64(&f64_json(f64::INFINITY)).unwrap(), f64::INFINITY);
    }

    #[test]
    fn deep_replies_error_instead_of_truncating() {
        let mut v = IDLValue::Null;
        for _ in 0..=MAX_DEPTH + 1 {
            v = IDLValue::Vec(vec![v]);
        }
        assert!(idl_to_json(&v, 0, &mut ValueBudget::default()).unwrap_err().contains("nested deeper"));
        // `opt` adds no JSON level, so it does not use up the depth (both directions).
        let mut v = IDLValue::Null;
        for _ in 0..200 {
            v = IDLValue::Opt(Box::new(v));
        }
        assert_eq!(idl_to_json(&v, 0, &mut ValueBudget::default()).unwrap(), Value::Null);
        let s = Service::parse("type L = opt record { nat; L }; service : { f : (L) -> () }").unwrap();
        let mut list = json!(null);
        for i in 0..60 {
            list = json!([i, list]);
        }
        assert!(s.encode_args("f", &json!([list])).is_ok(), "60 nodes, 60 JSON levels");
        // Numbers: rounded floats and over-long decimal text are refused.
        let n = Service::parse("service : { f : (nat) -> (); g : (nat64) -> () }").unwrap();
        assert!(n.encode_args("f", &json!(9_007_199_254_740_993.0)).err().unwrap().contains("decimal string"));
        assert!(n.encode_args("g", &json!(1e18)).is_err());
        assert!(n.encode_args("f", &json!("18446744073709551616")).is_ok());
        assert!(n.encode_args("f", &json!("9".repeat(MAX_INT_DIGITS + 1))).err().unwrap().contains("digits"));
        let huge = IDLValue::Nat(candid::Nat(candid::Nat::from(1u8).0 << 200_000u32));
        assert!(idl_to_json(&huge, 0, &mut ValueBudget::default()).unwrap_err().contains("bits"));
    }

    /// 164: errors quote at most a short prefix of script- or target-authored text.
    #[test]
    fn errors_quote_only_a_prefix() {
        let big = "z".repeat(1 << 20);
        let s = Service::parse("service : { p : (principal) -> (); f : (float64) -> (); v : (variant { a }) -> () }").unwrap();
        for (m, v) in [("p", json!(big)), ("f", json!(big)), ("v", json!(big)), ("v", json!({ big.clone(): null }))] {
            let e = s.encode_args(m, &v).err().unwrap();
            assert!(e.len() < 400, "{m}: {}", e.len());
        }
        let e = Service::parse(&format!("service : {{ f : ({}) -> () }}", "Q".repeat(500_000))).err().unwrap();
        assert!(e.len() <= 300, "{}", e.len());
    }

    /// 189: one hex string in an array is a single blob argument.
    #[test]
    fn a_wrapped_hex_string_is_a_blob_argument() {
        let s = Service::parse("service : { put : (blob) -> () }").unwrap();
        let hex = "ab".repeat(250_000);
        let wrapped = s.encode_args("put", &json!([hex])).unwrap().bytes;
        assert_eq!(wrapped, s.encode_args("put", &json!(hex)).unwrap().bytes);
        assert!(wrapped.len() > 250_000);
    }

    #[test]
    fn decode_reply_rejects_mismatched_opt_type() {
        let target = Service::parse("service : { f : (record { val : opt nat }) -> () }").unwrap();
        let stale = Service::parse("service : { f : () -> (record { val : opt text }) }").unwrap();
        let bytes = target.encode_args("f", &json!({ "val": 42 })).unwrap().bytes;
        let stale_func = stale.method("f").unwrap();
        assert!(stale.decode_reply(stale_func, &bytes).is_err());
        // Ordinary upgrades still decode: a new field we don't know, a missing optional one.
        let newer = Service::parse("service : { f : (record { val : opt nat; extra : text }) -> () }").unwrap();
        let bytes = newer.encode_args("f", &json!({ "val": 42, "extra": "x" })).unwrap().bytes;
        let older = Service::parse("service : { f : () -> (record { val : opt nat; gone : opt text }) }").unwrap();
        let v = older.decode_reply(older.method("f").unwrap(), &bytes).unwrap();
        assert_eq!(v, json!({ "val": 42, "gone": null }));
    }

    #[test]
    fn replies_over_the_value_limits_are_refused_after_decoding() {
        use crate::bridge::decode_checked;
        let target = Service::parse("service : { f : (vec nat8) -> () }").unwrap();
        let ours = Service::parse("service : { f : () -> (vec nat8) }").unwrap();
        let f = ours.method("f").unwrap();
        let reply = |n: usize| target.encode_args("f", &json!(vec![7; n])).unwrap().bytes;
        assert_eq!(decode_checked(&ours, f, &reply(1_000), "x.f").unwrap(), json!(vec![7; 1_000]));
        // Each decoded byte is one element: a 250 KB blob is over the 200,000 limit.
        let (e, stale) = decode_checked(&ours, f, &reply(250_000), "x.f").unwrap_err();
        assert!(!stale && e.contains("x.f is over the value limits") && e.contains("the call was executed"), "{e}");
        // A reply that does not decode means our copy of the interface is stale.
        assert!(decode_checked(&ours, f, b"DIDL", "x.f").unwrap_err().1);
    }

    #[test]
    fn deep_and_recursive_interfaces_are_safe() {
        // Rejected before the parser recurses (5,000 levels overflowed an 8 MiB stack).
        let deep = format!("service : {{ f : ({}nat) -> () }}", "vec ".repeat(100_000));
        assert!(Service::parse(&deep).err().unwrap().contains("nested"));
        let deep = format!("service : {{ f : ({}nat{}) -> () }}", "record { a : ".repeat(5_000), " }".repeat(5_000));
        assert!(Service::parse(&deep).err().unwrap().contains("nested"));
        // Comments and ordinary interfaces don't count as nesting.
        assert!(Service::parse(&format!("// {}\n{DID}", "{ ( ".repeat(1000))).is_ok());
        assert!(check_shape(DID).is_ok());
        // A long chain of named types: refused before candid's encoder recurses through it.
        let chain = |n: usize| -> String {
            let mut c: String = (0..n).map(|i| format!("type A{i} = opt A{};\n", i + 1)).collect();
            c.push_str(&format!("type A{n} = nat;\ntype R = opt R;\nservice : {{ f : (A0) -> (); g : (R) -> () }}"));
            c
        };
        assert!(Service::parse(&chain(20_000)).err().unwrap().contains("in total"));
        // Within the limits, everything that walks the types is safe on an 8 MiB stack (the
        // canister's), including at the limit.
        std::thread::Builder::new()
            .stack_size(8 << 20)
            .spawn(move || {
                assert!(Service::parse(&chain(500)).unwrap().encode_args("f", &json!([null])).is_ok());
                let s = Service::parse(&chain(MAX_DID_NESTING_SUM - 10)).unwrap();
                // At the limit candid's own recursion guard answers with an error, not a crash.
                let _ = s.encode_args("f", &json!([null]));
                // `type R = opt R` used to recurse forever deciding whether `[x]` wraps an argument.
                assert!(s.encode_args("g", &json!([null])).is_ok());
                assert!(s.encode_args("g", &json!([1])).err().unwrap().contains("nested too deeply"));
                let entry = crate::search::Entry {
                    canister: Principal::anonymous(),
                    allowed: vec!["*".into()],
                    service: Ok(Some(std::rc::Rc::new(s))),
                };
                let out = crate::search::render(&Default::default(), &[entry]);
                assert_eq!(out["canisters"][0]["types"].as_object().unwrap().len(), MAX_DID_NESTING_SUM - 10 + 2); // A0..=A1990 and R
                let deep = format!("service : {{ f : ({}nat) -> () }}", "vec ".repeat(MAX_DID_NESTING - 2)); // + the two brackets
                let s = Service::parse(&deep).unwrap();
                assert!(s.encode_args("f", &json!(null)).is_err());
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn single_arg_nested_vec_does_not_unwrap_outer_array() {
        let s = Service::parse("service : { f : (vec vec nat) -> () }").unwrap();
        let enc = s.encode_args("f", &json!([[1, 2]])).unwrap();
        let as_reply = Function { modes: vec![], args: vec![], rets: enc.func.args.clone() };
        assert_eq!(s.decode_reply(&as_reply, &enc.bytes).unwrap(), json!([[1, 2]]));
    }

    #[test]
    fn blob_hex_accepts_uppercase_prefix_and_rejects_repeated_prefix() {
        let s = Service::parse(DID).unwrap();
        assert!(s.encode_args("balance_of", &json!({ "owner": "aaaaa-aa", "subaccount": "0X0aff" })).is_ok());
        assert!(s.encode_args("balance_of", &json!({ "owner": "aaaaa-aa", "subaccount": "0x0x0aff" })).is_err());
    }

    #[test]
    fn all_candid_types_and_error_edge_cases() {
        let rich = r#"
            type Sparse = record { 0 : text; 5 : nat16 };
            type EmptyRec = record {};
            type F = func (nat) -> (text);
            type S = service { p : () -> () };
            service : (nat) -> {
              scalars : (null, reserved, bool, nat16, nat32, int8, int16, int32, int64, float32, float64, S, F)
                     -> (null, reserved, bool, nat16, nat32, int8, int16, int32, int64, float32, float64, S, F);
              sparse : (Sparse) -> (Sparse);
              empty_rec : (EmptyRec) -> (EmptyRec);
              unit : () -> ();
              dead : (empty) -> ();
              vec_text : (vec text) -> (vec text);
              opt_vec : (opt vec nat) -> (opt vec nat);
              big_nat : (nat) -> ();
            }
        "#;
        let s = Service::parse(rich).unwrap();
        let in_vals = json!([
            null,
            "anything",
            true,
            65535,
            4294967295u64,
            -128,
            -32768,
            -2147483648i64,
            "-9223372036854775808",
            1.5,
            -2.25,
            "aaaaa-aa",
            { "principal": "aaaaa-aa", "method": "p" }
        ]);
        let enc = s.encode_args("scalars", &in_vals).unwrap();
        let as_reply = Function { modes: vec![], args: vec![], rets: enc.func.rets.clone() };
        let dec = s.decode_reply(&as_reply, &enc.bytes).unwrap();
        assert_eq!(dec[2], json!(true));
        assert_eq!(dec[8], json!("-9223372036854775808"));
        assert_eq!(dec[12], json!({ "principal": "aaaaa-aa", "method": "p" }));

        // Func array form and float non-finite / -inf decoding.
        let mut arr_vals = in_vals.clone();
        arr_vals[12] = json!(["aaaaa-aa", "p"]);
        arr_vals[10] = json!("-inf");
        let enc2 = s.encode_args("scalars", &arr_vals).unwrap();
        let dec2 = s.decode_reply(&as_reply, &enc2.bytes).unwrap();
        assert_eq!(dec2[10], json!("-inf"));
        assert_eq!(dec2[12], json!({ "principal": "aaaaa-aa", "method": "p" }));

        // Sparse numeric record + `_5_` field name form.
        let sp = s.encode_args("sparse", &json!({ "0": "hi", "_5_": 9 })).unwrap();
        let sp_reply = Function { modes: vec![], args: vec![], rets: sp.func.rets.clone() };
        assert_eq!(s.decode_reply(&sp_reply, &sp.bytes).unwrap(), json!({ "0": "hi", "5": 9 }));

        // Zero-arg/zero-ret roundtrip & decoding errors.
        let unit = s.encode_args("unit", &json!(null)).unwrap();
        let unit_reply = Function { modes: vec![], args: vec![], rets: unit.func.rets.clone() };
        assert_eq!(s.decode_reply(&unit_reply, &unit.bytes).unwrap(), Value::Null);
        assert!(s.decode_reply(&unit_reply, b"not-didl").is_err());

        // opt (vec nat) array-like handling & empty record from null & >=1e19 float & IDLValue::Number.
        assert!(s.encode_args("opt_vec", &json!([[1, 2]])).is_ok());
        assert!(s.encode_args("opt_vec", &json!([1, 2])).is_ok());
        assert!(s.encode_args("opt_vec", &json!(null)).is_ok());
        assert!(s.encode_args("empty_rec", &json!(null)).is_ok());
        assert!(s.encode_args("big_nat", &json!(1e25)).is_err());
        assert_eq!(idl_to_json(&IDLValue::Number("1_000".into()), 0, &mut ValueBudget::default()).unwrap(), json!(1000));

        // Error branches across kind(), int(), float(), vec, record, variant, func, empty, depth.
        assert!(s.encode_args("scalars", &json!({})).is_err());
        assert!(s.encode_args("dead", &json!(null)).is_err());
        assert!(s.encode_args("vec_text", &json!("not-array")).is_err());
        assert!(s.encode_args("vec_text", &json!([123])).err().unwrap().contains("[0]:"));
        assert!(s.encode_args("sparse", &json!("not-obj")).is_err());
        assert!(s.encode_args("unit", &json!([])).is_ok());

        let base = Service::parse(DID).unwrap();
        assert!(base.encode_args("pair", &json!([[1]])).err().unwrap().contains("tuple of 2"));
        assert!(base.encode_args("set_status", &json!(["aaaaa-aa", {}])).is_err());
        assert!(base.encode_args("balance_of", &json!({ "owner": null })).err().unwrap().contains("null"));
        assert!(base.encode_args("many", &json!([true, null])).err().unwrap().contains("bool"));
        assert!(base.encode_args("many", &json!([[], null])).err().unwrap().contains("array"));
        assert!(base.encode_args("many", &json!([{}, null])).err().unwrap().contains("object"));
        assert!(s.encode_args("scalars", &json!([null, null, false, 0, 0, 0, 0, 0, 0, "badf", 0.0, "aaaaa-aa", ["aaaaa-aa", "p"]])).is_err());
        assert!(s.encode_args("scalars", &json!([null, null, false, 0, 0, 0, 0, 0, 0, true, 0.0, "aaaaa-aa", ["aaaaa-aa", "p"]])).is_err());
        assert!(s.encode_args("scalars", &json!([null, null, false, 0, 0, 0, 0, 0, 0, 1.0, 1.0, "aaaaa-aa", 42])).is_err());
        assert!(s.encode_args("scalars", &json!([null, null, false, 0, 0, 0, 0, 0, 0, 3e25, 0.0, "aaaaa-aa", ["aaaaa-aa", "p"]])).is_ok());

        let mut deep = json!(1);
        for _ in 0..=MAX_DEPTH + 1 {
            deep = json!([deep]);
        }
        assert!(json_to_idl(&s.env, &candid::types::TypeInner::Nat.into(), &deep, MAX_DEPTH + 1).is_err());
    }

    fn reply_of(s: &Service, method: &str) -> Function {
        Function { modes: vec![], args: vec![], rets: s.method(method).unwrap().args.clone() }
    }

    /// Runs `f` on a thread with the canister's 8 MiB stack (or more).
    fn on_stack(mb: usize, f: impl FnOnce() + Send + 'static) {
        std::thread::Builder::new().stack_size(mb << 20).spawn(f).unwrap().join().unwrap();
    }

    #[test]
    fn deeply_nested_reply_is_refused_inside_the_decoder() {
        // Finding 142: `type T = opt T` with 20,000 levels overflowed the Wasm stack inside
        // candid's decoder, whose guard did nothing on wasm. The vendored guard stops it at a
        // fixed depth on every target (a big stack here, so only that bound can stop it).
        on_stack(256, || {
            let s = Service::parse("type T = opt T; service : { f : (T) -> () }").unwrap();
            let mut bytes = b"DIDL\x01\x6e\x00\x01\x00".to_vec();
            bytes.extend(std::iter::repeat_n(1u8, 20_000));
            bytes.push(0);
            let e = s.decode_reply(&reply_of(&s, "f"), &bytes).err().unwrap();
            assert!(e.msg.contains("Recursion limit exceeded at depth 1025"), "{}", e.msg);
            assert!(!e.stale, "a depth limit is not a stale interface");
            // A deep wire type table (9,000 chained records; candid allows 10,000 entries) is
            // refused the same way.
            let n = 9_000u32;
            let mut bytes = b"DIDL".to_vec();
            // LEB128 of a non-negative number; `signed` keeps the sign bit of the last byte clear.
            let leb = |bytes: &mut Vec<u8>, mut n: u32, signed: bool| loop {
                let b = (n & 0x7f) as u8;
                n >>= 7;
                if n == 0 && !(signed && b & 0x40 != 0) {
                    break bytes.push(b);
                }
                bytes.push(b | 0x80);
            };
            leb(&mut bytes, n, false);
            for i in 1..n {
                bytes.extend([0x6c, 1, 0]);
                leb(&mut bytes, i, true);
            }
            bytes.extend([0x6c, 0, 1, 0]);
            let s = Service::parse("type R = record { opt R }; service : { f : (R) -> () }").unwrap();
            let e = s.decode_reply(&reply_of(&s, "f"), &bytes).err().unwrap();
            assert!(e.msg.contains("Recursion limit"), "{}", e.msg);
        });
    }

    #[test]
    fn large_replies_decode_and_over_quota_says_so() {
        // Findings 87 / R17: the quota ignored candid's 50x cost per byte for IDLValue, so
        // replies over ~400 KB failed with a misleading type error.
        let s = Service::parse(
            "service : { t : (text) -> (); b : (blob) -> (); r : (record { a : nat; big : text }) -> (); \
             old : (record { a : nat }) -> () }",
        )
        .unwrap();
        let big = "x".repeat(1_900_000);
        let enc = s.encode_args("t", &json!(big)).unwrap();
        assert_eq!(s.decode_reply(&reply_of(&s, "t"), &enc.bytes).unwrap(), json!(big));
        let enc = s.encode_args("b", &json!(hex::encode(vec![7u8; 1_900_000]))).unwrap();
        assert_eq!(s.decode_reply(&reply_of(&s, "b"), &enc.bytes).unwrap().as_array().unwrap().len(), 1_900_000);
        // A reply field newer than our interface is skipped, however large.
        let enc = s.encode_args("r", &json!({ "a": 1, "big": "y".repeat(1_500_000) })).unwrap();
        assert_eq!(s.decode_reply(&reply_of(&s, "old"), &enc.bytes).unwrap(), json!({ "a": 1 }));
        let enc = s.encode_args("t", &json!("z".repeat(2_500_000))).unwrap();
        let e = s.decode_reply(&reply_of(&s, "t"), &enc.bytes).err().unwrap();
        assert!(e.msg.contains("Decoding cost exceeds the limit") && !e.stale, "{}", e.msg);
    }

    #[test]
    fn reply_conversion_is_within_the_value_budget() {
        // Finding 145: replies entered the runtimes with no node, byte or key bound.
        let s = Service::parse("service : { f : (vec bool) -> () }").unwrap();
        let enc = s.encode_args("f", &json!(vec![true; crate::bridge::MAX_VALUE_NODES])).unwrap();
        let e = s.decode_reply(&reply_of(&s, "f"), &enc.bytes).err().unwrap();
        assert!(e.msg.contains("elements") && !e.stale, "{}", e.msg);
        // Keys per object are bounded by refusing wider interfaces.
        let wide = |n: usize| format!("service : {{ f : (record {{ {} }}) -> () }}", (0..n).map(|i| format!("f{i} : nat;")).collect::<String>());
        assert!(Service::parse(&wide(MAX_FIELDS)).is_ok());
        assert!(Service::parse(&wide(MAX_FIELDS + 1)).err().unwrap().contains("fields"));
        let tags = |n: usize| format!("service : {{ f : (variant {{ {} }}) -> () }}", (0..n).map(|i| format!("t{i};")).collect::<String>());
        assert!(Service::parse(&tags(MAX_FIELDS + 1)).err().unwrap().contains("fields"));
        // Wide objects are bounded in total too: building one is quadratic when its keys collide.
        use crate::bridge::MAX_WIDE_KEYS;
        let s = Service::parse(&wide(MAX_FIELDS).replace("(record", "(vec record")).unwrap();
        let rec = Value::Object((0..MAX_FIELDS).map(|i| (format!("f{i}"), json!(i))).collect());
        let reply = |n: usize| s.decode_reply(&reply_of(&s, "f"), &s.encode_args("f", &json!([vec![rec.clone(); n]])).unwrap().bytes);
        assert!(reply(MAX_WIDE_KEYS / MAX_FIELDS).is_ok());
        let e = reply(MAX_WIDE_KEYS / MAX_FIELDS + 1).err().unwrap();
        assert!(e.msg.contains("keys in objects") && !e.stale, "{}", e.msg);
    }

    #[test]
    fn opt_values_pay_for_the_subtype_memo() {
        // Finding 86 (vendor/candid/src/de.rs): every `opt` value clones the decoder's memo of
        // subtype checks, which a service reference with many methods fills. Upstream charged
        // nothing for the copy; this reply decoded, slowly, within the quota.
        let methods: String = (0..MAX_FIELDS).map(|i| format!("m{i} : (R{i}) -> ();")).collect();
        let recs: String = (0..MAX_FIELDS).map(|i| format!("type R{i} = record {{ f{i} : nat }};")).collect();
        let s = Service::parse(&format!("{recs} type S = service {{ {methods} }}; service : {{ f : (S, vec opt nat8) -> () }}")).unwrap();
        let enc = s.encode_args("f", &json!(["aaaaa-aa", vec![1; 60_000]])).unwrap();
        let e = s.decode_reply(&reply_of(&s, "f"), &enc.bytes).err().unwrap();
        assert!(e.msg.contains("Decoding cost exceeds the limit") && !e.stale, "{}", e.msg);
    }

    #[test]
    fn opt_cycles_and_list_depth_limit() {
        // Finding 127: a pure `opt` cycle recursed without bound for a non-null value, and the
        // depth limit is exact: 128 list nodes pass, 129 are refused.
        on_stack(8, || {
            let s = Service::parse("type R = opt R; type L = opt record { nat; L }; service : { g : (R) -> (); f : (L) -> () }").unwrap();
            assert!(s.encode_args("g", &json!(1)).err().unwrap().contains("nested too deeply"));
            assert!(s.encode_args("g", &json!(null)).is_ok());
            let list = |n: usize| (0..n).rev().fold(json!(null), |tail, i| json!([i, tail]));
            let enc = s.encode_args("f", &json!([list(128)])).unwrap();
            assert_eq!(s.decode_reply(&reply_of(&s, "f"), &enc.bytes).unwrap(), list(128));
            assert!(s.encode_args("f", &json!([list(129)])).err().unwrap().contains("nested too deeply"));
        });
    }

    #[test]
    fn single_array_argument_round_trips() {
        // Findings 135 / R37: a decoded reply passed back unchanged must encode to itself.
        let s = Service::parse(
            "service : { vv : (vec vec nat) -> (); vb : (vec blob) -> (); ov : (opt vec nat) -> (); \
             tv : (record { vec nat }) -> (); v : (vec nat) -> () }",
        )
        .unwrap();
        let cases = [
            ("vv", json!([[]])),
            ("vv", json!([])),
            ("vv", json!([[1, 2]])),
            ("vb", json!([[]])),
            ("vb", json!([[1]])),
            ("ov", json!([])),
            ("ov", json!(null)),
            ("ov", json!([1])),
            ("tv", json!([[]])),
            ("tv", json!([[1, 2]])),
        ];
        for (m, v) in cases {
            let first = s.encode_args(m, &v).unwrap();
            let back = s.decode_reply(&reply_of(&s, m), &first.bytes).unwrap();
            assert_eq!(back, v, "{m}");
            assert_eq!(s.encode_args(m, &back).unwrap().bytes, first.bytes, "{m} {v}");
        }
        // `[x]` is still a wrapper when the bare reading does not fit.
        let a = s.encode_args("v", &json!([[1, 2]])).unwrap().bytes;
        assert_eq!(a, s.encode_args("v", &json!([1, 2])).unwrap().bytes);
        assert!(s.encode_args("tv", &json!([[[1]]])).is_ok());
    }

    #[test]
    fn floats_round_once_and_never_overflow_silently() {
        // Finding 157.
        let s = Service::parse("service : { f : (float32) -> (); d : (float64) -> () }").unwrap();
        let f32_bits = |v: Value| match IDLArgs::from_bytes(&s.encode_args("f", &v).unwrap().bytes).unwrap().args[0] {
            IDLValue::Float32(f) => f.to_bits(),
            ref o => panic!("{o:?}"),
        };
        assert!(s.encode_args("f", &json!(1e39)).err().unwrap().contains("out of range"));
        assert!(s.encode_args("d", &json!("1e400")).err().unwrap().contains("out of range"));
        assert!(s.encode_args("f", &json!("1e39")).is_err());
        assert_eq!(f32_bits(json!("1.0000001788139343261718749")), 0x3f80_0001);
        assert_eq!(f32_bits(json!((1u64 << 60) + (1 << 36) + 1)), 0x5d80_0001);
        for v in ["inf", "-inf", "NaN"] {
            assert!(s.encode_args("f", &json!(v)).is_ok() && s.encode_args("d", &json!(v)).is_ok(), "{v}");
        }
        assert!(s.encode_args("d", &json!("infinity")).is_err());
    }

    #[test]
    fn strict_null_numbers_and_func_objects() {
        // Finding 159.
        let s = Service::parse(
            "service : { n : (null) -> (); v : (variant { A; B : nat }) -> (); k : (nat) -> (); i : (int) -> (); \
             f : (func (nat) -> ()) -> () }",
        )
        .unwrap();
        let ok = |m: &str, v: Value| s.encode_args(m, &v).is_ok();
        assert!(!ok("n", json!("x")) && ok("n", json!(null)));
        assert!(!ok("v", json!({ "A": 5 })) && ok("v", json!("A")) && ok("v", json!({ "A": null })));
        for bad in ["_5_", "1_", "5__0", "_", "-_1"] {
            assert!(!ok("k", json!(bad)) && !ok("i", json!(bad)), "{bad}");
        }
        assert!(ok("k", json!("1_000")) && ok("i", json!("-1_000")));
        assert!(!ok("f", json!({ "principal": "aaaaa-aa", "method": "p", "n": 7 })));
        assert!(ok("f", json!({ "principal": "aaaaa-aa", "method": "p" })));
    }

    #[test]
    fn field_keys_are_unambiguous_and_round_trip() {
        // Finding 160: aliases (`05`, `+5`, `_5_` with `5`) silently dropped or swapped
        // values, and names that read as numbers collided with ids on the way back.
        let five = Service::parse("service : { f : (record { 5 : nat; 6 : opt nat }) -> () }").unwrap();
        assert!(five.encode_args("f", &json!({ "5": 1, "05": 2 })).err().unwrap().contains("unknown record field `05`"));
        assert!(five.encode_args("f", &json!({ "5": 1, "+5": 2 })).is_err());
        assert!(five.encode_args("f", &json!({ "5": 1, "_5_": 2 })).err().unwrap().contains("name the same field"));
        let h1 = format!("_{}_", candid::idl_hash("1"));
        let h0 = format!("_{}_", candid::idl_hash("_0_"));
        let cases = [
            ("record { \"01\" : nat; 1 : nat }", json!({ "01": 7, "1": 9 })),
            ("record { \"1\" : nat; 1 : text }", json!({ "1": "x", h1.clone(): 5 })),
            ("variant { \"1\" : nat; 1 : text }", json!({ "1": "x" })),
            ("variant { \"1\" : nat; 1 : text }", json!({ h1.clone(): 5 })),
            ("variant { 0 : nat; \"_0_\" : nat }", json!({ "0": 1 })),
            ("variant { 0 : nat; \"_0_\" : nat }", json!({ h0.clone(): 2 })),
            ("record { 1_000 : nat; \"true\" : nat; \"5\" : nat }", json!({ "1000": 1, "true": 2, format!("_{}_", candid::idl_hash("5")): 3 })),
        ];
        for (ty, v) in cases {
            let s = Service::parse(&format!("service : {{ f : ({ty}) -> () }}")).unwrap();
            let first = s.encode_args("f", &v).unwrap();
            let back = s.decode_reply(&reply_of(&s, "f"), &first.bytes).unwrap();
            assert_eq!(back, v, "{ty}");
            assert_eq!(s.encode_args("f", &back).unwrap().bytes, first.bytes, "{ty}");
        }
        // The spellings search shows (candid's printer) are accepted too: `1_000`, and a
        // quoted number when no id takes it.
        let s = Service::parse("service : { f : (record { 1_000 : nat; \"5\" : nat }) -> () }").unwrap();
        assert!(s.encode_args("f", &json!({ "1_000": 1, "5": 2 })).is_ok());
    }

    #[test]
    fn interface_shape_limits() {
        // Findings 74 / R23 / 86: a pure alias chain scored 0 and its cycle check was
        // quadratic; every definition now counts, and chains are bounded.
        let aliases = |n: usize| -> String {
            let mut c: String = (0..n).map(|i| format!("type A{i} = A{};\n", i + 1)).collect();
            c.push_str(&format!("type A{n} = nat;\nservice : {{ f : (A0) -> () }}"));
            c
        };
        assert!(Service::parse(&aliases(MAX_ALIAS_CHAIN - 1)).is_ok());
        assert!(Service::parse(&aliases(MAX_ALIAS_CHAIN + 1)).err().unwrap().contains("alias chain"));
        let t = std::time::Instant::now();
        on_stack(1, move || assert!(Service::parse(&aliases(16_000)).err().unwrap().contains("in total")));
        assert!(t.elapsed().as_millis() < 1_000, "{:?}", t.elapsed());
        let reserved: String = (0..5_000).map(|i| format!("type r{i} = reserved;\n")).collect();
        assert!(Service::parse(&format!("{reserved}service : {{}}")).err().unwrap().contains("in total"));
        // Real interfaces are well inside the limits.
        for did in [include_str!("../icp_code_mode.did"), include_str!("../demo_store/demo_store.did")] {
            Service::parse(did).unwrap();
        }
        // Finding 172: field ids past u32::MAX wrapped (release) or panicked (debug) in the parser.
        for ty in ["record { 4294967294 : bool; nat; nat }", "record { 4294967295 : nat }", "record { 0xffffffff : nat }"] {
            let e = Service::parse(&format!("service : {{ f : ({ty}) -> () }}")).err().unwrap();
            assert!(e.contains("out of range"), "{ty}: {e}");
        }
        assert!(Service::parse("service : { f : (record { 4294967293 : bool; nat }) -> () }").is_ok());
        // Variants only sort their tags (no `id + 1`), so the top id is legal there; a bare
        // tag is a name, not the next number.
        for ty in ["variant { 4294967295 : nat }", "variant { 4294967294 : nat; A }", "variant { 4294967295 }"] {
            assert!(Service::parse(&format!("service : {{ f : ({ty}) -> () }}")).is_ok(), "{ty}");
        }
        // Finding 200: imports are refused, not silently ignored.
        assert!(Service::parse("import \"/etc/passwd\"; service : {}").err().unwrap().contains("import"));
        assert!(Service::parse("import service \"ledger.did\"; service : { m : () -> () }").err().unwrap().contains("import"));
    }

    #[test]
    fn label_lookup_is_not_keys_times_fields() {
        // Finding 86: tags and keys were found by linear scans per value.
        let tags: String = (0..MAX_FIELDS).map(|i| format!("t{i} : nat;")).collect();
        let fields: String = (0..MAX_FIELDS).map(|i| format!("f{i} : nat;")).collect();
        let s = Service::parse(&format!("type V = variant {{ {tags} }}; type R = record {{ {fields} }}; service : {{}}")).unwrap();
        let var = |n: &str| -> Type { TypeInner::Vec(TypeInner::Var(n.into()).into()).into() };
        let values: Vec<Value> = (0..200_000).map(|i| json!({ format!("t{}", i % MAX_FIELDS): i })).collect();
        let record = Value::Object((0..MAX_FIELDS).map(|i| (format!("f{i}"), json!(i))).collect());
        let t = std::time::Instant::now();
        json_to_idl(&s.env, &var("V"), &Value::Array(values), 0).unwrap();
        json_to_idl(&s.env, &var("R"), &Value::Array(vec![record; 10]), 0).unwrap();
        assert!(t.elapsed().as_secs() < 5, "{:?}", t.elapsed());
    }

    #[test]
    fn reply_type_errors_name_the_mismatch() {
        // Finding 122 (c).
        let target = Service::parse("service : { f : (record { val : opt nat }) -> () }").unwrap();
        let stale = Service::parse("service : { f : () -> (record { val : opt text }) }").unwrap();
        let bytes = target.encode_args("f", &json!({ "val": 42 })).unwrap().bytes;
        let e = stale.decode_reply(stale.method("f").unwrap(), &bytes).err().unwrap();
        assert!(e.stale && e.msg.contains("val"), "{}", e.msg);
    }

    #[test]
    fn uncallable_methods() {
        let long = "m".repeat(MAX_METHOD_LEN + 1);
        let s = Service::parse(&format!(
            "service : {{ ok : () -> (); o : () -> () oneway; c : () -> () composite_query; e : (empty) -> (); {long} : () -> () }}"
        ))
        .unwrap();
        for m in ["o", "c", "e", &long] {
            let f = s.method(m).unwrap();
            assert!(s.uncallable(m, f).is_some() && s.encode_args(m, &json!(null)).is_err(), "{m}");
        }
        assert!(s.uncallable("ok", s.method("ok").unwrap()).is_none());
    }
}
