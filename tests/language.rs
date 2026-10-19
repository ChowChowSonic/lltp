use lltp::backend::{ExprEmitter, Language, StmtEmitter};
use lltp::hir::expr::{BinaryExpr, CallExpr, CastExpr, IndexExpr, UnaryExpr};
use lltp::hir::stmt::{GotoStmt, IfStmt, LabelStmt, LetStmt, LoopStmt, RetStmt, TryCatchStmt};
use lltp::hir::{Lit, Stmt};
use std::fmt::{Formatter, Result};
pub struct CLanguage;

impl Language for CLanguage {
    fn name(&self) -> &str {
        "C"
    }
}
impl StmtEmitter for CLanguage {
    fn emit_label(&self, stmt: &LabelStmt, out: &mut Formatter) -> Result {
        writeln!(out, "{}:", stmt.name)
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
    fn emit_if(&self, stmt: &IfStmt, out: &mut Formatter) -> std::fmt::Result {
        write!(out, "if (")?;
        self.emit_expr(&stmt.cond, out)?;
        write!(out, ")")?;
        self.emit_block(&stmt.then_stmts, out)?;
        write!(out, "else")?;
        self.emit_block(&stmt.else_stmts, out)?;
        Ok(())
    }
    fn emit_loop(&self, stmt: &LoopStmt, out: &mut Formatter) -> std::fmt::Result {
        write!(out, "while(")?;
        if let Some(cond) = &stmt.cond {
            self.emit_expr(cond, out)?;
        } else {
            write!(out, "true")?;
        }
        write!(out, ")")?;
        self.emit_block(&stmt.body, out)
    }
    fn emit_return(&self, stmt: &RetStmt, out: &mut Formatter) -> std::fmt::Result {
        write!(out, "return ")?;
        if let Some(v) = &stmt.value {
            self.emit_expr(v, out)?;
        }
        writeln!(out)
    }
    fn emit_break(&self, _stmt: &Stmt, out: &mut Formatter) -> std::fmt::Result {
        write!(out, "break")
    }
    fn emit_continue(&self, _stmt: &Stmt, out: &mut Formatter) -> std::fmt::Result {
        write!(out, "continue")
    }
    fn emit_goto(&self, stmt: &GotoStmt, out: &mut Formatter) -> std::fmt::Result {
        write!(out, "goto {}", stmt.target)
    }
    fn emit_try_catch(&self, stmt: &TryCatchStmt, out: &mut Formatter) -> std::fmt::Result {
        unimplemented!("err")
    }
    fn emit_assignment(&self, expr: &LetStmt, out: &mut Formatter) -> std::fmt::Result {
        self.emit_expr(&expr.dest, out)?;
        write!(out, " = ")?;
        self.emit_expr(&expr.src, out)
    }
}

impl ExprEmitter for CLanguage {
    fn emit_call(&self, expr: &CallExpr, out: &mut Formatter) -> std::fmt::Result {
        let name = &expr.name;
        write!(out, "{name}(")?;
        for arg in expr.args[0..expr.args.len() - 1].iter() {
            self.emit_expr(arg, out)?;
            write!(out, ",")?;
        }
        self.emit_expr(expr.args.last().unwrap(), out)?;
        write!(out, ");")
    }
    fn emit_index(&self, expr: &IndexExpr, out: &mut Formatter) -> std::fmt::Result {
        self.emit_expr(&expr.base, out)?;
        write!(out, "[")?;
        self.emit_expr(&expr.index, out)?;
        write!(out, "]")
    }
    fn emit_variable(
        &self,
        expr: &lltp::hir::expr::VarExpr,
        out: &mut Formatter,
    ) -> std::fmt::Result {
        let name = &expr.name;
        write!(out, "{name}")
    }
    fn emit_literal(&self, expr: &Lit, out: &mut Formatter) -> std::fmt::Result {
        match expr {
            Lit::Int { value, bits, .. } => write!(out, "{value}{bits}"),
            Lit::Bool(b) => write!(out, "{b}"),
            Lit::Float { value, .. } => write!(out, "{value}"),
            Lit::NullPtr => write!(out, "null"),
            Lit::Void => write!(out, "void"),
        }
    }
    fn emit_cast(&self, expr: &CastExpr, out: &mut Formatter) -> std::fmt::Result {
        self.emit_expr(&expr.dtype, out)?;
        write!(out, " ")?;
        self.emit_expr(&expr.data, out)
    }
    fn emit_binary_op(&self, expr: &BinaryExpr, out: &mut Formatter) -> std::fmt::Result {
        self.emit_expr(&expr.arg1, out)?;
        match expr.op {
            inkwell::values::InstructionOpcode::FAdd | inkwell::values::InstructionOpcode::Add => {
                write!(out, " + ")?;
            }
            inkwell::values::InstructionOpcode::Sub | inkwell::values::InstructionOpcode::FSub => {
                write!(out, " - ")?
            }
            inkwell::values::InstructionOpcode::FDiv
            | inkwell::values::InstructionOpcode::SDiv
            | inkwell::values::InstructionOpcode::UDiv => write!(out, " / ")?,
            inkwell::values::InstructionOpcode::And => write!(out, " & ")?,
            inkwell::values::InstructionOpcode::AShr => write!(out, " > ")?,
            inkwell::values::InstructionOpcode::Shl => write!(out, " < ")?,
            inkwell::values::InstructionOpcode::Mul | inkwell::values::InstructionOpcode::FMul => {
                write!(out, " * ")?
            }
            _ => unreachable!("Illegal binary operation"),
        }
        self.emit_expr(&expr.arg2, out)
    }
    fn emit_unary_op(&self, expr: &UnaryExpr, _out: &mut Formatter) -> std::fmt::Result {
        let _ = expr;
        todo!("Unary ops")
    }
}
