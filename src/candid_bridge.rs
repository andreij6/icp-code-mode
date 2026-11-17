//! Candid interface parsing plus type-directed JSON <-> Candid conversion.
//!
//! Scripts speak JSON; target canisters speak Candid. The target's `.did` is the
//! source of truth: arguments are coerced against the method's declared argument
//! types, and replies are decoded against its declared return types so record
//! and variant fields come back with their real names.

use candid::types::value::{IDLField, VariantValue};
use candid::types::{Field, FuncMode, Function, Label, Type, TypeInner};
use candid::{IDLArgs, IDLValue, Int, Nat, Principal, TypeEnv};
use serde_json::{Map, Number, Value};

/// JSON integers beyond this magnitude are emitted as strings (JS `Number` safety).
const MAX_SAFE_INT: u64 = (1 << 53) - 1;
/// Candid decoding work budget per reply (see `DecoderConfig::set_decoding_quota`).
const DECODING_QUOTA: usize = 20_000_000;
/// Guards against pathological nesting in either direction. Runtimes accept values
/// at least this deep (see `runtimes::js::MAX_VALUE_DEPTH`).
pub const MAX_DEPTH: usize = 128;

/// Deepest type nesting accepted in one definition. The parser and type checker recurse
/// once per level, and about 5,000 levels overflow the 8 MiB Wasm stack: a trap on every
/// fetch of a hostile interface.
pub const MAX_DID_NESTING: usize = 256;
/// Bound on the nesting summed over all definitions. Candid's encoder recurses through
/// named types (`type A1 = opt A2; type A2 = ...`), once per name, and a chain of 20,000
/// overflows the stack. A path visits each name once, so this sum bounds its depth.
pub const MAX_DID_NESTING_SUM: usize = 2_000;

/// (deepest definition, sum over definitions) of the type nesting in `did`, from its
/// tokens: each `vec`/`opt` prefix and each open bracket is a level. Lexical errors stop
/// the count; the parser reports them.
fn nesting(did: &str) -> (usize, usize) {
    use candid_parser::token::{Token, Tokenizer};
    let (mut open, mut run, mut max, mut stmt, mut sum) = (vec![0usize], 0usize, 0usize, 0usize, 0usize);
    for tok in Tokenizer::new(did) {
        let Ok((_, tok, _)) = tok else { break };
        let base = *open.last().unwrap_or(&0);
        match tok {
            // A top-level `;` or `type` ends a definition.
            Token::Semi | Token::Type if open.len() == 1 => {
                sum += stmt;
                stmt = 0;
                run = 0;
            }
            Token::Vec | Token::Opt => run += 1,
            // keywords that come before the bracket they open
            Token::Record | Token::Variant | Token::Func | Token::Service | Token::Arrow => {}
            Token::LParen | Token::LBrace => {
                open.push(base + run + 1);
                run = 0;
            }
            Token::RParen | Token::RBrace => {
                if open.len() > 1 {
                    open.pop();
                }
                run = 0;
            }
            _ => run = 0,
        }
        stmt = stmt.max(open.last().unwrap_or(&0) + run);
        max = max.max(stmt);
    }
    (max, sum + stmt)
}

/// A parsed `.did` service.
pub struct Service {
    pub env: TypeEnv,
    pub actor: Type,
}

/// Argument bytes plus everything needed to decode the reply.
pub struct EncodedCall {
    pub bytes: Vec<u8>,
    pub func: Function,
}

impl Service {
    pub fn parse(did: &str) -> Result<Self, String> {
        match nesting(did) {
            (n, _) if n > MAX_DID_NESTING => {
                return Err(format!("invalid candid interface: types nested {n} levels deep (limit {MAX_DID_NESTING})"));
            }
            (_, n) if n > MAX_DID_NESTING_SUM => {
                return Err(format!(
                    "invalid candid interface: type definitions nested {n} levels in total (limit {MAX_DID_NESTING_SUM})"
                ));
            }
            _ => {}
        }
        let invalid = |e: candid_parser::Error| format!("invalid candid interface: {e}");
        let prog: candid_parser::IDLProg = did.parse().map_err(invalid)?;
        let mut env = TypeEnv::new();
        let actor = candid_parser::check_prog(&mut env, &prog).map_err(invalid)?;
        let actor = actor.ok_or("candid interface declares no service")?;
        // `service : (init) -> { .. }` is a class; we only call methods.
        let actor = match actor.as_ref() {
            TypeInner::Class(_, t) => t.clone(),
            _ => actor,
        };
        Ok(Self { env, actor })
    }

