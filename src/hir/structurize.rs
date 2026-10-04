use std::collections::{BTreeSet, VecDeque};

pub use crate::hir::flow::reducible;
use crate::hir::flow::{
    DomInfo, NaturalLoop, PostDomInfo, dominators, natural_loops, reachable, reachable_from,
};
use crate::hir::{Cfg, Expr, Lit, Stmt, StructurizeError, Ty};

pub struct LoopTree {
    pub loops: Vec<NaturalLoop>,
    pub parent: Vec<Option<usize>>,
    pub roots: Vec<usize>,
    pub exit: Vec<Option<String>>,
}

/// Context passed down during recursive region structuring to track the
/// enclosing loop boundary and fallouts.
#[derive(Clone, Debug)]
struct LoopContext {
    header: String,
    exit: Option<String>,
    latch: Option<String>,
}
/// Reduce a reducible CFG to structured statements. Irreducible graphs,
/// multi-exit / multi-entry loops, and parallel acyclic joins are rejected
/// here; Task 5 (node splitting + differential execution) lifts those.
pub fn structurize(cfg: &Cfg) -> Result<Vec<Stmt>, StructurizeError> {
    if !reducible(cfg) {
        return Err(StructurizeError::IrreducibleControlFlow);
    }
    let dom = dominators(cfg);
    let pdom = PostDomInfo::from(cfg);
    let reach = reachable(cfg);
    let tree = LoopTree::build(cfg, &dom, &reach)?;
    structure_region(
        cfg,
        &dom,
        &pdom,
        &tree,
        &reach,
        &cfg.entry,
        &BTreeSet::new(),
        None,
    )
}

