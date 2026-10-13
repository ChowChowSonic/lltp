use lltp::backend::{ExprEmitter, Language, StmtEmitter};
use lltp::hir::Stmt;
use std::fmt::{Formatter, Result};
pub struct CLanguage;

impl Language for CLanguage {
    fn name(&self) -> &str {
        "C"
    }
}
impl StmtEmitter for CLanguage {
    fn emit_label(&self, stmt: &Stmt, out: &mut Formatter) -> Result {
        let Stmt::Label { name } = stmt else { panic!() };
        writeln!(out, "{}:", name)
    }
    fn emit_block(&self, stmts: &[Stmt], out: &mut Formatter) -> std::fmt::Result {
        writeln!(out, "{{")?;

        for stmt in stmts {
            write!(out, "\t")?;
            self.emit_stmt(stmt, out)?;
            writeln!(out)?;
        }

        writeln!(out, "}}")
    }
    fn emit_if(&self, stmt: &Stmt, out: &mut Formatter) -> std::fmt::Result {
        let Stmt::If {
            cond,
            then_stmts,
            else_stmts,
        } = stmt
        else {
            panic!()
        };
        write!(out, "if (")?;
        self.emit_expr(cond, out)?;
        write!(out, ")")?;
        self.emit_block(then_stmts, out)?;
        write!(out, "else")?;
        self.emit_block(else_stmts, out)?;
        Ok(())
    }
    fn emit_loop(&self, stmt: &Stmt, out: &mut Formatter) -> std::fmt::Result {
        let Stmt::Loop { cond, body } = stmt else {
            panic!()
        };
        write!(out, "while(")?;
        if let Some(cond) = cond {
            self.emit_expr(cond, out)?;
        } else {
            write!(out, "true")?;
        }
        write!(out, ")")?;
        self.emit_block(body, out)
    }
}

impl ExprEmitter for CLanguage {}