    pub fn method<'a>(&'a self, name: &'a str) -> Result<&'a Function, String> {
        self.env
            .get_method(&self.actor, name)
            .map_err(|_| format!("method `{name}` is not declared in the target's candid interface"))
    }

    /// Coerce `args` into the method's argument list and serialize to Candid bytes.
    ///
    /// Accepted shapes: `null` (zero args), a JSON array (positional args), or — for
    /// single-argument methods — the bare argument value.
    pub fn encode_args(&self, method: &str, args: &Value) -> Result<EncodedCall, String> {
        let func = self.method(method)?.clone();
        if func.modes.contains(&FuncMode::Oneway) {
            return Err(format!("method `{method}` is oneway; it returns no reply"));
        }
        let n = func.args.len();
        let positional: Vec<&Value> = match args {
            Value::Null => vec![],
            Value::Object(o) if n == 0 && o.is_empty() => vec![],
            Value::Array(a) if n != 1 => a.iter().collect(),
            // Single-arg method: `[x]` is a positional wrapper unless the argument type is
            // itself array-shaped (vec/tuple) and `x` is not an array.
            Value::Array(a) if a.len() == 1 && (a[0].is_array() || !is_array_like(&self.env, &func.args[0])) => {
                // `[[1, 2]]` for a `vec vec nat` argument is the bare value, not a wrapper:
                // when the wrapped reading does not fit the type, fall back to the bare one.
                if is_array_like(&self.env, &func.args[0]) && json_to_idl(&self.env, &func.args[0], &a[0], 0).is_err() {
                    vec![args]
                } else {
                    vec![&a[0]]
                }
            }
            v if n == 1 => vec![v],
            _ => return Err(format!("method `{method}` takes {n} arguments; pass them as a JSON array")),
        };
        if positional.len() > func.args.len() {
            return Err(format!(
                "method `{method}` takes {} arguments, got {}",
                func.args.len(),
                positional.len()
            ));
        }
        let mut values = Vec::with_capacity(func.args.len());
        for (i, ty) in func.args.iter().enumerate() {
            let v = positional.get(i).copied().unwrap_or(&Value::Null);
            values.push(json_to_idl(&self.env, ty, v, 0).map_err(|e| format!("argument {i}: {e}"))?);
        }
        let bytes = IDLArgs::new(&values)
            .to_bytes_with_types(&self.env, &func.args)
            .map_err(|e| format!("candid encoding failed: {e}"))?;
        Ok(EncodedCall { bytes, func })
    }