fn structure_loop(
    cfg: &Cfg,
    dom: &DomInfo,
    pdom: &PostDomInfo,
    tree: &LoopTree,
    loop_idx: usize,
) -> Result<Stmt, StructurizeError> {
    let l = &tree.loops[loop_idx];
    let header = &l.header;
    let exit_target = tree.exit[loop_idx].as_ref();

    let (prefix, branch) = cfg.split_block(header)?;
    let succs = &cfg.blocks[header].succ;

    let has_exit_edge = if let Some(exit_name) = exit_target {
        succs.contains(exit_name)
    } else {
        false
    };

    let back_edge_srcs: Vec<String> = l.back_edges.iter().map(|(src, _)| src.clone()).collect();
    let latch = if back_edge_srcs.len() == 1 {
        Some(back_edge_srcs[0].clone())
    } else {
        None
    };

    let ctx = LoopContext {
        header: header.clone(),
        exit: exit_target.cloned(),
        latch,
    };

    let mut body_frame_exits = BTreeSet::from([header.clone()]);
    if let Some(e) = exit_target {
        body_frame_exits.insert(e.clone());
    }

    if has_exit_edge {
        let body_succs: Vec<String> = succs
            .iter()
            .filter(|s| l.body.contains(*s))
            .cloned()
            .collect();
        if body_succs.is_empty() {
            return Err(StructurizeError::AbnormalLoopExit {
                block: header.clone(),
                target: "empty loop body".to_string(),
            });
        }
        let body_entry = &body_succs[0];

        let cond = if branch.as_ref().is_none_or(|b| b.then_block == *body_entry) {
            branch
                .as_ref()
                .and_then(|b| b.cond.clone())
                .unwrap_or(Expr::Nop)
        } else {
            let c = branch
                .as_ref()
                .and_then(|b| b.cond.clone())
                .unwrap_or(Expr::Nop);
            Expr::BinaryOp {
                op: inkwell::values::InstructionOpcode::Xor,
                arg1: Box::new(c),
                arg2: Box::new(Expr::Literal(Lit::Bool(true))),
            }
        };

        let mut body_nodes = l.body.clone();
        body_nodes.remove(header);

        let mut body_stmts = prefix;
        if body_nodes.is_empty() {
            if body_stmts.is_empty() {
                body_stmts.push(placeholder());
            }
        } else {
            body_stmts.extend(structure_region(
                cfg,
                dom,
                pdom,
                tree,
                &body_nodes,
                body_entry,
                &body_frame_exits,
                Some(&ctx),
            )?);
            if body_stmts.is_empty() {
                body_stmts.push(placeholder());
            }
        }

        Ok(Stmt::Loop {
            cond: Some(cond),
            body: body_stmts,
        })
    } else {
        let body_succs: Vec<String> = succs
            .iter()
            .filter(|s| l.body.contains(*s))
            .cloned()
            .collect();
        if body_succs.is_empty() {
            return Err(StructurizeError::AbnormalLoopExit {
                block: header.clone(),
                target: "empty loop body".to_string(),
            });
        }

        if body_succs.len() == 1 && body_succs[0] != *header {
            let body_entry = &body_succs[0];
            let mut body_nodes = l.body.clone();
            body_nodes.remove(header);

            let mut body_stmts = prefix;
            body_stmts.extend(structure_region(
                cfg,
                dom,
                pdom,
                tree,
                &body_nodes,
                body_entry,
                &body_frame_exits,
                Some(&ctx),
            )?);
            if body_stmts.is_empty() {
                body_stmts.push(placeholder());
            }

            Ok(Stmt::Loop {
                cond: None,
                body: body_stmts,
            })
        } else if body_succs.len() == 2 {
            let cond = branch
                .as_ref()
                .and_then(|b| b.cond.clone())
                .unwrap_or(Expr::Nop);
            let then_is_first = branch
                .as_ref()
                .is_none_or(|b| b.then_block == body_succs[0]);
            let (first_b, second_b) = if then_is_first {
                (&body_succs[0], &body_succs[1])
            } else {
                (&body_succs[1], &body_succs[0])
            };

            let mut body_nodes = l.body.clone();
            body_nodes.remove(header);

            let a_then = reachable_from(cfg, first_b, &body_nodes);
            let a_else = reachable_from(cfg, second_b, &body_nodes);

            let mut region_then = structure_region(
                cfg,
                dom,
                pdom,
                tree,
                &a_then,
                first_b,
                &body_frame_exits,
                Some(&ctx),
            )?;
            let mut region_else = structure_region(
                cfg,
                dom,
                pdom,
                tree,
                &a_else,
                second_b,
                &body_frame_exits,
                Some(&ctx),
            )?;

            if region_then.is_empty() {
                region_then.push(placeholder());
            }
            if region_else.is_empty() {
                region_else.push(placeholder());
            }

            let mut body_stmts = prefix;
            body_stmts.push(Stmt::If {
                cond,
                then_stmts: region_then,
                else_stmts: region_else,
            });

            Ok(Stmt::Loop {
                cond: None,
                body: body_stmts,
            })
        } else {
            let mut body_stmts = prefix;
            if body_stmts.is_empty() {
                body_stmts.push(placeholder());
            }

            Ok(Stmt::Loop {
                cond: None,
                body: body_stmts,
            })
        }
    }
}

fn partition_arm(
    cfg: &Cfg,
    branch: &str,
    join: Option<&String>,
    within_nodes: &BTreeSet<String>,
) -> BTreeSet<String> {
    let mut ret = BTreeSet::new();
    let mut q: VecDeque<String> = VecDeque::new();
    q.push_back(branch.to_string());
    while let Some(name) = q.pop_front() {
        if !ret.insert(name.clone()) {
            continue;
        }
        let current = cfg.blocks.get(&name);
        if let Some(block) = current {
            for x in &block.succ {
                if within_nodes.contains(x) && Some(x) != join {
                    q.push_back(x.clone());
                }
            }
        }
    }
    ret
}
fn partition_arms(
    cfg: &Cfg,
    then_branch: &str,
    else_branch: &str,
    join: Option<&String>,
    nodes: &BTreeSet<String>,
) -> (BTreeSet<String>, BTreeSet<String>) {
    let then_arm: BTreeSet<String> = partition_arm(cfg, then_branch, join, nodes);
    let else_arm: BTreeSet<String> = partition_arm(cfg, else_branch, join, nodes);
    (then_arm, else_arm)
}

