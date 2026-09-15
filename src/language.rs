use std::fmt::Formatter;

pub trait Language {
    fn name(&self) -> &str;
    fn emit_function(&self, func: i32 /*crate::hir::func pending*/, out: &mut Formatter);
    fn emit_ty(&self, ty: i32 /*crate::hir::ty pending*/, out: &mut Formatter);
    fn emit_stmt(&self, stmt: i32 /*crate::hir::stmt pending*/, out: &mut Formatter);
    fn emit_expr(&self, expr: i32 /*crate::hir::expr pending*/, out: &mut Formatter);
    fn emit_module(&self, id: i32 /*crate::hir::module pending*/, out: &mut Formatter);
    fn emit_type(&self, id: i32 /*crate::hir::ty pending*/, out: &mut Formatter);

    fn emit_if(&self, id: i32 /*crate::hir::stmt pending*/, out: &mut Formatter);
    fn emit_while(&self, id: i32 /*crate::hir::stmt pending*/, out: &mut Formatter);
    fn emit_for(&self, id: i32 /*crate::hir::stmt pending*/, out: &mut Formatter);
    fn emit_do_while(&self, id: i32 /*crate::hir::stmt pending*/, out: &mut Formatter);
    fn emit_switch(&self, id: i32 /*crate::hir::stmt pending*/, out: &mut Formatter);
    fn emit_return(&self, id: i32 /*crate::hir::stmt pending*/, out: &mut Formatter);
    fn emit_break(&self, id: i32 /*crate::hir::stmt pending*/, out: &mut Formatter);
    fn emit_continue(&self, id: i32 /*crate::hir::stmt pending*/, out: &mut Formatter);
    fn emit_goto(&self, id: i32 /*crate::hir::stmt pending*/, out: &mut Formatter);

    fn emit_call(&self, id: i32 /*crate::hir::expr pending*/, out: &mut Formatter);
    fn emit_binary_op(&self, id: i32 /*crate::hir::expr pending*/, out: &mut Formatter);
    fn emit_unary_op(&self, id: i32 /*crate::hir::expr pending*/, out: &mut Formatter);
    fn emit_assignment(&self, id: i32 /*crate::hir::expr pending*/, out: &mut Formatter);
    fn emit_index(&self, id: i32 /*crate::hir::expr pending*/, out: &mut Formatter);
    fn emit_field(&self, id: i32 /*crate::hir::expr pending*/, out: &mut Formatter);
    fn emit_cast(&self, id: i32 /*crate::hir::expr pending*/, out: &mut Formatter);
    fn emit_literal(&self, id: i32 /*crate::hir::expr pending*/, out: &mut Formatter);
    fn emit_closure(&self, id: i32 /*crate::hir::expr pending*/, out: &mut Formatter);

    fn emit_try(&self, id: i32 /*crate::hir::stmt pending*/, out: &mut Formatter);
    fn emit_catch(&self, id: i32 /*crate::hir::stmt pending*/, out: &mut Formatter);
    fn emit_thread(&self, id: i32 /*crate::hir::stmt pending*/, out: &mut Formatter);
    fn emit_mutex(&self, id: i32 /*crate::hir::stmt pending*/, out: &mut Formatter);
    fn emit_atomic(&self, id: i32 /*crate::hir::expr pending*/, out: &mut Formatter);

    fn allows_match(&self) -> bool;
    fn allows_closures(&self) -> bool;
    fn allows_generics(&self) -> bool;
    fn allows_async(&self) -> bool;
    fn allows_threads(&self) -> bool;
    fn allows_locks(&self) -> bool;
    fn allows_raw_pointers(&self) -> bool;
    fn allows_exceptions(&self) -> bool;
    fn allows_multiple_return(&self) -> bool;
    fn allows_inheritance(&self) -> bool;
    fn allows_default_args(&self) -> bool;
    fn allows_union(&self) -> bool;
    fn allows_macros(&self) -> bool;
    fn allows_goto(&self) -> bool;
    fn allows_switch(&self) -> bool;
}
