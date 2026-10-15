use std::fmt::{self, Formatter};

use crate::hir::{
    Expr, Lit,
    expr::{BinaryExpr, CallExpr, CastExpr, FieldExpr, IndexExpr, UnaryExpr, VarExpr},
};

pub trait ExprEmitter {
    /// Default: this backend cannot express the node. Panics on reach — a
    /// reaching call means either capability was declared without an override
    /// (`allows_*() == true` + no `emit_*`) or a lowering pass violated its
    /// contract. A future best-effort mode may use a recoverable signal here.
    fn emit_expr(&self, expr: &Expr, out: &mut Formatter) -> fmt::Result {
        match expr {
            Expr::Literal(i) => self.emit_literal(i, out),
            Expr::Var(i) => self.emit_assignment(i, out),
            Expr::UnaryOp(i) => self.emit_unary_op(i, out),
            Expr::BinaryOp(i) => self.emit_binary_op(i, out),
            Expr::Call(i) => self.emit_call(i, out),
            Expr::Index(i) => self.emit_index(i, out),
            Expr::Field(i) => self.emit_field(i, out),
            Expr::Cast(i) => self.emit_cast(i, out),
            Expr::Nop => Ok(()),
        }
    }
    fn emit_call(&self, expr: &CallExpr, out: &mut Formatter) -> fmt::Result {
        let _ = (expr, out);
        unreachable!(
            "`emit_call` has no override for this backend; set the matching `allows_*()` flag to false or implement the emitter"
        )
    }
    fn emit_binary_op(&self, expr: &BinaryExpr, out: &mut Formatter) -> fmt::Result {
        let _ = (expr, out);
        unreachable!(
            "`emit_binary_op` has no override for this backend; set the matching `allows_*()` flag to false or implement the emitter"
        )
    }
    fn emit_unary_op(&self, expr: &UnaryExpr, out: &mut Formatter) -> fmt::Result {
        let _ = (expr, out);
        unreachable!(
            "`emit_unary_op` has no override for this backend; set the matching `allows_*()` flag to false or implement the emitter"
        )
    }
    fn emit_assignment(&self, expr: &VarExpr, out: &mut Formatter) -> fmt::Result {
        let _ = (expr, out);
        unreachable!(
            "`emit_assignment` has no override for this backend; set the matching `allows_*()` flag to false or implement the emitter"
        )
    }
    fn emit_index(&self, expr: &IndexExpr, out: &mut Formatter) -> fmt::Result {
        let _ = (expr, out);
        unreachable!(
            "`emit_index` has no override for this backend; set the matching `allows_*()` flag to false or implement the emitter"
        )
    }
    fn emit_field(&self, expr: &FieldExpr, out: &mut Formatter) -> fmt::Result {
        let _ = (expr, out);
        unreachable!(
            "`emit_field` has no override for this backend; set the matching `allows_*()` flag to false or implement the emitter"
        )
    }
    fn emit_cast(&self, expr: &CastExpr, out: &mut Formatter) -> fmt::Result {
        let _ = (expr, out);
        unreachable!(
            "`emit_cast` has no override for this backend; set the matching `allows_*()` flag to false or implement the emitter"
        )
    }
    fn emit_literal(&self, expr: &Lit, out: &mut Formatter) -> fmt::Result {
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
