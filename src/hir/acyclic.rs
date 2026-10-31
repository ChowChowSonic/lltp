//! Acyclic structuring (SoW 2.3.3): turn a source-to-sink region into an
//! ordered sequence of guarded nodes: the order, the reaching conditions
//! written as HIR expressions, and the sequence built from both.

use std::collections::BTreeSet;

use crate::errors::StructurizeError;
use crate::hir::op::BinOp;
use crate::hir::reaching::{Cond, EdgeSet, reaching_conditions};
use crate::hir::stmt::BranchStmt;
use crate::hir::{Cfg, Expr, Lit, Stmt, Ty};

/// Blocks of `region` reachable from `source`, in reverse postorder over the
/// edges not in `skip`. On an acyclic region every edge points forward in
/// the result, and a branch's first successor comes before its later ones.
/// Empty when `source` is not in `region`.
pub fn reverse_postorder(
    cfg: &Cfg,
    source: &str,
    region: &BTreeSet<String>,
    skip: &EdgeSet,
) -> Vec<String> {
    let mut order = Vec::new();
    if !region.contains(source) {
        return order;
    }
    let mut seen = BTreeSet::from([source.to_string()]);
    // Each frame holds a block and its successors still to visit; popping
    // from the end visits the first successor last, so it ends up first.
    let mut stack = vec![(source.to_string(), cfg.blocks[source].succ.clone())];
    while let Some((n, pending)) = stack.last_mut() {
        match pending.pop() {
            Some(m) => {
                if region.contains(&m)
                    && !skip.contains(&(n.clone(), m.clone()))
                    && seen.insert(m.clone())
                {
                    let next = cfg.blocks[&m].succ.clone();
                    stack.push((m, next));
                }
            }
            None => {
                order.push(n.clone());
                stack.pop();
            }
        }
    }
    order.reverse();
    order
}

/// Name of the boolean that holds the outcome of `block`'s branch. Guards read
/// this instead of re-evaluating the branch condition, because a block that
/// runs in between may have reassigned a variable the condition reads.
pub fn cond_var(block: &str) -> String {
    format!("{block}.cond")
}

/// `!e` in the HIR's own idiom (`e ^ true`), undoing a double negation.
fn negate(e: Expr) -> Expr {
    match e {
        Expr::Literal(Lit::Bool(b)) => Expr::bool(!b),
        Expr::BinaryOp(b) if b.op == BinOp::Xor && *b.arg2 == Expr::bool(true) => *b.arg1,
        other => Expr::binary(BinOp::Xor, other, Expr::bool(true)),
    }
}

/// The edge `from -> to` as an expression: the branch boolean when `to` is
/// the then-target, its negation when `to` is the else-target. Errors when
/// `from` has no conditional branch to `to`.
fn edge_expr(cfg: &Cfg, from: &str, to: &str) -> Result<Expr, StructurizeError> {
    let bad = || StructurizeError::BadBranchEdge {
        block: from.to_string(),
        target: to.to_string(),
    };
    let (_, branch) = cfg.split_block(from)?;
    let Some(BranchStmt {
        cond: Some(_),
        then_block,
        else_block,
    }) = branch
    else {
        return Err(bad());
    };
    let taken = Expr::var(cond_var(from), Ty::Bool);
    if then_block == to {
        Ok(taken)
    } else if else_block.as_deref() == Some(to) {
        Ok(negate(taken))
    } else {
        Err(bad())
    }
}

fn combine(cfg: &Cfg, terms: &[Cond], op: BinOp) -> Result<Expr, StructurizeError> {
    let Some((first, rest)) = terms.split_first() else {
        return Ok(Expr::bool(op == BinOp::And));
    };
    let mut acc = cond_to_expr(cfg, first)?;
    for t in rest {
        acc = Expr::binary(op, acc, cond_to_expr(cfg, t)?);
    }
    Ok(acc)
}

