//! This pass computes liveness, builds an interference graph, and merges
//! the two ends of a `dest = src` copy into one variable whenever their
//! live ranges never overlap, then drops the copies that became `x = x`.
//! A parameter can absorb a local it is copied into (the `x.addr` slot clang
//! makes for `x`), so the source-level parameter name survives.

use std::collections::{BTreeMap, BTreeSet};

use crate::hir::expr::{BinaryExpr, CallExpr, CastExpr, CmpExpr, IndexExpr, UnaryExpr, VarExpr};
use crate::hir::stmt::LetStmt;
use crate::hir::{Block, Expr, Stmt};

use super::flat::FlatFunction;
use super::op::UnOp;
use super::ty::Ty;

type Names = BTreeSet<String>;

/// Coalesce copy-related variables in `func`. Returns how many variables were merged.
pub fn coalesce(func: &mut FlatFunction) -> usize {
    let fixed = fixed_names(func);
    let params: Names = func.param_names.iter().cloned().collect();
    // Parameters sort before every local, so a merge always keeps the parameter.
    let order: BTreeMap<String, usize> = func
        .param_names
        .iter()
        .chain(func.locals.iter().map(|(n, _)| n))
        .enumerate()
        .map(|(i, n)| (n.clone(), i))
        .collect();
    let mut ty_of: BTreeMap<String, Ty> = func.locals.iter().cloned().collect();
    ty_of.extend(
        func.param_names
            .iter()
            .cloned()
            .zip(func.sig.params.iter().cloned()),
    );

    let live_out = liveness(func);
    let mut adj = interference(func, &live_out);

    let copies: Vec<(String, String)> = func
        .blocks
        .iter()
        .flat_map(|b| b.stmts.iter().chain(std::iter::once(b.term())))
        .filter_map(copy_pair)
        .collect();

    let mut rep: BTreeMap<String, String> = BTreeMap::new();
    for (d, s) in copies {
        let a = find(&rep, &d);
        let b = find(&rep, &s);
        if a == b {
            continue;
        }
        let (Some(&ia), Some(&ib)) = (order.get(&a), order.get(&b)) else {
            continue;
        };
        if fixed.contains(&a) || fixed.contains(&b) || ty_of.get(&a) != ty_of.get(&b) {
            continue;
        }
        // Two parameters are both live on entry, so they can never share a name.
        if params.contains(&a) && params.contains(&b) {
            continue;
        }
        if adj.get(&a).is_some_and(|n| n.contains(&b)) {
            continue;
        }
        let (keep, gone) = if ia <= ib { (a, b) } else { (b, a) };
        // The merged node interferes with everything either end did.
        for n in adj.remove(&gone).unwrap_or_default() {
            if let Some(set) = adj.get_mut(&n) {
                set.remove(&gone);
                set.insert(keep.clone());
            }
            adj.entry(keep.clone()).or_default().insert(n);
        }
        rep.insert(gone, keep);
    }

    if rep.is_empty() {
        return 0;
    }
    let rename = |n: &str| find(&rep, n);
    for block in &mut func.blocks {
        for stmt in &mut block.stmts {
            rename_stmt(stmt, &rename);
        }
        rename_stmt(block.term_mut(), &rename);
        block.stmts.retain(|stmt| {
            !matches!(stmt, Stmt::Let(LetStmt { dest, src }) if
                    is_plain_var(dest) && same_name(dest, src))
        });
    }
    func.locals.retain(|(n, _)| !rep.contains_key(n));
    rep.len()
}

/// Names that must keep their identity: slots that are still reached through
/// `load`/`store` or have their address taken.
fn fixed_names(func: &FlatFunction) -> Names {
    let mut fixed = Names::new();
    for block in &func.blocks {
        for stmt in &block.stmts {
            for e in exprs(stmt) {
                collect_slots(e, &mut fixed);
            }
        }
        for e in exprs(block.term()) {
            collect_slots(e, &mut fixed);
        }
    }
    fixed
}

/// The expressions a statement evaluates, including the place it writes to.
pub(super) fn exprs(stmt: &Stmt) -> Vec<&Expr> {
    match stmt {
        Stmt::Let(LetStmt { dest, src }) => vec![dest, src],
        Stmt::Eval(e) => vec![e],
        Stmt::Branch(b) => b.cond.iter().collect(),
        Stmt::Ret(r) => r.value.iter().collect(),
        _ => vec![],
    }
}

