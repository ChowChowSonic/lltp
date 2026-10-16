//! This pass computes liveness, builds an interference graph, and merges
//! the two ends of a `dest = src` copy into one variable whenever their
//! live ranges never overlap, then drops the copies that became `x = x`.

use std::collections::{BTreeMap, BTreeSet};

use super::flat::{FlatBlock, FlatFunction, Inst, Term, Val};
use super::ty::Ty;

type Names = BTreeSet<String>;

/// Coalesce phi-related copies in `func`. Returns how many variables were merged.
pub fn coalesce(func: &mut FlatFunction) -> usize {
    let fixed = fixed_names(func);
    let order: BTreeMap<String, usize> = func
        .locals
        .iter()
        .enumerate()
        .map(|(i, (n, _))| (n.clone(), i))
        .collect();
    let ty_of: BTreeMap<String, Ty> = func.locals.iter().cloned().collect();

    let live_out = liveness(func);
    let mut adj = interference(func, &live_out);

    let copies: Vec<(String, String)> = func
        .blocks
        .iter()
        .flat_map(|b| b.insts.iter())
        .filter_map(|inst| match inst {
            Inst::Copy {
                dest,
                src: Val::Var(s, _),
            } => Some((dest.clone(), s.clone())),
            _ => None,
        })
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
        for inst in &mut block.insts {
            rename_inst(inst, &rename);
        }
        rename_term(&mut block.term, &rename);
        block
            .insts
            .retain(|inst| !matches!(inst, Inst::Copy { dest, src: Val::Var(s, _) } if dest == s));
    }
    func.locals.retain(|(n, _)| !rep.contains_key(n));
    rep.len()
}

/// Names that must keep their identity: parameters and alloca slots.
fn fixed_names(func: &FlatFunction) -> Names {
    let mut fixed: Names = func.param_names.iter().cloned().collect();
    for block in &func.blocks {
        for inst in &block.insts {
            if let Inst::Load { ptr, .. } | Inst::Store { ptr, .. } = inst {
                fixed.insert(ptr.clone());
            }
            for v in inst_vals(inst) {
                if let Val::Addr(n) = v {
                    fixed.insert(n.clone());
                }
            }
        }
        for v in term_vals(&block.term) {
            if let Val::Addr(n) = v {
                fixed.insert(n.clone());
            }
        }
    }
    fixed
}

fn find(rep: &BTreeMap<String, String>, name: &str) -> String {
    let mut cur = name.to_string();
    while let Some(next) = rep.get(&cur) {
        cur = next.clone();
    }
    cur
}

// ---------------------------------------------------------------------------
// liveness and interference
// ---------------------------------------------------------------------------

pub(super) fn inst_vals(inst: &Inst) -> Vec<&Val> {
    match inst {
        Inst::Load { .. } => vec![],
        Inst::Store { value, .. } => vec![value],
        Inst::Bin { lhs, rhs, .. } | Inst::Cmp { lhs, rhs, .. } => vec![lhs, rhs],
        Inst::Cast { src, .. } | Inst::Copy { src, .. } => vec![src],
        Inst::Call { args, .. } => args.iter().collect(),
    }
}

pub(super) fn term_vals(term: &Term) -> Vec<&Val> {
    match term {
        Term::Br(_) | Term::Unreachable => vec![],
        Term::CondBr { cond, .. } => vec![cond],
        Term::Ret(v) => v.iter().collect(),
    }
}

fn inst_def(inst: &Inst) -> Option<&str> {
    match inst {
        Inst::Load { dest, .. }
        | Inst::Bin { dest, .. }
        | Inst::Cmp { dest, .. }
        | Inst::Cast { dest, .. }
        | Inst::Copy { dest, .. } => Some(dest.as_str()),
        Inst::Call { dest, .. } => dest.as_deref(),
        Inst::Store { .. } => None,
    }
}

fn succs(term: &Term) -> Vec<&str> {
    match term {
        Term::Br(l) => vec![l.as_str()],
        Term::CondBr {
            then_bb, else_bb, ..
        } => vec![then_bb.as_str(), else_bb.as_str()],
        Term::Ret(_) | Term::Unreachable => vec![],
    }
}

fn add_uses(vals: Vec<&Val>, live: &mut Names) {
    for v in vals {
        if let Val::Var(n, _) = v {
            live.insert(n.clone());
        }
    }
}

