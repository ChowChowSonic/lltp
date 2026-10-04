use crate::hir::expr::Expr;
use crate::hir::ty::Ty;
use inkwell::values::Operand;
pub mod expr;
pub mod func;
pub mod graph;
pub mod stmt;
pub mod ty;
pub mod flat;
pub mod op;
pub use func::Function;

#[derive(Debug, Clone)]
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
                dtype: Ty::from(bve),
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
                dtype: Ty::from(bve),
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
                dtype: Ty::from(bve),
            })
        }
    } else {
        Err("Unsupported operand value kind")
    }
}

#[cfg(test)]
mod tests {
    use inkwell::context::Context;

    use super::{Lit, expr::Expr, stmt::Stmt};

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