#[allow(clippy::too_many_arguments)]
/// Structure a region of `nodes` starting from `entry`.
fn structure_region(
    cfg: &Cfg,
    dom: &DomInfo,
    pdom: &PostDomInfo,
    tree: &LoopTree,
    nodes: &BTreeSet<String>,
    entry: &str,
    frame_exits: &BTreeSet<String>,
    loop_ctx: Option<&LoopContext>,
) -> Result<Vec<Stmt>, StructurizeError> {
    if nodes.is_empty() || frame_exits.contains(entry) || !nodes.contains(entry) {
        return Ok(Vec::new());
    }

    if let Some(loop_idx) = tree.get_loop_for_header(entry)
        && loop_ctx.is_none_or(|ctx| ctx.header != entry)
    {
        let loop_stmt = structure_loop(cfg, dom, pdom, tree, loop_idx)?;
        let mut stmts = vec![loop_stmt];

        if let Some(exit_node) = &tree.exit[loop_idx]
            && !frame_exits.contains(exit_node)
        {
            let remaining_nodes: BTreeSet<String> = nodes
                .difference(&tree.loops[loop_idx].body)
                .cloned()
                .collect();
            if remaining_nodes.contains(exit_node) {
                let cont_stmts = structure_region(
                    cfg,
                    dom,
                    pdom,
                    tree,
                    &remaining_nodes,
                    exit_node,
                    frame_exits,
                    loop_ctx,
                )?;
                stmts.extend(cont_stmts);
            }
        }
        return Ok(stmts);
    }
    let (mut prefix, branch) = cfg.split_block(entry)?;

    let blk = &cfg.blocks[entry];
    let mut s_in: Vec<String> = Vec::new();
    let mut s_out: Vec<String> = Vec::new();
    for s in &blk.succ {
        if nodes.contains(s) {
            s_in.push(s.clone());
        } else {
            s_out.push(s.clone());
        }
    }
    // Every out-of-region successor must be a legitimate frame boundary: a
    // known exit of the enclosing frame, the loop exit, or the loop header
    // (a continue edge).
    for x in &s_out {
        if !frame_exits.contains(x) {
            if let Some(ctx) = loop_ctx
                && (ctx.exit.as_ref() == Some(x) || &ctx.header == x)
            {
                continue;
            }
            return Err(StructurizeError::AbnormalLoopExit {
                block: entry.to_string(),
                target: x.to_string(),
            });
        }
    }
    match s_in.len() {
        0 => {
            if s_out.is_empty() {
                // The path terminates here; make it explicit so the rebuilt
                // CFG keeps this block as a real exit.
                if prefix.is_empty() {
                    prefix.push(Stmt::Ret { value: None });
                }
                return Ok(prefix);
            }
            if s_out.len() == 1 {
                let target = &s_out[0];
                if let Some(ctx) = loop_ctx {
                    if ctx.exit.as_ref() == Some(target) {
                        prefix.push(Stmt::Break);
                        return Ok(prefix);
                    }
                    if &ctx.header == target {
                        if ctx.latch.as_ref() == Some(&entry.to_string()) {
                            // The latch falls out of the body; the Loop
                            // rebuild materializes the back edge itself.
                            return Ok(prefix);
                        }
                        prefix.push(Stmt::Continue);
                        return Ok(prefix);
                    }
                }
                // Otherwise the target is a frame exit; falling through is
                // the correct control flow.
                return Ok(prefix);
            }
            if s_out.len() == 2
                && let Some(ctx) = loop_ctx
            {
                let cond = branch
                    .as_ref()
                    .and_then(|b| b.cond.clone())
                    .unwrap_or(Expr::Nop);
                let then_is_first = branch.as_ref().is_none_or(|b| b.then_block == s_out[0]);
                let (tb, eb) = if then_is_first {
                    (&s_out[0], &s_out[1])
                } else {
                    (&s_out[1], &s_out[0])
                };

                let emit_target = |target: &String| -> Option<Stmt> {
                    if ctx.exit.as_ref() == Some(target) {
                        Some(Stmt::Break)
                    } else if &ctx.header == target {
                        if ctx.latch.as_ref() == Some(&entry.to_string()) {
                            None
                        } else {
                            Some(Stmt::Continue)
                        }
                    } else {
                        None
                    }
                };

                prefix.push(Stmt::If {
                    cond,
                    then_stmts: emit_target(tb).into_iter().collect(),
                    else_stmts: emit_target(eb).into_iter().collect(),
                });
                return Ok(prefix);
            }
            Ok(prefix)
        }
        1 => {
            let mut remaining = nodes.clone();
            remaining.remove(entry);
            if s_out.is_empty() {
                prefix.extend(structure_region(
                    cfg,
                    dom,
                    pdom,
                    tree,
                    &remaining,
                    &s_in[0],
                    frame_exits,
                    loop_ctx,
                )?);
                return Ok(prefix);
            }
            // One in-region successor and one out-of-region exit: the
            // `if (c) break;` / `if (c) continue;` shape inside a loop.
            let cond = branch
                .as_ref()
                .and_then(|b| b.cond.clone())
                .unwrap_or(Expr::Nop);
            let in_arm = reachable_from(cfg, &s_in[0], &remaining);
            let exit_target = &s_out[0];
            let in_arm_stmts = structure_region(
                cfg,
                dom,
                pdom,
                tree,
                &in_arm,
                &s_in[0],
                frame_exits,
                loop_ctx,
            )?;
            let in_arm_stmts = if in_arm_stmts.is_empty() {
                vec![placeholder()]
            } else {
                in_arm_stmts
            };
            let then_is_exit = branch
                .as_ref()
                .is_some_and(|b| b.then_block == *exit_target);
            if let Some(ctx) = loop_ctx {
                if ctx.exit.as_ref() == Some(exit_target) {
                    let (then_stmts, else_stmts) = if then_is_exit {
                        (vec![Stmt::Break], in_arm_stmts.clone())
                    } else {
                        (in_arm_stmts.clone(), vec![Stmt::Break])
                    };
                    prefix.push(Stmt::If {
                        cond,
                        then_stmts,
                        else_stmts,
                    });
                    return Ok(prefix);
                }
                if &ctx.header == exit_target {
                    let (then_stmts, else_stmts) = if then_is_exit {
                        (vec![Stmt::Continue], in_arm_stmts.clone())
                    } else {
                        (in_arm_stmts.clone(), vec![Stmt::Continue])
                    };
                    prefix.push(Stmt::If {
                        cond,
                        then_stmts,
                        else_stmts,
                    });
                    return Ok(prefix);
                }
            }
            // The exit target is a frame exit; the false path falls through
            // to it, so only the in-region arm goes inside the If.
            prefix.push(Stmt::If {
                cond,
                then_stmts: in_arm_stmts,
                else_stmts: vec![],
            });
            Ok(prefix)
        }
        2 => {
            // LLVM lists the false target first in `br` operands; order the
            // arms by the recorded branch info when available.
            let then_is_first = branch.as_ref().is_none_or(|b| b.then_block == s_in[0]);
            let (first, second) = if then_is_first {
                (&s_in[0], &s_in[1])
            } else {
                (&s_in[1], &s_in[0])
            };
            let join = pdom.ipdom.get(entry);

            let (then_arm, else_arm) = partition_arms(cfg, first, second, join, nodes);
            let mut arm_frame_exits = frame_exits.clone();
            if let Some(j) = join {
                arm_frame_exits.insert(j.clone());
            }

            let then_stmts = structure_region(
                cfg,
                dom,
                pdom,
                tree,
                &then_arm,
                first,
                &arm_frame_exits,
                loop_ctx,
            )?;
            let mut else_stmts = structure_region(
                cfg,
                dom,
                pdom,
                tree,
                &else_arm,
                second,
                &arm_frame_exits,
                loop_ctx,
            )?;
            // An empty else arm must still materialize a block in the
            // rebuilt CFG so the arm's edge to the join is preserved —
            // unless the arm *is* the join itself, which the tail
            // continuation below materializes.
            let join_materialized_by_tail = join.is_some_and(|jn| {
                else_arm.len() == 1
                    && else_arm.first() == Some(jn)
                    && nodes.contains(jn)
                    && !frame_exits.contains(jn)
            });
            if else_stmts.is_empty() && !join_materialized_by_tail {
                else_stmts.push(placeholder());
            }
            let cond = branch
                .as_ref()
                .and_then(|b| b.cond.clone())
                .unwrap_or(Expr::Literal(Lit::Bool(true)));
            prefix.push(Stmt::If {
                cond,
                then_stmts,
                else_stmts,
            });

            if let Some(jn) = join
                && nodes.contains(jn)
                && !frame_exits.contains(jn)
            {
                let tail_nodes = reachable_from(cfg, jn, nodes);
                let cont_stmts =
                    structure_region(cfg, dom, pdom, tree, &tail_nodes, jn, frame_exits, loop_ctx)?;
                prefix.extend(cont_stmts);
            }
            Ok(prefix)
        }
        _ => {
            // Fan-out >= 3 (switch shape). `ipdom(entry)` cannot serve as the
            // join here: when any arm terminates early the only common
            // post-dominator is the virtual sink. Derive the join from the
            // arms instead — every arm that re-converges inside the region
            // must agree on one immediate post-dominator, arms whose ipdom is
            // the virtual sink simply terminate, and arms converging to
            // different nodes are parallel joins that need node splitting.
            let entry_blk = cfg
                .get_block(entry)
                .ok_or(StructurizeError::ParallelJoinsNeedNodeSplitting)?;
            let mut join: Option<String> = None;
            for current_succ in &entry_blk.succ {
                let Some(candidate) = pdom.ipdom.get(current_succ) else {
                    continue;
                };
                if !nodes.contains(candidate) || frame_exits.contains(candidate) {
                    continue;
                }
                match &join {
                    Some(j) if j == candidate => {}
                    Some(_) => return Err(StructurizeError::ParallelJoinsNeedNodeSplitting),
                    None => join = Some(candidate.clone()),
                }
            }
            let joins = join.as_ref();

            let mut switch_arms: Vec<(Lit, Vec<Stmt>)> = Vec::new();
            for (i, current_succ) in entry_blk.succ.iter().enumerate() {
                let mut arm_frame_exits = frame_exits.clone();
                if let Some(jn) = joins {
                    arm_frame_exits.insert(jn.clone());
                }

                let arm = partition_arm(cfg, current_succ, joins, nodes);
                let mut arm_stmts = structure_region(
                    cfg,
                    dom,
                    pdom,
                    tree,
                    &arm,
                    current_succ,
                    &arm_frame_exits,
                    loop_ctx,
                )?;
                if arm_stmts.is_empty() && arm.len() != 1 || arm.first() != Some(joins.unwrap()) {
                    arm_stmts.push(placeholder());
                }
                switch_arms.push((
                    Lit::Int {
                        value: i as u64,
                        bits: 32,
                        signed: true,
                    },
                    arm_stmts,
                ));
            }
            let starting = Expr::Var {
                name: "_switch_cond".into(),
                dtype: Ty::Int(32, true),
            }; //cfg.get_block(entry).unwrap().stmts;
            let default: Vec<Stmt> = switch_arms.pop().unwrap().1;
            let mut prefix: Vec<Stmt> = vec![Stmt::Switch {
                value: starting,
                cases: switch_arms,
                default,
            }];

            if let Some(jn) = joins
                && nodes.contains(jn)
                && !frame_exits.contains(jn)
            {
                let tail_nodes = reachable_from(cfg, jn, nodes);
                let cont_stmts =
                    structure_region(cfg, dom, pdom, tree, &tail_nodes, jn, frame_exits, loop_ctx)?;
                prefix.extend(cont_stmts);
            }

            Ok(prefix)
        }
    }
}

