use lltp::backend::{ExprEmitter, Language, LowerTarget, StmtEmitter};
use lltp::frontend::{FrontendConfig, compile_c_to_ir};
use lltp::hir::expr::{BinaryExpr, CallExpr, CastExpr, CmpExpr, IndexExpr, UnaryExpr};
use lltp::hir::op::{BinOp, Cmp, IntCmp};
use lltp::hir::stmt::{GotoStmt, IfStmt, LabelStmt, LetStmt, LoopStmt, RetStmt, TryCatchStmt};
use lltp::hir::{Function, Lit, Stmt, Ty};
use lltp::pipeline;
use std::fmt::{Result, Write};
use std::path::PathBuf;
use std::process::ExitCode;
pub struct CLanguage;

impl LowerTarget for CLanguage {
    fn name(&self) -> &str {
        "C"
    }
}

impl Language for CLanguage {
    fn emit_function(&self, func: &Function, out: &mut impl Write) -> std::fmt::Result {
        self.emit_ty(&func.sig.ret, out)?;
        write!(out, " {}(", func.sig.name)?;
        for x in &func.sig.params {
            self.emit_ty(&x.ty, out)?;
            write!(out, " {},", x.name)?;
        }
        writeln!(out, ") {{")?;
        for x in &func.locals {
            self.emit_ty(&x.1, out)?;
            writeln!(out, " {};", x.0)?;
        }
        for x in &func.body {
            self.emit_stmt(x, out)?;
        }
        write!(out, "}}")
    }
    fn emit_ty(&self, ty: &Ty, out: &mut impl Write) -> std::fmt::Result {
        match ty {
            Ty::Void => write!(out, "void"),
            Ty::Int(sz, sig) => {
                if !sig {
                    write!(out, "unsigned ")?;
                }
                match sz {
                    64 => write!(out, "long"),
                    32 => write!(out, "int"),
                    16 => write!(out, "short"),
                    8 => write!(out, "char"),
                    _ => Err(std::fmt::Error),
                }
            }
            Ty::Bool => write!(out, "bool"),
            Ty::Float(sz) if *sz == 64usize => write!(out, "double"),
            Ty::Float(sz) if *sz == 32usize => write!(out, "float"),
            Ty::Float(sz) if *sz == 16usize => write!(out, "half"),
            other => unreachable!("toy emitter does not support type `{other:?}`"),
        }
    }
}
impl StmtEmitter for CLanguage {
    fn emit_label(&self, stmt: &LabelStmt, out: &mut impl Write) -> Result {
        writeln!(out, "{}:", stmt.name)
    }
    fn emit_block(&self, stmts: &[Stmt], out: &mut impl Write) -> std::fmt::Result {
        writeln!(out, "{{")?;

        for stmt in stmts {
            write!(out, "\t")?;
            self.emit_stmt(stmt, out)?;
            writeln!(out)?;
        }

        writeln!(out, "}}")
    }
    fn emit_if(&self, stmt: &IfStmt, out: &mut impl Write) -> std::fmt::Result {
        write!(out, "if (")?;
        self.emit_expr(&stmt.cond, out)?;
        write!(out, ")")?;
        self.emit_block(&stmt.then_stmts, out)?;
        write!(out, "else")?;
        self.emit_block(&stmt.else_stmts, out)?;
        Ok(())
    }
    fn emit_loop(&self, stmt: &LoopStmt, out: &mut impl Write) -> std::fmt::Result {
        write!(out, "while(")?;
        if let Some(cond) = &stmt.cond {
            self.emit_expr(cond, out)?;
        } else {
            write!(out, "true")?;
        }
        write!(out, ")")?;
        self.emit_block(&stmt.body, out)
    }
    fn emit_return(&self, stmt: &RetStmt, out: &mut impl Write) -> std::fmt::Result {
        write!(out, "return ")?;
        if let Some(v) = &stmt.value {
            self.emit_expr(v, out)?;
        }
        writeln!(out)
    }
    fn emit_break(&self, _stmt: &Stmt, out: &mut impl Write) -> std::fmt::Result {
        write!(out, "break")
    }
    fn emit_continue(&self, _stmt: &Stmt, out: &mut impl Write) -> std::fmt::Result {
        write!(out, "continue")
    }
    fn emit_goto(&self, stmt: &GotoStmt, out: &mut impl Write) -> std::fmt::Result {
        write!(out, "goto {}", stmt.target)
    }
    fn emit_try_catch(&self, stmt: &TryCatchStmt, out: &mut impl Write) -> std::fmt::Result {
        write!(out, "try {{")?;
        for x in &stmt.try_body {
            self.emit_stmt(x, out)?;
        }
        write!(out, "}}")?;
        for x in &stmt.catches {
            if let Some(ty) = &x.exception_ty
                && let Some(arg) = &x.param
            {
                write!(out, "catch(")?;
                self.emit_ty(ty, out)?;
                write!(out, " ")?;
                self.emit_variable(arg, out)?;
                write!(out, "){{")?;
                for y in &x.body {
                    self.emit_stmt(y, out)?;
                }
                write!(out, "}}")?;
            } else {
                write!(out, "catch(...){{")?;
                for y in &x.body {
                    self.emit_stmt(y, out)?;
                }
                write!(out, "}}")?;
            }
        }
        Ok(())
    }
    fn emit_assignment(&self, expr: &LetStmt, out: &mut impl Write) -> std::fmt::Result {
        self.emit_expr(&expr.dest, out)?;
        write!(out, " = ")?;
        self.emit_expr(&expr.src, out)
    }
}

