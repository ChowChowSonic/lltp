//! Operator enums for the flat (pre-structuring) HIR.
//!
//! `InstructionOpcode` collapses every integer comparison into `ICmp`, so a
//! `slt` and a `ugt` look identical. These enums keep the distinction.

use inkwell::values::InstructionOpcode;
use inkwell::{FloatPredicate, IntPredicate};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    SDiv,
    UDiv,
    SRem,
    URem,
    Shl,
    LShr,
    AShr,
    And,
    Or,
    Xor,
    FAdd,
    FSub,
    FMul,
    FDiv,
    FRem,
}

impl BinOp {
    pub fn from_opcode(op: InstructionOpcode) -> Option<Self> {
        use InstructionOpcode as O;
        Some(match op {
            O::Add => Self::Add,
            O::Sub => Self::Sub,
            O::Mul => Self::Mul,
            O::SDiv => Self::SDiv,
            O::UDiv => Self::UDiv,
            O::SRem => Self::SRem,
            O::URem => Self::URem,
            O::Shl => Self::Shl,
            O::LShr => Self::LShr,
            O::AShr => Self::AShr,
            O::And => Self::And,
            O::Or => Self::Or,
            O::Xor => Self::Xor,
            O::FAdd => Self::FAdd,
            O::FSub => Self::FSub,
            O::FMul => Self::FMul,
            O::FDiv => Self::FDiv,
            O::FRem => Self::FRem,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntCmp {
    Eq,
    Ne,
    Slt,
    Sle,
    Sgt,
    Sge,
    Ult,
    Ule,
    Ugt,
    Uge,
}

impl From<IntPredicate> for IntCmp {
    fn from(p: IntPredicate) -> Self {
        match p {
            IntPredicate::EQ => Self::Eq,
            IntPredicate::NE => Self::Ne,
            IntPredicate::SLT => Self::Slt,
            IntPredicate::SLE => Self::Sle,
            IntPredicate::SGT => Self::Sgt,
            IntPredicate::SGE => Self::Sge,
            IntPredicate::ULT => Self::Ult,
            IntPredicate::ULE => Self::Ule,
            IntPredicate::UGT => Self::Ugt,
            IntPredicate::UGE => Self::Uge,
        }
    }
}

/// `O*` = ordered (false if either side is NaN), 
/// `U*` = unordered (true if either side is NaN).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloatCmp {
    False,
    True,
    Oeq,
    Ogt,
    Oge,
    Olt,
    Ole,
    One,
    Ord,
    Ueq,
    Ugt,
    Uge,
    Ult,
    Ule,
    Une,
    Uno,
}

impl From<FloatPredicate> for FloatCmp {
    fn from(p: FloatPredicate) -> Self {
        match p {
            FloatPredicate::PredicateFalse => Self::False,
            FloatPredicate::PredicateTrue => Self::True,
            FloatPredicate::OEQ => Self::Oeq,
            FloatPredicate::OGT => Self::Ogt,
            FloatPredicate::OGE => Self::Oge,
            FloatPredicate::OLT => Self::Olt,
            FloatPredicate::OLE => Self::Ole,
            FloatPredicate::ONE => Self::One,
            FloatPredicate::ORD => Self::Ord,
            FloatPredicate::UEQ => Self::Ueq,
            FloatPredicate::UGT => Self::Ugt,
            FloatPredicate::UGE => Self::Uge,
            FloatPredicate::ULT => Self::Ult,
            FloatPredicate::ULE => Self::Ule,
            FloatPredicate::UNE => Self::Une,
            FloatPredicate::UNO => Self::Uno,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmp {
    Int(IntCmp),
    Float(FloatCmp),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CastKind {
    Trunc,
    ZExt,
    SExt,
    FpToSi,
    FpToUi,
    SiToFp,
    UiToFp,
    FpTrunc,
    FpExt,
}

impl CastKind {
    pub fn from_opcode(op: InstructionOpcode) -> Option<Self> {
        use InstructionOpcode as O;
        Some(match op {
            O::Trunc => Self::Trunc,
            O::ZExt => Self::ZExt,
            O::SExt => Self::SExt,
            O::FPToSI => Self::FpToSi,
            O::FPToUI => Self::FpToUi,
            O::SIToFP => Self::SiToFp,
            O::UIToFP => Self::UiToFp,
            O::FPTrunc => Self::FpTrunc,
            O::FPExt => Self::FpExt,
            _ => return None,
        })
    }
}
