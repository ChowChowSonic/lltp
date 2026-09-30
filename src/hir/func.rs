use crate::hir::{Cfg, Stmt, StructurizeError, Ty};

/// A recovered or generated HIR function.
#[derive(Debug, Clone)]
pub struct Function {
    pub name: String,
    pub params: Vec<Ty>,
    pub return_ty: Ty,
    pub cfg: Cfg,
    pub body: Vec<Stmt>,
}

impl Function {
    /// Create a new Function AST representation.
    pub fn new(name: impl Into<String>, params: Vec<Ty>, return_ty: Ty, cfg: Cfg) -> Self {
        Function {
            name: name.into(),
            params,
            return_ty,
            cfg,
            body: Vec::new(),
        }
    }

    /// Run structurization on this function's CFG, populating and returning `self.body`.
    pub fn structurize(&mut self) -> Result<&[Stmt], StructurizeError> {
        let stmts = self.cfg.structurize()?;
        self.body = stmts;
        Ok(&self.body)
    }
}
