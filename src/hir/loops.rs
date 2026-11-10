//! Loop shapes: which block tests a loop's guard, whether that test runs before
//! or after the body, and which block control returns to when the guard holds.
//!
//! A natural loop alone does not say which of these it is. At `-O0` clang writes
//! `while (c)` as a header block holding the guard computation and the branch, so
//! the guard is read before any body statement; it writes `do { … } while (c)` as
//! one block that is both the region entry and the body, so at least one body
//! statement runs before the guard. Both are the same `NaturalLoop`, so the shape
//! has to be read off the block's own statements.

use std::collections::BTreeSet;

use crate::errors::StructurizeError;
use crate::hir::coalesce::{def_name, uses};
use crate::hir::flow::NaturalLoop;
use crate::hir::stmt::{BranchStmt, LetStmt, LoopKind};
use crate::hir::structurize::LoopTree;
use crate::hir::{Cfg, Expr, Stmt};

/// What one block contributes to a guarded test: the statements it runs before the
/// condition, the condition, and the arms it was written for.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockTest {
    pub name: String,
    pub preamble: Vec<Stmt>,
    pub guard: Option<Expr>,
    pub branch: Option<BranchStmt>,
    /// The variable whose definition became the guard, when one was folded in.
    pub folded: Option<String>,
}

/// The shape of one natural loop, as the structurer will render it.
#[derive(Debug, Clone, PartialEq)]
pub struct LoopShape {
    pub kind: LoopKind,
    pub header: String,
    /// The block the guard is read in.
    pub test_block: Option<String>,
    /// Whether the loop node itself reads the guard. When the test point is inside
    /// the body, the tree keeps the `If` there and the loop node carries no guard.
    pub guard_at_node: bool,
    /// The condition that must hold for the loop to run again.
    pub guard: Option<Expr>,
    /// The block control returns to when the guard holds. `None` when the back edges
    /// do not agree on one, which is what several `continue` paths look like.
    pub latch: Option<String>,
    pub join: Option<String>,
    /// The statements the test block runs before its guard.
    pub preamble: Vec<Stmt>,
    pub folded: Option<String>,
}

impl LoopShape {
    /// Whether the guard is read before any body statement.
    pub fn guards_before_body(&self) -> bool {
        self.kind == LoopKind::PreTest
    }
}

/// Whether `name` both stays in the body and leaves it: a block that decides
/// whether the loop runs again.
pub fn is_test_point(cfg: &Cfg, body: &BTreeSet<String>, name: &str) -> bool {
    let Some(blk) = cfg.get_block(name) else {
        return false;
    };
    let mut stays = false;
    let mut leaves = false;
    for s in &blk.succ {
        if body.contains(s) {
            stays = true;
        } else {
            leaves = true;
        }
    }
    stays && leaves
}

/// The body blocks that both stay in the body and leave it: each is a candidate for
/// the block that decides whether the loop runs again.
fn guard_candidates(cfg: &Cfg, body: &BTreeSet<String>) -> Vec<String> {
    let mut out = Vec::new();
    for n in body {
        let Some(blk) = cfg.get_block(n) else {
            continue;
        };
        let mut stays = false;
        let mut leaves = false;
        for s in &blk.succ {
            if body.contains(s) {
                stays = true;
            } else {
                leaves = true;
            }
        }
        if stays && leaves {
            out.push(n.clone());
        }
    }
    out
}

/// The block that decides iteration. The region entry is it whenever it tests at
/// all: a body node that leaves the loop is a `break` test, not the loop's own guard.
/// Otherwise the candidate that comes back here — a back-edge source — is the guard,
/// because the arm that leaves the loop is the way out and the arm that comes back is
/// the way round. When no candidate does, or several disagree, no single block decides
/// iteration and the loop has no guard: it is left by breaking out of an arm.
fn choose_guard_block(l: &NaturalLoop, candidates: &[String]) -> Option<String> {
    if candidates.contains(&l.header) {
        return Some(l.header.clone());
    }
    let back: BTreeSet<String> = l.back_edges.iter().map(|(src, _)| src.clone()).collect();
    let here: Vec<String> = candidates
        .iter()
        .filter(|c| back.contains(*c))
        .cloned()
        .collect();
    match here.split_first() {
        Some((first, [])) => Some(first.clone()),
        _ => None,
    }
}

