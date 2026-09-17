use crate::hir::{Lit, expr::Expr, value_to_expr};
use inkwell::values::InstructionOpcode::*;
use inkwell::values::InstructionValue;
/// Stmts are not value producing code
#[derive(Debug, Clone)]
pub enum Stmt {
    Branch {
        cond: Option<Expr>,
        then_block: String,
        else_block: Option<String>,
    },
    Ret {
        value: Option<Expr>,
    },
    /// Structured if/else, produced by recovery passes and lowering.
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
    /// Represents a new SSA binding (not a mutable variable use)
    /// Alternatively could be called "Assign", but that
    /// technically implies mutability, which would be wrong
    Let {
        dest: Box<Expr>,
        src: Box<Expr>,
    },
}
impl Stmt {
    pub fn build(op: InstructionValue) -> Result<Self, &'static str> {
        match op.get_opcode() {
            Add | FAdd | Sub | FSub | Mul | FMul | SDiv | UDiv | FDiv | SRem | URem | FRem
            | Shl | LShr | AShr | And | Or | Xor => {
                Err("Instruction opcode not supported by Stmt, use Expr instead!")
            }
            AddrSpaceCast => Err("Instruction opcode not implemented yet!"),
            Alloca => Err("Instruction opcode not implemented yet!"),
            AtomicCmpXchg => Err("Instruction opcode not implemented yet!"),
            AtomicRMW => Err("Instruction opcode not implemented yet!"),
            BitCast => Err("Instruction opcode not implemented yet!"),
            Br => {
                let mut ops = op.get_operands();
                let op1 = ops
                    .next()
                    .and_then(|o| o)
                    .ok_or("BinaryOp: missing operand 1")?;
                if op1.is_block()
                    && let blockval = op1.unwrap_block()
                {
                    Ok(Stmt::Branch {
                        cond: None,
                        then_block: blockval.get_name().to_string_lossy().to_string(),
                        else_block: None,
                    })
                } else {
                    let opval = value_to_expr(op1)?;
                    let else_block = ops
                        .next()
                        .and_then(|o| o)
                        .ok_or("Branch: missing false target")?
                        .block()
                        .ok_or("Branch: expected block for false target")?;
                    let then_block = ops
                        .next()
                        .and_then(|o| o)
                        .ok_or("Branch: missing true target")?
                        .block()
                        .ok_or("Branch: expected block for true target")?;
                    Ok(Stmt::Branch {
                        cond: Some(opval),
                        then_block: then_block.get_name().to_string_lossy().to_string(),
                        else_block: Some(else_block.get_name().to_string_lossy().to_string()),
                    })
                }
            }
            Return => {
                let mut ops = op.get_operands();
                let op1 = ops.next().and_then(|o| o);
                if let Some(op1) = op1 {
                    let value = value_to_expr(op1)?;
                    Ok(Stmt::Ret { value: Some(value) })
                } else {
                    Ok(Stmt::Ret { value: None })
                }
            }
            _ => Err("Instruction opcode not implemented yet!"),
        }
    }
}
