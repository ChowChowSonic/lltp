//! Pointee recovery (SoW 2.2.1, scalar part). LLVM pointers are opaque, so
//! every pointer starts out as `Ptr(unresolved)`. A pointer is typed from what
//! is stored into it: `p = &x` gives `p` the type `Ptr(type of x)`, and a copy
//! `q = p` hands the pointee on, in either direction. Rules run to a fixpoint.
//!
//! shortcut: the first typed value wins and a pointer is never retyped, so a
//! pointer used for two different pointees keeps the first; revisit with the
//! GEP/aggregate step, where conflicting uses must be reported.

use std::collections::BTreeMap;

use super::coalesce::visit_stmt_vars_mut;
use super::expr::{Expr, UnaryExpr, VarExpr};
use super::func::Function;
use super::op::UnOp;
use super::stmt::LetStmt;
use super::ty::Ty;
use crate::hir::Stmt;

type Env = BTreeMap<String, Ty>;

fn is_unresolved_ptr(t: &Ty) -> bool {
    matches!(t, Ty::Ptr(_)) && t.has_unresolved_pointee()
}

fn is_resolved_ptr(t: &Ty) -> bool {
    matches!(t, Ty::Ptr(_)) && !t.has_unresolved_pointee()
}

/// The variable a `Let` writes: `x`, or the cell `AddrOf(x)`.
fn written_var(e: &Expr) -> Option<&str> {
    match e {
        Expr::Var(VarExpr { name, .. }) => Some(name),
        Expr::UnaryOp(UnaryExpr {
            op: UnOp::AddrOf,
            arg,
        }) => match arg.as_ref() {
            Expr::Var(VarExpr { name, .. }) => Some(name),
            _ => None,
        },
        _ => None,
    }
}

/// The variable a `Let` reads: `x`, or the cell `ValuAt(x)`.
fn read_var(e: &Expr) -> Option<&str> {
    match e {
        Expr::Var(VarExpr { name, .. }) => Some(name),
        Expr::UnaryOp(UnaryExpr {
            op: UnOp::ValuAt,
            arg,
        }) => match arg.as_ref() {
            Expr::Var(VarExpr { name, .. }) => Some(name),
            _ => None,
        },
        _ => None,
    }
}

/// The type of a value read from `e`, using the types learned so far.
fn value_ty(e: &Expr, env: &Env) -> Ty {
    if let Expr::UnaryOp(UnaryExpr {
        op: UnOp::AddrOf,
        arg,
    }) = e
        && let Expr::Var(VarExpr { name, .. }) = arg.as_ref()
        && let Some(t) = env.get(name)
    {
        return Ty::Ptr(Box::new(t.clone()));
    }
    match read_var(e).and_then(|n| env.get(n)) {
        Some(t) => t.clone(),
        None => e.ty(),
    }
}

/// Apply the rules to one `dest = src`; true when a pointer got its pointee.
fn learn(env: &mut Env, dest: &Expr, src: &Expr) -> bool {
    if let Some(d) = written_var(dest)
        && env.get(d).is_some_and(is_unresolved_ptr)
    {
        let t = value_ty(src, env);
        if is_resolved_ptr(&t) {
            env.insert(d.to_string(), t);
            return true;
        }
    }
    if let Some(s) = read_var(src)
        && env.get(s).is_some_and(is_unresolved_ptr)
        && let Some(t) = written_var(dest).and_then(|d| env.get(d)).cloned()
        && is_resolved_ptr(&t)
    {
        env.insert(s.to_string(), t);
        return true;
    }
    false
}