impl ExprEmitter for CLanguage {
    fn emit_call(&self, expr: &CallExpr, out: &mut impl Write) -> std::fmt::Result {
        let name = &expr.name;
        write!(out, "{name}(")?;
        for arg in expr.args[0..expr.args.len() - 1].iter() {
            self.emit_expr(arg, out)?;
            write!(out, ",")?;
        }
        self.emit_expr(expr.args.last().unwrap(), out)?;
        write!(out, ");")
    }
    fn emit_index(&self, expr: &IndexExpr, out: &mut impl Write) -> std::fmt::Result {
        self.emit_expr(&expr.base, out)?;
        write!(out, "[")?;
        self.emit_expr(&expr.index, out)?;
        write!(out, "]")
    }
    fn emit_variable(
        &self,
        expr: &lltp::hir::expr::VarExpr,
        out: &mut impl Write,
    ) -> std::fmt::Result {
        let name = &expr.name;
        write!(out, "{name}")
    }
    fn emit_literal(&self, expr: &Lit, out: &mut impl Write) -> std::fmt::Result {
        match expr {
            Lit::Int { value, bits, .. } => write!(out, "{value}{bits}"),
            Lit::Bool(b) => write!(out, "{b}"),
            Lit::Float { value, .. } => write!(out, "{value}"),
            Lit::NullPtr => write!(out, "null"),
            Lit::Void => write!(out, "void"),
        }
    }
    fn emit_cast(&self, expr: &CastExpr, out: &mut impl Write) -> std::fmt::Result {
        self.emit_ty(&expr.dtype, out)?;
        write!(out, " ")?;
        self.emit_expr(&expr.data, out)
    }
    fn emit_cmp(&self, expr: &CmpExpr, out: &mut impl Write) -> std::fmt::Result {
        self.emit_expr(&expr.arg1, out)?;
        match expr.pred {
            Cmp::Int(IntCmp::Eq) => write!(out, " == ")?,
            other => unreachable!("toy emitter does not support predicate `{other:?}`"),
        }
        self.emit_expr(&expr.arg2, out)
    }
    fn emit_binary_op(&self, expr: &BinaryExpr, out: &mut impl Write) -> std::fmt::Result {
        self.emit_expr(&expr.arg1, out)?;
        match expr.op {
            BinOp::Add | BinOp::FAdd => write!(out, " + ")?,
            BinOp::Sub | BinOp::FSub => write!(out, " - ")?,
            BinOp::SDiv | BinOp::UDiv | BinOp::FDiv => write!(out, " / ")?,
            BinOp::And => write!(out, " & ")?,
            BinOp::AShr => write!(out, " > ")?,
            BinOp::Shl => write!(out, " < ")?,
            BinOp::Mul | BinOp::FMul => write!(out, " * ")?,
            other => unreachable!("Illegal binary operation `{other:?}`"),
        }
        self.emit_expr(&expr.arg2, out)
    }
    fn emit_unary_op(&self, expr: &UnaryExpr, _out: &mut impl Write) -> std::fmt::Result {
        let _ = expr;
        todo!("Unary ops")
    }
}

pub fn main() -> ExitCode {
    let src = PathBuf::from("./tests/fixtures/alpha_beta.c".to_string());
    let cfg = FrontendConfig::default();
    let ir = match compile_c_to_ir(&cfg, &src) {
        Ok(compiled) => compiled,
        Err(e) => {
            eprintln!("{:?}", e);
            return ExitCode::FAILURE;
        }
    };
    let t = CLanguage {};
    match pipeline(&t, &ir.ir) {
        Ok(code) => println!("{}", code),
        Err(e) => {
            eprintln!("{:?}", e);
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}
