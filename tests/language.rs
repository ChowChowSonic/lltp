use lltp::backend::{ExprEmitter, Language, StmtEmitter};
use lltp::hir::{Expr, Lit, Stmt};
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
    fn emit_return(&self, stmt: &Stmt, out: &mut Formatter) -> std::fmt::Result {
        let Stmt::Ret { value } = stmt else { panic!() };
        write!(out, "return ")?;
        if let Some(v) = value {
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
    fn emit_goto(&self, stmt: &Stmt, out: &mut Formatter) -> std::fmt::Result {
        let Stmt::Goto { target } = stmt else {
            panic!()
        };
        write!(out, "goto {}", target)
    }
    fn emit_try(&self, stmt: &Stmt, out: &mut Formatter) -> std::fmt::Result {
        let _ = stmt;
        let _ = out;
        unimplemented!("I forgot about try statements. Whoops")
    }
    fn emit_catch(&self, stmt: &Stmt, out: &mut Formatter) -> std::fmt::Result {
        let _ = stmt;
        let _ = out;
        unimplemented!("I forgot about catch statements. whoops")
    }
}

impl ExprEmitter for CLanguage {
    fn emit_call(&self, expr: &Expr, out: &mut Formatter) -> std::fmt::Result {
        let Expr::Call { name, args } = expr else {
            panic!()
        };
        write!(out, "{name}(")?;
        for arg in args[0..args.len() - 1].iter() {
            self.emit_expr(arg, out)?;
            write!(out, ",")?;
        }
        self.emit_expr(args.last().unwrap(), out)?;
        write!(out, ");")
    }
    ///TODO: Needs to be properly reworked. Should be a Stmt defined by the `Let` enum
    fn emit_assignment(&self, expr: &Expr, out: &mut Formatter) -> std::fmt::Result {
        let Expr::Var { name, dtype } = expr else {
            panic!()
        };
        self.emit_ty(dtype, out)?;
        write!(out, "{name} = ")
    }
    fn emit_index(&self, expr: &Expr, out: &mut Formatter) -> std::fmt::Result {
        let Expr::Index { base, index } = expr else {
            panic!()
        };
        self.emit_expr(base, out)?;
        write!(out, "[")?;
        self.emit_expr(index, out)?;
        write!(out, "]")
    }
    fn emit_literal(&self, expr: &Expr, out: &mut Formatter) -> std::fmt::Result {
        let Expr::Literal(e) = expr else { panic!() };
        match e {
            Lit::Int { value, bits, .. } => write!(out, "{value}{bits}"),
            Lit::Bool(b) => write!(out, "{b}"),
            Lit::Float { value, .. } => write!(out, "{value}"),
            Lit::NullPtr => write!(out, "null"),
            Lit::Void => write!(out, "void"),
        }
    }
    fn emit_cast(&self, expr: &Expr, out: &mut Formatter) -> std::fmt::Result {
        let Expr::Cast { data, dtype } = expr else {
            panic!()
        };
        self.emit_expr(dtype, out)?;
        write!(out, " ")?;
        self.emit_expr(data, out)
    }
    fn emit_binary_op(&self, expr: &Expr, out: &mut Formatter) -> std::fmt::Result {
        let Expr::BinaryOp { op, arg1, arg2 } = expr else {
            panic!()
        };
        self.emit_expr(arg1, out)?;
        match op {
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
        self.emit_expr(arg2, out)
    }
    fn emit_unary_op(&self, expr: &Expr, _out: &mut Formatter) -> std::fmt::Result {
        let Expr::UnaryOp { op, arg } = expr else {
            panic!()
        };
        let (_, _) = (op, arg);
        todo!("Unary ops")
    }
}
