use std::collections::BTreeSet;

use tracing::{debug, info};

use crate::backend::LowerTarget;
use crate::errors::LowerError;
use crate::hir::{Function, flow};
use crate::passes::Lowering;

/// Structural pass (SoW 2.3.3-2.3.5): rebuilds a loop-free, single-exit function's
/// tree from the reaching conditions of its graph, so a guard reads as the compare
/// the branch came from, complementary guards pair into one `if`/`else`, and a run of
/// `v == c` guards becomes a `switch` with the constants it actually tested.
///
/// It is whole-tree only: a pass is handed one `Function`, so it can rebuild the tree
/// once, not per arm, and it has no loop or frame-boundary notion. What it cannot
/// represent it declines, leaving the tree the structurer built.
///
/// No capability flag gates it — registering it is the choice. Order matters, though:
/// a target that cannot print a `switch` has to list `SwitchToIfElse` after it,
/// because folding a run of equality guards is what produces the switch.
pub struct AcyclicRegions;

impl Lowering for AcyclicRegions {
    fn name(&self) -> &'static str {
        "acyclic_regions"
    }

    fn apply(&self, func: &mut Function, _target: &dyn LowerTarget) -> Result<(), LowerError> {
        // The reachable set is the same view `Function::structurize` structures: a
        // block no path reaches has no reaching condition to read.
        let cfg = flow::keep_reachable(&func.cfg);
        if cfg.exits.len() != 1 {
            debug!(func = %func.sig.name, "acyclic regions: more than one exit, declined");
            return Ok(());
        }
        let dom = flow::dominators(&cfg);
        if !flow::natural_loops(&cfg, &dom).is_empty() {
            debug!(func = %func.sig.name, "acyclic regions: a cycle in the graph, declined");
            return Ok(());
        }

        // `skip` is empty because the graph has no back edge to skip, and `boundary`
        // is empty because a whole-tree region has no enclosing frame to fall out of.
        let built = crate::hir::acyclic::structure_acyclic(
            &cfg,
            &cfg.entry,
            &cfg.exits[0],
            &crate::hir::reaching::EdgeSet::new(),
            &BTreeSet::new(),
            &mut func.locals,
        )
        .map_err(LowerError::from)?;
        info!(func = %func.sig.name, "acyclic region structuring");
        func.body = built;
        Ok(())
    }

    /// A guard stands for the block that tests it, so the tree does not carry that
    /// block as a node. `are_equivalent` counts nodes, so this pass cannot be
    /// confirmed against the graph it was read from.
    fn preserves_shape(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::AcyclicRegions;
    use crate::backend::{ExprEmitter, LowerTarget, StmtEmitter};
    use crate::hir::expr::VarExpr;
    use crate::hir::op::{Cmp, IntCmp};
    use crate::hir::stmt::{BranchStmt, SwitchStmt};
    use crate::hir::{Block, Cfg, Expr, Function, Lit, Param, Stmt, Ty};
    use crate::passes::{Lowering, SwitchToIfElse};

    struct TestLang {
        switch: bool,
    }
    impl LowerTarget for TestLang {
        fn name(&self) -> &str {
            "test"
        }
        fn allows_switch(&self) -> bool {
            self.switch
        }
    }
    impl ExprEmitter for TestLang {}
    impl StmtEmitter for TestLang {}

    fn int_lit(value: u64) -> Lit {
        Lit::Int {
            value,
            bits: 32,
            signed: true,
        }
    }
    fn var(name: &str, ty: Ty) -> Expr {
        Expr::Var(VarExpr {
            name: name.to_string(),
            dtype: ty,
        })
    }
    fn x() -> Expr {
        var("x", Ty::Int(32, true))
    }
    fn guard(value: u64) -> Expr {
        Expr::cmp(Cmp::Int(IntCmp::Eq), x(), Expr::Literal(int_lit(value)))
    }
    fn flag(name: &str) -> Expr {
        var(name, Ty::Bool)
    }
    fn mark(name: &str) -> Stmt {
        Stmt::Eval(flag(name))
    }
    fn bind(name: &str, cond: Expr) -> Stmt {
        Stmt::let_binding(var(name, Ty::Bool), cond)
    }
    fn br(cond: Option<Expr>, then: &str, other: Option<&str>) -> Stmt {
        Stmt::Branch(BranchStmt {
            cond,
            then_block: then.to_string(),
            else_block: other.map(|e| e.to_string()),
        })
    }

    /// A chain of `x == c` tests: the shape SoW 2.3.5 folds into a switch.
    fn chain() -> Cfg {
        let entry = br(Some(flag("t1")), "a", Some("b"));
        let b = br(Some(flag("t2")), "c", Some("d"));
        Cfg::assemble(
            "entry",
            vec![
                Block::build("entry", vec![bind("t1", guard(1))], Some(entry)),
                Block::build("a", vec![mark("a")], Some(br(None, "join", None))),
                Block::build("b", vec![bind("t2", guard(2))], Some(b)),
                Block::build("c", vec![mark("c")], Some(br(None, "join", None))),
                Block::build("d", vec![mark("d")], Some(br(None, "join", None))),
                Block::build("join", vec![mark("join")], Some(Stmt::ret_void())),
            ],
        )
    }

    fn func() -> Function {
        Function::new(
            "f",
            vec![Param {
                name: "x".to_string(),
                ty: Ty::Int(32, true),
            }],
            Ty::Void,
            chain(),
        )
    }

    #[test]
    fn the_pass_names_the_constants_the_guards_tested() {
        // The structurer has no case constants to read: an arm of a chain is a branch,
        // so the tree it builds is nested `if`s on the boolean the branch read, and a
        // switch it does make is labelled by arm position.
        let mut func = func();
        func.body = func
            .cfg
            .structurize()
            .expect("the structurer should take the chain");
        assert!(
            !func.body.iter().any(|s| matches!(s, Stmt::Switch(..))),
            "{:?}",
            func.body
        );

        AcyclicRegions
            .apply(&mut func, &TestLang { switch: true })
            .expect("the pass should run");
        assert!(
            matches!(
                &func.body[0],
                Stmt::Switch(SwitchStmt {
                    value: Expr::Var(v),
                    cases,
                    ..
                }) if v.name == "x"
                    && cases.iter().any(|(c, _)| *c == int_lit(1))
                    && cases.iter().any(|(c, _)| *c == int_lit(2))
            ),
            "{:?}",
            func.body
        );
    }

    #[test]
    fn the_pass_declines_what_it_cannot_represent() {
        // A cycle: no single source-to-sink region.
        let mut func = Function::new(
            "f",
            Vec::new(),
            Ty::Void,
            Cfg::assemble(
                "h",
                vec![
                    Block::build("h", vec![], Some(br(Some(flag("c")), "b", Some("x")))),
                    Block::build("b", vec![], Some(br(None, "h", None))),
                    Block::build("x", vec![], Some(Stmt::ret_void())),
                ],
            ),
        );
        func.body = func.cfg.structurize().expect("the loop should structure");
        let before = func.body.clone();
        AcyclicRegions
            .apply(&mut func, &TestLang { switch: true })
            .expect("the pass should run");
        assert_eq!(func.body, before, "a loop is not an acyclic region");

        // Two exits: the run has no single sink to read the conditions against.
        let mut multi = Function::new(
            "g",
            Vec::new(),
            Ty::Void,
            Cfg::cfg_from("e", &[("e", "a"), ("e", "b")]),
        );
        multi.body = multi.cfg.structurize().expect("the fork should structure");
        let before = multi.body.clone();
        AcyclicRegions
            .apply(&mut multi, &TestLang { switch: true })
            .expect("the pass should run");
        assert_eq!(multi.body, before, "two exits have no sink");
    }

    #[test]
    fn the_fold_has_to_be_unfolded_after_it_runs() {
        // A target that cannot print a switch is only expressible when the rewrite
        // pass follows this one, in that order.
        let mut func = func();
        AcyclicRegions
            .apply(&mut func, &TestLang { switch: false })
            .expect("the pass should run");
        assert!(matches!(&func.body[0], Stmt::Switch(..)), "{:?}", func.body);

        SwitchToIfElse
            .apply(&mut func, &TestLang { switch: false })
            .expect("the rewrite should run");
        assert!(
            !func.body.iter().any(|s| matches!(s, Stmt::Switch(..))),
            "{:?}",
            func.body
        );
    }

    #[test]
    fn the_pass_cannot_be_confirmed_by_the_shape_check() {
        assert!(!AcyclicRegions.preserves_shape());
    }
}
