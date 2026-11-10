//! Condition-based refinement (SoW 2.3.4): group guarded nodes with
//! complementary reaching conditions into if/else.

use crate::hir::{Expr, Stmt};

/// True when `a` and `b` are exact opposites: one is the other's negation,
/// or both are boolean constants of opposite value.
// shortcut: syntactic match only, so `x < y` and `x >= y` are not seen as
// complementary; upgrade if refinement needs to group raw comparison guards.
pub fn complementary(a: &Expr, b: &Expr) -> bool {
    Expr::negate(a.clone()) == *b || Expr::negate(b.clone()) == *a
}

/// Names of the variables `e` reads, or false when `e` has a shape this check
/// does not understand.
fn read_vars(e: &Expr, out: &mut Vec<String>) -> bool {
    match e {
        Expr::Literal(_) => true,
        Expr::Var(v) => {
            out.push(v.name.clone());
            true
        }
        Expr::UnaryOp(u) => read_vars(&u.arg, out),
        Expr::BinaryOp(b) => read_vars(&b.arg1, out) && read_vars(&b.arg2, out),
        Expr::Cmp(c) => read_vars(&c.arg1, out) && read_vars(&c.arg2, out),
        _ => false,
    }
}

/// True when `stmts` may assign one of `names`. Statements that can hide
/// writes (try/catch, invoke) count as writing.
fn writes_any(stmts: &[Stmt], names: &[String]) -> bool {
    stmts.iter().any(|s| match s {
        Stmt::Let(l) => matches!(&*l.dest, Expr::Var(v) if names.contains(&v.name)),
        Stmt::If(i) => writes_any(&i.then_stmts, names) || writes_any(&i.else_stmts, names),
        Stmt::Loop(l) => writes_any(&l.body, names),
        Stmt::Switch(sw) => {
            sw.cases.iter().any(|(_, b)| writes_any(b, names)) || writes_any(&sw.default, names)
        }
        Stmt::TryCatch(_) | Stmt::Invoke(_) => true,
        _ => false,
    })
}

/// Fold each `if (c) {A}` that is directly followed by `if (!c) {B}` into
/// `if (c) {A} else {B}`. The merge is skipped when `A` could change what `c`
/// reads, since the second guard would then see a different value.
// shortcut: only directly adjacent pairs are merged, and arms are not
// refined recursively; upgrade when guarded nodes interleave or nest.
pub fn merge_complementary(stmts: Vec<Stmt>) -> Vec<Stmt> {
    let mut out: Vec<Stmt> = Vec::new();
    for s in stmts {
        if let Some(Stmt::If(prev)) = out.last_mut()
            && let Stmt::If(next) = &s
            && prev.else_stmts.is_empty()
            && next.else_stmts.is_empty()
            && complementary(&prev.cond, &next.cond)
        {
            let mut names = Vec::new();
            if read_vars(&prev.cond, &mut names) && !writes_any(&prev.then_stmts, &names) {
                prev.else_stmts = next.then_stmts.clone();
                continue;
            }
        }
        out.push(s);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::Ty;

    fn flag(name: &str) -> Expr {
        Expr::var(name, Ty::Bool)
    }

    #[test]
    fn a_flag_and_its_negation_are_complementary() {
        let c = flag("entry.cond");
        assert!(complementary(&c, &Expr::negate(c.clone())));
        assert!(complementary(&Expr::negate(c.clone()), &c));
    }

    #[test]
    fn boolean_constants_are_complementary() {
        assert!(complementary(&Expr::bool(true), &Expr::bool(false)));
    }

    #[test]
    fn unrelated_or_identical_guards_are_not() {
        let (a, b) = (flag("a.cond"), flag("b.cond"));
        assert!(!complementary(&a, &b));
        assert!(!complementary(&a, &a));
        assert!(!complementary(&a, &Expr::negate(b)));
    }

    fn mark(name: &str) -> Stmt {
        Stmt::Eval(flag(name))
    }

    fn guarded(cond: Expr, name: &str) -> Stmt {
        Stmt::if_then_else(cond, vec![mark(name)], Vec::new())
    }

    #[test]
    fn complementary_neighbours_become_if_else() {
        let c = flag("entry.cond");
        let out = merge_complementary(vec![
            guarded(c.clone(), "a"),
            guarded(Expr::negate(c.clone()), "b"),
        ]);
        assert_eq!(
            out,
            [Stmt::if_then_else(c, vec![mark("a")], vec![mark("b")])]
        );
    }

    #[test]
    fn other_statements_and_unrelated_guards_are_kept() {
        let (c, d) = (flag("c"), flag("d"));
        let input = vec![
            mark("pre"),
            guarded(c.clone(), "a"),
            guarded(d, "b"),
            guarded(Expr::negate(c), "c"),
        ];
        assert_eq!(merge_complementary(input.clone()), input);
    }

    #[test]
    fn pairs_with_a_non_empty_else_are_kept() {
        let c = flag("c");
        let first = Stmt::if_then_else(c.clone(), vec![mark("a")], vec![mark("x")]);
        let input = vec![first, guarded(Expr::negate(c), "b")];
        assert_eq!(merge_complementary(input.clone()), input);
    }

    #[test]
    fn no_merge_when_the_first_arm_rewrites_the_guard() {
        let c = flag("c");
        let rewrite = Stmt::let_binding(c.clone(), Expr::bool(false));
        let input = vec![
            Stmt::if_then_else(c.clone(), vec![rewrite], Vec::new()),
            guarded(Expr::negate(c), "b"),
        ];
        assert_eq!(merge_complementary(input.clone()), input);
    }
}
