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
/// Where a loop's guard is evaluated, relative to its body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopKind {
    /// The guard runs before any body statement: `while (guard)`.
    PreTest,
    /// At least one body statement runs before the guard: `do { … } while (guard)`.
    PostTest,
    /// A pre-test guard plus an induction variable the loop steps by a constant:
    /// `for (init; guard; step)`.
    Counted,
    /// No guard: the loop is left only through `Break`.
    Unconditional,
}

/// The variable a counted loop steps. `init` is the value that reaches the guard on
/// the entry edge, `step` the update on the way back; either may be absent, since a
/// `for` with no init clause is still a counted loop.
#[derive(Debug, Clone, PartialEq)]
pub struct InductionVar {
    pub name: String,
    pub init: Option<Expr>,
    pub step: Option<Expr>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LoopStmt {
    pub kind: LoopKind,
    /// The condition that must hold for the loop to run again. `None` when the loop
    /// node itself does not test: the test then stays inside the body.
    pub guard: Option<Expr>,
    /// The block the guard is read in.
    pub test_block: Option<String>,
    /// Whether this node reads the guard. When the test point is inside the body,
    /// the `If` stays there and the node carries no guard.
    pub guard_at_node: bool,
    /// The block control returns to when the guard holds.
    pub latch: Option<String>,
    /// The block the loop region is entered at, so a node can be read back against
    /// the graph it came from.
    pub header: Option<String>,
    /// The induction variable, when the shape claims a counted loop.
    pub induction: Option<InductionVar>,
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
/// An `invoke`. Its arms are defined here rather than inherited from IR
/// operand order: `normal` is the control arm, `catches` are the exception arms
/// in the order recovery recorded them.
#[derive(Debug, Clone, PartialEq)]
pub struct InvokeStmt {
    pub callee: String,
    pub args: Vec<Expr>,
    pub normal: String,
    pub catches: Vec<String>,
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
    /// `invoke`: control flows to `normal`; `catches` are exception arms, kept
    /// out of the graph's control edges.
    Invoke(InvokeStmt),
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