fn live_in_of(block: &FlatBlock, live_out: &Names) -> Names {
    let mut live = live_out.clone();
    add_uses(term_vals(&block.term), &mut live);
    for inst in block.insts.iter().rev() {
        if let Some(d) = inst_def(inst) {
            live.remove(d);
        }
        add_uses(inst_vals(inst), &mut live);
    }
    live
}

fn liveness(func: &FlatFunction) -> Vec<Names> {
    let index: BTreeMap<&str, usize> = func
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.label.as_str(), i))
        .collect();
    let n = func.blocks.len();
    let mut live_in = vec![Names::new(); n];
    let mut live_out = vec![Names::new(); n];
    loop {
        let mut changed = false;
        for i in (0..n).rev() {
            let block = &func.blocks[i];
            let mut out = Names::new();
            for s in succs(&block.term) {
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
        add_uses(term_vals(&block.term), &mut live);
        for inst in block.insts.iter().rev() {
            if let Some(d) = inst_def(inst) {
                let copy_src = match inst {
                    Inst::Copy {
                        src: Val::Var(s, _),
                        ..
                    } => Some(s.as_str()),
                    _ => None,
                };
                for l in &live {
                    if l.as_str() != d && Some(l.as_str()) != copy_src {
                        add_edge(&mut adj, d, l);
                    }
                }
                live.remove(d);
            }
            add_uses(inst_vals(inst), &mut live);
        }
    }
    adj
}

// ---------------------------------------------------------------------------
// renaming
// ---------------------------------------------------------------------------

fn rename_val(v: &mut Val, f: &dyn Fn(&str) -> String) {
    if let Val::Var(n, _) = v {
        *n = f(n);
    }
}

fn rename_inst(inst: &mut Inst, f: &dyn Fn(&str) -> String) {
    match inst {
        Inst::Load { dest, .. } => *dest = f(dest),
        Inst::Store { value, .. } => rename_val(value, f),
        Inst::Bin { dest, lhs, rhs, .. } | Inst::Cmp { dest, lhs, rhs, .. } => {
            *dest = f(dest);
            rename_val(lhs, f);
            rename_val(rhs, f);
        }
        Inst::Cast { dest, src, .. } | Inst::Copy { dest, src } => {
            *dest = f(dest);
            rename_val(src, f);
        }
        Inst::Call { dest, args, .. } => {
            if let Some(d) = dest {
                *d = f(d);
            }
            for a in args {
                rename_val(a, f);
            }
        }
    }
}

fn rename_term(term: &mut Term, f: &dyn Fn(&str) -> String) {
    match term {
        Term::CondBr { cond, .. } => rename_val(cond, f),
        Term::Ret(Some(v)) => rename_val(v, f),
        Term::Br(_) | Term::Ret(None) | Term::Unreachable => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::flat::FnSig;
    use crate::hir::op::{BinOp, Cmp, IntCmp};

    fn i32t() -> Ty {
        Ty::Int(32, true)
    }

    fn var(n: &str) -> Val {
        Val::Var(n.to_string(), i32t())
    }

    fn int(value: i64) -> Val {
        Val::Int { value, bits: 32 }
    }

    fn copy(dest: &str, src: Val) -> Inst {
        Inst::Copy {
            dest: dest.to_string(),
            src,
        }
    }

    fn bin(dest: &str, op: BinOp, lhs: Val, rhs: Val) -> Inst {
        Inst::Bin {
            dest: dest.to_string(),
            ty: i32t(),
            op,
            lhs,
            rhs,
        }
    }

    fn block(label: &str, insts: Vec<Inst>, term: Term) -> FlatBlock {
        FlatBlock {
            label: label.to_string(),
            insts,
            term,
        }
    }

    fn function(locals: &[&str], blocks: Vec<FlatBlock>) -> FlatFunction {
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
            .flat_map(|b| b.insts.iter())
            .filter(|i| matches!(i, Inst::Copy { .. }))
            .count()
    }

    fn get(env: &BTreeMap<String, i64>, v: &Val) -> i64 {
        match v {
            Val::Var(n, _) => env[n],
            Val::Int { value, .. } => *value,
            other => panic!("unsupported operand {other:?}"),
        }
    }

    /// Tiny interpreter: enough to run the by-hand loops.
    fn eval(func: &FlatFunction) -> i64 {
        let mut env: BTreeMap<String, i64> = BTreeMap::new();
        let mut label = func.blocks[0].label.clone();
        for _ in 0..1000 {
            let block = func
                .blocks
                .iter()
                .find(|b| b.label == label)
                .expect("branch to a missing block");
            for inst in &block.insts {
                let (dest, value) = match inst {
                    Inst::Copy { dest, src } => (dest, get(&env, src)),
                    Inst::Bin {
                        dest,
                        op: BinOp::Add,
                        lhs,
                        rhs,
                        ..
                    } => (dest, get(&env, lhs) + get(&env, rhs)),
                    Inst::Bin {
                        dest,
                        op: BinOp::Mul,
                        lhs,
                        rhs,
                        ..
                    } => (dest, get(&env, lhs) * get(&env, rhs)),
                    Inst::Cmp {
                        dest,
                        pred: Cmp::Int(IntCmp::Eq),
                        lhs,
                        rhs,
                    } => (dest, i64::from(get(&env, lhs) == get(&env, rhs))),
                    other => panic!("unsupported instruction {other:?}"),
                };
                env.insert(dest.clone(), value);
            }
            match &block.term {
                Term::Br(l) => label = l.clone(),
                Term::CondBr {
                    cond,
                    then_bb,
                    else_bb,
                } => {
                    label = if get(&env, cond) != 0 {
                        then_bb.clone()
                    } else {
                        else_bb.clone()
                    }
                }
                Term::Ret(Some(v)) => return get(&env, v),
                other => panic!("unsupported terminator {other:?}"),
            }
        }
        panic!("did not terminate");
    }

    fn swap_loop() -> FlatFunction {
        let done = Val::Var("done".to_string(), Ty::Bool);
        function(
            &["a", "b", "i", "i_next", "done", "phi_tmp", "phi_tmp_1"],
            vec![
                block(
                    "entry",
                    vec![copy("a", int(1)), copy("b", int(2)), copy("i", int(0))],
                    Term::Br("loop".to_string()),
                ),
                block(
                    "loop",
                    vec![
                        bin("i_next", BinOp::Add, var("i"), int(1)),
                        Inst::Cmp {
                            dest: "done".to_string(),
                            pred: Cmp::Int(IntCmp::Eq),
                            lhs: var("i_next"),
                            rhs: int(3),
                        },
                    ],
                    Term::CondBr {
                        cond: done,
                        then_bb: "exit".to_string(),
                        else_bb: "loop_to_loop".to_string(),
                    },
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
                    Term::Br("loop".to_string()),
                ),
                block("exit", vec![], Term::Ret(Some(var("a")))),
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
        assert!(matches!(
            &f.blocks[1].insts[0],
            Inst::Bin { dest, .. } if dest == "i"
        ));
    }

    #[test]
    fn rotation_of_three_phis_keeps_its_result() {
        // (a, b, c) = (b, c, a) for 4 iterations, then 16a + 4b + c.
        let done = Val::Var("done".to_string(), Ty::Bool);
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
                    Term::Br("loop".to_string()),
                ),
                block(
                    "loop",
                    vec![
                        bin("i_next", BinOp::Add, var("i"), int(1)),
                        Inst::Cmp {
                            dest: "done".to_string(),
                            pred: Cmp::Int(IntCmp::Eq),
                            lhs: var("i_next"),
                            rhs: int(4),
                        },
                    ],
                    Term::CondBr {
                        cond: done,
                        then_bb: "exit".to_string(),
                        else_bb: "loop_to_loop".to_string(),
                    },
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
                    Term::Br("loop".to_string()),
                ),
                block(
                    "exit",
                    vec![
                        bin("m1", BinOp::Mul, var("a"), int(16)),
                        bin("m2", BinOp::Mul, var("b"), int(4)),
                        bin("s1", BinOp::Add, var("m1"), var("m2")),
                        bin("s2", BinOp::Add, var("s1"), var("c")),
                    ],
                    Term::Ret(Some(var("s2"))),
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
                Term::Ret(Some(var("z"))),
            )],
        );
        assert_eq!(eval(&f), 11);
        assert_eq!(coalesce(&mut f), 0);
        assert_eq!(eval(&f), 11);
    }

    #[test]
    fn parameters_are_never_merged() {
        let mut f = function(
            &["x"],
            vec![block(
                "entry",
                vec![copy("x", var("p"))],
                Term::Ret(Some(var("x"))),
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
}
