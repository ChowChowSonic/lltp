use std::fmt::{self, Formatter};

use crate::hir::expr::Expr;

pub trait ExprEmitter {
    /// Default: this backend cannot express the node. Panics on reach — a
    /// reaching call means either capability was declared without an override
    /// (`allows_*() == true` + no `emit_*`) or a lowering pass violated its
    /// contract. A future best-effort mode may use a recoverable signal here.
    fn emit_expr(&self, expr: &Expr, out: &mut Formatter) -> fmt::Result {
        let _ = (expr, out);
        unreachable!(
            "`emit_expr` has no override for this backend; set the matching `allows_*()` flag to false or implement the emitter"
        )
    }
    fn emit_call(&self, expr: &Expr, out: &mut Formatter) -> fmt::Result {
        let _ = (expr, out);
        unreachable!(
            "`emit_call` has no override for this backend; set the matching `allows_*()` flag to false or implement the emitter"
        )
    }
    fn emit_binary_op(&self, expr: &Expr, out: &mut Formatter) -> fmt::Result {
        let _ = (expr, out);
        unreachable!(
            "`emit_binary_op` has no override for this backend; set the matching `allows_*()` flag to false or implement the emitter"
        )
    }
    fn emit_unary_op(&self, expr: &Expr, out: &mut Formatter) -> fmt::Result {
        let _ = (expr, out);
        unreachable!(
            "`emit_unary_op` has no override for this backend; set the matching `allows_*()` flag to false or implement the emitter"
        )
    }
    fn emit_assignment(&self, expr: &Expr, out: &mut Formatter) -> fmt::Result {
        let _ = (expr, out);
        unreachable!(
            "`emit_assignment` has no override for this backend; set the matching `allows_*()` flag to false or implement the emitter"
        )
    }
    fn emit_index(&self, expr: &Expr, out: &mut Formatter) -> fmt::Result {
        let _ = (expr, out);
        unreachable!(
            "`emit_index` has no override for this backend; set the matching `allows_*()` flag to false or implement the emitter"
        )
    }
    fn emit_field(&self, expr: &Expr, out: &mut Formatter) -> fmt::Result {
        let _ = (expr, out);
        unreachable!(
            "`emit_field` has no override for this backend; set the matching `allows_*()` flag to false or implement the emitter"
        )
    }
    fn emit_cast(&self, expr: &Expr, out: &mut Formatter) -> fmt::Result {
        let _ = (expr, out);
        unreachable!(
            "`emit_cast` has no override for this backend; set the matching `allows_*()` flag to false or implement the emitter"
        )
    }
    fn emit_literal(&self, expr: &Expr, out: &mut Formatter) -> fmt::Result {
        let _ = (expr, out);
        unreachable!(
            "`emit_literal` has no override for this backend; set the matching `allows_*()` flag to false or implement the emitter"
        )
    }
    fn emit_closure(&self, expr: &Expr, out: &mut Formatter) -> fmt::Result {
        let _ = (expr, out);
        unreachable!(
            "`emit_closure` has no override for this backend; set the matching `allows_*()` flag to false or implement the emitter"
        )
    }
}
