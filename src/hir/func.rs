use inkwell::{module::Module, values::FunctionValue};
use tracing::{debug, warn};

use crate::hir::{cfg::Cfg, expr::Expr, graph::build_graph};

use super::{stmt::Stmt, ty::Ty};

#[derive(Debug)]
pub struct Function {
    pub name: String,
    pub params: Vec<Ty>,
    pub return_ty: Ty,
    pub cfg: Cfg,
    pub body: Vec<Stmt>,
}

impl Function {
    pub fn collect_fns(module: &Module) -> Vec<Function> {
        let fns = module.get_functions();
        let recovered: Vec<Function> = fns.into_iter().map(|x| Function::build(&x)).collect();
        debug!(count = recovered.len(), "recovered functions from module");
        recovered
    }

    pub fn build(val: &FunctionValue) -> Self {
        let name = val.get_name().to_string_lossy().to_string();
        debug!(name = %name, "recovering function");
        let llvm_params = val.get_params();
        let param_types: Vec<Ty> = llvm_params.iter().map(|param| Ty::from(*param)).collect();
        let ret_ty = match val.get_type().get_return_type() {
            Some(x) => Ty::from(x),
            None => {
                warn!(name = %name, "function missing return type; treating recovery as void");
                Ty::Void
            }
        };
        let stmt_blocks = val
            .get_basic_block_iter()
            .map(|block| {
                let last = match block.get_last_instruction() {
                    Some(i) => i,
                    None => {
                        warn!(name = %name, "malformed basic block (no instructions) in function");
                        unreachable!("Malformed BasicBlock found")
                    }
                };
                match Stmt::build(last) {
                    Ok(x) => x,
                    Err(y) => {
                        warn!(name = %name, error = %y, "could not recover terminator from basic block");
                        unreachable!("Error when unpacking BasicBlocks")
                    }
                }
            })
            .collect();
        match build_graph(val) {
            Ok(mut cfg) => {
                Self::collect_instructions(val, &mut cfg);
                Function {
                    name,
                    params: param_types,
                    return_ty: ret_ty,
                    cfg,
                    body: stmt_blocks,
                }
            }
            _ => unreachable!("Unable to build function call graph"),
        }
    }

    fn collect_instructions(val: &FunctionValue, cfg: &mut Cfg) {
        for bb in val.get_basic_block_iter() {
            let name = bb.get_name().to_string_lossy().to_string();
            let mut stmts = Vec::new();
            for inst in bb.get_instructions() {
                if let Ok(s) = Stmt::build(inst) {
                    stmts.push(s); // terminators: Branch/Ret (last)
                } else if let Ok(e) = Expr::build(inst) {
                    let dest: String = inst
                        .get_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    let dtype = Ty::try_from(inst.get_type()).unwrap_or(Ty::Void);
                    if !dest.is_empty() {
                        stmts.push(Stmt::Let {
                            dest: Box::new(Expr::Var { name: dest, dtype }),
                            src: Box::new(e),
                        });
                    } else {
                        tracing::debug!("Skipping unnamed value")
                    }
                } else {
                    tracing::debug!(%name, "skipping unrecovered instruction"); // load/store/etc. — later milestones
                }
            }
            cfg.blocks
                .get_mut(&name)
                .expect("block from build_graph")
                .stmts = stmts;
        }
    }
}
#[cfg(test)]
mod test {
    use crate::hir::{Function, stmt::Stmt};
    const IR: &str = r#"define void @f(i32 %x) {
entry:
  %c = icmp sgt i32 %x, 0
  br i1 %c, label %pos, label %nonpos
pos:
  ret void
nonpos:
  ret void
}
"#;
    #[test]
    fn block_statements_are_recovered() {
        // lib.rs fixture: entry has %c = icmp (Let) then the terminator (Branch).
        let ctxt = inkwell::context::Context::create();
        let mut ir_str = IR.to_string();
        ir_str.push('\0');
        let module = crate::build_module(&ctxt, IR);
        let func = module.get_first_function().unwrap();
        let f = Function::build(&func);
        assert_eq!(f.cfg.blocks["entry"].stmts.len(), 2);
        assert!(matches!(&f.cfg.blocks["entry"].stmts[0], Stmt::Let { .. }));
        assert!(matches!(
            &f.cfg.blocks["entry"].stmts[1],
            Stmt::Branch { .. }
        ));
    }
}