/// A statement that occupies a block without changing its semantics, used to
/// materialize bare (statement-free) arm blocks in the rebuilt CFG.
fn placeholder() -> Stmt {
    Stmt::Let {
        dest: Box::new(Expr::Var {
            name: "_unused".into(),
            dtype: Ty::Void,
        }),
        src: Box::new(Expr::Literal(Lit::Int {
            value: 0,
            bits: 32,
            signed: true,
        })),
    }
}

impl LoopTree {
    pub fn get_loop_for_header(&self, header: &str) -> Option<usize> {
        self.loops.iter().position(|l| l.header == header)
    }

    fn build(
        cfg: &Cfg,
        dom: &DomInfo,
        reach: &BTreeSet<String>,
    ) -> Result<LoopTree, StructurizeError> {
        let mut loops = natural_loops(cfg, dom);
        loops.sort_by(|a, b| (a.body.len(), &a.header).cmp(&(b.body.len(), &b.header)));
        let parent: Vec<Option<usize>> = (0..loops.len())
            .map(|i| {
                (0..loops.len())
                    .filter(|&j| {
                        j != i
                            && loops[j].body.len() > loops[i].body.len()
                            && loops[j].body.is_superset(&loops[i].body)
                    })
                    .min_by_key(|&j| (loops[j].body.len(), loops[j].header.clone()))
            })
            .collect();
        let mut exit = Vec::with_capacity(loops.len());
        let ancestors: Vec<Vec<usize>> = (0..loops.len())
            .map(|i| {
                (0..loops.len())
                    .filter(|&a| {
                        a != i
                            && loops[a].body.len() > loops[i].body.len()
                            && loops[a].body.is_superset(&loops[i].body)
                    })
                    .collect()
            })
            .collect();
        for (i, l) in loops.iter().enumerate() {
            for x in &l.body {
                if x == &l.header {
                    continue;
                }
                if cfg.blocks[x].pred.iter().any(|p| !l.body.contains(p)) {
                    return Err(StructurizeError::MultiExitLoop {
                        header: l.header.clone(),
                        exits: vec![],
                    });
                }
            }
            let mut targets: BTreeSet<String> = BTreeSet::new();
            for x in &l.body {
                for s in &cfg.blocks[x].succ {
                    if !l.body.contains(s)
                        && reach.contains(s)
                        && !ancestors[i].iter().any(|&a| loops[a].header == *s)
                    {
                        targets.insert(s.clone());
                    }
                }
            }
            exit.push(match targets.len() {
                0 => None,
                1 => targets.into_iter().next(),
                _ => {
                    return Err(StructurizeError::MultiExitLoop {
                        header: l.header.clone(),
                        exits: targets.into_iter().collect(),
                    });
                }
            });
        }
        for i in 0..loops.len() {
            let Some(e) = &exit[i] else { continue };
            if let Some(p) = parent[i]
                && !loops[p].body.contains(e)
            {
                return Err(StructurizeError::NestedLoopEscapedParent {
                    header: loops[i].header.clone(),
                    exit: e.clone(),
                });
            }
        }

        let roots: Vec<usize> = (0..loops.len()).filter(|&i| parent[i].is_none()).collect();
        Ok(LoopTree {
            loops,
            parent,
            roots,
            exit,
        })
    }
}

