//! Acyclic structuring (SoW 2.3.3): turn a source-to-sink region into an
//! ordered sequence of guarded nodes: the order, the reaching conditions
//! written as HIR expressions, and the sequence built from both. Guards that
//! test one variable against constants are then folded into a switch (2.3.5).

use std::collections::{BTreeMap, BTreeSet};

use crate::errors::StructurizeError;
use crate::hir::op::BinOp;
use crate::hir::reaching::{Cond, EdgeSet, reaching_conditions};
use crate::hir::refine::{
    equality_guard, merge_complementary, merge_equalities, simplify_guard, writes_any,
};
use crate::hir::stmt::BranchStmt;
use crate::hir::{Cfg, Expr, Stmt, Ty};

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
        Ok(Expr::negate(taken))
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
        Cond::Not(inner) => Ok(Expr::negate(cond_to_expr(cfg, inner)?)),
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

/// Number of times variable `name` appears in `e`.
fn count_expr(e: &Expr, name: &str) -> usize {
    match e {
        Expr::Var(v) => usize::from(v.name == name),
        Expr::UnaryOp(u) => count_expr(&u.arg, name),
        Expr::BinaryOp(b) => count_expr(&b.arg1, name) + count_expr(&b.arg2, name),
        Expr::Cmp(c) => count_expr(&c.arg1, name) + count_expr(&c.arg2, name),
        Expr::Call(c) => c.args.iter().map(|a| count_expr(a, name)).sum(),
        Expr::Index(i) => count_expr(&i.base, name) + count_expr(&i.index, name),
        Expr::Field(f) => count_expr(&f.base, name),
        Expr::Cast(c) => count_expr(&c.data, name),
        Expr::Literal(_) | Expr::Nop => 0,
    }
}

/// Number of times variable `name` appears in `stmts`, binding sites included.
fn count_stmts(stmts: &[Stmt], name: &str) -> usize {
    stmts
        .iter()
        .map(|s| match s {
            Stmt::Branch(b) => b.cond.as_ref().map_or(0, |c| count_expr(c, name)),
            Stmt::Ret(r) => r.value.as_ref().map_or(0, |v| count_expr(v, name)),
            Stmt::If(i) => {
                count_expr(&i.cond, name)
                    + count_stmts(&i.then_stmts, name)
                    + count_stmts(&i.else_stmts, name)
            }
            Stmt::Switch(sw) => {
                count_expr(&sw.value, name)
                    + sw.cases
                        .iter()
                        .map(|(_, b)| count_stmts(b, name))
                        .sum::<usize>()
                    + count_stmts(&sw.default, name)
            }
            Stmt::Let(l) => count_expr(&l.dest, name) + count_expr(&l.src, name),
            Stmt::Eval(e) => count_expr(e, name),
            Stmt::Loop(l) => {
                l.cond.as_ref().map_or(0, |c| count_expr(c, name)) + count_stmts(&l.body, name)
            }
            Stmt::TryCatch(t) => {
                let catches: usize = t
                    .catches
                    .iter()
                    .map(|c| {
                        let param = c.param.as_ref().is_some_and(|p| p.name == name);
                        usize::from(param) + count_stmts(&c.body, name)
                    })
                    .sum();
                count_stmts(&t.try_body, name)
                    + catches
                    + t.finally_body
                        .as_deref()
                        .map_or(0, |f| count_stmts(f, name))
            }
            Stmt::Invoke(i) => i.args.iter().map(|a| count_expr(a, name)).sum(),
            Stmt::Unreachable | Stmt::Break | Stmt::Continue | Stmt::Goto(_) | Stmt::Label(_) => 0,
        })
        .sum()
}

/// Number of times variable `name` appears anywhere in `cfg`.
fn uses(cfg: &Cfg, name: &str) -> usize {
    cfg.blocks
        .values()
        .map(|b| count_stmts(&b.stmts, name))
        .sum()
}