/// Give every pointer parameter and local a pointee type where the function's
/// assignments determine one. Returns how many pointers were resolved.
pub fn resolve_pointees(func: &mut Function) -> usize {
    let mut env: Env = func
        .sig
        .params
        .iter()
        .map(|p| (p.name.clone(), p.ty.clone()))
        .chain(func.locals.iter().cloned())
        .collect();
    let lets: Vec<(Expr, Expr)> = func
        .blocks()
        .iter()
        .flat_map(|b| b.stmts.iter())
        .filter_map(|s| match s {
            Stmt::Let(LetStmt { dest, src }) => Some(((**dest).clone(), (**src).clone())),
            _ => None,
        })
        .collect();

    // Each success resolves one pointer for good, so this stops.
    let mut resolved = 0;
    loop {
        let before = resolved;
        for (dest, src) in &lets {
            if learn(&mut env, dest, src) {
                resolved += 1;
            }
        }
        if resolved == before {
            break;
        }
    }
    if resolved == 0 {
        return 0;
    }

    for p in &mut func.sig.params {
        if let Some(t) = env.get(&p.name) {
            p.ty = t.clone();
        }
    }
    for (name, ty) in &mut func.locals {
        if let Some(t) = env.get(name) {
            *ty = t.clone();
        }
    }
    for block in func.blocks_mut() {
        for stmt in &mut block.stmts {
            visit_stmt_vars_mut(stmt, &mut |v| {
                if v.dtype.has_unresolved_pointee()
                    && let Some(t) = env.get(&v.name)
                {
                    v.dtype = t.clone();
                }
            });
        }
    }
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::Cfg;
    use crate::hir::cfg::Block;

    fn i32t() -> Ty {
        Ty::Int(32, true)
    }

    fn var(name: &str, ty: &Ty) -> Expr {
        Expr::var(name, ty.clone())
    }

    fn addr(name: &str, ty: &Ty) -> Expr {
        Expr::unary(UnOp::AddrOf, var(name, ty))
    }

    fn let_(dest: Expr, src: Expr) -> Stmt {
        Stmt::let_binding(dest, src)
    }

    fn function(locals: Vec<(&str, Ty)>, stmts: Vec<Stmt>) -> Function {
        let mut f = Function::new("f", vec![], Ty::Void, Cfg::empty("entry"));
        f.locals = locals
            .into_iter()
            .map(|(n, t)| (n.to_string(), t))
            .collect();
        f.cfg
            .push_block(Block::build("entry", stmts, Some(Stmt::ret_void())));
        f.cfg.link_edges();
        f
    }

    fn local<'a>(f: &'a Function, name: &str) -> &'a Ty {
        &f.locals.iter().find(|(n, _)| n == name).unwrap().1
    }

    #[test]
    fn address_of_a_variable_types_the_pointer() {
        let up = Ty::unresolved_ptr();
        let mut f = function(
            vec![("x", i32t()), ("p", up.clone())],
            vec![let_(var("p", &up), addr("x", &i32t()))],
        );
        assert_eq!(resolve_pointees(&mut f), 1);
        let want = Ty::Ptr(Box::new(i32t()));
        assert_eq!(local(&f, "p"), &want);
        // The reference inside the statement is retyped too.
        let blocks = f.blocks();
        let Stmt::Let(LetStmt { dest, .. }) = &blocks[0].stmts[0] else {
            panic!("expected a Let");
        };
        assert_eq!(dest.ty(), want);
    }

    #[test]
    fn pointee_flows_through_copies_in_any_statement_order() {
        let up = Ty::unresolved_ptr();
        // q = p comes before p = &x, so one pass is not enough.
        let mut f = function(
            vec![("x", i32t()), ("p", up.clone()), ("q", up.clone())],
            vec![
                let_(var("q", &up), var("p", &up)),
                let_(var("p", &up), addr("x", &i32t())),
            ],
        );
        assert_eq!(resolve_pointees(&mut f), 2);
        let want = Ty::Ptr(Box::new(i32t()));
        assert_eq!(local(&f, "p"), &want);
        assert_eq!(local(&f, "q"), &want);
    }

    #[test]
    fn a_pointer_stored_through_a_cell_is_typed() {
        let up = Ty::unresolved_ptr();
        // *slot = &x, where `slot` is an unpromoted pointer-typed alloca.
        let mut f = function(
            vec![("x", i32t()), ("slot", up.clone())],
            vec![let_(addr("slot", &up), addr("x", &i32t()))],
        );
        assert_eq!(resolve_pointees(&mut f), 1);
        assert_eq!(local(&f, "slot"), &Ty::Ptr(Box::new(i32t())));
    }

    #[test]
    fn nothing_to_learn_leaves_types_alone() {
        let up = Ty::unresolved_ptr();
        let mut f = function(
            vec![("p", up.clone()), ("q", up.clone())],
            vec![let_(var("q", &up), var("p", &up))],
        );
        assert_eq!(resolve_pointees(&mut f), 0);
        assert_eq!(local(&f, "p"), &up);
        assert_eq!(local(&f, "q"), &up);
    }

    #[test]
    fn the_first_pointee_wins() {
        let up = Ty::unresolved_ptr();
        let c = Ty::Int(8, true);
        let mut f = function(
            vec![("x", i32t()), ("c", c.clone()), ("p", up.clone())],
            vec![
                let_(var("p", &up), addr("x", &i32t())),
                let_(var("p", &up), addr("c", &c)),
            ],
        );
        assert_eq!(resolve_pointees(&mut f), 1);
        assert_eq!(local(&f, "p"), &Ty::Ptr(Box::new(i32t())));
    }
}