/// Every slot named by an `AddrOf`/`ValuAt` inside `e`.
fn collect_slots(e: &Expr, fixed: &mut Names) {
    if let Some(slot) = slot_name(e) {
        fixed.insert(slot.to_string());
    }
    for child in children(e) {
        collect_slots(child, fixed);
    }
}

fn find(rep: &BTreeMap<String, String>, name: &str) -> String {
    let mut cur = name.to_string();
    while let Some(next) = rep.get(&cur) {
        cur = next.clone();
    }
    cur
}

// ---------------------------------------------------------------------------
// def and use
// ---------------------------------------------------------------------------

/// The single name a statement defines, if it defines one.
pub(super) fn def_name(stmt: &Stmt) -> Option<&str> {
    def_place(stmt).and_then(place_name)
}

/// The place a statement writes to. A store writes to a place, so its def is
/// the cell it points at.
fn def_place(stmt: &Stmt) -> Option<&Expr> {
    match stmt {
        Stmt::Let(LetStmt { dest, .. }) => Some(dest),
        _ => None,
    }
}

/// The name a place refers to: a plain variable, or the cell an `AddrOf` points at.
fn place_name(e: &Expr) -> Option<&str> {
    match e {
        Expr::Var(VarExpr { name, .. }) => Some(name),
        Expr::UnaryOp(UnaryExpr {
            op: UnOp::AddrOf,
            arg,
        }) => place_name(arg),
        _ => None,
    }
}

/// A plain variable, not a place pointing at one.
fn is_plain_var(e: &Expr) -> bool {
    matches!(e, Expr::Var(..))
}

/// The slot a memory expression refers to (`AddrOf(x)` or `ValuAt(x)`).
pub(super) fn slot_name(e: &Expr) -> Option<&str> {
    match e {
        Expr::UnaryOp(UnaryExpr { arg, .. }) => place_name(arg),
        _ => None,
    }
}

/// The names a statement reads.
pub(super) fn uses(stmt: &Stmt) -> Vec<&Expr> {
    let mut out = Vec::new();
    match stmt {
        Stmt::Let(LetStmt { src, .. }) => collect_names(src, &mut out),
        Stmt::Eval(e) => collect_names(e, &mut out),
        Stmt::Branch(b) => {
            if let Some(c) = &b.cond {
                out.push(c);
            }
        }
        Stmt::Ret(r) => {
            if let Some(v) = &r.value {
                out.push(v);
            }
        }
        _ => {}
    }
    out
}

/// The expressions an expression node contains.
pub(super) fn children(e: &Expr) -> Vec<&Expr> {
    match e {
        Expr::UnaryOp(UnaryExpr { arg, .. }) => vec![arg],
        Expr::BinaryOp(BinaryExpr { arg1, arg2, .. }) => vec![arg1, arg2],
        Expr::Cmp(CmpExpr { arg1, arg2, .. }) => vec![arg1, arg2],
        Expr::Cast(CastExpr { data, .. }) => vec![data],
        Expr::Index(IndexExpr { base, index }) => vec![base, index],
        Expr::Call(CallExpr { args, .. }) => args.iter().collect(),
        _ => vec![],
    }
}

/// Every variable reference inside an expression tree.
fn collect_names<'a>(e: &'a Expr, out: &mut Vec<&'a Expr>) {
    if let Expr::Var(..) = e {
        out.push(e);
        return;
    }
    for child in children(e) {
        collect_names(child, out);
    }
}

/// A statement of the form `x = y`: the pair the coalescer can merge.
fn copy_pair(stmt: &Stmt) -> Option<(String, String)> {
    let Stmt::Let(LetStmt { dest, src }) = stmt else {
        return None;
    };
    if !is_plain_var(dest) {
        return None;
    }
    let (Some(d), Some(s)) = (place_name(dest), place_name(src)) else {
        return None;
    };
    Some((d.to_string(), s.to_string()))
}

fn same_name(dest: &Expr, src: &Expr) -> bool {
    match (place_name(dest), place_name(src)) {
        (Some(d), Some(s)) => d == s,
        _ => false,
    }
}

fn succs(term: &Stmt) -> Vec<&str> {
    let mut ret: Vec<&str> = vec![];
    if let Stmt::Branch(b) = term {
        ret.push(&b.then_block);
        if let Some(e) = &b.else_block {
            ret.push(e);
        }
    }
    ret
}