/// Write a reaching condition as an expression over the branch booleans named
/// by `cond_var`. Errors when an edge in `cond` is not a branch edge of `cfg`.
// shortcut: `and`/`or` use the bitwise operators on booleans (the HIR has no
// logical ones), so they do not short-circuit; upgrade if a target needs `&&`.
pub fn cond_to_expr(cfg: &Cfg, cond: &Cond) -> Result<Expr, StructurizeError> {
    match cond {
        Cond::True => Ok(Expr::bool(true)),
        Cond::False => Ok(Expr::bool(false)),
        Cond::Edge { from, to } => edge_expr(cfg, from, to),
        Cond::Not(inner) => Ok(negate(cond_to_expr(cfg, inner)?)),
        Cond::And(terms) => combine(cfg, terms, BinOp::And),
        Cond::Or(terms) => combine(cfg, terms, BinOp::Or),
    }
}

/// Collect the blocks whose branch outcome `c` tests.
fn tested_branches(c: &Cond, out: &mut BTreeSet<String>) {
    match c {
        Cond::Edge { from, .. } => {
            out.insert(from.clone());
        }
        Cond::Not(inner) => tested_branches(inner, out),
        Cond::And(terms) | Cond::Or(terms) => {
            terms.iter().for_each(|t| tested_branches(t, out));
        }
        Cond::True | Cond::False => {}
    }
}

