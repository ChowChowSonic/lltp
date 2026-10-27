use crate::hir::{
    Lit, Ty,
    op::{BinOp, CastKind, Cmp, UnOp},
};

#[derive(Debug, Clone, PartialEq)]
pub struct VarExpr {
    pub name: String,
    pub dtype: Ty,
}
#[derive(Debug, Clone, PartialEq)]
pub struct UnaryExpr {
    pub op: UnOp,
    pub arg: Box<Expr>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct BinaryExpr {
    pub op: BinOp,
    pub arg1: Box<Expr>,
    pub arg2: Box<Expr>,
}
/// A comparison. Kept separate from `BinaryExpr` because `BinOp` collapses
/// `slt` and `ugt` into one opcode, and int and float predicates are two
/// different families.
#[derive(Debug, Clone, PartialEq)]
pub struct CmpExpr {
    pub pred: Cmp,
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
    pub dtype: Ty,
    /// The kind of cast matters: `zext` and `sext` do not print the same way.
    pub kind: CastKind,
}

/// Pure HIR expression AST node.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Literal(Lit),
    Var(VarExpr),
    UnaryOp(UnaryExpr),
    BinaryOp(BinaryExpr),
    Cmp(CmpExpr),
    Call(CallExpr),
    Index(IndexExpr),
    Field(FieldExpr),
    Cast(CastExpr),
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
    pub fn binary(op: BinOp, arg1: Expr, arg2: Expr) -> Self {
        Expr::BinaryOp(BinaryExpr {
            op,
            arg1: Box::new(arg1),
            arg2: Box::new(arg2),
        })
    }

    /// Construct a comparison.
    pub fn cmp(pred: Cmp, arg1: Expr, arg2: Expr) -> Self {
        Expr::Cmp(CmpExpr {
            pred,
            arg1: Box::new(arg1),
            arg2: Box::new(arg2),
        })
    }

    /// Construct a unary operation.
    pub fn unary(op: UnOp, arg: Expr) -> Self {
        Expr::UnaryOp(UnaryExpr {
            op,
            arg: Box::new(arg),
        })
    }

    /// The type this expression evaluates to.
    pub fn ty(&self) -> Ty {
        match self {
            Expr::Literal(l) => match l {
                Lit::Int { bits, signed, .. } => Ty::Int(*bits, *signed),
                Lit::Bool(_) => Ty::Bool,
                Lit::Float { bits, .. } => Ty::Float(*bits),
                Lit::NullPtr => Ty::Ptr(Box::new(Ty::Opaque("unresolved pointee".to_string()))),
                Lit::Void => Ty::Void,
            },
            Expr::Var(v) => v.dtype.clone(),
            Expr::UnaryOp(u) => match u.op {
                UnOp::AddrOf => Ty::Ptr(Box::new(u.arg.ty())),
                // Reading a cell yields what the cell holds.
                UnOp::ValuAt => match u.arg.ty() {
                    Ty::Ptr(pointee) => *pointee,
                    other => other,
                },
            },
            Expr::BinaryOp(b) => b.arg1.ty(),
            Expr::Cmp(_) => Ty::Bool,
            Expr::Cast(c) => c.dtype.clone(),
            Expr::Index(i) => match i.base.ty() {
                Ty::Ptr(pointee) => *pointee,
                Ty::Array(element, _) => *element,
                other => other,
            },
            Expr::Field(_) | Expr::Call(_) | Expr::Nop => Ty::Opaque("unresolved".to_string()),
        }
    }

    /// Check if this is a Nop expression.
    pub fn is_nop(&self) -> bool {
        matches!(self, Expr::Nop)
    }
}