    /// Decode reply bytes against the method's return types. Single returns are
    /// unwrapped; zero returns become `null`; several become an array.
    pub fn decode_reply(&self, func: &Function, bytes: &[u8]) -> Result<Value, String> {
        // Quotas bound the work a hostile reply can force on us.
        let mut cfg = candid::DecoderConfig::new();
        // Skipping covers reply fields newer than our copy of the interface.
        cfg.set_decoding_quota(DECODING_QUOTA)
            .set_skipping_quota(DECODING_QUOTA)
            .set_full_error_message(false); // otherwise errors embed the whole reply in hex
        let args = IDLArgs::from_bytes_with_types_with_config(bytes, &self.env, &func.rets, &cfg)
            .map_err(|e| format!("candid decoding failed: {e}"))?;
        self.check_reply_types(func, bytes)?;
        let mut out: Vec<Value> = args.args.iter().map(|v| idl_to_json(v, 0)).collect::<Result<_, _>>()?;
        Ok(match out.len() {
            0 => Value::Null,
            1 => out.pop().unwrap(),
            _ => Value::Array(out),
        })
    }
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
        subtype_with_config(OptReport::Error, &mut Gamma::default(), &env, &wire, &tuple(func.rets.clone())).map_err(|_| {
            "reply types do not match the interface (an optional value would have been dropped); \
             the target's interface has probably changed"
                .to_string()
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

fn label_name(l: &Label) -> String {
    match l {
        Label::Named(s) => s.clone(),
        Label::Id(n) | Label::Unnamed(n) => n.to_string(),
    }
}

/// Does JSON key `k` name this Candid field? By name, by decimal id, or as `_<hash>_`.
fn key_matches(k: &str, label: &Label) -> bool {
    match label {
        Label::Named(n) => k == n,
        Label::Id(n) | Label::Unnamed(n) => {
            let digits = k.strip_prefix('_').and_then(|k| k.strip_suffix('_')).unwrap_or(k);
            digits.parse::<u32>() == Ok(*n)
        }
    }
}

fn field_value<'a>(obj: &'a Map<String, Value>, label: &Label) -> Option<&'a Value> {
    obj.iter().find(|(k, _)| key_matches(k, label)).map(|(_, v)| v)
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
        Value::String(s) => Some(s.replace('_', "")),
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

fn float(v: &Value) -> Result<f64, String> {
    match v {
        Value::Number(n) => n.as_f64().ok_or_else(|| "invalid float".into()),
        Value::String(s) => s.parse().map_err(|_| format!("`{s}` is not a valid float")),
        _ => Err(format!("expected float, got {}", kind(v))),
    }
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
    Principal::from_text(s).map_err(|e| format!("invalid principal `{s}`: {e}"))
}

/// Convert JSON into a Candid value of type `ty`.
pub fn json_to_idl(env: &TypeEnv, ty: &Type, v: &Value, depth: usize) -> Result<IDLValue, String> {
    if depth > MAX_DEPTH {
        return Err("value nested too deeply".into());
    }
    let ty = env.trace_type(ty).map_err(|e| e.to_string())?;
    let d = depth + 1;
    Ok(match ty.as_ref() {
        TypeInner::Null => IDLValue::Null,
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
        TypeInner::Float32 => IDLValue::Float32(float(v)? as f32),
        TypeInner::Float64 => IDLValue::Float64(float(v)?),
        TypeInner::Principal => IDLValue::Principal(principal(v)?),
        TypeInner::Service(_) => IDLValue::Service(principal(v)?),
        TypeInner::Func(_) => {
            let (p, m) = match v {
                Value::Array(a) if a.len() == 2 => (&a[0], &a[1]),
                Value::Object(o) => (
                    o.get("principal").unwrap_or(&Value::Null),
                    o.get("method").unwrap_or(&Value::Null),
                ),
                _ => return Err("expected func as {\"principal\", \"method\"}".into()),
            };
            IDLValue::Func(principal(p)?, m.as_str().ok_or("func method must be text")?.to_owned())
        }
        TypeInner::Opt(inner) => match v {
            Value::Null => IDLValue::None,
            // `opt` adds no JSON level, so it does not count toward the depth.
            _ => IDLValue::Opt(Box::new(json_to_idl(env, inner, v, depth)?)),
        },
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
                    // A typo in an optional field must not silently become `null`.
                    if let Some(k) = obj.keys().find(|k| !fields.iter().any(|f| key_matches(k, &f.id))) {
                        return Err(format!("unknown record field `{k}`"));
                    }
                    for f in fields {
                        let x = field_value(obj, &f.id).unwrap_or(&Value::Null);
                        let val = json_to_idl(env, &f.ty, x, d).map_err(|e| format!(".{}: {e}", label_name(&f.id)))?;
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
            let (idx, f) = fields
                .iter()
                .enumerate()
                .find(|(_, f)| label_name(&f.id) == tag || format!("_{}_", f.id.get_id()) == tag)
                .ok_or_else(|| format!("unknown variant tag `{tag}`"))?;
            let val = json_to_idl(env, &f.ty, payload, d).map_err(|e| format!("{tag}: {e}"))?;
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

fn f64_json(f: f64) -> Value {
    // Non-finite floats as strings (`float()` parses them back).
    match Number::from_f64(f) {
        Some(n) => Value::Number(n),
        None if f.is_nan() => Value::String("NaN".into()),
        None if f > 0.0 => Value::String("inf".into()),
        None => Value::String("-inf".into()),
    }
}

/// Convert a decoded Candid value into JSON.
pub fn idl_to_json(v: &IDLValue, depth: usize) -> Result<Value, String> {
    if depth > MAX_DEPTH {
        return Err(format!("reply nested deeper than {MAX_DEPTH} levels"));
    }
    let d = depth + 1;
    let all = |items: &mut dyn Iterator<Item = &IDLValue>| items.map(|x| idl_to_json(x, d)).collect::<Result<Vec<_>, _>>();
    Ok(match v {
        IDLValue::Bool(b) => Value::Bool(*b),
        IDLValue::Null | IDLValue::None | IDLValue::Reserved => Value::Null,
        IDLValue::Text(s) => Value::String(s.clone()),
        IDLValue::Number(s) => big(s.replace('_', "")),
        IDLValue::Float64(f) => f64_json(*f),
        IDLValue::Float32(f) => f64_json(*f as f64),
        IDLValue::Opt(inner) => idl_to_json(inner, depth)?,
        IDLValue::Vec(items) => Value::Array(all(&mut items.iter())?),
        IDLValue::Blob(bytes) => Value::Array(bytes.iter().map(|b| Value::from(*b)).collect()),
        IDLValue::Record(fields) => {
            let tuple = !fields.is_empty()
                && fields.iter().enumerate().all(|(i, f)| matches!(f.id, Label::Unnamed(n) | Label::Id(n) if n == i as u32));
            if tuple {
                Value::Array(all(&mut fields.iter().map(|f| &f.val))?)
            } else {
                let mut m = Map::new();
                for f in fields {
                    m.insert(label_name(&f.id), idl_to_json(&f.val, d)?);
                }
                Value::Object(m)
            }
        }
        IDLValue::Variant(VariantValue(f, _)) => Value::Object(Map::from_iter([(label_name(&f.id), idl_to_json(&f.val, d)?)])),
        IDLValue::Principal(p) | IDLValue::Service(p) => Value::String(p.to_text()),
        IDLValue::Func(p, m) => serde_json::json!({ "principal": p.to_text(), "method": m }),
        // Decimal text of a huge number costs time quadratic in its size.
        IDLValue::Nat(n) if n.0.bits() > MAX_INT_BITS => return Err(format!("reply has a nat of {} bits", n.0.bits())),
        IDLValue::Int(n) if n.0.bits() > MAX_INT_BITS => return Err(format!("reply has an int of {} bits", n.0.bits())),
        IDLValue::Nat(n) => big(n.0.to_str_radix(10)),
        IDLValue::Int(n) => big(n.0.to_str_radix(10)),
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
        assert_eq!(float(&f64_json(f64::INFINITY)).unwrap(), f64::INFINITY);
    }

    #[test]
    fn deep_replies_error_instead_of_truncating() {
        let mut v = IDLValue::Null;
        for _ in 0..=MAX_DEPTH + 1 {
            v = IDLValue::Vec(vec![v]);
        }
        assert!(idl_to_json(&v, 0).unwrap_err().contains("nested deeper"));
        // `opt` adds no JSON level, so it does not use up the depth (both directions).
        let mut v = IDLValue::Null;
        for _ in 0..200 {
            v = IDLValue::Opt(Box::new(v));
        }
        assert_eq!(idl_to_json(&v, 0).unwrap(), Value::Null);
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
        assert!(idl_to_json(&huge, 0).unwrap_err().contains("bits"));
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
    fn deep_and_recursive_interfaces_are_safe() {
        // Rejected before the parser recurses (5,000 levels overflowed an 8 MiB stack).
        let deep = format!("service : {{ f : ({}nat) -> () }}", "vec ".repeat(100_000));
        assert!(Service::parse(&deep).err().unwrap().contains("nested"));
        let deep = format!("service : {{ f : ({}nat{}) -> () }}", "record { a : ".repeat(5_000), " }".repeat(5_000));
        assert!(Service::parse(&deep).err().unwrap().contains("nested"));
        // Comments and ordinary interfaces don't count as nesting.
        assert!(Service::parse(&format!("// {}\n{DID}", "{ ( ".repeat(1000))).is_ok());
        assert!(nesting(DID).0 < 10);
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
                assert!(s.encode_args("g", &json!([1])).is_err());
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
        assert_eq!(idl_to_json(&IDLValue::Number("1_000".into()), 0).unwrap(), json!(1000));

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
}
