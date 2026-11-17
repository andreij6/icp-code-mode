//! The `search` tool: read-only discovery of what scripts may call.
//!
//! Lists allowlisted canisters and the allowed methods of each, with Candid signatures
//! and the type definitions those signatures reference, optionally filtered by keywords.
//! This is a pure function of the policy and the cached interfaces, so the query call
//! can answer it without consensus. Only an interface that hasn't been fetched yet
//! forces an upgrade to an update call.

use crate::candid_bridge::Service;
use candid::types::{Function, Type, TypeInner};
use candid::{Principal, TypeEnv};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::rc::Rc;

/// Most methods returned by one search (the rest are only counted).
pub const MAX_METHODS: usize = 40;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SearchArgs {
    /// Space-separated keywords; a method matches if every keyword appears in its name or
    /// signature (case-insensitive). `None` matches everything.
    pub query: Option<String>,
    /// Restrict to one canister.
    pub canister_id: Option<String>,
}

/// One allowlisted canister, as seen by a search.
pub struct Entry {
    pub canister: Principal,
    pub allowed: Vec<String>,
    /// `Ok(None)`: not fetched yet. `Err`: fetching failed.
    pub service: Result<Option<Rc<Service>>, String>,
}

/// Allowlist entries in scope for `args`. A `canister_id` that is not allowlisted (or
/// not a principal) yields no entries; `render` reports that instead of failing.
pub fn scope(args: &SearchArgs, allowlist: Vec<(Principal, Vec<String>)>) -> Vec<(Principal, Vec<String>)> {
    let wanted = args.canister_id.as_deref().map(|id| Principal::from_text(id.trim()).ok());
    allowlist
        .into_iter()
        .filter(|(p, _)| match &wanted {
            None => true,
            Some(w) => w.as_ref() == Some(p),
        })
        .collect()
}

// Infinite width: one line per signature/type.
fn signature(f: &Function) -> String {
    candid::pretty::candid::pp_function(f).pretty(usize::MAX).to_string()
}

fn type_text(t: &Type) -> String {
    candid::pretty::candid::pp_ty(t).pretty(usize::MAX).to_string()
}

/// Named types referenced (transitively) by `ty`.
fn collect_vars(env: &TypeEnv, ty: &Type, out: &mut BTreeSet<String>) {
    match ty.as_ref() {
        TypeInner::Var(name) => {
            if out.insert(name.clone()) {
                if let Ok(t) = env.find_type(name) {
                    collect_vars(env, t, out);
                }
            }
        }
        TypeInner::Opt(t) | TypeInner::Vec(t) => collect_vars(env, t, out),
        TypeInner::Record(fs) | TypeInner::Variant(fs) => fs.iter().for_each(|f| collect_vars(env, &f.ty, out)),
        TypeInner::Func(f) => f.args.iter().chain(&f.rets).for_each(|t| collect_vars(env, t, out)),
        TypeInner::Service(ms) => ms.iter().for_each(|(_, t)| collect_vars(env, t, out)),
        TypeInner::Class(args, t) => {
            args.iter().for_each(|a| collect_vars(env, a, out));
            collect_vars(env, t, out);
        }
        _ => {}
    }
}

fn keywords(q: &Option<String>) -> Vec<String> {
    q.as_deref().unwrap_or("").split_whitespace().map(str::to_lowercase).collect()
}

