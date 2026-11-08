//! Condition-based refinement (SoW 2.3.4): group guarded nodes with
//! complementary reaching conditions into if/else. Condition-aware refinement
//! (SoW 2.3.5): group guarded nodes that test one variable for equality into
//! a switch.

use crate::hir::acyclic::negate;
use crate::hir::expr::VarExpr;
use crate::hir::op::{Cmp, IntCmp};
use crate::hir::stmt::{IfStmt, SwitchStmt};
use crate::hir::{Expr, Lit, Stmt};

/// True when `a` and `b` are exact opposites: one is the other's negation,
/// in the form `negate` produces (`e` and `e ^ true`, or `true` and `false`).
// shortcut: syntactic match only, so `x < y` and `x >= y` are not seen as
// complementary; upgrade if refinement needs to group raw comparison guards.
pub fn complementary(a: &Expr, b: &Expr) -> bool {
    negate(a.clone()) == *b || negate(b.clone()) == *a
}

/// `(v, c)` when `cond` is `v == c` (either operand order) for a variable `v`
/// and an integer constant `c`: the shape one switch case is made from.
// shortcut: only integer constants compared with `==` against a plain variable
// are recognized; upgrade when switches over other literals are wanted.
pub fn equality_guard(cond: &Expr) -> Option<(&VarExpr, &Lit)> {
    let Expr::Cmp(c) = cond else {
        return None;
    };
    if c.pred != Cmp::Int(IntCmp::Eq) {
        return None;
    }
    let (v, l) = match (&*c.arg1, &*c.arg2) {
        (Expr::Var(v), Expr::Literal(l)) | (Expr::Literal(l), Expr::Var(v)) => (v, l),
        _ => return None,
    };
    matches!(l, Lit::Int { .. }).then_some((v, l))
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

/// True when `stmts` contain a `break` or `continue`, which would bind to a
/// switch instead of the loop they leave if the statements moved into one.
fn jumps_loop(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|s| match s {
        Stmt::Break | Stmt::Continue => true,
        Stmt::If(i) => jumps_loop(&i.then_stmts) || jumps_loop(&i.else_stmts),
        Stmt::Loop(l) => jumps_loop(&l.body),
        Stmt::Switch(sw) => sw.cases.iter().any(|(_, b)| jumps_loop(b)) || jumps_loop(&sw.default),
        Stmt::TryCatch(t) => {
            jumps_loop(&t.try_body)
                || t.catches.iter().any(|c| jumps_loop(&c.body))
                || t.finally_body.as_deref().is_some_and(jumps_loop)
        }
        _ => false,
    })
}

/// `(v, c)` when `i` can become the case `c` of a switch on `v`: a plain
/// `if (v == c) {..}` whose body leaves `v` alone and does not `break` or
/// `continue`.
fn case_guard(i: &IfStmt) -> Option<(&VarExpr, &Lit)> {
    let (v, c) = equality_guard(&i.cond)?;
    let fits = i.else_stmts.is_empty()
        && !writes_any(&i.then_stmts, std::slice::from_ref(&v.name))
        && !jumps_loop(&i.then_stmts);
    fits.then_some((v, c))
}

/// True when `i` tests the same variable as `run` for a constant that no
/// earlier case in `run` uses.
fn joins(run: &[IfStmt], i: &IfStmt) -> bool {
    let Some((first, _)) = run.first().and_then(case_guard) else {
        return false;
    };
    let Some((v, c)) = case_guard(i) else {
        return false;
    };
    first.name == v.name
        && run
            .iter()
            .all(|r| case_guard(r).is_some_and(|(_, c0)| c0 != c))
}

/// Emit `run` as one switch when it holds two or more cases, otherwise as the
/// plain `if`s it came from.
fn flush(run: &mut Vec<IfStmt>, out: &mut Vec<Stmt>) {
    let var = run
        .first()
        .and_then(|i| equality_guard(&i.cond))
        .map(|(v, _)| v.clone());
    let Some(var) = var.filter(|_| run.len() > 1) else {
        out.extend(run.drain(..).map(Stmt::If));
        return;
    };
    let cases = run
        .drain(..)
        .filter_map(|i| {
            let c = equality_guard(&i.cond)?.1.clone();
            Some((c, i.then_stmts))
        })
        .collect();
    out.push(Stmt::Switch(SwitchStmt {
        value: Expr::Var(var),
        cases,
        default: Vec::new(),
    }));
}

