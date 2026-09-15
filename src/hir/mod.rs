use inkwell::values::InstructionOpcode::{self, *};
use inkwell::values::{BasicValueEnum, InstructionValue, Operand};
pub mod graph;

#[derive(Debug)]
pub enum Ty {
    Void,
    Bool,
    Int(usize /*# of bits*/, bool /*is_signed*/),
    Float(usize /*bits*/),
    Ptr(Box<Ty>),
    Array(Box<Ty>, usize),
}

#[derive(Debug)]
pub enum Lit {
    Int {
        value: u64,
        bits: usize,
        signed: bool,
    },
    Bool(bool),
    Float {
        bits: usize,
        value: f64,
    },
    NullPtr,
    Void,
}

/// Stmts are not value producing code
#[derive(Debug)]
pub enum Stmt {
    Branch {
        cond: Option<Expr>,
        then_block: String,
        else_block: Option<String>,
    },
    Ret {
        value: Option<Expr>,
    },
}
/// Exprs are value producing code
#[derive(Debug)]
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
    Assignment {
        dest: Box<Expr>,
        src: Box<Expr>,
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

fn ty_from<'c>(bve: BasicValueEnum<'c>) -> Result<Ty, &'static str> {
    let ty = bve.get_type();
    if ty.is_int_type() {
        let bits = ty.into_int_type().get_bit_width() as usize;
        if bits == 1 {
            Ok(Ty::Bool)
        } else {
            Ok(Ty::Int(bits, true))
        }
    } else if ty.is_float_type() {
        Ok(Ty::Float(ty.into_float_type().get_bit_width() as usize))
    } else if ty.is_pointer_type() {
        Ok(Ty::Ptr(Box::new(Ty::Void)))
    } else {
        Err("Unsupported LLVM operand type")
    }
}

fn value_to_expr<'c>(operand: Operand<'c>) -> Result<Expr, &'static str> {
    let bve = operand.value().ok_or("Expected a value operand")?;

    if bve.is_int_value() {
        let int = bve.into_int_value();
        if let Some(inst) = int.as_instruction() {
            Expr::build(inst)
        } else if int.is_const() {
            let bits = int.get_type().get_bit_width() as usize;
            let value = int
                .get_zero_extended_constant()
                .ok_or("Could not extract integer constant")?;
            if bits == 1 {
                Ok(Expr::Literal(Lit::Bool(value != 0)))
            } else {
                Ok(Expr::Literal(Lit::Int {
                    value,
                    bits,
                    signed: true,
                }))
            }
        } else {
            let name = int.get_name().to_string_lossy().into_owned();
            Ok(Expr::Var {
                name,
                dtype: ty_from(bve)?,
            })
        }
    } else if bve.is_float_value() {
        let float = bve.into_float_value();
        if let Some(inst) = float.as_instruction() {
            Expr::build(inst)
        } else if float.is_const() {
            let (value, _lossy) = float
                .get_constant()
                .ok_or("Could not extract float constant")?;
            let bits = float.get_type().get_bit_width() as usize;
            Ok(Expr::Literal(Lit::Float { bits, value }))
        } else {
            let name = float.get_name().to_string_lossy().into_owned();
            Ok(Expr::Var {
                name,
                dtype: ty_from(bve)?,
            })
        }
    } else if bve.is_pointer_value() {
        let ptr = bve.into_pointer_value();
        if let Some(inst) = ptr.as_instruction() {
            Expr::build(inst)
        } else if ptr.is_null() {
            Ok(Expr::Literal(Lit::NullPtr))
        } else {
            let name = ptr.get_name().to_string_lossy().into_owned();
            Ok(Expr::Var {
                name,
                dtype: ty_from(bve)?,
            })
        }
    } else {
        Err("Unsupported operand value kind")
    }
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
                if op1.is_none() {
                    Ok(Stmt::Ret { value: None })
                } else {
                    let value = value_to_expr(op1.unwrap())?;
                    Ok(Stmt::Ret { value: Some(value) })
                }
            }
            _ => Err("Instruction opcode not implemented yet!"),
        }
    }
}

#[cfg(test)]
mod tests {
    use inkwell::context::Context;

    use super::{Expr, Lit, Stmt};

    fn parse<'a>(ctxt: &'a Context, ir: &str) -> inkwell::module::Module<'a> {
        let mut bytes = ir.as_bytes().to_vec();
        bytes.push(b'\0');
        let mem = inkwell::memory_buffer::MemoryBuffer::create_from_memory_range(&bytes, "test");
        ctxt.create_module_from_ir(mem).unwrap()
    }

    fn collect_stmts(module: &inkwell::module::Module<'_>) -> Vec<(String, Stmt)> {
        let mut out = Vec::new();
        let func = module.get_first_function().unwrap();
        for bb in func.get_basic_blocks() {
            let name = bb.get_name().to_str().unwrap_or_default().to_owned();
            for inst in bb.get_instructions() {
                if let Ok(stmt) = Stmt::build(inst) {
                    out.push((
                        format!("{name}::{opcode:?}", opcode = inst.get_opcode()),
                        stmt,
                    ));
                }
            }
        }
        out
    }

    fn find<'a>(stmts: &'a [(String, Stmt)], key: &str) -> &'a Stmt {
        for (name, stmt) in stmts {
            if name == key {
                return stmt;
            }
        }
        panic!("no stmt with key {key}");
    }

    const BR_IR: &str = r#"define i32 @f() {
entry:
  br label %mid
mid:
  br i1 1, label %tru, label %fal
tru:
  ret i32 0
fal:
  ret i32 1
}
"#;

    #[test]
    fn unconditional_br_maps_to_then_block() {
        let ctxt = Context::create();
        let module = parse(&ctxt, BR_IR);
        let out = collect_stmts(&module);
        match find(&out, "entry::Br") {
            Stmt::Branch {
                cond: None,
                then_block,
                else_block: None,
            } => {
                assert_eq!(then_block, "mid")
            }
            other => panic!("expected unconditional Branch, got {other:?}"),
        }
    }

    #[test]
    fn conditional_br_successor_order() {
        let ctxt = Context::create();
        let module = parse(&ctxt, BR_IR);
        let out = collect_stmts(&module);
        match find(&out, "mid::Br") {
            Stmt::Branch {
                cond,
                then_block,
                else_block,
            } => {
                assert!(matches!(cond, Some(Expr::Literal(Lit::Bool(true)))));
                assert_eq!(then_block, "tru");
                assert_eq!(else_block.as_deref(), Some("fal"));
            }
            other => panic!("expected conditional Branch, got {other:?}"),
        }
    }

    #[test]
    fn ret_void_and_ret_value() {
        let ctxt = Context::create();
        let module = parse(&ctxt, BR_IR);
        let out = collect_stmts(&module);
        assert!(matches!(
            find(&out, "tru::Return"),
            Stmt::Ret {
                value: Some(Expr::Literal(Lit::Int { value: 0, .. }))
            }
        ));
        assert!(matches!(
            find(&out, "fal::Return"),
            Stmt::Ret {
                value: Some(Expr::Literal(Lit::Int { value: 1, .. }))
            }
        ));

        let ctxt = Context::create();
        let ir = "define void @g() {\nentry:\n  ret void\n}\n";
        let module = parse(&ctxt, ir);
        let out = collect_stmts(&module);
        assert!(matches!(
            find(&out, "entry::Return"),
            Stmt::Ret { value: None }
        ));
    }
}
