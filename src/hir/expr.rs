use crate::hir::value_to_expr;
use crate::hir::{Lit, ty::Ty};
use inkwell::values::InstructionOpcode::{self, *};
use inkwell::values::InstructionValue;
/// Exprs are value producing code
#[derive(Debug, Clone)]
pub enum Expr {
    Literal(Lit),
    Var {
        name: String,
        dtype: Ty,
    },
    UnaryOp {
        op: InstructionOpcode,
        arg: Box<Expr>,
    },
    BinaryOp {
        op: InstructionOpcode,
        arg1: Box<Expr>,
        arg2: Box<Expr>,
    },
    Call {
        name: String,
        args: Vec<Expr>,
    },
    Index {
        base: Box<Expr>,
        index: Box<Expr>,
    },
    Field {
        base: Box<Expr>,
        name: String,
    },
    Cast {
        data: Box<Expr>,
        dtype: Box<Expr>,
    },
    Nop,
}
impl Expr {
    pub fn build(op: InstructionValue) -> Result<Self, &'static str> {
        match op.get_opcode() {
            Add | FAdd | Sub | FSub | Mul | FMul | SDiv | UDiv | FDiv | SRem | URem | FRem
            | Shl | LShr | AShr | And | Or | Xor => {
                let mut ops = op.get_operands();
                let op1 = ops
                    .next()
                    .and_then(|o| o)
                    .ok_or("BinaryOp: missing operand 1")?;
                let op2 = ops
                    .next()
                    .and_then(|o| o)
                    .ok_or("BinaryOp: missing operand 2")?;
                let op1_expr = value_to_expr(op1)?;
                let op2_expr = value_to_expr(op2)?;
                Ok(Expr::BinaryOp {
                    op: op.get_opcode(),
                    arg1: Box::new(op1_expr),
                    arg2: Box::new(op2_expr),
                })
            }
            Br | Switch | IndirectBr | CallBr | Return | Unreachable => {
                Err("Terminator: not an expression")
            }
            _ => Err("Instruction opcode not implemented yet!"),
        }
    }
}
