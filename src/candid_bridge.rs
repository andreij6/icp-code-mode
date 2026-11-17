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
                vec![&a[0]]
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
        let mut out: Vec<Value> = args.args.iter().map(|v| idl_to_json(v, 0)).collect::<Result<_, _>>()?;
        Ok(match out.len() {
            0 => Value::Null,
            1 => out.pop().unwrap(),
            _ => Value::Array(out),
        })
    }
}

/// `true` when the type is itself encoded as a JSON array (vec / tuple record),
/// which makes `[x]` ambiguous for single-argument methods.
fn is_array_like(env: &TypeEnv, ty: &Type) -> bool {
    match env.trace_type(ty).as_deref() {
        Ok(TypeInner::Vec(_)) => true,
        Ok(TypeInner::Record(fs)) => is_tuple(fs),
        Ok(TypeInner::Opt(inner)) => is_array_like(env, inner),
        _ => false,
    }
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
            Some(f) if f.fract() == 0.0 && f.abs() < 1e19 => Some(format!("{f:.0}")),
            _ => Some(n.to_string()),
        },
        Value::Number(n) => Some(n.to_string()),
        Value::String(s) => Some(s.replace('_', "")),
        _ => None,
    }
}

fn int<T: std::str::FromStr>(v: &Value, what: &str) -> Result<T, String> {
    let s = num_text(v).ok_or_else(|| format!("expected {what}, got {}", kind(v)))?;
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
            _ => IDLValue::Opt(Box::new(json_to_idl(env, inner, v, d)?)),
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
                    hex::decode(s.trim_start_matches("0x")).map_err(|e| format!("blob hex: {e}"))?,
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
        IDLValue::Opt(inner) => idl_to_json(inner, d)?,
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
        assert_eq!(f64_json(f64::NAN), json!("NaN"));
        assert_eq!(float(&f64_json(f64::INFINITY)).unwrap(), f64::INFINITY);
    }

    #[test]
    fn deep_replies_error_instead_of_truncating() {
        let mut v = IDLValue::Null;
        for _ in 0..=MAX_DEPTH + 1 {
            v = IDLValue::Opt(Box::new(v));
        }
        assert!(idl_to_json(&v, 0).unwrap_err().contains("nested deeper"));
    }
}