/// Every read of `name` in the graph.
fn uses_of(cfg: &Cfg, name: &str) -> usize {
    let mut count = 0;
    for block in cfg.blocks.values() {
        for stmt in &block.stmts {
            for e in uses(stmt) {
                if let Expr::Var(v) = e
                    && v.name == name
                {
                    count += 1;
                }
            }
        }
    }
    count
}

/// The index of `name`'s single read-only definition in `block`, with its value.
fn single_use_def(cfg: &Cfg, block: &str, name: &str) -> Option<(usize, Expr)> {
    let blk = cfg.get_block(block)?;
    let idx = blk.stmts.iter().position(|s| def_name(s) == Some(name))?;
    if uses_of(cfg, name) != 1 {
        return None;
    }
    let Some(Stmt::Let(LetStmt { src, .. })) = blk.stmts.get(idx) else {
        return None;
    };
    Some((idx, (**src).clone()))
}

/// Read one block as a guarded test.
pub fn block_test(cfg: &Cfg, name: &str) -> Result<BlockTest, StructurizeError> {
    let (mut preamble, branch) = cfg.split_block(name)?;
    let Some(b) = branch else {
        return Ok(BlockTest {
            name: name.to_string(),
            preamble,
            guard: None,
            branch: None,
            folded: None,
        });
    };

    // A guard that is a variable read exactly once, defined by a plain assignment in
    // the same block, is that assignment's value: keeping the assignment as a
    // statement would run the definition after the test point on the entry path.
    if let Some(Expr::Var(v)) = &b.cond {
        let name_of_guard = v.name.clone();
        if let Some((idx, value)) = single_use_def(cfg, name, &name_of_guard) {
            preamble = preamble[..idx].to_vec();
            return Ok(BlockTest {
                name: name.to_string(),
                preamble,
                guard: Some(value),
                branch: Some(b),
                folded: Some(name_of_guard),
            });
        }
    }

    Ok(BlockTest {
        name: name.to_string(),
        preamble,
        guard: b.cond.clone(),
        branch: Some(b),
        folded: None,
    })
}

/// The only back-edge source, or `None` when several disagree.
fn single_back_edge(l: &NaturalLoop) -> Option<String> {
    let names: BTreeSet<String> = l.back_edges.iter().map(|(s, _)| s.clone()).collect();
    match names.len() {
        1 => names.into_iter().next(),
        _ => None,
    }
}

/// The arm that stays inside the body: the way back into the test block.
fn body_arm(cfg: &Cfg, test: &BlockTest, body: &BTreeSet<String>) -> Option<String> {
    let blk = cfg.get_block(&test.name)?;
    blk.succ.iter().find(|s| body.contains(*s)).cloned()
}

/// Whether the condition is written for the arm that comes back to the test point.
/// A condition written for the arm that leaves the loop is the `break` test, so the
/// guard has to be read the other way.
fn arm_returns_here(cfg: &Cfg, test: &BlockTest, body: &BTreeSet<String>) -> bool {
    let Some(back) = body_arm(cfg, test, body) else {
        return false;
    };
    match &test.branch {
        Some(b) => b.then_block == back,
        None => true,
    }
}

