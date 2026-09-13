use inkwell::{module::Module, values::GlobalValue};
mod hir;
mod language;

pub fn get_globals<'ctx>(module: &Module<'ctx>) -> Vec<GlobalValue<'ctx>> {
    let mut ret: Vec<GlobalValue<'ctx>> = Vec::new();
    for x in module.get_globals() {
        ret.push(x);
    }
    ret
}
