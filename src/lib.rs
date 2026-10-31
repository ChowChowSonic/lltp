use inkwell::{context::Context, memory_buffer::MemoryBuffer, module::Module as IrModule};
use tracing::info;
pub mod backend;
pub mod errors;
pub mod frontend;
pub mod hir;
pub mod ingest;
pub mod passes;
use crate::errors::{IngestError, PipelineError};
use crate::{
    backend::Language,
    hir::{Module, Stmt},
};
pub use ingest::{IrFormat, load_module};

/// Parse textual IR
pub fn build_module<'ctx>(ctxt: &'ctx Context, ir: &str) -> Result<IrModule<'ctx>, IngestError> {
    let mut bytes = ir.as_bytes().to_vec();
    if bytes.last() != Some(&0) {
        bytes.push(0);
    }
    let mem_buf = MemoryBuffer::create_from_memory_range(&bytes, "test_module");
    ctxt.create_module_from_ir(mem_buf)
        .map_err(|e| IngestError::Parse {
            origin: "<memory>".to_string(),
            format: IrFormat::Text,
            detail: e.to_string(),
        })
}

pub fn pipeline(lang: &impl Language, ir: &str) -> Result<String, PipelineError> {
    let ctxt = Context::create();
    let module = build_module(&ctxt, ir).expect("Unable to build module");
    let mut recovered = match Module::from_module(&module) {
        Ok(m) => m,
        Err(e) => {
            return Err(PipelineError::ModuleParsingError(e));
        }
    };
    let res = recovered
        .funcs
        .iter_mut()
        .try_for_each(|item| match lang.lower(item) {
            Ok(_) => Ok(()),
            Err(e) => Err(format!("Error while applying passes to LLVM IR:\n{}", e)),
        });
    match res {
        Ok(_) => {
            info!("Successfully applied all passes to IR");
        }
        Err(e) => {
            return Err(PipelineError::PassApplicationError(e));
        }
    };
    let is_valid: Result<(), PipelineError> = recovered.funcs.iter().try_for_each(|item| {
        for stmt in &item.body {
            match stmt {
                Stmt::Ret(_)
                    if !lang.allows_multiple_return()
                        && item
                            .body
                            .iter()
                            .filter(|s| matches!(s, Stmt::Ret(_)))
                            .count()
                            > 1 =>
                {
                    // The flag only says the backend may assume single-value
                    // returns, so a lone `Ret` is fine; more than one is not.
                    return Err(PipelineError::DissallowedStmtError(
                        lang.name().to_string(),
                        "Ret".to_string(),
                    ));
                }
                Stmt::Switch(_) if !lang.allows_switch() => {
                    return Err(PipelineError::DissallowedStmtError(
                        lang.name().to_string(),
                        "Switch".to_string(),
                    ));
                }
                Stmt::Goto(_) if !lang.allows_goto() => {
                    return Err(PipelineError::DissallowedStmtError(
                        lang.name().to_string(),
                        "Goto".to_string(),
                    ));
                }
                Stmt::Label(_) if !lang.allows_goto() => {
                    // Labels are only expressible where `goto` is, since they
                    // exist solely as `goto` targets.
                    {
                        return Err(PipelineError::DissallowedStmtError(
                            lang.name().to_string(),
                            "Label".to_string(),
                        ));
                    }
                }
                Stmt::TryCatch(_) if !lang.allows_exceptions() => {
                    return Err(PipelineError::DissallowedStmtError(
                        lang.name().to_string(),
                        "TryCatch".to_string(),
                    ));
                }
                Stmt::Invoke(_) if !lang.allows_unwind_fork() => {
                    // Without unwind/fork the contract is error-code
                    // propagation or try/catch, never an `invoke` arm.
                    return Err(PipelineError::DissallowedStmtError(
                        lang.name().to_string(),
                        "Invoke".to_string(),
                    ));
                }
                // The remaining nodes are core structured control flow with no
                // capability flag, so every backend may see them.
                _ => (),
            }
        }
        Ok(())
    });
    match is_valid {
        Ok(_) => {
            info!("Successfully verified that no invalid instructions are in the IR");
        }
        Err(e) => return Err(e),
    }
    let mut code: String = String::new();
    for func in &recovered.funcs {
        match lang.emit_function(func, &mut code) {
            Ok(_) => {
                info!("Emitted function {}", func.sig.name);
            }
            Err(e) => {
                return Err(PipelineError::FunctionEmissionError(
                    func.sig.name.to_string(),
                    e,
                ));
            }
        }
    }
    Ok(code)
}

#[cfg(test)]
mod tests {
    use crate::hir::lowering::build_graph;
    use crate::{build_module, hir::cfg::Cfg};
    use inkwell::context::Context;

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
    fn cfgs_build_from_ingested_module() {
        let ctxt = Context::create();
        let module = build_module(&ctxt, IR).unwrap();
        let func = module.get_first_function().expect("IR contains a function");
        let cfg: Cfg = build_graph(&func).expect("CFG build should succeed");

        assert_eq!(cfg.entry, "entry");
        assert_eq!(
            cfg.blocks["entry"].succ,
            vec!["pos".to_string(), "nonpos".to_string()]
        );

        assert_eq!(cfg.blocks["pos"].pred, vec!["entry".to_string()]);
        assert_eq!(cfg.blocks["nonpos"].pred, vec!["entry".to_string()]);
        assert!(cfg.blocks["pos"].succ.is_empty());
        assert!(cfg.blocks["nonpos"].succ.is_empty());
        assert_eq!(cfg.exits, vec!["pos".to_string(), "nonpos".to_string()]);
    }
}