pub fn render(args: &SearchArgs, entries: &[Entry]) -> Value {
    let terms = keywords(&args.query);
    let mut budget = MAX_METHODS;
    let mut omitted = 0usize;
    let mut canisters = vec![];

    for e in entries {
        let id = e.canister.to_text();
        let svc = match &e.service {
            Ok(Some(s)) => s,
            Ok(None) => {
                canisters.push(json!({ "canister_id": id, "allowed_methods": e.allowed, "error": "interface not fetched" }));
                continue;
            }
            Err(err) => {
                canisters.push(json!({ "canister_id": id, "allowed_methods": e.allowed, "error": err }));
                continue;
            }
        };
        let any = e.allowed.iter().any(|m| m == "*");
        let methods: Vec<(&String, &Function)> = svc
            .env
            .as_service(&svc.actor)
            .unwrap_or_default()
            .iter()
            .filter(|(name, _)| any || e.allowed.contains(name))
            .filter_map(|(name, t)| svc.env.as_func(t).ok().map(|f| (name, f)))
            .collect();

        let mut shown = vec![];
        let mut vars = BTreeSet::new();
        for (name, f) in methods {
            let sig = signature(f);
            let hay = format!("{name} {sig}").to_lowercase();
            if !terms.iter().all(|t| hay.contains(t)) {
                continue;
            }
            if budget == 0 {
                omitted += 1;
                continue;
            }
            budget -= 1;
            f.args.iter().chain(&f.rets).for_each(|t| collect_vars(&svc.env, t, &mut vars));
            shown.push(json!({ "name": name, "signature": sig }));
        }
        // With a keyword filter, skip canisters that have no matching method.
        if shown.is_empty() && !terms.is_empty() {
            continue;
        }
        let types: serde_json::Map<String, Value> = vars
            .into_iter()
            .filter_map(|v| svc.env.find_type(&v).ok().map(|t| (v, Value::String(type_text(t)))))
            .collect();
        canisters.push(json!({ "canister_id": id, "allowed_methods": e.allowed, "methods": shown, "types": types }));
    }

    let mut out = json!({ "canisters": canisters });
    if omitted > 0 {
        out["omitted_methods"] = json!(omitted);
        out["note"] = json!(format!("{omitted} more matching methods; narrow the query or pass canister_id"));
    } else if canisters.is_empty() {
        out["note"] = json!(match (&args.canister_id, terms.is_empty()) {
            (Some(id), _) if entries.is_empty() => format!("canister {id} is not allowlisted"),
            (_, false) => "no allowed method matches the query".to_string(),
            _ => "the allowlist is empty; a controller adds canisters with set_allowlist".to_string(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use candid::Principal;

    const LEDGER: &str = "ryjl3-tyaaa-aaaaa-aaaba-cai";
    const DID: &str = r#"
        type Subaccount = blob;
        type Account = record { owner : principal; subaccount : opt Subaccount };
        type TransferArg = record { to : Account; amount : nat };
        type TransferError = variant { InsufficientFunds : record { balance : nat }; TooOld };
        service : {
          icrc1_balance_of : (Account) -> (nat) query;
          icrc1_symbol : () -> (text) query;
          icrc1_transfer : (TransferArg) -> (variant { Ok : nat; Err : TransferError });
        }
    "#;

    fn entry(allowed: &[&str], service: Result<Option<Rc<Service>>, String>) -> Entry {
        Entry {
            canister: Principal::from_text(LEDGER).unwrap(),
            allowed: allowed.iter().map(|s| s.to_string()).collect(),
            service,
        }
    }

    fn svc() -> Result<Option<Rc<Service>>, String> {
        Ok(Some(Rc::new(Service::parse(DID).unwrap())))
    }

    fn search(q: Option<&str>, allowed: &[&str]) -> Value {
        render(&SearchArgs { query: q.map(Into::into), canister_id: None }, &[entry(allowed, svc())])
    }

    #[test]
    fn lists_only_allowed_methods_with_referenced_types() {
        let out = search(None, &["icrc1_balance_of", "icrc1_symbol"]);
        let c = &out["canisters"][0];
        assert_eq!(c["canister_id"], LEDGER);
        let names: Vec<_> = c["methods"].as_array().unwrap().iter().map(|m| m["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["icrc1_balance_of", "icrc1_symbol"]); // icrc1_transfer not allowed
        assert_eq!(c["methods"][0]["signature"], "(Account) -> (nat) query");
        // Transitive: Account -> Subaccount; nothing from the hidden transfer method.
        assert_eq!(c["types"]["Account"], "record { owner : principal; subaccount : opt Subaccount }");
        assert_eq!(c["types"]["Subaccount"], "blob");
        assert!(c["types"].get("TransferError").is_none());
    }

    #[test]
    fn keyword_filter_matches_names_and_signatures() {
        let all = ["*"];
        let names = |v: &Value| -> Vec<String> {
            v["canisters"][0]["methods"].as_array().map_or(vec![], |a| a.iter().map(|m| m["name"].as_str().unwrap().to_string()).collect())
        };
        assert_eq!(names(&search(Some("TRANSFER"), &all)), ["icrc1_transfer"]);
        assert_eq!(names(&search(Some("query text"), &all)), ["icrc1_symbol"]); // every term must match
        let out = search(Some("nonexistent"), &all);
        assert_eq!(out["canisters"], json!([]));
        assert_eq!(out["note"], "no allowed method matches the query");
        assert!(search(Some("transfer"), &all)["canisters"][0]["types"]["TransferError"].is_string());
    }

    #[test]
    fn scope_canonicalises_and_reports_unknowns() {
        let p = Principal::from_text(LEDGER).unwrap();
        let allow = vec![(p, vec!["*".to_string()]), (Principal::management_canister(), vec!["raw_rand".to_string()])];
        let a = SearchArgs { query: None, canister_id: Some("RYJL3-TYAAA-AAAAA-AAABA-CAI".into()) };
        assert_eq!(scope(&a, allow.clone()), vec![(p, vec!["*".to_string()])]);
        let a = SearchArgs { query: None, canister_id: Some("not-a-principal".into()) };
        assert!(scope(&a, allow.clone()).is_empty());
        assert_eq!(render(&a, &[])["note"], "canister not-a-principal is not allowlisted");
        assert_eq!(scope(&SearchArgs::default(), allow).len(), 2);
        assert_eq!(render(&SearchArgs::default(), &[])["note"], "the allowlist is empty; a controller adds canisters with set_allowlist");
    }

    #[test]
    fn unfetched_and_failed_interfaces_and_cap() {
        let out = render(&SearchArgs::default(), &[entry(&["*"], Ok(None)), entry(&["*"], Err("boom".into()))]);
        assert_eq!(out["canisters"][0]["error"], "interface not fetched");
        assert_eq!(out["canisters"][1]["error"], "boom");
        let many: Vec<Entry> = (0..20).map(|_| entry(&["*"], svc())).collect(); // 60 methods
        let out = render(&SearchArgs::default(), &many);
        let shown: usize = out["canisters"].as_array().unwrap().iter().map(|c| c["methods"].as_array().unwrap().len()).sum();
        assert_eq!((shown, out["omitted_methods"].as_u64()), (MAX_METHODS, Some(20)));
    }
}
