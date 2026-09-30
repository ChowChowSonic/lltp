use std::collections::BTreeSet;

use crate::hir::cfg::Cfg;
use crate::hir::expr::Expr;
use crate::hir::flow::{DomInfo, NaturalLoop, dominators, natural_loops, reachable, reducible};
use crate::hir::stmt::Stmt;

pub struct LoopTree {
    pub loops: Vec<NaturalLoop>,
    pub parent: Vec<Option<usize>>,
    pub roots: Vec<usize>,
    pub exit: Vec<Option<String>>,
}

/// Reduce a reducible CFG to structured statements. Irreducible graphs,
/// multi-exit / multi-entry loops, and parallel acyclic joins are rejected
/// here; Task 5 (node splitting + differential execution) lifts those.
pub fn structurize(cfg: &Cfg) -> Result<Vec<Stmt>, &'static str> {
    if !reducible(cfg) {
        return Err("irreducible control flow (node splitting is Task 5)");
    }
    let dom = dominators(cfg);
    let reach = reachable(cfg);
    let tree = LoopTree::build(cfg, &dom, &reach)?;
    if !tree.loops.is_empty() {
        return Err("structurizer: loops not yet implemented");
    }
    structure_frame(cfg, &dom, &reach, &cfg.entry, &BTreeSet::new())
}

/// Structure one single-entry acyclic region `nodes` into statements.
///
/// `frame_exits` names the successor blocks that live outside the region. A
/// block inside the region may branch straight to one of them (it becomes an
/// empty If arm / a fall-through), but leaving toward anything else is an
/// error. At the top level there are no frame exits.
fn structure_frame(
    cfg: &Cfg,
    dom: &DomInfo,
    nodes: &BTreeSet<String>,
    entry: &str,
    frame_exits: &BTreeSet<String>,
) -> Result<Vec<Stmt>, &'static str> {
    let blk = &cfg.blocks[entry];
    // Successors inside the region vs. those leaving it.
    let mut s_in: Vec<String> = Vec::new();
    let mut s_out: Vec<String> = Vec::new();
    for s in &blk.succ {
        if nodes.contains(s) {
            s_in.push(s.clone());
        } else {
            s_out.push(s.clone());
        }
    }
    for x in &s_out {
        if !frame_exits.contains(x) {
            return Err("abnormal exit into an enclosing region");
        }
    }
    // Straight-line prefix (everything before the trailing Branch) plus the
    // stripped Branch, whose cond and arm order shape the emitted If.
    let (mut prefix, branch) = split_block(cfg, entry)?;

    if s_in.is_empty() {
        // No successors inside the region: either terminate here or fall out
        // of the region. A bare exit block still needs a terminating Ret so
        // the rebuild knows the path ends here.
        if s_out.is_empty() && prefix.is_empty() {
            prefix.push(Stmt::Ret { value: None });
        }
        return Ok(prefix);
    }

    let mut rest = nodes.clone();
    rest.remove(entry);

    // Join peeling: pick the outermost join (largest dominator set, name as
    // tie-break) and split the region into the tree-shaped `preset` that
    // drains into it and the `tail` that continues from it.
    let joins: BTreeSet<String> = nodes
        .iter()
        .filter(|n| **n != *entry)
        .filter(|n| {
            cfg.blocks[*n]
                .pred
                .iter()
                .filter(|p| nodes.contains(*p))
                .count()
                >= 2
        })
        .cloned()
        .collect();
    if !joins.is_empty() {
        let j = joins
            .iter()
            .max_by(|a, b| {
                (dom.dom[a.as_str()].len(), a.as_str())
                    .cmp(&(dom.dom[b.as_str()].len(), b.as_str()))
            })
            .unwrap();
        if !joins
            .iter()
            .all(|q| dom.dom[j.as_str()].contains(q.as_str()))
        {
            return Err("parallel joins need node splitting");
        }
        let tail = reachable_from(cfg, j, nodes);
        let preset: BTreeSet<String> = nodes.difference(&tail).cloned().collect();
        let preset_exits: BTreeSet<String> = BTreeSet::from([(*j).clone()]);
        let mut out = structure_frame(cfg, dom, &preset, entry, &preset_exits)?;
        out.extend(structure_frame(cfg, dom, &tail, j, frame_exits)?);
        return Ok(out);
    }

    // Tree case (no joins): the region is a plain if/else/chain shape. With
    // no joins the two (2,0) arms never reconverge, and a block hands off to
    // at most one in-set successor.
    match (s_in.len(), s_out.len()) {
        (1, 0) => {
            prefix.extend(structure_frame(cfg, dom, &rest, &s_in[0], frame_exits)?);
            Ok(prefix)
        }
        (2, 0) => {
            let cond = branch
                .as_ref()
                .and_then(|(c, _, _)| c.clone())
                .unwrap_or(Expr::Nop);
            let a_then = reachable_from(cfg, &s_in[0], &rest);
            let a_else = reachable_from(cfg, &s_in[1], &rest);
            let then_is_first = branch
                .as_ref()
                .is_none_or(|(_, then_block, _)| *then_block == s_in[0]);
            let region_then = structure_frame(cfg, dom, &a_then, &s_in[0], frame_exits)?;
            let mut region_else = structure_frame(cfg, dom, &a_else, &s_in[1], frame_exits)?;
            if region_else.is_empty() {
                // A bare (statement-free) block on the else side needs a
                // statement so the rebuild materializes it; ReBuilder creates
                // blocks for empty then arms on its own.
                region_else.push(placeholder());
            }
            let (then_stmts, else_stmts) = if then_is_first {
                (region_then, region_else)
            } else {
                (region_else, region_then)
            };
            prefix.push(Stmt::If {
                cond,
                then_stmts,
                else_stmts,
            });
            Ok(prefix)
        }
        (1, 1) => {
            let cond = branch
                .as_ref()
                .and_then(|(c, _, _)| c.clone())
                .unwrap_or(Expr::Nop);
            let in_arm = reachable_from(cfg, &s_in[0], &rest);
            // The exit arm is emitted as ReBuilder's empty-else side: it
            // falls straight into the join without introducing a block, which
            // is exactly the original edge to the frame exit.
            let then_stmts = structure_frame(cfg, dom, &in_arm, &s_in[0], frame_exits)?;
            prefix.push(Stmt::If {
                cond,
                then_stmts,
                else_stmts: vec![],
            });
            Ok(prefix)
        }
        _ => Err("unsupported fan-out (node splitting is Task 5)"),
    }
}

