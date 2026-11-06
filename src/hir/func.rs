use crate::errors::StructurizeError;
use crate::hir::{Block, Cfg, Stmt, Ty};

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    pub ty: Ty,
}

/// A function's target-printable signature: names, parameter types, return type.
#[derive(Debug, Clone, PartialEq)]
pub struct FnSig {
    pub name: String,
    pub params: Vec<Param>,
    pub ret: Ty,
    pub variadic: bool,
}

/// A recovered or generated HIR function.
///
/// `cfg` is the source of truth: one block per basic block, in recovery order,
/// with edges derived from each terminator. `body` is the structured view that
/// `structurize` writes over it — never hand-written, always recomputable.
#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    pub sig: FnSig,
    /// Every alloca slot and every instruction result, declared at function top
    /// so a branch can never jump over an initialization.
    pub locals: Vec<(String, Ty)>,
    pub cfg: Cfg,
    pub body: Vec<Stmt>,
}

impl Function {
    /// Create a new Function AST representation.
    pub fn new(name: impl Into<String>, params: Vec<Param>, ret: Ty, cfg: Cfg) -> Self {
        Function {
            sig: FnSig {
                name: name.into(),
                params,
                ret,
                variadic: false,
            },
            locals: Vec::new(),
            cfg,
            body: Vec::new(),
        }
    }

    /// The blocks in recovery order.
    pub fn blocks(&self) -> Vec<&Block> {
        self.cfg.ordered()
    }

    /// The blocks in recovery order, mutably.
    pub fn blocks_mut(&mut self) -> Vec<&mut Block> {
        self.cfg.ordered_mut()
    }

    /// The parameter names, in signature order.
    pub fn param_names(&self) -> Vec<String> {
        self.sig.params.iter().map(|p| p.name.clone()).collect()
    }

    /// Run structurization on this function's CFG, populating and returning `self.body`.
    ///
    /// The dispatches can introduce their own variable — the tag read after the loop
    /// or after the entry chain — which is registered on `locals` so the backend
    /// declares it. The `Let` the passes insert is an assignment, not a declaration, so
    /// the variable has to be declared at function top.
    pub fn structurize(&mut self) -> Result<&[Stmt], StructurizeError> {
        let reachable = self.cfg.keep_reachable();
        let cap = reachable.len().saturating_mul(8).max(64);
        let stmts = crate::hir::structurize::structurize_with(&reachable, &mut self.locals, cap)?;
        self.body = stmts;
        Ok(&self.body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::Expr;
    use crate::hir::flow::reducible;
    use crate::hir::stmt::BranchStmt;

    fn flag(name: &str) -> Expr {
        Expr::var(name, Ty::Bool)
    }

    fn br(cond: Option<Expr>, then: &str, otherwise: Option<&str>) -> Stmt {
        Stmt::Branch(BranchStmt {
            cond,
            then_block: then.to_string(),
            else_block: otherwise.map(str::to_string),
        })
    }

    /// The exit dispatch reads its tag after the loop, so the variable has to be
    /// declared at function top: `emit_function` prints `f.locals` before the
    /// blocks, and a `Stmt::Let` is an assignment, not a declaration.
    #[test]
    fn the_exit_tag_is_declared_by_the_backend() {
        let cfg = Cfg::assemble(
            "entry",
            vec![
                Block::build("entry", vec![], Some(br(None, "h", None))),
                Block::build("h", vec![], Some(br(Some(flag("c")), "b", Some("e1")))),
                Block::build("b", vec![], Some(br(Some(flag("d")), "e2", Some("h")))),
                Block::build("e1", vec![], Some(Stmt::ret_void())),
                Block::build("e2", vec![], Some(Stmt::ret_void())),
            ],
        );
        let mut func = Function::new("f", Vec::new(), Ty::Void, cfg);
        func.locals = vec![("x".to_string(), Ty::Int(32, true))];
        func.structurize().expect("dispatch should structure");
        assert!(
            func.locals
                .contains(&("_exit_tag_h".to_string(), Ty::Int(32, true))),
            "{:?}",
            func.locals
        );
        // The locals recovery recorded are not disturbed.
        assert!(func.locals.contains(&("x".to_string(), Ty::Int(32, true))));
    }

    /// Node splitting is the lift that makes a multi-entry region structurable, so it
    /// has to run before the entry dispatch: the dispatch keeps the region entered at
    /// every node it dispatches to, and `LoopTree` rejects a body node entered from
    /// outside the body. Running the dispatch first turns a graph splitting could
    /// structure into an error.
    #[test]
    fn splitting_runs_before_the_entry_dispatch() {
        let cfg = Cfg::cfg_from(
            "entry",
            &[
                ("entry", "a"),
                ("entry", "b"),
                ("a", "b"),
                ("b", "a"),
                ("b", "ex"),
            ],
        );
        assert!(!reducible(&cfg), "the region is entered at two nodes");
        let mut func = Function::new("f", Vec::new(), Ty::Void, cfg);
        func.structurize()
            .expect("the region should be lifted by cloning");
        // The region was lifted by cloning, not by a dispatch, so no entry tag.
        assert!(func.locals.is_empty(), "{:?}", func.locals);
        assert!(
            func.body.iter().any(|s| matches!(s, Stmt::Loop(_))),
            "{:?}",
            func.body
        );
    }
}
