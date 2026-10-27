use std::fmt::{self, Formatter};

use crate::hir::{
    Expr, Stmt,
    stmt::{GotoStmt, IfStmt, LabelStmt, LetStmt, LoopStmt, RetStmt, SwitchStmt, TryCatchStmt},
};

pub trait StmtEmitter {
    /// Default: this backend cannot express the node. Panics on reach — a
    /// reaching call means either capability was declared without an override
    /// (`allows_*() == true` + no `emit_*`) or a lowering pass violated its
    /// contract. A future best-effort mode may use a recoverable signal here.
    fn emit_stmt(&self, stmt: &Stmt, out: &mut Formatter) -> fmt::Result {
        match stmt {
            Stmt::If(i) => self.emit_if(i, out),
            Stmt::Loop(i) => self.emit_loop(i, out),
            Stmt::Branch(_i) => Ok(()),
            Stmt::Ret(i) => self.emit_return(i, out),
            Stmt::Switch(i) => self.emit_switch(i, out),
            Stmt::Let(i) => self.emit_assignment(i, out),
            Stmt::Eval(i) => self.emit_eval(i, out),
            Stmt::Unreachable => Ok(()),
            Stmt::Break => self.emit_break(stmt, out),
            Stmt::Continue => self.emit_continue(stmt, out),
            Stmt::Goto(i) => self.emit_goto(i, out),
            Stmt::Label(i) => self.emit_label(i, out),
            Stmt::TryCatch(i) => self.emit_try_catch(i, out),
        }
    }
    fn emit_label(&self, stmt: &LabelStmt, out: &mut Formatter) -> fmt::Result {
        let _ = (stmt, out);
        unreachable!(
            "`emit_label` has no override for this backend; set the matching `allows_*()` flag to false or implement the emitter"
        )
    }
    fn emit_block(&self, stmts: &[Stmt], out: &mut Formatter) -> fmt::Result {
        let _ = (stmts, out);
        unreachable!(
            "`emit_block` has no override for this backend; set the matching `allows_*()` flag to false or implement the emitter"
        )
    }

    fn emit_eval(&self, expr: &Expr, out: &mut Formatter) -> fmt::Result {
        let _ = (expr, out);
        unreachable!(
            "`emit_eval` has no override for this backend; set the matching `allows_*()` flag to false or implement the emitter"
        )
    }
    fn emit_assignment(&self, expr: &LetStmt, out: &mut Formatter) -> fmt::Result {
        let _ = (expr, out);
        unreachable!(
            "`emit_assignment` has no override for this backend; set the matching `allows_*()` flag to false or implement the emitter"
        )
    }
    fn emit_if(&self, stmt: &IfStmt, out: &mut Formatter) -> fmt::Result {
        let _ = (stmt, out);
        unreachable!(
            "`emit_if` has no override for this backend; set the matching `allows_*()` flag to false or implement the emitter"
        )
    }
    fn emit_loop(&self, stmt: &LoopStmt, out: &mut Formatter) -> fmt::Result {
        let _ = (stmt, out);
        unreachable!(
            "`emit_do_while` has no override for this backend; set the matching `allows_*()` flag to false or implement the emitter"
        )
    }
    fn emit_switch(&self, stmt: &SwitchStmt, out: &mut Formatter) -> fmt::Result {
        let _ = (stmt, out);
        unreachable!(
            "`emit_switch` has no override for this backend; set the matching `allows_*()` flag to false or implement the emitter"
        )
    }
    fn emit_return(&self, stmt: &RetStmt, out: &mut Formatter) -> fmt::Result {
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
    fn emit_goto(&self, stmt: &GotoStmt, out: &mut Formatter) -> fmt::Result {
        let _ = (stmt, out);
        unreachable!(
            "`emit_goto` has no override for this backend; set the matching `allows_*()` flag to false or implement the emitter"
        )
    }

    fn emit_try_catch(&self, stmt: &TryCatchStmt, out: &mut Formatter) -> fmt::Result {
        let _ = (stmt, out);
        unreachable!(
            "`emit_try_catch` has no override for this backend; set the matching `allows_*()` flag to false or implement the emitter"
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