fn add_uses(vals: Vec<&Expr>, live: &mut Names) {
    for v in vals {
        if let Expr::Var(VarExpr { name, .. }) = v {
            live.insert(name.clone());
        }
    }
}

fn live_in_of(block: &Block, live_out: &Names) -> Names {
    let mut live = live_out.clone();
    add_uses(uses(block.term()), &mut live);
    for stmt in block.stmts.iter().rev() {
        if let Some(d) = def_name(stmt) {
            live.remove(d);
        }
        add_uses(uses(stmt), &mut live);
    }
    live
}

fn liveness(func: &FlatFunction) -> Vec<Names> {
    let index: BTreeMap<&str, usize> = func
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.name.as_str(), i))
        .collect();
    let n = func.blocks.len();
    let mut live_in = vec![Names::new(); n];
    let mut live_out = vec![Names::new(); n];
    loop {
        let mut changed = false;
        for i in (0..n).rev() {
            let block = &func.blocks[i];
            let mut out = Names::new();
            for s in succs(block.term()) {
                if let Some(&j) = index.get(s) {
                    out.extend(live_in[j].iter().cloned());
                }
            }
            let inn = live_in_of(block, &out);
            if out != live_out[i] {
                live_out[i] = out;
                changed = true;
            }
            if inn != live_in[i] {
                live_in[i] = inn;
                changed = true;
            }
        }
        if !changed {
            return live_out;
        }
    }
}

fn add_edge(adj: &mut BTreeMap<String, Names>, a: &str, b: &str) {
    adj.entry(a.to_string()).or_default().insert(b.to_string());
    adj.entry(b.to_string()).or_default().insert(a.to_string());
}

fn interference(func: &FlatFunction, live_out: &[Names]) -> BTreeMap<String, Names> {
    let mut adj: BTreeMap<String, Names> = BTreeMap::new();
    for (block, out) in func.blocks.iter().zip(live_out) {
        let mut live = out.clone();
        add_uses(uses(block.term()), &mut live);
        for stmt in block.stmts.iter().rev() {
            if let Some(d) = def_name(stmt) {
                // The two ends of a copy are the pair this pass wants to merge,
                // so they must not be adjacent. A def only conflicts with names
                // it does not itself read.
                let read = match stmt {
                    Stmt::Let(LetStmt { src, .. }) => match src.as_ref() {
                        Expr::Var(VarExpr { name, .. }) => Some(name),
                        _ => None,
                    },
                    _ => None,
                };
                for l in &live {
                    if l != d && Some(l) != read {
                        add_edge(&mut adj, d, l);
                    }
                }
                live.remove(d);
            }
            add_uses(uses(stmt), &mut live);
        }
    }
    adj
}

// ---------------------------------------------------------------------------
// renaming
// ---------------------------------------------------------------------------

fn rename_expr(e: &mut Expr, f: &dyn Fn(&str) -> String) {
    match e {
        Expr::Var(VarExpr { name, .. }) => *name = f(name),
        Expr::UnaryOp(UnaryExpr { arg, .. }) => rename_expr(arg.as_mut(), f),
        Expr::BinaryOp(BinaryExpr { arg1, arg2, .. }) => {
            rename_expr(arg1.as_mut(), f);
            rename_expr(arg2.as_mut(), f);
        }
        Expr::Cmp(CmpExpr { arg1, arg2, .. }) => {
            rename_expr(arg1.as_mut(), f);
            rename_expr(arg2.as_mut(), f);
        }
        Expr::Cast(CastExpr { data, .. }) => rename_expr(data.as_mut(), f),
        Expr::Index(IndexExpr { base, index }) => {
            rename_expr(base.as_mut(), f);
            rename_expr(index.as_mut(), f);
        }
        Expr::Call(CallExpr { args, .. }) => {
            for a in args {
                rename_expr(a, f);
            }
        }
        _ => {}
    }
}

