//! Reachability-based refinement (SoW 2.3.6): neighbouring guarded nodes that
//! can never both run are written as one cascading `if` / `else if` / `else`
//! instead of separate `if`s.

use crate::hir::op::BinOp;
use crate::hir::refine::{conjuncts, merge_equalities, read_vars, writes_any};
use crate::hir::stmt::IfStmt;
use crate::hir::{Expr, Stmt};

/// Drop from a conjunction every operand that `facts` already settle: an
/// operand `p` is dropped when `!p` is one of the expressions known to be
/// false. In a cascade this turns the guard `!c1 & c2` of the second arm into
/// `c2`, and `!c1 & !c2` of the last into `true`, a plain `else`. A guard with
/// nothing to drop comes back unchanged.
pub fn drop_known_false(guard: Expr, facts: &[Expr]) -> Expr {
    let mut parts = Vec::new();
    conjuncts(guard.clone(), &mut parts);
    let before = parts.len();
    parts.retain(|p| !facts.contains(&Expr::negate(p.clone())));
    if parts.len() == before {
        return guard;
    }
    let mut rest = parts.into_iter();
    let first = rest.next().unwrap_or_else(|| Expr::bool(true));
    rest.fold(first, |acc, p| Expr::binary(BinOp::And, acc, p))
}

/// The variables the guard of `i` reads, when `i` can join a cascade: an `if`
/// with no else whose guard is a plain expression.
fn eligible(i: &IfStmt) -> Option<Vec<String>> {
    if !i.else_stmts.is_empty() {
        return None;
    }
    let mut names = Vec::new();
    read_vars(&i.cond, &mut names).then_some(names)
}

/// The `if` / `else if` / `else` chain for `run`, a list of guards that can
/// never hold together. Each arm tests only what the arms before it have not
/// already ruled out. When an arm's guard comes out as `true` it becomes the
/// final `else`, and any nodes after it are left as plain `if`s.
fn chain(run: Vec<IfStmt>) -> Vec<Stmt> {
    let mut facts: Vec<Expr> = Vec::new();
    let mut arms: Vec<(Option<Expr>, Vec<Stmt>)> = Vec::new();
    let mut nodes = run.into_iter();
    for i in nodes.by_ref() {
        let cond = drop_known_false(i.cond, &facts);
        if cond == Expr::bool(true) && !arms.is_empty() {
            arms.push((None, i.then_stmts));
            break;
        }
        facts.push(cond.clone());
        arms.push((Some(cond), i.then_stmts));
    }
    let mut tail: Vec<Stmt> = Vec::new();
    for (cond, body) in arms.into_iter().rev() {
        tail = match cond {
            Some(c) => vec![Stmt::if_then_else(c, body, tail)],
            None => body,
        };
    }
    tail.extend(nodes.map(Stmt::If));
    tail
}

/// Emit `run` as one cascade when it holds two or more nodes, otherwise as the
/// plain `if`s it came from. A run that `merge_equalities` can fold into a
/// switch is left as it is for that pass.
fn flush(run: &mut Vec<(String, IfStmt)>, out: &mut Vec<Stmt>) {
    let ifs: Vec<IfStmt> = run.drain(..).map(|(_, i)| i).collect();
    let flat: Vec<Stmt> = ifs.iter().cloned().map(Stmt::If).collect();
    if ifs.len() < 2 || merge_equalities(flat.clone()).len() < flat.len() {
        out.extend(flat);
    } else {
        out.extend(chain(ifs));
    }
}

