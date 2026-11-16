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
    hir::{Function, Module, Stmt},
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
    let module = match build_module(&ctxt, ir) {
        Ok(m) => m,
        Err(e) => return Err(PipelineError::IngestError(e)),
    };
    let mut recovered = match Module::from_module(&module) {
        Ok(m) => m,
        Err(e) => {
            return Err(PipelineError::ModuleParsingError(e));
        }
    };

    // The tree is what the passes and the capability check read, so structurize has
    // to run before either: recovery fills `cfg` and leaves `body` empty.
    let mut code: String = String::new();
    for func in &mut recovered.funcs {
        if let Err(e) = func.structurize() {
            return Err(PipelineError::FunctionEmissionError(
                func.sig.name.to_string(),
                format!("Failed to structurize the function: {e}"),
            ));
        }
        if let Err(e) = lang.lower(func) {
            return Err(PipelineError::PassApplicationError(format!(
                "Error while applying passes to {func}: {e}",
                func = func.sig.name
            )));
        }
        check_capabilities(lang, func)?;
        info!(
            "Successfully verified that no invalid instructions are in {func}",
            func = func.sig.name
        );
        if let Err(e) = lang.emit_function(func, &mut code) {
            return Err(PipelineError::FunctionEmissionError(
                func.sig.name.to_string(),
                e.to_string(),
            ));
        }
        info!("Emitted function {}", func.sig.name);
    }
    Ok(code)
}

/// Which nodes this backend can express. A node the target cannot print has to be
/// rewritten away before emission reaches it.
fn check_capabilities(lang: &impl Language, func: &Function) -> Result<(), PipelineError> {
    check_stmts(lang, &func.body)
}

fn check_stmts(lang: &impl Language, stmts: &[Stmt]) -> Result<(), PipelineError> {
    let mut returns = 0;
    for stmt in stmts {
        match stmt {
            Stmt::Ret(_) => {
                returns += 1;
                if !lang.allows_multiple_return() && returns > 1 {
                    return Err(PipelineError::DisallowedStmtError(
                        lang.name().to_string(),
                        "Ret".to_string(),
                    ));
                }
            }
            Stmt::Switch(node) => {
                if !lang.allows_switch() {
                    return Err(PipelineError::DisallowedStmtError(
                        lang.name().to_string(),
                        "Switch".to_string(),
                    ));
                }
                for (_, body) in &node.cases {
                    check_stmts(lang, body)?;
                }
                check_stmts(lang, &node.default)?;
            }
            Stmt::If(node) => {
                check_stmts(lang, &node.then_stmts)?;
                check_stmts(lang, &node.else_stmts)?;
            }
            Stmt::Loop(node) => {
                check_stmts(lang, &node.body)?;
            }
            Stmt::Goto(_) | Stmt::Label(_) if !lang.allows_goto() => {
                // Labels are only expressible where `goto` is, since they exist
                // solely as `goto` targets.
                return Err(PipelineError::DisallowedStmtError(
                    lang.name().to_string(),
                    "Goto".to_string(),
                ));
            }
            Stmt::TryCatch(_) if !lang.allows_exceptions() => {
                return Err(PipelineError::DisallowedStmtError(
                    lang.name().to_string(),
                    "TryCatch".to_string(),
                ));
            }
            Stmt::Invoke(_) if !lang.allows_unwind_fork() => {
                // Without unwind/fork the contract is error-code propagation or
                // try/catch, never an `invoke` arm.
                return Err(PipelineError::DisallowedStmtError(
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
