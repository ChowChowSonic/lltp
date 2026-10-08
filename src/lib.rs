use inkwell::{context::Context, module::Module, values::GlobalValue};
pub mod backend;
pub mod frontend;
pub mod hir;
pub mod ingest;
pub mod passes;
pub use ingest::{IngestError, IrFormat, load_module, parse_ir};

pub fn get_globals<'ctx>(module: &Module<'ctx>) -> Vec<GlobalValue<'ctx>> {
    let mut ret: Vec<GlobalValue<'ctx>> = Vec::new();
    for x in module.get_globals() {
        ret.push(x);
    }
    ret
}

/// Parse textual IR
pub fn build_module<'ctx>(ctxt: &'ctx Context, ir: &str) -> Module<'ctx> {
    parse_ir(ctxt, ir).unwrap_or_else(|e| panic!("Failed to create module from IR: {e}"))
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
        let module = build_module(&ctxt, IR);
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
