use crate::hir::{Expr, Lit, Ty, expr::VarExpr};

impl BranchStmt {
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

#[derive(Debug, Clone, PartialEq)]
pub struct BranchStmt {
    pub cond: Option<Expr>,
    pub then_block: String,
    pub else_block: Option<String>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct RetStmt {
    pub value: Option<Expr>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct IfStmt {
    pub cond: Expr,
    pub then_stmts: Vec<Stmt>,
    pub else_stmts: Vec<Stmt>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct SwitchStmt {
    pub value: Expr,
    pub cases: Vec<(Lit, Vec<Stmt>)>,
    pub default: Vec<Stmt>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct LetStmt {
    pub dest: Box<Expr>,
    pub src: Box<Expr>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct LoopStmt {
    pub cond: Option<Expr>,
    pub body: Vec<Stmt>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct GotoStmt {
    pub target: String,
}
#[derive(Debug, Clone, PartialEq)]
pub struct LabelStmt {
    pub name: String,
}
#[derive(Debug, Clone, PartialEq)]
pub struct CatchClause {
    pub param: Option<VarExpr>,
    pub exception_ty: Option<Ty>,
    pub body: Vec<Stmt>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct TryCatchStmt {
    pub try_body: Vec<Stmt>,
    pub catches: Vec<CatchClause>,
    pub finally_body: Option<Vec<Stmt>>,
}
/// Pure HIR statement AST node.
#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Branch(BranchStmt),
    Ret(RetStmt),
    /// Structured if/else, produced by recovery passes and structurization.
    If(IfStmt),
    /// Structured switch, produced by recovery; lowered to `If` chains when
    /// the target cannot express it (`Language::allows_switch`).
    Switch(SwitchStmt),
    /// Represents a new SSA binding. `dest` is a place: either a plain name or
    /// `AddrOf(name)` when the target is a memory cell.
    Let(LetStmt),
    /// Evaluate an expression for its effect only — a void call has no def.
    Eval(Expr),
    /// `unreachable`: a block that cannot be entered.
    Unreachable,
    /// Structured loop with an optional pre-condition (`while (cond)` or infinite `loop`).
    Loop(LoopStmt),
    Break,
    Continue,
    Goto(GotoStmt),
    Label(LabelStmt),
    TryCatch(TryCatchStmt),
}

impl Stmt {
    /// Helper to construct a `Stmt::Ret` with a value.
    pub fn ret(val: Expr) -> Self {
        Stmt::Ret(RetStmt { value: Some(val) })
    }

    /// Helper to construct a void `Stmt::Ret`.
    pub fn ret_void() -> Self {
        Stmt::Ret(RetStmt { value: None })
    }

    /// Helper to construct a `Stmt::If`.
    pub fn if_then_else(cond: Expr, then_stmts: Vec<Stmt>, else_stmts: Vec<Stmt>) -> Self {
        Stmt::If(IfStmt {
            cond,
            then_stmts,
            else_stmts,
        })
    }

    /// Helper to construct a `Stmt::Let` binding.
    pub fn let_binding(dest: Expr, src: Expr) -> Self {
        Stmt::Let(LetStmt {
            dest: Box::new(dest),
            src: Box::new(src),
        })
    }
}