/// Fold neighbouring guarded nodes that can never both run into cascading
/// if / else if / else. `nodes` pairs each statement with the block it came
/// from, and `exclusive(a, b)` says whether blocks `a` and `b` are mutually
/// unreachable. A node joins the run before it when its block is exclusive
/// with every block in the run and no earlier body rewrites a variable its
/// guard reads. Anything that is not such an `if` ends the run.
// shortcut: only neighbouring nodes are grouped, and the arms themselves are
// not refined further; upgrade when exclusive nodes are separated by others.
pub fn cascade_exclusive(
    nodes: Vec<(String, Stmt)>,
    exclusive: impl Fn(&str, &str) -> bool,
) -> Vec<Stmt> {
    let mut out = Vec::new();
    let mut run: Vec<(String, IfStmt)> = Vec::new();
    for (block, stmt) in nodes {
        let i = match stmt {
            Stmt::If(i) => i,
            other => {
                flush(&mut run, &mut out);
                out.push(other);
                continue;
            }
        };
        let Some(names) = eligible(&i) else {
            flush(&mut run, &mut out);
            out.push(Stmt::If(i));
            continue;
        };
        let joins = run
            .iter()
            .all(|(b, r)| exclusive(b, &block) && !writes_any(&r.then_stmts, &names));
        if !joins {
            flush(&mut run, &mut out);
        }
        run.push((block, i));
    }
    flush(&mut run, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::Ty;
    use crate::hir::op::{Cmp, IntCmp};

    fn flag(name: &str) -> Expr {
        Expr::var(name, Ty::Bool)
    }

    fn and(a: Expr, b: Expr) -> Expr {
        Expr::binary(BinOp::And, a, b)
    }

    #[test]
    fn operands_the_earlier_arms_settle_are_dropped() {
        let second = and(Expr::negate(flag("c1")), flag("c2"));
        assert_eq!(drop_known_false(second, &[flag("c1")]), flag("c2"));

        let third = and(Expr::negate(flag("c1")), Expr::negate(flag("c2")));
        let facts = [flag("c1"), flag("c2")];
        assert_eq!(drop_known_false(third, &facts), Expr::bool(true));
    }

    #[test]
    fn a_guard_the_facts_do_not_settle_is_unchanged() {
        let guard = and(Expr::negate(flag("c1")), flag("c2"));
        for facts in [vec![], vec![flag("c2")], vec![flag("other")]] {
            assert_eq!(drop_known_false(guard.clone(), &facts), guard);
        }
    }

    fn mark(name: &str) -> Stmt {
        Stmt::Eval(flag(name))
    }

    fn guarded(cond: Expr, body: Vec<Stmt>) -> Stmt {
        Stmt::if_then_else(cond, body, Vec::new())
    }

    /// Blocks whose names start with `arm` are exclusive with each other.
    fn arms(a: &str, b: &str) -> bool {
        a != b && a.starts_with("arm") && b.starts_with("arm")
    }

    fn at(block: &str, stmt: Stmt) -> (String, Stmt) {
        (block.to_string(), stmt)
    }

    #[test]
    fn exclusive_arms_become_an_else_if_chain() {
        let out = cascade_exclusive(
            vec![
                at("arm1", guarded(flag("c1"), vec![mark("a")])),
                at(
                    "arm2",
                    guarded(and(Expr::negate(flag("c1")), flag("c2")), vec![mark("b")]),
                ),
                at(
                    "arm3",
                    guarded(and(Expr::negate(flag("c1")), Expr::negate(flag("c2"))), vec![mark("d")]),
                ),
            ],
            arms,
        );
        assert_eq!(
            out,
            [Stmt::if_then_else(
                flag("c1"),
                vec![mark("a")],
                vec![Stmt::if_then_else(
                    flag("c2"),
                    vec![mark("b")],
                    vec![mark("d")]
                )],
            )]
        );
    }

    #[test]
    fn two_exclusive_arms_without_a_catch_all_end_in_an_if() {
        let out = cascade_exclusive(
            vec![
                at("arm1", guarded(flag("c1"), vec![mark("a")])),
                at("arm2", guarded(flag("c2"), vec![mark("b")])),
            ],
            arms,
        );
        assert_eq!(
            out,
            [Stmt::if_then_else(
                flag("c1"),
                vec![mark("a")],
                vec![guarded(flag("c2"), vec![mark("b")])],
            )]
        );
    }

    #[test]
    fn nodes_that_can_run_together_or_stand_alone_are_unchanged() {
        let input = vec![
            at("arm1", guarded(flag("c1"), vec![mark("a")])),
            at("join", guarded(flag("c2"), vec![mark("b")])),
            at("arm2", guarded(flag("c3"), vec![mark("c")])),
        ];
        let out = cascade_exclusive(input.clone(), arms);
        let plain: Vec<Stmt> = input.into_iter().map(|(_, s)| s).collect();
        assert_eq!(out, plain);
    }

    #[test]
    fn an_arm_that_rewrites_a_later_guard_variable_is_not_cascaded() {
        let rewrite = Stmt::let_binding(flag("c2"), Expr::bool(true));
        let input = vec![
            at("arm1", guarded(flag("c1"), vec![rewrite])),
            at("arm2", guarded(flag("c2"), vec![mark("b")])),
        ];
        let out = cascade_exclusive(input.clone(), arms);
        let plain: Vec<Stmt> = input.into_iter().map(|(_, s)| s).collect();
        assert_eq!(out, plain);
    }

    #[test]
    fn equality_runs_are_left_for_the_switch_pass() {
        let is = |value: u64| {
            Expr::cmp(
                Cmp::Int(IntCmp::Eq),
                Expr::var("x", Ty::Int(32, true)),
                Expr::int(value, 32, true),
            )
        };
        let input = vec![
            at("arm1", guarded(is(1), vec![mark("a")])),
            at("arm2", guarded(is(2), vec![mark("b")])),
        ];
        let out = cascade_exclusive(input.clone(), arms);
        let plain: Vec<Stmt> = input.into_iter().map(|(_, s)| s).collect();
        assert_eq!(out, plain);
    }

    #[test]
    fn other_statements_and_guards_with_an_else_end_the_run() {
        let with_else = Stmt::if_then_else(flag("c2"), vec![mark("b")], vec![mark("z")]);
        let input = vec![
            at("arm1", guarded(flag("c1"), vec![mark("a")])),
            at("arm2", with_else),
            at("arm3", mark("plain")),
        ];
        let out = cascade_exclusive(input.clone(), arms);
        let plain: Vec<Stmt> = input.into_iter().map(|(_, s)| s).collect();
        assert_eq!(out, plain);
    }
}
