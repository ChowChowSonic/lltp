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