#[cfg(test)]
mod test {
    use std::collections::BTreeSet;

    use crate::hir::{
        cfg::Cfg,
        error::StructurizeError,
        flow::{dominators, reachable},
        structurize::{LoopTree, structurize},
        verify::{are_equivalent, structured_to_cfg},
    };

    fn cfg_from(entry: &str, edges: &[(&str, &str)]) -> Cfg {
        let mut nodes: BTreeSet<String> = BTreeSet::new();
        nodes.insert(entry.to_string());
        for (a, b) in edges {
            nodes.insert((*a).to_string());
            nodes.insert((*b).to_string());
        }
        let mut succ: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        let mut pred: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for n in &nodes {
            succ.entry(n.clone()).or_default();
            pred.entry(n.clone()).or_default();
        }
        for (a, b) in edges {
            succ.get_mut(*a).unwrap().push((*b).to_string());
            pred.get_mut(*b).unwrap().push((*a).to_string());
        }
        let exits: Vec<String> = nodes
            .iter()
            .filter(|n| succ.get(*n).is_some_and(|s| s.is_empty()))
            .cloned()
            .collect();
        Cfg::new(entry.to_string(), succ, pred, exits)
    }

    fn tree(cfg: &Cfg) -> LoopTree {
        LoopTree::build(cfg, &dominators(cfg), &reachable(cfg)).unwrap()
    }

