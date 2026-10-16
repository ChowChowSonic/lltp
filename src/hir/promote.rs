use std::collections::{BTreeMap, BTreeSet};

use super::coalesce::{inst_vals, term_vals};
use super::flat::{FlatFunction, Inst, Val};
use super::ty::Ty;

pub fn promotable_slots(func: &FlatFunction) -> BTreeSet<String> {
    let ty_of: BTreeMap<&str, &Ty> = func
        .locals
        .iter()
        .map(|(name, ty)| (name.as_str(), ty))
        .collect();

    let mut slots: BTreeSet<String> = BTreeSet::new();
    let mut rejected: BTreeSet<String> = BTreeSet::new();

    for block in &func.blocks {
        for inst in &block.insts {
            match inst {
                Inst::Load { dest, ptr } => {
                    slots.insert(ptr.clone());
                    if ty_of.get(dest.as_str()) != ty_of.get(ptr.as_str()) {
                        rejected.insert(ptr.clone());
                    }
                }
                Inst::Store { ptr, value } => {
                    slots.insert(ptr.clone());
                    match ty_of.get(ptr.as_str()) {
                        Some(slot_ty) if **slot_ty == value.ty() => {}
                        _ => {
                            rejected.insert(ptr.clone());
                        }
                    }
                }
                _ => {}
            }
            note_escapes(inst_vals(inst), &mut rejected);
        }
        note_escapes(term_vals(&block.term), &mut rejected);
    }

    slots
        .into_iter()
        .filter(|name| !rejected.contains(name))
        .filter(|name| ty_of.get(name.as_str()).is_some_and(|ty| is_scalar(ty)))
        .collect()
}

fn note_escapes(vals: Vec<&Val>, rejected: &mut BTreeSet<String>) {
    for v in vals {
        if let Val::Addr(name) = v {
            rejected.insert(name.clone());
        }
    }
}

fn is_scalar(ty: &Ty) -> bool {
    matches!(ty, Ty::Bool | Ty::Int(..) | Ty::Float(_) | Ty::Ptr(_))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::flat::{FlatBlock, FnSig, Term};

    fn i32t() -> Ty {
        Ty::Int(32, true)
    }

    fn int(value: i64) -> Val {
        Val::Int { value, bits: 32 }
    }

    fn load(dest: &str, ptr: &str) -> Inst {
        Inst::Load {
            dest: dest.to_string(),
            ptr: ptr.to_string(),
        }
    }

    fn store(ptr: &str, value: Val) -> Inst {
        Inst::Store {
            ptr: ptr.to_string(),
            value,
        }
    }

    fn function(locals: Vec<(&str, Ty)>, insts: Vec<Inst>, term: Term) -> FlatFunction {
        FlatFunction {
            sig: FnSig {
                name: "f".to_string(),
                params: vec![],
                ret: i32t(),
                variadic: false,
            },
            param_names: vec![],
            locals: locals
                .into_iter()
                .map(|(n, t)| (n.to_string(), t))
                .collect(),
            blocks: vec![FlatBlock {
                label: "entry".to_string(),
                insts,
                term,
            }],
        }
    }

    fn ret(name: &str) -> Term {
        Term::Ret(Some(Val::Var(name.to_string(), i32t())))
    }

    fn names(set: &BTreeSet<String>) -> Vec<&str> {
        set.iter().map(String::as_str).collect()
    }

    #[test]
    fn plain_load_store_slot_is_promotable() {
        let f = function(
            vec![("x", i32t()), ("t0", i32t())],
            vec![store("x", int(1)), load("t0", "x")],
            ret("t0"),
        );
        assert_eq!(names(&promotable_slots(&f)), ["x"]);
    }

    #[test]
    fn slot_with_escaping_address_is_rejected() {
        let f = function(
            vec![("x", i32t()), ("t0", i32t()), ("r", i32t())],
            vec![
                store("x", int(1)),
                Inst::Call {
                    dest: Some("r".to_string()),
                    callee: "scan".to_string(),
                    args: vec![Val::Addr("x".to_string())],
                },
                load("t0", "x"),
            ],
            ret("t0"),
        );
        assert!(promotable_slots(&f).is_empty());
    }

    #[test]
    fn address_returned_from_terminator_is_rejected() {
        let f = function(
            vec![("x", i32t())],
            vec![store("x", int(1))],
            Term::Ret(Some(Val::Addr("x".to_string()))),
        );
        assert!(promotable_slots(&f).is_empty());
    }

    #[test]
    fn mismatched_access_type_is_rejected() {
        let f = function(
            vec![("x", i32t()), ("b", Ty::Int(8, true))],
            vec![store("x", int(1)), load("b", "x")],
            ret("b"),
        );
        assert!(promotable_slots(&f).is_empty());
    }

    #[test]
    fn mismatched_store_type_is_rejected() {
        let f = function(
            vec![("x", i32t())],
            vec![store("x", Val::Int { value: 1, bits: 8 })],
            Term::Ret(None),
        );
        assert!(promotable_slots(&f).is_empty());
    }

    #[test]
    fn non_scalar_slot_is_rejected() {
        let f = function(
            vec![("arr", Ty::Array(Box::new(i32t()), 4)), ("t0", i32t())],
            vec![load("t0", "arr")],
            ret("t0"),
        );
        assert!(promotable_slots(&f).is_empty());
    }

    #[test]
    fn one_bad_slot_does_not_reject_the_others() {
        let f = function(
            vec![("a", i32t()), ("b", i32t()), ("t0", i32t())],
            vec![
                store("a", int(1)),
                store("b", int(2)),
                Inst::Call {
                    dest: None,
                    callee: "use_ptr".to_string(),
                    args: vec![Val::Addr("b".to_string())],
                },
                load("t0", "a"),
            ],
            ret("t0"),
        );
        assert_eq!(names(&promotable_slots(&f)), ["a"]);
    }
}