/// Structure the region from `source` to `sink` as a flat sequence: each block
/// in reverse postorder, its statements guarded by `if (reaching condition)`
/// unless that condition is always true. A tested branch's condition is stored
/// in a fresh boolean (`cond_var`) at the end of its block, and that boolean is
/// added to `locals`. The sink's own statements are included, but whatever
/// follows the sink is left to the caller.
///
/// Errors with `NotAcyclicRegion` when the region still has a cycle after
/// skipping the edges in `skip`, when `sink` is unreachable, or when control
/// can leave the region anywhere but the sink.
// shortcut: every tested branch gets its own boolean even when one use would
// do, upgrade when if/else grouping (2.3.4) leaves the extra locals cluttering
// the output.
pub fn structure_acyclic(
    cfg: &Cfg,
    source: &str,
    sink: &str,
    skip: &EdgeSet,
    locals: &mut Vec<(String, Ty)>,
) -> Result<Vec<Stmt>, StructurizeError> {
    let bad = || StructurizeError::NotAcyclicRegion {
        entry: source.to_string(),
        exit: sink.to_string(),
    };
    let conds = reaching_conditions(cfg, source, sink, skip).ok_or_else(bad)?;
    let region: BTreeSet<String> = conds.keys().cloned().collect();
    if region.is_empty() {
        return Err(bad());
    }
    let leaks = region.iter().filter(|n| n.as_str() != sink).any(|n| {
        cfg.blocks[n]
            .succ
            .iter()
            .any(|m| !region.contains(m) && !skip.contains(&(n.clone(), m.clone())))
    });
    if leaks {
        return Err(bad());
    }

    let mut tested = BTreeSet::new();
    for c in conds.values() {
        tested_branches(c, &mut tested);
    }

    let mut out = Vec::new();
    for n in reverse_postorder(cfg, source, &region, skip) {
        let (mut body, branch) = cfg.split_block(&n)?;
        if tested.contains(&n)
            && let Some(BranchStmt { cond: Some(c), .. }) = branch
        {
            let name = cond_var(&n);
            if !locals.iter().any(|(l, _)| *l == name) {
                locals.push((name.clone(), Ty::Bool));
            }
            body.push(Stmt::let_binding(Expr::var(name, Ty::Bool), c));
        }
        if body.is_empty() {
            continue;
        }
        if conds[&n] == Cond::True {
            out.extend(body);
        } else {
            let guard = cond_to_expr(cfg, &conds[&n])?;
            out.push(Stmt::if_then_else(guard, body, Vec::new()));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::{Block, Stmt};

    fn set(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    fn names(order: &[String]) -> Vec<&str> {
        order.iter().map(String::as_str).collect()
    }

    #[test]
    fn diamond_puts_the_first_arm_first() {
        let cfg = Cfg::cfg_from(
            "entry",
            &[
                ("entry", "a"),
                ("entry", "b"),
                ("a", "join"),
                ("b", "join"),
                ("join", "exit"),
            ],
        );
        let region = set(&["entry", "a", "b", "join", "exit"]);
        let order = reverse_postorder(&cfg, "entry", &region, &EdgeSet::new());
        assert_eq!(names(&order), ["entry", "a", "b", "join", "exit"]);
    }

    #[test]
    fn every_edge_points_forward() {
        let edges = [
            ("entry", "a"),
            ("entry", "b"),
            ("a", "c"),
            ("b", "c"),
            ("a", "d"),
            ("c", "exit"),
            ("d", "exit"),
        ];
        let cfg = Cfg::cfg_from("entry", &edges);
        let region = set(&["entry", "a", "b", "c", "d", "exit"]);
        let order = reverse_postorder(&cfg, "entry", &region, &EdgeSet::new());
        assert_eq!(order.len(), region.len());
        let pos = |n: &str| order.iter().position(|x| x == n).unwrap();
        for (from, to) in edges {
            assert!(pos(from) < pos(to), "{from} should come before {to}");
        }
    }

    #[test]
    fn blocks_outside_the_region_are_left_out() {
        let cfg = Cfg::cfg_from("entry", &[("entry", "a"), ("a", "join"), ("entry", "dead")]);
        let region = set(&["entry", "a", "join"]);
        let order = reverse_postorder(&cfg, "entry", &region, &EdgeSet::new());
        assert_eq!(names(&order), ["entry", "a", "join"]);
    }

    #[test]
    fn skipped_back_edge_still_gives_a_forward_order() {
        let cfg = Cfg::cfg_from(
            "entry",
            &[("entry", "h"), ("h", "body"), ("body", "h"), ("h", "exit")],
        );
        let region = set(&["h", "body", "exit"]);
        let back: EdgeSet = [("body".to_string(), "h".to_string())].into();
        let order = reverse_postorder(&cfg, "h", &region, &back);
        assert_eq!(names(&order), ["h", "body", "exit"]);
    }

    #[test]
    fn source_outside_the_region_gives_nothing() {
        let cfg = Cfg::cfg_from("entry", &[("entry", "a")]);
        let order = reverse_postorder(&cfg, "entry", &set(&["a"]), &EdgeSet::new());
        assert!(order.is_empty());
    }

    fn flag(name: &str) -> Expr {
        Expr::var(name, Ty::Bool)
    }

    fn mark(name: &str) -> Stmt {
        Stmt::Eval(flag(name))
    }

    fn branch(cond: Option<Expr>, then: &str, otherwise: Option<&str>) -> Stmt {
        Stmt::Branch(BranchStmt {
            cond,
            then_block: then.to_string(),
            else_block: otherwise.map(str::to_string),
        })
    }

    /// entry branches on `c` to `a` or `b`, both join, join returns.
    fn diamond() -> Cfg {
        Cfg::assemble(
            "entry",
            vec![
                Block::build(
                    "entry",
                    vec![],
                    Some(branch(Some(flag("c")), "a", Some("b"))),
                ),
                Block::build("a", vec![mark("a")], Some(branch(None, "join", None))),
                Block::build("b", vec![mark("b")], Some(branch(None, "join", None))),
                Block::build("join", vec![mark("join")], Some(Stmt::ret_void())),
            ],
        )
    }

    fn taken() -> Expr {
        flag(&cond_var("entry"))
    }

    fn not_taken() -> Expr {
        Expr::binary(BinOp::Xor, taken(), Expr::bool(true))
    }

    #[test]
    fn then_edge_is_the_branch_boolean_and_else_edge_its_negation() {
        let cfg = diamond();
        assert_eq!(cond_to_expr(&cfg, &Cond::edge("entry", "a")), Ok(taken()));
        assert_eq!(
            cond_to_expr(&cfg, &Cond::edge("entry", "b")),
            Ok(not_taken())
        );
    }

    #[test]
    fn negation_and_constants_lower_cleanly() {
        let cfg = diamond();
        assert_eq!(cond_to_expr(&cfg, &Cond::True), Ok(Expr::bool(true)));
        assert_eq!(cond_to_expr(&cfg, &Cond::False), Ok(Expr::bool(false)));
        assert_eq!(cond_to_expr(&cfg, &!Cond::edge("entry", "b")), Ok(taken()));
    }

    #[test]
    fn and_or_combine_terms_in_order() {
        let cfg = diamond();
        let both = Cond::edge("entry", "a").and(Cond::edge("entry", "b"));
        assert_eq!(
            cond_to_expr(&cfg, &both),
            Ok(Expr::binary(BinOp::And, taken(), not_taken()))
        );
        let either = Cond::edge("entry", "a").or(Cond::edge("entry", "b"));
        assert_eq!(
            cond_to_expr(&cfg, &either),
            Ok(Expr::binary(BinOp::Or, taken(), not_taken()))
        );
    }

    #[test]
    fn edges_without_a_usable_branch_are_errors() {
        let cfg = diamond();
        let bad = |from: &str, to: &str| StructurizeError::BadBranchEdge {
            block: from.to_string(),
            target: to.to_string(),
        };
        // `a` jumps unconditionally, and `join` is not a target of `entry`.
        assert_eq!(
            cond_to_expr(&cfg, &Cond::edge("a", "join")),
            Err(bad("a", "join"))
        );
        assert_eq!(
            cond_to_expr(&cfg, &Cond::edge("entry", "join")),
            Err(bad("entry", "join"))
        );
        assert_eq!(
            cond_to_expr(&cfg, &Cond::edge("nope", "a")),
            Err(StructurizeError::UnknownBlock("nope".to_string()))
        );
    }

    fn run(cfg: &Cfg, source: &str, sink: &str) -> Result<Vec<Stmt>, StructurizeError> {
        structure_acyclic(cfg, source, sink, &EdgeSet::new(), &mut Vec::new())
    }

    fn not_acyclic(entry: &str, exit: &str) -> StructurizeError {
        StructurizeError::NotAcyclicRegion {
            entry: entry.to_string(),
            exit: exit.to_string(),
        }
    }

    #[test]
    fn diamond_becomes_guarded_nodes() {
        let cfg = diamond();
        let mut locals = Vec::new();
        let out = structure_acyclic(&cfg, "entry", "join", &EdgeSet::new(), &mut locals).unwrap();
        assert_eq!(
            out,
            vec![
                Stmt::let_binding(taken(), flag("c")),
                Stmt::if_then_else(taken(), vec![mark("a")], vec![]),
                Stmt::if_then_else(not_taken(), vec![mark("b")], vec![]),
                mark("join"),
                Stmt::ret_void(),
            ]
        );
        assert_eq!(locals, vec![(cond_var("entry"), Ty::Bool)]);
    }

    #[test]
    fn straight_line_region_is_just_its_statements() {
        let cfg = Cfg::assemble(
            "entry",
            vec![
                Block::build(
                    "entry",
                    vec![mark("entry")],
                    Some(branch(None, "mid", None)),
                ),
                Block::build("mid", vec![mark("mid")], Some(branch(None, "exit", None))),
                Block::build("exit", vec![], Some(Stmt::ret_void())),
            ],
        );
        let mut locals = Vec::new();
        let out = structure_acyclic(&cfg, "entry", "exit", &EdgeSet::new(), &mut locals).unwrap();
        assert_eq!(out, vec![mark("entry"), mark("mid"), Stmt::ret_void()]);
        assert!(locals.is_empty());
    }

    #[test]
    fn a_side_exit_makes_the_region_invalid() {
        // `bail` leaves the function without passing through `join`.
        let cfg = Cfg::assemble(
            "entry",
            vec![
                Block::build(
                    "entry",
                    vec![],
                    Some(branch(Some(flag("c")), "a", Some("bail"))),
                ),
                Block::build("a", vec![], Some(branch(None, "join", None))),
                Block::build("bail", vec![], Some(Stmt::ret_void())),
                Block::build("join", vec![], Some(Stmt::ret_void())),
            ],
        );
        assert_eq!(
            run(&cfg, "entry", "join"),
            Err(not_acyclic("entry", "join"))
        );
    }

    #[test]
    fn a_loop_makes_the_region_invalid() {
        let cfg = Cfg::assemble(
            "entry",
            vec![
                Block::build("entry", vec![], Some(branch(None, "h", None))),
                Block::build(
                    "h",
                    vec![],
                    Some(branch(Some(flag("c")), "body", Some("exit"))),
                ),
                Block::build("body", vec![], Some(branch(None, "h", None))),
                Block::build("exit", vec![], Some(Stmt::ret_void())),
            ],
        );
        assert_eq!(run(&cfg, "h", "exit"), Err(not_acyclic("h", "exit")));
    }

    #[test]
    fn unreachable_sink_makes_the_region_invalid() {
        let cfg = diamond();
        assert_eq!(run(&cfg, "a", "b"), Err(not_acyclic("a", "b")));
    }
}
