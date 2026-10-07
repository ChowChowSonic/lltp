use std::fmt::{self, Formatter};

use crate::hir::{Expr, Stmt};

pub trait StmtEmitter {
    /// Default: this backend cannot express the node. Panics on reach — a
    /// reaching call means either capability was declared without an override
    /// (`allows_*() == true` + no `emit_*`) or a lowering pass violated its
    /// contract. A future best-effort mode may use a recoverable signal here.
    fn emit_stmt(&self, stmt: &Stmt, out: &mut Formatter) -> fmt::Result {
        let _ = (stmt, out);
        unreachable!(
            "`emit_stmt` has no override for this backend; set the matching `allows_*()` flag to false or implement the emitter"
        )
    }
    fn emit_block(&self, stmts: &[Stmt], out: &mut Formatter) -> fmt::Result {
        let _ = (stmts, out);
        unreachable!(
            "`emit_block` has no override for this backend; set the matching `allows_*()` flag to false or implement the emitter"
        )
    }

    fn emit_if(&self, stmt: &Stmt, out: &mut Formatter) -> fmt::Result {
        let _ = (stmt, out);
        unreachable!(
            "`emit_if` has no override for this backend; set the matching `allows_*()` flag to false or implement the emitter"
        )
    }
    fn emit_while(&self, stmt: &Stmt, out: &mut Formatter) -> fmt::Result {
        let _ = (stmt, out);
        unreachable!(
            "`emit_while` has no override for this backend; set the matching `allows_*()` flag to false or implement the emitter"
        )
    }
    fn emit_for(&self, stmt: &Stmt, out: &mut Formatter) -> fmt::Result {
        let _ = (stmt, out);
        unreachable!(
            "`emit_for` has no override for this backend; set the matching `allows_*()` flag to false or implement the emitter"
        )
    }
    fn emit_do_while(&self, stmt: &Stmt, out: &mut Formatter) -> fmt::Result {
        let _ = (stmt, out);
        unreachable!(
            "`emit_do_while` has no override for this backend; set the matching `allows_*()` flag to false or implement the emitter"
        )
    }
    fn emit_switch(&self, stmt: &Stmt, out: &mut Formatter) -> fmt::Result {
        let _ = (stmt, out);
        unreachable!(
            "`emit_switch` has no override for this backend; set the matching `allows_*()` flag to false or implement the emitter"
        )
    }
    fn emit_return(&self, stmt: &Stmt, out: &mut Formatter) -> fmt::Result {
        let _ = (stmt, out);
        unreachable!(
            "`emit_return` has no override for this backend; set the matching `allows_*()` flag to false or implement the emitter"
        )
    }
    fn emit_break(&self, stmt: &Stmt, out: &mut Formatter) -> fmt::Result {
        let _ = (stmt, out);
        unreachable!(
            "`emit_break` has no override for this backend; set the matching `allows_*()` flag to false or implement the emitter"
        )
    }
    fn emit_continue(&self, stmt: &Stmt, out: &mut Formatter) -> fmt::Result {
        let _ = (stmt, out);
        unreachable!(
            "`emit_continue` has no override for this backend; set the matching `allows_*()` flag to false or implement the emitter"
        )
    }
    fn emit_goto(&self, stmt: &Stmt, out: &mut Formatter) -> fmt::Result {
        let _ = (stmt, out);
        unreachable!(
            "`emit_goto` has no override for this backend; set the matching `allows_*()` flag to false or implement the emitter"
        )
    }

    fn emit_try(&self, stmt: &Stmt, out: &mut Formatter) -> fmt::Result {
        let _ = (stmt, out);
        unreachable!(
            "`emit_try` has no override for this backend; set the matching `allows_*()` flag to false or implement the emitter"
        )
    }
    fn emit_catch(&self, stmt: &Stmt, out: &mut Formatter) -> fmt::Result {
        let _ = (stmt, out);
        unreachable!(
            "`emit_catch` has no override for this backend; set the matching `allows_*()` flag to false or implement the emitter"
        )
    }
    fn emit_thread(&self, stmt: &Stmt, out: &mut Formatter) -> fmt::Result {
        let _ = (stmt, out);
        unreachable!(
            "`emit_thread` has no override for this backend; set the matching `allows_*()` flag to false or implement the emitter"
        )
    }
    fn emit_mutex(&self, stmt: &Stmt, out: &mut Formatter) -> fmt::Result {
        let _ = (stmt, out);
        unreachable!(
            "`emit_mutex` has no override for this backend; set the matching `allows_*()` flag to false or implement the emitter"
        )
    }
    fn emit_atomic(&self, expr: &Expr, out: &mut Formatter) -> fmt::Result {
        let _ = (expr, out);
        unreachable!(
            "`emit_atomic` has no override for this backend; set the matching `allows_*()` flag to false or implement the emitter"
        )
    }
}