/// Fold a run of adjacent `if (v == c) {A}` statements on one variable into
/// `switch (v) { case c: A }`. Each constant may appear once, so at most one
/// guard can hold, and no body may rewrite `v`, so later guards still read
/// the value the first one did. A lone equality guard is left as it is.
// shortcut: only adjacent guards with an empty else are grouped, there is no
// default case, and a call inside a body is assumed not to change `v` (no
// alias analysis); upgrade when guards interleave or `v` can be aliased.
pub fn merge_equalities(stmts: Vec<Stmt>) -> Vec<Stmt> {
    let mut out = Vec::new();
    let mut run: Vec<IfStmt> = Vec::new();
    for s in stmts {
        let s = match s {
            Stmt::If(i) if joins(&run, &i) => {
                run.push(i);
                continue;
            }
            other => other,
        };
        flush(&mut run, &mut out);
        match s {
            Stmt::If(i) if case_guard(&i).is_some() => run.push(i),
            other => out.push(other),
        }
    }
    flush(&mut run, &mut out);
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
        assert!(complementary(&c, &negate(c.clone())));
        assert!(complementary(&negate(c.clone()), &c));
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
        assert!(!complementary(&a, &negate(b)));
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
            guarded(negate(c.clone()), "b"),
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
            guarded(negate(c), "c"),
        ];
        assert_eq!(merge_complementary(input.clone()), input);
    }

    #[test]
    fn pairs_with_a_non_empty_else_are_kept() {
        let c = flag("c");
        let first = Stmt::if_then_else(c.clone(), vec![mark("a")], vec![mark("x")]);
        let input = vec![first, guarded(negate(c), "b")];
        assert_eq!(merge_complementary(input.clone()), input);
    }

    #[test]
    fn no_merge_when_the_first_arm_rewrites_the_guard() {
        let c = flag("c");
        let rewrite = Stmt::let_binding(c.clone(), Expr::bool(false));
        let input = vec![
            Stmt::if_then_else(c.clone(), vec![rewrite], Vec::new()),
            guarded(negate(c), "b"),
        ];
        assert_eq!(merge_complementary(input.clone()), input);
    }

    fn eq(name: &str, value: u64) -> Expr {
        Expr::cmp(
            Cmp::Int(IntCmp::Eq),
            Expr::var(name, Ty::Int(32, true)),
            Expr::int(value, 32, true),
        )
    }

    #[test]
    fn a_variable_equal_to_a_constant_is_an_equality_guard() {
        let guard = eq("x", 3);
        let (v, l) = equality_guard(&guard).expect("x == 3 is an equality guard");
        assert_eq!(v.name, "x");
        assert_eq!(Expr::Literal(l.clone()), Expr::int(3, 32, true));

        let swapped = Expr::cmp(
            Cmp::Int(IntCmp::Eq),
            Expr::int(3, 32, true),
            Expr::var("x", Ty::Int(32, true)),
        );
        assert_eq!(
            equality_guard(&swapped).map(|(v, _)| v.name.as_str()),
            Some("x")
        );
    }

    #[test]
    fn other_comparisons_and_operands_are_not_equality_guards() {
        let x = || Expr::var("x", Ty::Int(32, true));
        let ne = Expr::cmp(Cmp::Int(IntCmp::Ne), x(), Expr::int(3, 32, true));
        let lt = Expr::cmp(Cmp::Int(IntCmp::Slt), x(), Expr::int(3, 32, true));
        let two_vars = Expr::cmp(Cmp::Int(IntCmp::Eq), x(), Expr::var("y", Ty::Int(32, true)));
        let on_bool = Expr::cmp(Cmp::Int(IntCmp::Eq), x(), Expr::bool(true));
        for e in [
            ne,
            lt,
            two_vars,
            on_bool,
            flag("entry.cond"),
            Expr::bool(true),
        ] {
            assert_eq!(equality_guard(&e), None);
        }
    }

    fn when_eq(name: &str, value: u64, body: Vec<Stmt>) -> Stmt {
        Stmt::if_then_else(eq(name, value), body, Vec::new())
    }

    fn lit(value: u64) -> Lit {
        Lit::Int {
            value,
            bits: 32,
            signed: true,
        }
    }

    #[test]
    fn adjacent_equalities_on_one_variable_become_a_switch() {
        let out = merge_equalities(vec![
            when_eq("x", 1, vec![mark("a")]),
            when_eq("x", 2, vec![mark("b")]),
        ]);
        assert_eq!(
            out,
            [Stmt::Switch(SwitchStmt {
                value: Expr::var("x", Ty::Int(32, true)),
                cases: vec![(lit(1), vec![mark("a")]), (lit(2), vec![mark("b")])],
                default: Vec::new(),
            })]
        );
    }

    #[test]
    fn a_lone_equality_stays_an_if() {
        let input = vec![mark("pre"), when_eq("x", 1, vec![mark("a")])];
        assert_eq!(merge_equalities(input.clone()), input);
    }

    #[test]
    fn other_variables_repeats_and_other_statements_split_the_run() {
        let by_var = vec![
            when_eq("x", 1, vec![mark("a")]),
            when_eq("y", 2, vec![mark("b")]),
        ];
        let repeat = vec![
            when_eq("x", 1, vec![mark("a")]),
            when_eq("x", 1, vec![mark("b")]),
        ];
        let split = vec![
            when_eq("x", 1, vec![mark("a")]),
            mark("between"),
            when_eq("x", 2, vec![mark("b")]),
        ];
        for input in [by_var, repeat, split] {
            assert_eq!(merge_equalities(input.clone()), input);
        }
    }

    #[test]
    fn guards_with_an_else_are_kept() {
        let with_else = Stmt::if_then_else(eq("x", 1), vec![mark("a")], vec![mark("z")]);
        let input = vec![with_else, when_eq("x", 2, vec![mark("b")])];
        assert_eq!(merge_equalities(input.clone()), input);
    }

    #[test]
    fn a_body_that_rewrites_the_variable_is_not_grouped() {
        let x = Expr::var("x", Ty::Int(32, true));
        let rewrite = Stmt::let_binding(x, Expr::int(9, 32, true));
        let out = merge_equalities(vec![
            when_eq("x", 1, vec![rewrite.clone()]),
            when_eq("x", 2, vec![mark("a")]),
            when_eq("x", 3, vec![mark("b")]),
        ]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], when_eq("x", 1, vec![rewrite]));
        assert!(matches!(&out[1], Stmt::Switch(sw) if sw.cases.len() == 2));
    }

    #[test]
    fn a_body_that_leaves_a_loop_is_not_grouped() {
        let nested = Stmt::if_then_else(flag("c"), vec![Stmt::Break], Vec::new());
        for jump in [Stmt::Break, Stmt::Continue, nested] {
            let input = vec![
                when_eq("x", 1, vec![jump]),
                when_eq("x", 2, vec![mark("b")]),
            ];
            assert_eq!(merge_equalities(input.clone()), input);
        }
    }
}
