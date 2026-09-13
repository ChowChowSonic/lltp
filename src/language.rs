use std::fmt::Formatter;

pub trait Language {
    fn name(&self) -> &str;
    fn emit_function(&self, func: i32 /*crate::hir::func pending*/, out: &mut Formatter);
    fn allows_goto(&self) -> bool;
    fn allows_switch(&self) -> bool;
}
