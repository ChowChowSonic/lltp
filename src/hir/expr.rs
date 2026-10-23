use inkwell::values::InstructionOpcode;

use crate::hir::{Lit, Ty};

#[derive(Debug, Clone, PartialEq)]
pub struct VarExpr {
    pub name: String,
    pub dtype: Ty,
}
#[derive(Debug, Clone, PartialEq)]
pub struct UnaryExpr {
    pub op: InstructionOpcode,
    pub arg: Box<Expr>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct BinaryExpr {
    pub op: InstructionOpcode,
    pub arg1: Box<Expr>,
    pub arg2: Box<Expr>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct CallExpr {
    pub name: String,
    pub args: Vec<Expr>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct IndexExpr {
    pub base: Box<Expr>,
    pub index: Box<Expr>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct FieldExpr {
    pub base: Box<Expr>,
    pub name: String,
}
#[derive(Debug, Clone, PartialEq)]
pub struct CastExpr {
    pub data: Box<Expr>,
    pub dtype: Box<Expr>,
}

/// Pure HIR expression AST node.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Literal(Lit),
    Var(VarExpr),
    UnaryOp(UnaryExpr),
    BinaryOp(BinaryExpr),
    Call(CallExpr),
    Index(IndexExpr),
    Field(FieldExpr),
    Cast(CastExpr),
    Addr(String),
    Nop,
}

impl Expr {
    /// Construct a boolean literal expression.
    pub fn bool(val: bool) -> Self {
        Expr::Literal(Lit::Bool(val))
    }

    /// Construct an integer literal expression.
    pub fn int(value: u64, bits: usize, signed: bool) -> Self {
        Expr::Literal(Lit::Int {
            value,
            bits,
            signed,
        })
    }

    /// Construct a variable reference.
    pub fn var(name: impl Into<String>, dtype: Ty) -> Self {
        Expr::Var(VarExpr {
            name: name.into(),
            dtype,
        })
    }

    /// Construct a binary operation.
    pub fn binary(op: InstructionOpcode, arg1: Expr, arg2: Expr) -> Self {
        Expr::BinaryOp(BinaryExpr {
            op,
            arg1: Box::new(arg1),
            arg2: Box::new(arg2),
        })
    }

    /// Construct a unary operation.
    pub fn unary(op: InstructionOpcode, arg: Expr) -> Self {
        Expr::UnaryOp(UnaryExpr {
            op,
            arg: Box::new(arg),
        })
    }

    /// Check if this is a Nop expression.
    pub fn is_nop(&self) -> bool {
        matches!(self, Expr::Nop)
    }
}
