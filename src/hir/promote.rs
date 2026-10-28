//! At `-O0`, clang keeps every source variable in an `alloca` slot and talks to
//! it through `load`/`store`. A slot whose address is never taken is just a
//! variable, so its loads and stores are plain copies. `promotable_slots`
//! decides which slots qualify, and `promote_slots` rewrites their accesses
//! into copies so the coalescer can merge the temporaries into the slot.

use std::collections::{BTreeMap, BTreeSet};

use super::coalesce::{children, exprs, slot_name};
use super::expr::{Expr, UnaryExpr, VarExpr};
use super::flat::FlatFunction;
use super::op::UnOp;
use super::stmt::LetStmt;
use super::ty::Ty;
use crate::hir::Stmt;

/// A slot qualifies when it is only reached through `load`/`store`, has a
/// scalar type, and every access uses exactly that type.
/// A slot is rejected if its address escapes (an `AddrOf` of it anywhere)
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
        for stmt in &block.stmts {
            let Stmt::Let(LetStmt { dest, src }) = stmt else {
                for e in exprs(stmt) {
                    note_escapes(e, &mut rejected);
                }
                continue;
            };
            // A `Let` whose place is a slot is a store; one whose value reads a
            // slot is a load. The other side must match the slot's recorded type.
            for (slot, other) in [(dest, src), (src, dest)] {
                if let Some(slot) = slot_name(slot) {
                    note_slot(&mut slots, &mut rejected, slot, &ty_of, Some(other));
                }
            }
            // The place being written is the store itself, so an `AddrOf` there
            // is not an escape; only the value can leak the slot's address.
            if let Expr::UnaryOp(UnaryExpr {
                op: UnOp::AddrOf,
                arg,
            }) = dest.as_ref()
            {
                note_escapes(arg, &mut rejected);
            } else {
                note_escapes(dest, &mut rejected);
            }
            note_escapes(src, &mut rejected);
        }
        for e in exprs(&block.term()) {
            note_escapes(e, &mut rejected);
        }
    }

    slots
        .into_iter()
        .filter(|name| !rejected.contains(name))
        .filter(|name| ty_of.get(name.as_str()).is_some_and(|ty| is_scalar(ty)))
        .collect()
}

/// Record that `slot` is accessed, and check the value being read or written
/// against the type the slot was recorded with.
fn note_slot(
    slots: &mut BTreeSet<String>,
    rejected: &mut BTreeSet<String>,
    slot: &str,
    ty_of: &BTreeMap<&str, &Ty>,
    other: Option<&Expr>,
) {
    slots.insert(slot.to_string());
    let Some(own) = ty_of.get(slot) else {
        return;
    };
    match other.map(|e| value_ty(e, ty_of)) {
        Some(ty) if ty == **own => {}
        _ => {
            rejected.insert(slot.to_string());
        }
    }
}

/// The type an expression evaluates to, preferring the type `locals` recorded
/// for a name over the one written into the node.
fn value_ty(e: &Expr, ty_of: &BTreeMap<&str, &Ty>) -> Ty {
    match e {
        Expr::Var(v) => match ty_of.get(v.name.as_str()) {
            Some(t) => (*t).clone(),
            None => v.dtype.clone(),
        },
        Expr::UnaryOp(UnaryExpr {
            op: UnOp::ValuAt,
            arg,
        }) => value_ty(arg, ty_of),
        Expr::UnaryOp(UnaryExpr {
            op: UnOp::AddrOf,
            arg,
        }) => Ty::Ptr(Box::new(value_ty(arg, ty_of))),
        other => other.ty(),
    }
}

/// Rewrite `ValuAt(slot)` and `AddrOf(slot)` of promotable slots into plain
/// variable references.
pub fn promote_slots(func: &mut FlatFunction) -> usize {
    let slots = promotable_slots(func);
    if slots.is_empty() {
        return 0;
    }
    let ty_of: BTreeMap<String, Ty> = func.locals.iter().cloned().collect();

    let rewrite = |e: &mut Expr, ty_of: &BTreeMap<String, Ty>| {
        if let Expr::UnaryOp(UnaryExpr {
            op: UnOp::ValuAt | UnOp::AddrOf,
            arg,
        }) = e
            && let Expr::Var(VarExpr { name, .. }) = arg.as_ref()
            && slots.contains(name)
        {
            *e = Expr::Var(VarExpr {
                name: name.clone(),
                dtype: ty_of.get(name).cloned().unwrap_or(Ty::Void),
            });
        }
    };

    for block in &mut func.blocks {
        for stmt in &mut block.stmts {
            let Stmt::Let(LetStmt { dest, src }) = stmt else {
                continue;
            };
            rewrite(dest.as_mut(), &ty_of);
            rewrite(src.as_mut(), &ty_of);
        }
    }
    slots.len()
}

fn note_escapes(e: &Expr, rejected: &mut BTreeSet<String>) {
    if let Expr::UnaryOp(UnaryExpr {
        op: UnOp::AddrOf,
        arg,
    }) = e
        && let Expr::Var(VarExpr { name, .. }) = arg.as_ref()
    {
        rejected.insert(name.clone());
    }
    for child in children(e) {
        note_escapes(child, rejected);
    }
}

