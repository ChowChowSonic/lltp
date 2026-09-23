use inkwell::{context::Context, module::Module, values::GlobalValue};
pub mod backend;
pub mod hir;
pub mod passes;

pub fn get_globals<'ctx>(module: &Module<'ctx>) -> Vec<GlobalValue<'ctx>> {
    let mut ret: Vec<GlobalValue<'ctx>> = Vec::new();
    for x in module.get_globals() {
        ret.push(x);
    }
    ret
}

pub fn build_module<'ctx>(ctxt: &'ctx Context, ir: &str) -> Module<'ctx> {
    let ir_complete = if ir.ends_with('\0') {
        ir.to_string()
    } else {
        let mut s = ir.to_string();
        s.push('\0');
        s
    };
    let bytes = ir_complete.into_bytes();
    let mem_buf =
        inkwell::memory_buffer::MemoryBuffer::create_from_memory_range(&bytes, "test_module");
    ctxt.create_module_from_ir(mem_buf)
        .expect("Failed to create module from IR")
}

#[cfg(test)]
mod tests {
    use crate::hir::graph::build_graph;
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

    /// Spec: ingest IR through the public `build_module` seam, then build a
    /// per-function CFG from the resulting module.
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
