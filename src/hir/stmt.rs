use crate::hir::{Expr, Lit};

/// Information extracted from a basic block's terminating branch.
#[derive(Debug, Clone, PartialEq)]
pub struct BranchInfo {
    pub cond: Option<Expr>,
    pub then_block: String,
    pub else_block: Option<String>,
}

impl BranchInfo {
    pub fn is_conditional(&self) -> bool {
        self.cond.is_some()
    }

    pub fn then_target(&self) -> &str {
        &self.then_block
    }

    pub fn else_target(&self) -> Option<&str> {
        self.else_block.as_deref()
    }
}

/// Pure HIR statement AST node.
#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Branch {
        cond: Option<Expr>,
        then_block: String,
        else_block: Option<String>,
    },
    Ret {
        value: Option<Expr>,
    },
    /// Structured if/else, produced by recovery passes and structurization.
    If {
        cond: Expr,
        then_stmts: Vec<Stmt>,
        else_stmts: Vec<Stmt>,
    },
    /// Structured switch, produced by recovery; lowered to `If` chains when
    /// the target cannot express it (`Language::allows_switch`).
    Switch {
        value: Expr,
        cases: Vec<(Lit, Vec<Stmt>)>,
        default: Vec<Stmt>,
    },
    /// Represents a new SSA binding.
    Let {
        dest: Box<Expr>,
        src: Box<Expr>,
    },
    /// Structured loop with an optional pre-condition (`while (cond)` or infinite `loop`).
    Loop {
        cond: Option<Expr>,
        body: Vec<Stmt>,
    },
    Break,
    Continue,
    Goto {
        target: String,
    },
    Label {
        name: String,
    },
}

impl Stmt {
    /// Helper to construct a `Stmt::Ret` with a value.
    pub fn ret(val: Expr) -> Self {
        Stmt::Ret { value: Some(val) }
    }

    /// Helper to construct a void `Stmt::Ret`.
    pub fn ret_void() -> Self {
        Stmt::Ret { value: None }
    }

    /// Helper to construct a `Stmt::If`.
    pub fn if_then_else(cond: Expr, then_stmts: Vec<Stmt>, else_stmts: Vec<Stmt>) -> Self {
        Stmt::If {
            cond,
            then_stmts,
            else_stmts,
        }
    }

    /// Helper to construct a `Stmt::Let` binding.
    pub fn let_binding(dest: Expr, src: Expr) -> Self {
        Stmt::Let {
            dest: Box::new(dest),
            src: Box::new(src),
        }
    }
}