fn is_scalar(ty: &Ty) -> bool {
    matches!(ty, Ty::Bool | Ty::Int(..) | Ty::Float(_) | Ty::Ptr(_))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::Lit;
    use crate::hir::coalesce::coalesce;
    use crate::hir::flat::FnSig;
    use crate::hir::op::UnOp;
    use crate::hir::stmt::{LetStmt, RetStmt};
    use crate::hir::{Block, Expr, Stmt};

    fn i32t() -> Ty {
        Ty::Int(32, true)
    }

    fn var(n: &str) -> Expr {
        Expr::Var(VarExpr {
            name: n.to_string(),
            dtype: i32t(),
        })
    }

    fn int(value: i64) -> Expr {
        Expr::Literal(Lit::Int {
            value: value as u64,
            bits: 32,
            signed: true,
        })
    }

    fn put(dest: Expr, src: Expr) -> Stmt {
        Stmt::Let(LetStmt {
            dest: Box::new(dest),
            src: Box::new(src),
        })
    }

    /// `dest = *ptr`
    fn load(dest: &str, ptr: &str) -> Stmt {
        put(var(dest), Expr::unary(UnOp::ValuAt, var(ptr)))
    }

    /// `*ptr = value`
    fn store(ptr: &str, value: Expr) -> Stmt {
        put(Expr::unary(UnOp::AddrOf, var(ptr)), value)
    }

    fn call(dest: Option<&str>, callee: &str, args: Vec<Expr>) -> Stmt {
        let expr = Expr::Call(crate::hir::expr::CallExpr {
            name: callee.to_string(),
            args,
        });
        match dest {
            Some(d) => put(var(d), expr),
            None => Stmt::Eval(expr),
        }
    }

    fn function(locals: Vec<(&str, Ty)>, stmts: Vec<Stmt>, term: Stmt) -> FlatFunction {
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
            blocks: vec![Block::build("entry", stmts, Some(term))],
        }
    }

    fn ret(name: &str) -> Stmt {
        Stmt::Ret(RetStmt {
            value: Some(var(name)),
        })
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
                call(Some("r"), "scan", vec![Expr::unary(UnOp::AddrOf, var("x"))]),
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
            Stmt::Ret(RetStmt {
                value: Some(Expr::unary(UnOp::AddrOf, var("x"))),
            }),
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
            vec![put(
                var("x"),
                Expr::Literal(Lit::Int {
                    value: 1,
                    bits: 8,
                    signed: true,
                }),
            )],
            Stmt::Ret(RetStmt { value: None }),
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
                call(None, "use_ptr", vec![Expr::unary(UnOp::AddrOf, var("b"))]),
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
        let stmts = &f.blocks[0].stmts;
        assert!(
            matches!(&stmts[0], Stmt::Let(LetStmt { dest, src }) if is_name(dest, "x") && is_literal(src))
        );
        assert!(
            matches!(&stmts[1], Stmt::Let(LetStmt { dest, src }) if is_name(dest, "t0") && is_name(src, "x"))
        );
    }

    #[test]
    fn promotion_leaves_escaping_slots_alone() {
        let mut f = function(
            vec![("x", i32t()), ("t0", i32t())],
            vec![
                store("x", int(1)),
                call(None, "use_ptr", vec![Expr::unary(UnOp::AddrOf, var("x"))]),
                load("t0", "x"),
            ],
            ret("t0"),
        );
        assert_eq!(promote_slots(&mut f), 0);
        let stmts = &f.blocks[0].stmts;
        assert!(writes(&stmts[0], "x"));
        assert!(is_deref(&stmts[2], "x"));
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
        let stmts = &f.blocks[0].stmts;
        assert_eq!(stmts.len() - 1, 1);
        assert!(writes(&stmts[0], "x"));
        assert!(
            matches!(f.blocks[0].term(), Stmt::Ret(RetStmt { value: Some(v) }) if is_name(v, "x"))
        );
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

    // Small matchers so the assertions read as "which node is this".
    fn name_of(e: &Expr) -> Option<&str> {
        match e {
            Expr::Var(VarExpr { name, .. }) => Some(name),
            _ => None,
        }
    }

    fn is_name(e: &Expr, want: &str) -> bool {
        name_of(e) == Some(want)
    }

    fn is_literal(e: &Expr) -> bool {
        matches!(e, Expr::Literal(..))
    }

    /// The statement writes to `want`, as a plain name or as a cell.
    fn writes(stmt: &Stmt, want: &str) -> bool {
        matches!(stmt, Stmt::Let(LetStmt { dest, .. }) if is_name(dest, want) || is_place_expr(dest, want))
    }

    fn is_place_expr(e: &Expr, want: &str) -> bool {
        matches!(e, Expr::UnaryOp(UnaryExpr { op: UnOp::AddrOf, arg }) if is_name(arg, want))
    }

    fn is_deref(stmt: &Stmt, want: &str) -> bool {
        matches!(stmt, Stmt::Let(LetStmt { src, .. }) if matches!(src.as_ref(), Expr::UnaryOp(UnaryExpr { op: UnOp::ValuAt, arg }) if is_name(arg, want)))
    }
}
