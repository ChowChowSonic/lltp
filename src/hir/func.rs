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
    pub fn structurize(&mut self) -> Result<&[Stmt], StructurizeError> {
        let reachable = self.cfg.keep_reachable();
        let stmts = reachable.structurize()?;
        self.body = stmts;
        Ok(&self.body)
    }
}