/// A branch on a compare that was stored in a temporary becomes a branch on
/// the compare itself, so the guards below can see `v == c`. The compare's
/// `let` is taken out of `body`. Only done when the temporary is used by that
/// branch alone and nothing after the `let` rewrites the compared variable.
fn inline_compare(cfg: &Cfg, body: &mut Vec<Stmt>, cond: Expr) -> Expr {
    let Expr::Var(t) = &cond else {
        return cond;
    };
    let name = t.name.clone();
    let Some(pos) = body.iter().rposition(
        |s| matches!(s, Stmt::Let(l) if matches!(&*l.dest, Expr::Var(d) if d.name == name)),
    ) else {
        return cond;
    };
    let Stmt::Let(l) = &body[pos] else {
        return cond;
    };
    let Some((v, _)) = equality_guard(&l.src) else {
        return cond;
    };
    if uses(cfg, &name) != 2 || writes_any(&body[pos + 1..], std::slice::from_ref(&v.name)) {
        return cond;
    }
    let compare = (*l.src).clone();
    body.remove(pos);
    compare
}

/// Replace each variable of `e` that `known` holds a compare for by that
/// compare.
fn inline_flags(e: Expr, known: &BTreeMap<String, Expr>) -> Expr {
    match e {
        Expr::Var(v) => known.get(&v.name).cloned().unwrap_or(Expr::Var(v)),
        Expr::BinaryOp(b) => Expr::binary(
            b.op,
            inline_flags(*b.arg1, known),
            inline_flags(*b.arg2, known),
        ),
        other => other,
    }
}

/// True when `body` may rewrite the variable that the compare `eq` reads.
fn rewrites(body: &[Stmt], eq: &Expr) -> bool {
    equality_guard(eq).is_some_and(|(v, _)| writes_any(body, std::slice::from_ref(&v.name)))
}

/// Collect the variables the guards of `stmts` read.
fn guard_vars(stmts: &[Stmt], out: &mut BTreeSet<String>) {
    fn vars(e: &Expr, out: &mut BTreeSet<String>) {
        match e {
            Expr::Var(v) => {
                out.insert(v.name.clone());
            }
            Expr::BinaryOp(b) => {
                vars(&b.arg1, out);
                vars(&b.arg2, out);
            }
            _ => {}
        }
    }
    for s in stmts {
        if let Stmt::If(i) = s {
            vars(&i.cond, out);
            guard_vars(&i.then_stmts, out);
            guard_vars(&i.else_stmts, out);
        }
    }
}

/// Drop the `let` of every variable in `dead`, and any `if` left empty.
fn prune(stmts: Vec<Stmt>, dead: &BTreeSet<String>) -> Vec<Stmt> {
    stmts
        .into_iter()
        .filter_map(|s| match s {
            Stmt::Let(l) if matches!(&*l.dest, Expr::Var(v) if dead.contains(&v.name)) => None,
            Stmt::If(mut i) => {
                i.then_stmts = prune(i.then_stmts, dead);
                i.else_stmts = prune(i.else_stmts, dead);
                if i.then_stmts.is_empty() && i.else_stmts.is_empty() {
                    None
                } else {
                    Some(Stmt::If(i))
                }
            }
            other => Some(other),
        })
        .collect()
}

