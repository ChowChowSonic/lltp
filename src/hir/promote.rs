//! At `-O0`, clang keeps every source variable in an `alloca` slot and talks to
//! it through `load`/`store`. A slot whose address is never taken is just a
//! variable, so its loads and stores are plain copies. `promotable_slots`
//! decides which slots qualify, and `promote_slots` rewrites their accesses
//! into copies so the coalescer can merge the temporaries into the slot.

use std::collections::{BTreeMap, BTreeSet};

use super::coalesce::{inst_vals, term_vals};
use super::flat::{FlatFunction, Inst, Val};
use super::ty::Ty;

/// A slot qualifies when it is only reached through `load`/`store`, has a
/// scalar type, and every access uses exactly that type.
/// A slot is rejected if its address escapes (`Val::Addr` as any operand)
/// or if it is accessed through a different type.
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

/// Rewrite loads and stores of promotable slots into plain copies.
pub fn promote_slots(func: &mut FlatFunction) -> usize {
    let slots = promotable_slots(func);
    if slots.is_empty() {
        return 0;
    }
    let ty_of: BTreeMap<String, Ty> = func.locals.iter().cloned().collect();

    for block in &mut func.blocks {
        for inst in &mut block.insts {
            let copy = match &*inst {
                Inst::Load { dest, ptr } if slots.contains(ptr.as_str()) => Some(Inst::Copy {
                    dest: dest.clone(),
                    src: Val::Var(ptr.clone(), ty_of[ptr.as_str()].clone()),
                }),
                Inst::Store { ptr, value } if slots.contains(ptr.as_str()) => Some(Inst::Copy {
                    dest: ptr.clone(),
                    src: value.clone(),
                }),
                _ => None,
            };
            if let Some(copy) = copy {
                *inst = copy;
            }
        }
    }
    slots.len()
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
    use crate::hir::coalesce::coalesce;
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

    #[test]
    fn promotion_turns_slot_accesses_into_copies() {
        let mut f = function(
            vec![("x", i32t()), ("t0", i32t())],
            vec![store("x", int(1)), load("t0", "x")],
            ret("t0"),
        );
        assert_eq!(promote_slots(&mut f), 1);
        let insts = &f.blocks[0].insts;
        assert!(matches!(
            &insts[0],
            Inst::Copy { dest, src: Val::Int { value: 1, .. } } if dest == "x"
        ));
        assert!(matches!(
            &insts[1],
            Inst::Copy { dest, src: Val::Var(src, _) } if dest == "t0" && src == "x"
        ));
    }

    #[test]
    fn promotion_leaves_escaping_slots_alone() {
        let mut f = function(
            vec![("x", i32t()), ("t0", i32t())],
            vec![
                store("x", int(1)),
                Inst::Call {
                    dest: None,
                    callee: "use_ptr".to_string(),
                    args: vec![Val::Addr("x".to_string())],
                },
                load("t0", "x"),
            ],
            ret("t0"),
        );
        assert_eq!(promote_slots(&mut f), 0);
        let insts = &f.blocks[0].insts;
        assert!(matches!(&insts[0], Inst::Store { .. }));
        assert!(matches!(&insts[2], Inst::Load { .. }));
    }

    #[test]
    fn promoted_temporary_merges_back_into_the_slot() {
        let mut f = function(
            vec![("x", i32t()), ("t0", i32t())],
            vec![store("x", int(1)), load("t0", "x")],
            ret("t0"),
        );
        promote_slots(&mut f);
        assert_eq!(coalesce(&mut f), 1);

        let names: Vec<&str> = f.locals.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["x"]);
        let insts = &f.blocks[0].insts;
        assert_eq!(insts.len(), 1);
        assert!(matches!(&insts[0], Inst::Copy { dest, .. } if dest == "x"));
        assert!(matches!(
            &f.blocks[0].term,
            Term::Ret(Some(Val::Var(name, _))) if name == "x"
        ));
    }

    #[test]
    fn temporary_stays_separate_when_the_slot_changes_while_it_is_live() {
        // t0 = x; x = 2; ret t0  -- t0 still holds the old value after x changes.
        let mut f = function(
            vec![("x", i32t()), ("t0", i32t())],
            vec![store("x", int(1)), load("t0", "x"), store("x", int(2))],
            ret("t0"),
        );
        promote_slots(&mut f);
        assert_eq!(coalesce(&mut f), 0);
        assert_eq!(f.locals.len(), 2);
    }
}