    #[test]
    fn structurize_rejects_irreducible() {
        let cfg = cfg_from(
            "A",
            &[
                ("A", "B"),
                ("A", "C"),
                ("B", "A"),
                ("B", "C"),
                ("C", "A"),
                ("C", "B"),
            ],
        );
        assert_eq!(
            structurize(&cfg),
            Err(StructurizeError::IrreducibleControlFlow)
        );
    }

    #[test]
    fn multi_exit_loop_is_rejected() {
        let cfg = cfg_from(
            "entry",
            &[
                ("entry", "h"),
                ("h", "b"),
                ("h", "e1"),
                ("b", "h"),
                ("b", "e2"),
            ],
        );
        assert!(matches!(
            LoopTree::build(&cfg, &dominators(&cfg), &reachable(&cfg)),
            Err(StructurizeError::MultiExitLoop { .. })
        ));
    }

    #[test]
    fn loop_tree_nests_while_in_while() {
        let cfg = cfg_from(
            "entry",
            &[
                ("entry", "oh"),
                ("oh", "B"),
                ("oh", "OEX"),
                ("B", "ih"),
                ("ih", "C"),
                ("ih", "IEX"),
                ("C", "ih"),
                ("IEX", "T"),
                ("T", "oh"),
            ],
        );
        let t = tree(&cfg);
        assert_eq!(t.roots, vec![1]);
        assert_eq!(t.parent, vec![Some(1), None]);
        assert_eq!(t.exit, vec![Some("IEX".into()), Some("OEX".into())]);
        assert_eq!(
            t.loops[0].body,
            BTreeSet::from(["ih".to_string(), "C".to_string()])
        );
    }

