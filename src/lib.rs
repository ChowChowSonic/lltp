use inkwell::{context::Context, memory_buffer::MemoryBuffer, module::Module};
pub mod backend;
pub mod errors;
pub mod frontend;
pub mod hir;
pub mod ingest;
pub mod passes;
use crate::errors::{IngestError, LowerError};
use crate::{backend::Language, hir::flat::FlatModule};
pub use ingest::{IrFormat, load_module};

/// Parse textual IR
pub fn build_module<'ctx>(ctxt: &'ctx Context, ir: &str) -> Result<Module<'ctx>, IngestError> {
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

pub fn pipeline<T: Language>(ir: &str) -> Result<(), LowerError> {
    let ctxt = Context::create();
    let module = build_module(&ctxt, ir).expect("Unable to build module");
    let flat = match FlatModule::from_module(&module) {
        Ok(m) => m,
        Err(_) => {
            return Err(LowerError::Other(
                "Failed to retrieve FlatModule from Module".to_string(),
            ));
        }
    };
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
            vec!["nonpos".to_string(), "pos".to_string()]
        );
        assert_eq!(cfg.blocks["pos"].pred, vec!["entry".to_string()]);
        assert_eq!(cfg.blocks["nonpos"].pred, vec!["entry".to_string()]);
        assert!(cfg.blocks["pos"].succ.is_empty());
        assert!(cfg.blocks["nonpos"].succ.is_empty());
        assert_eq!(cfg.exits, vec!["pos".to_string(), "nonpos".to_string()]);
    }
}