/// The shape of loop `idx`.
pub fn loop_shape(cfg: &Cfg, tree: &LoopTree, idx: usize) -> Result<LoopShape, StructurizeError> {
    let l = &tree.loops[idx];
    let body = &l.body;
    let join = tree.exit[idx].clone();

    let candidates = guard_candidates(cfg, body);
    let Some(guard_block) = choose_guard_block(l, &candidates) else {
        return Ok(LoopShape {
            kind: LoopKind::Unconditional,
            header: l.header.clone(),
            test_block: None,
            guard_at_node: false,
            guard: None,
            latch: single_back_edge(l),
            join,
            preamble: Vec::new(),
            folded: None,
        });
    };

    let test = block_test(cfg, &guard_block)?;
    let at_entry = guard_block == l.header;
    let kind = if test.preamble.is_empty() && at_entry {
        LoopKind::PreTest
    } else {
        LoopKind::PostTest
    };
    // A test point inside the body cannot be lifted to the loop node: the node would
    // have to test on paths that never reach it, so the `If` stays in the body.
    let guard_at_node = at_entry;

    if !guard_at_node {
        // The `continue` has to pass the block that reads the guard. A back edge that
        // comes back to the entry instead skips whatever the entry runs on the way to
        // the test point, so the shape cannot name it.
        let entry = block_test(cfg, &l.header)?;
        if !entry.preamble.is_empty() {
            for (src, _) in &l.back_edges {
                if src != &guard_block {
                    return Err(StructurizeError::ContinueSkipsGuard {
                        block: src.clone(),
                        latch: guard_block.to_string(),
                    });
                }
            }
        }
    }

    // A condition written for the arm that leaves the loop is the `break` test, so the
    // guard has to be read the other way.
    let guard = if guard_at_node {
        let g = test.guard.clone().unwrap_or(Expr::Nop);
        if arm_returns_here(cfg, &test, body) {
            Some(g)
        } else {
            Some(Expr::negate(g))
        }
    } else {
        None
    };

    let latch = single_back_edge(l);

    Ok(LoopShape {
        kind,
        header: l.header.clone(),
        test_block: Some(test.name.clone()),
        guard_at_node,
        guard,
        latch,
        join,
        preamble: test.preamble.clone(),
        folded: test.folded.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::StructurizeError;
    use crate::hir::flow::{dominators, reachable};
    use crate::hir::op::{Cmp, IntCmp};
    use crate::hir::stmt::{BranchStmt, LoopStmt};
    use crate::hir::structurize::{LoopTree, structurize};
    use crate::hir::verify::{are_equivalent, structured_to_cfg};
    use crate::hir::{Block, Expr, Function, Stmt, Ty};

    const I32: Ty = Ty::Int(32, true);
    const BOOL: Ty = Ty::Bool;

    fn flag(name: &str) -> Expr {
        Expr::var(name, BOOL)
    }

    fn br(cond: Option<Expr>, then: &str, other: Option<&str>) -> Stmt {
        Stmt::Branch(BranchStmt {
            cond,
            then_block: then.to_string(),
            else_block: other.map(str::to_string),
        })
    }

    fn def(name: &str, value: Expr) -> Stmt {
        Stmt::let_binding(Expr::var(name, BOOL), value)
    }

    fn guard(name: &str) -> Expr {
        Expr::cmp(
            Cmp::Int(IntCmp::Slt),
            Expr::var(name, I32),
            Expr::int(5, 32, true),
        )
    }

    fn shape_of(cfg: &Cfg) -> Result<LoopShape, StructurizeError> {
        let dom = dominators(cfg);
        let tree = LoopTree::build(cfg, &dom, &reachable(cfg)).unwrap();
        loop_shape(cfg, &tree, 0)
    }

    #[test]
    fn a_test_at_the_entry_before_any_body_statement_is_pre_test() {
        let cfg = Cfg::assemble(
            "entry",
            vec![
                Block::build("entry", vec![], Some(br(None, "cond", None))),
                Block::build(
                    "cond",
                    vec![def("g", guard("i"))],
                    Some(br(Some(flag("g")), "body", Some("exit"))),
                ),
                Block::build("body", vec![], Some(br(None, "cond", None))),
                Block::build("exit", vec![], Some(Stmt::ret_void())),
            ],
        );
        let s = shape_of(&cfg).unwrap();
        assert_eq!(s.kind, LoopKind::PreTest, "{s:?}");
        assert_eq!(s.test_block.as_deref(), Some("cond"));
        assert_eq!(s.latch.as_deref(), Some("body"));
        // The single-use definition is the guard itself.
        assert_eq!(s.guard, Some(guard("i")));
        assert_eq!(s.folded.as_deref(), Some("g"));
    }

    #[test]
    fn body_work_before_the_test_is_post_test() {
        let cfg = Cfg::assemble(
            "loop",
            vec![
                Block::build(
                    "loop",
                    vec![def("a", Expr::int(1, 32, true)), def("g", guard("i"))],
                    Some(br(Some(flag("g")), "loop", Some("exit"))),
                ),
                Block::build("exit", vec![], Some(Stmt::ret_void())),
            ],
        );
        let s = shape_of(&cfg).unwrap();
        assert_eq!(s.kind, LoopKind::PostTest, "{s:?}");
        assert_eq!(s.test_block.as_deref(), Some("loop"));
        assert_eq!(s.preamble.len(), 1, "the body work runs before the test");
        assert_eq!(s.folded.as_deref(), Some("g"));
    }

    #[test]
    fn a_definition_read_twice_stays_a_statement() {
        let cfg = Cfg::assemble(
            "entry",
            vec![
                Block::build(
                    "cond",
                    vec![def("g", guard("i"))],
                    Some(br(Some(flag("g")), "body", Some("exit"))),
                ),
                Block::build(
                    "body",
                    vec![Stmt::let_binding(
                        Expr::var("print", I32),
                        Expr::negate(flag("g")),
                    )],
                    Some(br(None, "cond", None)),
                ),
                Block::build("entry", vec![], Some(br(None, "cond", None))),
                Block::build("exit", vec![], Some(Stmt::ret_void())),
            ],
        );
        let s = shape_of(&cfg).unwrap();
        assert_eq!(s.folded, None, "two reads: the definition is not the guard");
        assert_eq!(s.kind, LoopKind::PostTest, "{s:?}");
    }

    #[test]
    fn a_test_inside_the_body_is_not_lifted_to_the_loop_node() {
        let cfg = Cfg::assemble(
            "entry",
            vec![
                Block::build("entry", vec![], Some(br(None, "loop", None))),
                Block::build("loop", vec![], Some(br(None, "test", None))),
                Block::build(
                    "test",
                    vec![def("g", flag("x"))],
                    Some(br(Some(flag("g")), "loop", Some("exit"))),
                ),
                Block::build("exit", vec![], Some(Stmt::ret_void())),
            ],
        );
        let s = shape_of(&cfg).unwrap();
        assert_eq!(s.test_block.as_deref(), Some("test"));
        assert_eq!(s.guard, None, "the loop node does not read it");
        assert!(!s.guard_at_node);
        assert_eq!(s.kind, LoopKind::PostTest, "{s:?}");
    }

    #[test]
    fn an_unguarded_loop_has_no_test_block() {
        let cfg = Cfg::cfg_from("entry", &[("entry", "h"), ("h", "b"), ("b", "h")]);
        let dom = dominators(&cfg);
        let tree = LoopTree::build(&cfg, &dom, &reachable(&cfg)).unwrap();
        let s = loop_shape(&cfg, &tree, 0).unwrap();
        assert_eq!(s.kind, LoopKind::Unconditional, "{s:?}");
        assert_eq!(s.test_block, None);
        assert_eq!(s.latch.as_deref(), Some("b"));
    }

    #[test]
    fn a_back_edge_that_skips_the_test_point_is_refused() {
        // The guard is read in `guard`, after body work in `loop`; `skip` comes back to
        // `loop` without passing it, so a `continue` there would run the guard twice or
        // never, depending on the path.
        let cfg = Cfg::assemble(
            "entry",
            vec![
                Block::build("entry", vec![], Some(br(None, "loop", None))),
                Block::build(
                    "loop",
                    vec![def("a", Expr::int(1, 32, true))],
                    Some(br(Some(flag("x")), "guard", Some("skip"))),
                ),
                Block::build(
                    "guard",
                    vec![],
                    Some(br(Some(flag("g")), "loop", Some("exit"))),
                ),
                Block::build("skip", vec![], Some(br(None, "loop", None))),
                Block::build("exit", vec![], Some(Stmt::ret_void())),
            ],
        );
        let dom = dominators(&cfg);
        let tree = LoopTree::build(&cfg, &dom, &reachable(&cfg)).unwrap();
        assert_eq!(
            tree.loops[0].back_edges.len(),
            2,
            "{:?}",
            tree.loops[0].back_edges
        );
        let err = loop_shape(&cfg, &tree, 0).expect_err("the shape cannot name the test");
        assert!(
            matches!(err, StructurizeError::ContinueSkipsGuard { .. }),
            "{err:?}"
        );
    }

    #[test]
    fn the_tree_records_the_shape_the_graph_has() {
        let cfg = Cfg::assemble(
            "entry",
            vec![
                Block::build("entry", vec![], Some(br(None, "cond", None))),
                Block::build(
                    "cond",
                    vec![def("g", guard("i"))],
                    Some(br(Some(flag("g")), "body", Some("exit"))),
                ),
                Block::build("body", vec![], Some(br(None, "cond", None))),
                Block::build("exit", vec![], Some(Stmt::ret_void())),
            ],
        );
        let stmts = structurize(&cfg).unwrap();
        let Stmt::Loop(node) = &stmts[0] else {
            panic!("expected the loop node, got {stmts:?}");
        };
        assert_eq!(node.kind, LoopKind::PreTest);
        assert_eq!(node.guard, Some(guard("i")));
        assert_eq!(node.test_block.as_deref(), Some("cond"));
        assert_eq!(node.latch.as_deref(), Some("body"));
    }

    /// A post-test loop read as a pre-test one has to change the rebuilt shape: the
    /// guard node moves from after the body work to before it.
    #[test]
    fn claiming_the_wrong_position_changes_the_shape() {
        let cfg = Cfg::assemble(
            "entry",
            vec![
                Block::build("entry", vec![], Some(br(None, "loop", None))),
                Block::build(
                    "loop",
                    vec![def("a", Expr::int(1, 32, true)), def("g", guard("i"))],
                    Some(br(Some(flag("g")), "loop", Some("exit"))),
                ),
                Block::build("exit", vec![], Some(Stmt::ret_void())),
            ],
        );
        let work = def("a", Expr::int(1, 32, true));
        let post = Stmt::Loop(LoopStmt {
            kind: LoopKind::PostTest,
            guard: Some(flag("g")),
            test_block: Some("loop".to_string()),
            guard_at_node: true,
            latch: Some("loop".to_string()),
            header: Some("loop".to_string()),
            body: vec![work.clone()],
        });
        let pre = Stmt::Loop(LoopStmt {
            kind: LoopKind::PreTest,
            guard: Some(flag("g")),
            test_block: Some("loop".to_string()),
            guard_at_node: true,
            latch: Some("loop".to_string()),
            header: Some("loop".to_string()),
            body: vec![work],
        });
        let post_tree = [post];
        let pre_tree = [pre];
        assert!(are_equivalent(&cfg, &structured_to_cfg(&post_tree)));
        assert!(
            !are_equivalent(&cfg, &structured_to_cfg(&pre_tree)),
            "a post-test loop read as pre-test must not match"
        );
    }

    #[test]
    fn a_folded_definition_is_not_declared_any_more() {
        let cfg = Cfg::assemble(
            "entry",
            vec![
                Block::build("entry", vec![], Some(br(None, "cond", None))),
                Block::build(
                    "cond",
                    vec![def("g", guard("i"))],
                    Some(br(Some(flag("g")), "body", Some("exit"))),
                ),
                Block::build("body", vec![], Some(br(None, "cond", None))),
                Block::build("exit", vec![], Some(Stmt::ret_void())),
            ],
        );
        let mut func = Function::new("f", Vec::new(), Ty::Void, cfg);
        func.locals = vec![("i".to_string(), I32), ("g".to_string(), BOOL)];
        func.structurize().expect("the loop should structure");
        assert!(
            func.locals.contains(&("i".to_string(), I32)),
            "{:?}",
            func.locals
        );
        assert!(
            !func.locals.contains(&("g".to_string(), BOOL)),
            "the folded guard is no longer read: {:?}",
            func.locals
        );
    }
}
