use std::collections::BTreeMap;

use inkwell::{module::Module, values::FunctionValue};
use tracing::{debug, warn};

use crate::hir::{cfg::Cfg, graph::build_graph};

use super::{stmt::Stmt, ty::Ty};

#[derive(Debug)]
pub struct Function {
    pub name: String,
    pub params: Vec<Ty>,
    pub return_ty: Ty,
    pub blocks: Cfg,
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
            Some(x) => x,
            None => {
                warn!(name = %name, "function missing return type; treating recovery as abort");
                unreachable!("Error when unpacking return type of fn")
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
        match build_graph(&val) {
            Ok(cfg) => Function {
                name,
                params: param_types,
                return_ty: Ty::from(ret_ty),
                blocks: cfg,
                body: stmt_blocks,
            },
            _ => unreachable!("Unable to build function call graph"),
        }
    }
}