/// The set of blocks reachable from `src` following successors that stay
/// inside `within`. Includes `src` itself.
fn reachable_from(cfg: &Cfg, src: &str, within: &BTreeSet<String>) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut stack = vec![src.to_string()];
    while let Some(n) = stack.pop() {
        if !seen.insert(n.clone()) {
            continue;
        }
        for s in &cfg.blocks[&n].succ {
            if within.contains(s) {
                stack.push(s.clone());
            }
        }
    }
    seen
}

/// A block's stripped `Branch` (cond, then target, optional else target).
type BranchInfo = (Option<Expr>, String, Option<String>);

/// The block's straight-line statements (all of it except the trailing
/// `Branch`) plus the Branch itself, so callers can re-emit its cond and arm
/// order as a structured `If`.
#[allow(clippy::type_complexity)]
fn split_block(cfg: &Cfg, name: &str) -> Result<(Vec<Stmt>, Option<BranchInfo>), &'static str> {
    let mut body = cfg
        .blocks
        .get(name)
        .ok_or("structurizer: unknown block")?
        .stmts
        .clone();
    match body.last() {
        Some(Stmt::Branch {
            cond,
            then_block,
            else_block,
        }) => {
            let br = (cond.clone(), then_block.clone(), else_block.clone());
            body.pop();
            Ok((body, Some(br)))
        }
        _ => Ok((body, None)),
    }
}

/// A statement that occupies a block without changing its semantics, used to
/// materialize bare (statement-free) arm blocks in the rebuilt CFG.
fn placeholder() -> Stmt {
    Stmt::Let {
        dest: Box::new(Expr::Nop),
        src: Box::new(Expr::Nop),
    }
}

impl LoopTree {
    fn build(cfg: &Cfg, dom: &DomInfo, reach: &BTreeSet<String>) -> Result<LoopTree, &'static str> {
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
        // `ancestors[i]`: every loop whose body strictly contains loop i. An
        // edge from a body node of i to an ancestor's *header* is that
        // ancestor's back edge (a "continue outer"), not an exit of i.
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
                    return Err("multi-entry loop");
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
                _ => return Err("multi-exit loop"),
            });
        }
        // A nested loop's exit must land inside its parent's body. Exiting to
        // the parent's own exit (skip past it) is unrepresentable with
        // innermost-bound Break.
        for i in 0..loops.len() {
            let Some(e) = &exit[i] else { continue };
            if let Some(p) = parent[i]
                && !loops[p].body.contains(e)
            {
                return Err("abnormal exit into an enclosing region");
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
            succ.entry((*a).to_string())
                .or_default()
                .push((*b).to_string());
            pred.entry((*b).to_string())
                .or_default()
                .push((*a).to_string());
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
        assert!(matches!(
            structurize(&cfg),
            Err(e) if e.contains("irreducible")
        ));
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
            Err(e) if e.contains("multi-exit")
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
        // sorted by (body.len(), header): {ih, C} is index 0, the outer is 1
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
        // inner loop (ih/C) exits to e, landing outside the outer loop's body
        // and skipping past it — unrepresentable with innermost Break.
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
            Err(e) if e.contains("abnormal exit")
        ));
    }

    #[test]
    fn reducible_cfg_hits_the_temporary_seam() {
        let cfg = cfg_from(
            "entry",
            &[("entry", "h"), ("h", "b"), ("h", "ex"), ("b", "h")],
        );
        assert!(matches!(
            structurize(&cfg),
            Err(e) if e.contains("not yet implemented")
        ));
    }

    // --- acyclic round-trips (Task 3 fixtures) -----------------------------

    #[test]
    fn straight_line_round_trips() {
        let cfg = cfg_from("e", &[("e", "a"), ("a", "b"), ("b", "c")]);
        let stmts = structurize(&cfg).unwrap();
        assert!(are_equivalent(&cfg, &structured_to_cfg(&stmts)));
    }

    #[test]
    fn diamond_join_round_trips() {
        let cfg = cfg_from(
            "e",
            &[("e", "a"), ("e", "b"), ("a", "j"), ("b", "j"), ("j", "t")],
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
        // e branches to the body a and straight to the exit j; a falls into j.
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
        // two joins in different branches, neither dominating the other
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
        assert!(matches!(
            structurize(&cfg),
            Err(e) if e.contains("parallel joins")
        ));
    }
}