fn rename_stmt(stmt: &mut Stmt, f: &dyn Fn(&str) -> String) {
    match stmt {
        Stmt::Let(LetStmt { dest, src }) => {
            rename_expr(dest.as_mut(), f);
            rename_expr(src.as_mut(), f);
        }
        Stmt::Eval(e) => rename_expr(e, f),
        Stmt::Branch(b) => {
            if let Some(c) = &mut b.cond {
                rename_expr(c, f);
            }
        }
        Stmt::Ret(r) => {
            if let Some(v) = &mut r.value {
                rename_expr(v, f);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::flat::FnSig;
    use crate::hir::op::{BinOp, Cmp, IntCmp};
    use crate::hir::stmt::{BranchStmt, LetStmt, RetStmt};
    use crate::hir::{Block, Lit};
    use crate::hir::{Expr, Stmt};

    fn i32t() -> Ty {
        Ty::Int(32, true)
    }

    fn var(n: &str) -> Expr {
        Expr::Var(VarExpr {
            name: n.to_string(),
            dtype: Ty::Int(32, true),
        })
    }

    fn int(value: i64) -> Expr {
        Expr::Literal(Lit::Int {
            value: value as u64,
            bits: 32,
            signed: true,
        })
    }

    fn copy(dest: &str, src: Expr) -> Stmt {
        Stmt::Let(LetStmt {
            dest: Box::new(var(dest)),
            src: Box::new(src),
        })
    }

    /// An assignment through a cell: `*dest = src`.
    fn cell(dest: &str, src: Expr) -> Stmt {
        Stmt::Let(LetStmt {
            dest: Box::new(Expr::unary(UnOp::AddrOf, var(dest))),
            src: Box::new(src),
        })
    }

    fn bin(dest: &str, op: BinOp, lhs: Expr, rhs: Expr) -> Stmt {
        Stmt::Let(LetStmt {
            dest: Box::new(var(dest)),
            src: Box::new(Expr::binary(op, lhs, rhs)),
        })
    }

    fn cmp(dest: &str, lhs: Expr, rhs: Expr) -> Stmt {
        Stmt::Let(LetStmt {
            dest: Box::new(var(dest)),
            src: Box::new(Expr::cmp(Cmp::Int(IntCmp::Eq), lhs, rhs)),
        })
    }

    fn br(target: &str) -> Stmt {
        Stmt::Branch(BranchStmt {
            cond: None,
            then_block: target.to_string(),
            else_block: None,
        })
    }

    fn cbr(cond: Expr, then: &str, else_: &str) -> Stmt {
        Stmt::Branch(BranchStmt {
            cond: Some(cond),
            then_block: then.to_string(),
            else_block: Some(else_.to_string()),
        })
    }

    fn ret(v: Option<Expr>) -> Stmt {
        Stmt::Ret(RetStmt { value: v })
    }

    fn block(label: &str, stmts: Vec<Stmt>, term: Stmt) -> Block {
        Block::build(label, stmts, Some(term))
    }

    fn function(locals: &[&str], blocks: Vec<Block>) -> FlatFunction {
        FlatFunction {
            sig: FnSig {
                name: "f".to_string(),
                params: vec![],
                ret: i32t(),
                variadic: false,
            },
            param_names: vec![],
            locals: locals.iter().map(|n| (n.to_string(), i32t())).collect(),
            blocks,
        }
    }

    fn copies_in(func: &FlatFunction) -> usize {
        func.blocks
            .iter()
            .flat_map(|b| b.stmts.iter())
            .filter(|s| copy_pair(s).is_some())
            .count()
    }

    fn get(env: &BTreeMap<String, u64>, v: &Expr) -> u64 {
        match v {
            Expr::Var(VarExpr { name, .. }) => env[name],
            Expr::Literal(Lit::Int { value, .. }) => *value,
            Expr::Cmp(CmpExpr { arg1, arg2, .. }) => u64::from(get(env, arg1) == get(env, arg2)),
            Expr::BinaryOp(BinaryExpr { op, arg1, arg2, .. }) => match op {
                BinOp::Add => get(env, arg1) + get(env, arg2),
                BinOp::Mul => get(env, arg1) * get(env, arg2),
                other => panic!("unsupported operator {other:?}"),
            },
            other => panic!("unsupported operand {other:?}"),
        }
    }

    /// Tiny interpreter: enough to run the by-hand loops.
    fn eval(func: &FlatFunction) -> u64 {
        let mut env: BTreeMap<String, u64> = BTreeMap::new();
        let mut label = func.blocks[0].name.clone();
        for _ in 0..1000 {
            let block = func
                .blocks
                .iter()
                .find(|b| b.name == label)
                .expect("branch to a missing block");
            for stmt in &block.stmts {
                let Stmt::Let(LetStmt { dest, src }) = stmt else {
                    continue;
                };
                let name = place_name(dest)
                    .expect("test place is a variable")
                    .to_string();
                env.insert(name, get(&env, src));
            }
            match &block.term() {
                Stmt::Branch(b) => label = b.then_block.clone(),
                Stmt::Ret(RetStmt { value: Some(v) }) => return get(&env, v),
                other => panic!("unsupported terminator {other:?}"),
            }
        }
        panic!("did not terminate");
    }

    fn swap_loop() -> FlatFunction {
        let done = Expr::Var(VarExpr {
            name: "done".to_string(),
            dtype: Ty::Bool,
        });
        function(
            &["a", "b", "i", "i_next", "done", "phi_tmp", "phi_tmp_1"],
            vec![
                block(
                    "entry",
                    vec![copy("a", int(1)), copy("b", int(2)), copy("i", int(0))],
                    br("loop"),
                ),
                block(
                    "loop",
                    vec![
                        bin("i_next", BinOp::Add, var("i"), int(1)),
                        cmp("done", var("i_next"), int(3)),
                    ],
                    cbr(done, "exit", "loop_to_loop"),
                ),
                block(
                    "loop_to_loop",
                    vec![
                        copy("phi_tmp", var("b")),
                        copy("phi_tmp_1", var("a")),
                        copy("a", var("phi_tmp")),
                        copy("b", var("phi_tmp_1")),
                        copy("i", var("i_next")),
                    ],
                    br("loop"),
                ),
                block("exit", vec![], ret(Some(var("a")))),
            ],
        )
    }

    #[test]
    fn swap_loop_keeps_its_result_with_fewer_copies() {
        let mut f = swap_loop();
        assert_eq!(eval(&f), 1);
        let before = copies_in(&f);

        let merged = coalesce(&mut f);

        assert_eq!(eval(&f), 1, "coalescing changed the program's result");
        assert_eq!(merged, 2);
        assert!(copies_in(&f) < before, "{before} -> {}", copies_in(&f));
        let names: Vec<&str> = f.locals.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["a", "b", "i", "done", "phi_tmp_1"]);
        // the counter now updates in place: i = i + 1
        assert!(
            matches!(&f.blocks[1].stmts[0], Stmt::Let(LetStmt { dest, .. }) if place_name(dest) == Some("i"))
        );
    }

    #[test]
    fn rotation_of_three_phis_keeps_its_result() {
        // (a, b, c) = (b, c, a) for 4 iterations, then 16a + 4b + c.
        let done = Expr::Var(VarExpr {
            name: "done".to_string(),
            dtype: Ty::Bool,
        });
        let mut f = function(
            &[
                "a",
                "b",
                "c",
                "i",
                "i_next",
                "done",
                "phi_tmp",
                "phi_tmp_1",
                "phi_tmp_2",
                "m1",
                "m2",
                "s1",
                "s2",
            ],
            vec![
                block(
                    "entry",
                    vec![
                        copy("a", int(1)),
                        copy("b", int(2)),
                        copy("c", int(3)),
                        copy("i", int(0)),
                    ],
                    br("loop"),
                ),
                block(
                    "loop",
                    vec![
                        bin("i_next", BinOp::Add, var("i"), int(1)),
                        cmp("done", var("i_next"), int(4)),
                    ],
                    cbr(done, "exit", "loop_to_loop"),
                ),
                block(
                    "loop_to_loop",
                    vec![
                        copy("phi_tmp", var("b")),
                        copy("phi_tmp_1", var("c")),
                        copy("phi_tmp_2", var("a")),
                        copy("a", var("phi_tmp")),
                        copy("b", var("phi_tmp_1")),
                        copy("c", var("phi_tmp_2")),
                        copy("i", var("i_next")),
                    ],
                    br("loop"),
                ),
                block(
                    "exit",
                    vec![
                        bin("m1", BinOp::Mul, var("a"), int(16)),
                        bin("m2", BinOp::Mul, var("b"), int(4)),
                        bin("s1", BinOp::Add, var("m1"), var("m2")),
                        bin("s2", BinOp::Add, var("s1"), var("c")),
                    ],
                    ret(Some(var("s2"))),
                ),
            ],
        );
        assert_eq!(eval(&f), 27);
        let before = copies_in(&f);
        coalesce(&mut f);
        assert_eq!(eval(&f), 27, "coalescing changed the program's result");
        assert!(copies_in(&f) < before);
    }

    #[test]
    fn interfering_copy_is_not_merged() {
        // y = x, then x is redefined while y is still needed.
        let mut f = function(
            &["x", "y", "z"],
            vec![block(
                "entry",
                vec![
                    copy("x", int(5)),
                    copy("y", var("x")),
                    bin("x", BinOp::Add, var("x"), int(1)),
                    bin("z", BinOp::Add, var("x"), var("y")),
                ],
                ret(Some(var("z"))),
            )],
        );
        assert_eq!(eval(&f), 11);
        assert_eq!(coalesce(&mut f), 0);
        assert_eq!(eval(&f), 11);
    }

    #[test]
    fn assignment_to_a_place_is_not_a_copy() {
        // `*y = x` writes through a cell. The coalescer merges copies between
        // names, not writes to a place, so the cell keeps its identity.
        let mut f = function(
            &["x", "y"],
            vec![block(
                "entry",
                vec![copy("x", int(5)), cell("y", var("x"))],
                ret(Some(var("y"))),
            )],
        );
        assert_eq!(eval(&f), 5);
        assert_eq!(coalesce(&mut f), 0);
        assert_eq!(copies_in(&f), 0);
    }

    #[test]
    fn parameters_are_never_merged() {
        let mut f = function(
            &["x"],
            vec![block(
                "entry",
                vec![copy("x", var("p"))],
                ret(Some(var("x"))),
            )],
        );
        f.param_names = vec!["p".to_string()];
        assert_eq!(coalesce(&mut f), 0);
        assert_eq!(copies_in(&f), 1);
    }

    #[test]
    fn coalescing_is_deterministic() {
        let mut first = swap_loop();
        let mut second = swap_loop();
        coalesce(&mut first);
        coalesce(&mut second);
        assert_eq!(format!("{first:?}"), format!("{second:?}"));
    }

    fn function_with_param(locals: &[&str], blocks: Vec<Block>) -> FlatFunction {
        let mut f = function(locals, blocks);
        f.sig.params = vec![i32t()];
        f.param_names = vec!["x".to_string()];
        f
    }

    #[test]
    fn slot_copied_from_a_parameter_merges_into_it() {
        // x_addr = x; add = x_addr + 1; ret add
        let mut f = function_with_param(
            &["x_addr", "add"],
            vec![block(
                "entry",
                vec![
                    copy("x_addr", var("x")),
                    bin("add", BinOp::Add, var("x_addr"), int(1)),
                ],
                ret(Some(var("add"))),
            )],
        );
        assert_eq!(coalesce(&mut f), 1);
        assert_eq!(f.param_names, ["x"]);
        let names: Vec<&str> = f.locals.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["add"]);
        assert_eq!(f.blocks[0].stmts.len() - 1, 1);
        assert!(
            matches!(&f.blocks[0].stmts[0], Stmt::Let(LetStmt { dest, src }) if
            place_name(dest) == Some("add")
                && matches!(src.as_ref(), Expr::BinaryOp(BinaryExpr { arg1, .. }) if
                    place_name(arg1) == Some("x")))
        );
    }

    #[test]
    fn parameter_stays_separate_while_still_live_after_the_slot_changes() {
        // x_addr = x; x_addr = 5; add = x_addr + x; ret add
        let mut f = function_with_param(
            &["x_addr", "add"],
            vec![block(
                "entry",
                vec![
                    copy("x_addr", var("x")),
                    copy("x_addr", int(5)),
                    bin("add", BinOp::Add, var("x_addr"), var("x")),
                ],
                ret(Some(var("add"))),
            )],
        );
        coalesce(&mut f);
        assert!(f.locals.iter().any(|(n, _)| n == "x_addr"));
    }

    #[test]
    fn two_parameters_are_never_merged() {
        let mut f = function(
            &[],
            vec![block(
                "entry",
                vec![copy("b", var("a"))],
                ret(Some(var("b"))),
            )],
        );
        f.sig.params = vec![i32t(), i32t()];
        f.param_names = vec!["a".to_string(), "b".to_string()];
        assert_eq!(coalesce(&mut f), 0);
        assert_eq!(copies_in(&f), 1);
    }
}