/// Structure the region from `source` to `sink` as a flat sequence: each block
/// in reverse postorder, its statements guarded by `if (reaching condition)`
/// unless that condition is always true. A tested branch's condition is stored
/// in a fresh boolean (`cond_var`) at the end of its block, and that boolean is
/// added to `locals`. A branch on `v == c` is kept as that compare in later
/// guards while nothing rewrites `v`, and its boolean is dropped once no guard
/// reads it. Runs of guards `v == c` then become a switch (2.3.5) and
/// neighbouring nodes with complementary guards one if/else (2.3.4). The
/// sink's own statements are included, but whatever follows the sink is left
/// to the caller.
///
/// Errors with `NotAcyclicRegion` when the region still has a cycle after
/// skipping the edges in `skip`, when `sink` is unreachable, or when control
/// can leave the region anywhere but the sink.
// shortcut: a branch boolean is only replaced by its compare when that
// compare is `variable == integer constant`; other conditions keep their own
// boolean even when if/else grouping leaves it used once, so upgrade with a
// def-use check.
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

    // Branch booleans that currently equal a compare `v == c`, and every one
    // that ever did.
    let mut known = BTreeMap::new();
    let mut inlined = BTreeSet::new();
    let mut out = Vec::new();
    for n in reverse_postorder(cfg, source, &region, skip) {
        let (mut body, branch) = cfg.split_block(&n)?;
        // This block's guard runs before its body, so it sees `known` as it is
        // now, and the body then retires what it rewrites.
        let before = known.clone();
        known.retain(|_, eq| !rewrites(&body, eq));
        if tested.contains(&n)
            && let Some(BranchStmt { cond: Some(c), .. }) = branch
        {
            let c = inline_compare(cfg, &mut body, c);
            let name = cond_var(&n);
            if !locals.iter().any(|(l, _)| *l == name) {
                locals.push((name.clone(), Ty::Bool));
            }
            if equality_guard(&c).is_some() {
                known.insert(name.clone(), c.clone());
                inlined.insert(name.clone());
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
            let guard = simplify_guard(inline_flags(guard, &before));
            out.push(Stmt::if_then_else(guard, body, Vec::new()));
        }
    }
    let mut read = BTreeSet::new();
    guard_vars(&out, &mut read);
    let dead: BTreeSet<String> = inlined.difference(&read).cloned().collect();
    locals.retain(|(l, _)| !dead.contains(l));
    Ok(merge_complementary(merge_equalities(prune(out, &dead))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::op::{Cmp, IntCmp};
    use crate::hir::stmt::SwitchStmt;
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
        Expr::negate(taken())
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
    fn diamond_becomes_if_else() {
        let cfg = diamond();
        let mut locals = Vec::new();
        let out = structure_acyclic(&cfg, "entry", "join", &EdgeSet::new(), &mut locals).unwrap();
        assert_eq!(
            out,
            vec![
                Stmt::let_binding(taken(), flag("c")),
                Stmt::if_then_else(taken(), vec![mark("a")], vec![mark("b")]),
                mark("join"),
                Stmt::ret_void(),
            ]
        );
        assert_eq!(locals, vec![(cond_var("entry"), Ty::Bool)]);
    }

    #[test]
    fn a_one_armed_branch_stays_an_if_then() {
        let cfg = Cfg::assemble(
            "entry",
            vec![
                Block::build(
                    "entry",
                    vec![],
                    Some(branch(Some(flag("c")), "a", Some("join"))),
                ),
                Block::build("a", vec![mark("a")], Some(branch(None, "join", None))),
                Block::build("join", vec![mark("join")], Some(Stmt::ret_void())),
            ],
        );
        assert_eq!(
            run(&cfg, "entry", "join"),
            Ok(vec![
                Stmt::let_binding(taken(), flag("c")),
                Stmt::if_then_else(taken(), vec![mark("a")], vec![]),
                mark("join"),
                Stmt::ret_void(),
            ])
        );
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

    fn x() -> Expr {
        Expr::var("x", Ty::Int(32, true))
    }

    fn x_is(value: u64) -> Expr {
        Expr::cmp(Cmp::Int(IntCmp::Eq), x(), Expr::int(value, 32, true))
    }

    /// `temp = (x == value)`, the way the lifter stores a compare.
    fn compare(temp: &str, value: u64) -> Stmt {
        Stmt::let_binding(flag(temp), x_is(value))
    }

    fn set_x(value: u64) -> Stmt {
        Stmt::let_binding(x(), Expr::int(value, 32, true))
    }

    /// `entry` runs `entry_stmts` and branches on `t`; both targets return.
    fn branch_on_temp(entry_stmts: Vec<Stmt>) -> Cfg {
        Cfg::assemble(
            "entry",
            vec![
                Block::build(
                    "entry",
                    entry_stmts,
                    Some(branch(Some(flag("t")), "a", Some("b"))),
                ),
                Block::build("a", vec![], Some(Stmt::ret_void())),
                Block::build("b", vec![], Some(Stmt::ret_void())),
            ],
        )
    }

    #[test]
    fn a_single_use_compare_moves_into_its_branch() {
        let cfg = branch_on_temp(vec![compare("t", 1)]);
        let mut body = vec![compare("t", 1)];
        assert_eq!(inline_compare(&cfg, &mut body, flag("t")), x_is(1));
        assert!(body.is_empty());
    }

    #[test]
    fn a_compare_that_is_used_again_or_whose_variable_changes_stays() {
        let reused = vec![compare("t", 1), mark("t")];
        let rewritten = vec![compare("t", 1), set_x(9)];
        for stmts in [reused, rewritten] {
            let cfg = branch_on_temp(stmts.clone());
            let mut body = stmts.clone();
            assert_eq!(inline_compare(&cfg, &mut body, flag("t")), flag("t"));
            assert_eq!(body, stmts);
        }
    }

    #[test]
    fn a_branch_on_an_equality_keeps_the_compare_in_both_guards() {
        let cfg = Cfg::assemble(
            "entry",
            vec![
                Block::build("entry", vec![], Some(branch(Some(x_is(1)), "a", Some("b")))),
                Block::build("a", vec![mark("a")], Some(branch(None, "join", None))),
                Block::build("b", vec![mark("b")], Some(branch(None, "join", None))),
                Block::build("join", vec![mark("join")], Some(Stmt::ret_void())),
            ],
        );
        let mut locals = Vec::new();
        let out = structure_acyclic(&cfg, "entry", "join", &EdgeSet::new(), &mut locals).unwrap();
        assert_eq!(
            out,
            vec![
                Stmt::if_then_else(x_is(1), vec![mark("a")], vec![mark("b")]),
                mark("join"),
                Stmt::ret_void(),
            ]
        );
        assert!(locals.is_empty());
    }

    #[test]
    fn a_boolean_stays_once_the_variable_it_compared_is_rewritten() {
        let cfg = Cfg::assemble(
            "entry",
            vec![
                Block::build("entry", vec![], Some(branch(Some(x_is(1)), "a", Some("b")))),
                Block::build("a", vec![set_x(9)], Some(branch(None, "join", None))),
                Block::build("b", vec![mark("b")], Some(branch(None, "join", None))),
                Block::build("join", vec![mark("join")], Some(Stmt::ret_void())),
            ],
        );
        let mut locals = Vec::new();
        let out = structure_acyclic(&cfg, "entry", "join", &EdgeSet::new(), &mut locals).unwrap();
        assert_eq!(
            out,
            vec![
                Stmt::let_binding(taken(), x_is(1)),
                Stmt::if_then_else(x_is(1), vec![set_x(9)], vec![]),
                Stmt::if_then_else(not_taken(), vec![mark("b")], vec![]),
                mark("join"),
                Stmt::ret_void(),
            ]
        );
        assert_eq!(locals, vec![(cond_var("entry"), Ty::Bool)]);
    }

    #[test]
    fn a_chain_of_equality_tests_becomes_a_switch_with_a_default() {
        let jump = || Some(branch(None, "join", None));
        let cfg = Cfg::assemble(
            "entry",
            vec![
                Block::build(
                    "entry",
                    vec![compare("t1", 1)],
                    Some(branch(Some(flag("t1")), "a", Some("b"))),
                ),
                Block::build("a", vec![mark("a")], jump()),
                Block::build(
                    "b",
                    vec![compare("t2", 2)],
                    Some(branch(Some(flag("t2")), "c", Some("d"))),
                ),
                Block::build("c", vec![mark("c")], jump()),
                Block::build("d", vec![mark("d")], jump()),
                Block::build("join", vec![mark("join")], Some(Stmt::ret_void())),
            ],
        );
        let mut locals = Vec::new();
        let out = structure_acyclic(&cfg, "entry", "join", &EdgeSet::new(), &mut locals).unwrap();
        let lit = |value: u64| Lit::Int {
            value,
            bits: 32,
            signed: true,
        };
        // The join's own guard is the unsimplified "a, c or d ran", so it stays
        // an `if`; the three arms before it are one switch.
        assert_eq!(out.len(), 2);
        assert_eq!(
            out[0],
            Stmt::Switch(SwitchStmt {
                value: x(),
                cases: vec![(lit(1), vec![mark("a")]), (lit(2), vec![mark("c")])],
                default: vec![mark("d")],
            })
        );
        assert!(matches!(
            &out[1],
            Stmt::If(i) if i.then_stmts == [mark("join"), Stmt::ret_void()]
        ));
        assert!(locals.is_empty());
    }
}