    #[test]
    fn sequential_loops_are_sibling_roots() {
        let cfg = cfg_from(
            "entry",
            &[
                ("entry", "h1"),
                ("h1", "b1"),
                ("h1", "e1"),
                ("b1", "h1"),
                ("e1", "h2"),
                ("h2", "b2"),
                ("h2", "e2"),
                ("b2", "h2"),
            ],
        );
        let t = tree(&cfg);
        assert_eq!(t.parent, vec![None, None]);
        assert_eq!(t.roots, vec![0, 1]);
        assert_eq!(t.exit, vec![Some("e1".into()), Some("e2".into())]);
    }

    #[test]
    fn dispatch_loop_has_no_exit() {
        let cfg = cfg_from(
            "entry",
            &[
                ("entry", "h"),
                ("h", "b"),
                ("h", "c"),
                ("b", "h"),
                ("c", "h"),
            ],
        );
        let t = tree(&cfg);
        assert_eq!(t.roots, vec![0]);
        assert_eq!(t.exit, vec![None]);
        assert_eq!(
            t.loops[0].back_edges,
            vec![("b".into(), "h".into()), ("c".into(), "h".into())]
        );
    }

    #[test]
    fn nested_loop_jumping_out_of_its_parent_is_rejected() {
        let cfg = cfg_from(
            "entry",
            &[
                ("entry", "oh"),
                ("oh", "B"),
                ("oh", "e"),
                ("B", "ih"),
                ("ih", "C"),
                ("ih", "e"),
                ("C", "ih"),
                ("C", "oh"),
            ],
        );
        assert!(matches!(
            LoopTree::build(&cfg, &dominators(&cfg), &reachable(&cfg)),
            Err(StructurizeError::NestedLoopEscapedParent { .. })
        ));
    }

    #[test]
    fn while_loop_round_trips() {
        let cfg = cfg_from(
            "entry",
            &[("entry", "h"), ("h", "b"), ("h", "ex"), ("b", "h")],
        );
        let stmts = structurize(&cfg).unwrap();
        assert!(are_equivalent(&cfg, &structured_to_cfg(&stmts)));
    }

    #[test]
    fn while_loop_with_break_round_trips() {
        let cfg = cfg_from(
            "entry",
            &[("entry", "h"), ("h", "b"), ("h", "ex"), ("b", "ex")],
        );
        let stmts = structurize(&cfg).unwrap();
        assert!(are_equivalent(&cfg, &structured_to_cfg(&stmts)));
    }

    #[test]
    fn while_loop_with_continue_round_trips() {
        let cfg = cfg_from(
            "entry",
            &[
                ("entry", "h"),
                ("h", "b"),
                ("h", "ex"),
                ("b", "cont"),
                ("b", "latch"),
                ("cont", "h"),
                ("latch", "h"),
            ],
        );
        let stmts = structurize(&cfg).unwrap();
        assert!(are_equivalent(&cfg, &structured_to_cfg(&stmts)));
    }

    #[test]
    fn nested_while_loops_round_trip() {
        let cfg = cfg_from(
            "entry",
            &[
                ("entry", "oh"),
                ("oh", "B"),
                ("oh", "OEX"),
                ("B", "ih"),
                ("ih", "C"),
                ("ih", "IEX"),
                ("C", "ih"),
                ("IEX", "T"),
                ("T", "oh"),
            ],
        );
        let stmts = structurize(&cfg).unwrap();
        assert!(are_equivalent(&cfg, &structured_to_cfg(&stmts)));
    }

    #[test]
    fn sequential_loops_round_trip() {
        let cfg = cfg_from(
            "entry",
            &[
                ("entry", "h1"),
                ("h1", "b1"),
                ("h1", "e1"),
                ("b1", "h1"),
                ("e1", "h2"),
                ("h2", "b2"),
                ("h2", "e2"),
                ("b2", "h2"),
            ],
        );
        let stmts = structurize(&cfg).unwrap();
        assert!(are_equivalent(&cfg, &structured_to_cfg(&stmts)));
    }

    #[test]
    fn infinite_loop_with_break_round_trips() {
        let cfg = cfg_from(
            "entry",
            &[
                ("entry", "h"),
                ("h", "b"),
                ("b", "ex"),
                ("b", "latch"),
                ("latch", "h"),
            ],
        );
        let stmts = structurize(&cfg).unwrap();
        assert!(are_equivalent(&cfg, &structured_to_cfg(&stmts)));
    }

    #[test]
    fn dispatch_loop_round_trips() {
        let cfg = cfg_from(
            "entry",
            &[
                ("entry", "h"),
                ("h", "c1"),
                ("h", "c2"),
                ("c1", "h"),
                ("c2", "h"),
            ],
        );
        let stmts = structurize(&cfg).unwrap();
        assert!(are_equivalent(&cfg, &structured_to_cfg(&stmts)));
    }

    #[test]
    fn straight_line_round_trips() {
        let cfg = cfg_from("a", &[("a", "b"), ("b", "c")]);
        let stmts = structurize(&cfg).unwrap();
        assert!(are_equivalent(&cfg, &structured_to_cfg(&stmts)));
    }

    #[test]
    fn diamond_join_round_trips() {
        let cfg = cfg_from(
            "entry",
            &[
                ("entry", "l"),
                ("entry", "r"),
                ("l", "join"),
                ("r", "join"),
                ("join", "exit"),
            ],
        );
        let stmts = structurize(&cfg).unwrap();
        assert!(are_equivalent(&cfg, &structured_to_cfg(&stmts)));
    }

    #[test]
    fn if_else_round_trips() {
        let cfg = cfg_from("e", &[("e", "a"), ("e", "b")]);
        let stmts = structurize(&cfg).unwrap();
        assert!(are_equivalent(&cfg, &structured_to_cfg(&stmts)));
    }

    #[test]
    fn if_without_else_round_trips() {
        let cfg = cfg_from("e", &[("e", "a"), ("e", "j"), ("a", "j")]);
        let stmts = structurize(&cfg).unwrap();
        assert!(are_equivalent(&cfg, &structured_to_cfg(&stmts)));
    }

    #[test]
    fn sequential_joins_round_trip() {
        let cfg = cfg_from(
            "e",
            &[
                ("e", "a"),
                ("e", "b"),
                ("a", "j1"),
                ("b", "j1"),
                ("j1", "c"),
                ("j1", "d"),
                ("c", "j2"),
                ("d", "j2"),
                ("j2", "t"),
            ],
        );
        let stmts = structurize(&cfg).unwrap();
        assert!(are_equivalent(&cfg, &structured_to_cfg(&stmts)));
    }

    #[test]
    fn parallel_joins_need_node_splitting() {
        let cfg = cfg_from(
            "e",
            &[
                ("e", "a"),
                ("e", "b"),
                ("e", "c"),
                ("e", "d"),
                ("a", "j1"),
                ("b", "j1"),
                ("c", "j2"),
                ("d", "j2"),
                ("j1", "x"),
                ("j2", "y"),
            ],
        );
        assert_eq!(
            structurize(&cfg),
            Err(StructurizeError::ParallelJoinsNeedNodeSplitting)
        );
    }
    #[test]
    fn three_way_round_trips() {
        let cfg = cfg_from(
            "entry",
            &[
                ("entry", "case_a"),
                ("entry", "case_b"),
                ("entry", "case_c"),
                ("case_a", "join"),
                ("case_c", "join"),
                ("join", "exit"),
            ],
        );
        let stmts = structurize(&cfg).unwrap();
        assert!(are_equivalent(&cfg, &structured_to_cfg(&stmts)))
    }
}
