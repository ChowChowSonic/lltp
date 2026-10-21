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

#[cfg(test)]
mod test {
    use std::fmt::Formatter;

    use inkwell::values::InstructionOpcode;

    use lltp::backend::{ExprEmitter, Language, StmtEmitter};
    use lltp::hir::expr::{CallExpr, CastExpr, IndexExpr};
    use lltp::hir::stmt::{BranchStmt, CatchClause, GotoStmt, LabelStmt, LoopStmt, TryCatchStmt};
    use lltp::hir::{Expr, Lit, Stmt, Ty};

    use super::CLanguage;

    const I32: Ty = Ty::Int(32, true);

    /// `std::fmt::Formatter` has no public constructor, so a `String` sink only
    /// ever receives one inside a `Display` impl. This shim is how the helpers
    /// below get a formatter to hand to the backend, and it keeps the "emit
    /// failed" behaviour the tests expect from a formatter error.
    struct Render<'a, F> {
        run: &'a F,
    }

    impl<'a, F> std::fmt::Display for Render<'a, F>
    where
        F: Fn(&mut Formatter) -> std::fmt::Result,
    {
        fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
            (self.run)(f).expect("emit failed");
            Ok(())
        }
    }

    /// Run the emitter through the shim and return what it wrote.
    fn render(run: impl Fn(&mut Formatter) -> std::fmt::Result) -> String {
        format!("{}", Render { run: &run })
    }

    /// Render one expression through the backend.
    fn emit_expr(expr: &Expr) -> String {
        render(|f| CLanguage.emit_expr(expr, f))
    }

    /// Render one statement through the backend.
    fn emit_stmt(stmt: &Stmt) -> String {
        render(|f| CLanguage.emit_stmt(stmt, f))
    }

    fn emit_block(stmts: &[Stmt]) -> String {
        render(|f| CLanguage.emit_block(stmts, f))
    }

    fn var(name: &str) -> Expr {
        Expr::var(name, I32)
    }

    fn int(value: u64) -> Expr {
        Expr::int(value, 32, true)
    }

    fn binding(dest: &str, src: Expr) -> Stmt {
        Stmt::let_binding(var(dest), src)
    }

    #[test]
    fn backend_name() {
        assert_eq!(CLanguage.name(), "C");
    }

    #[test]
    fn int_literals_write_value_then_bit_width() {
        // No separator between value and width, so 42 at 32 bits reads `4232`.
        assert_eq!(emit_expr(&int(42)), "4232");
        assert_eq!(emit_expr(&Expr::int(7, 8, false)), "78");
    }

    #[test]
    fn other_literals() {
        assert_eq!(emit_expr(&Expr::bool(true)), "true");
        assert_eq!(emit_expr(&Expr::bool(false)), "false");
        assert_eq!(
            emit_expr(&Expr::Literal(Lit::Float {
                bits: 64,
                value: 1.5
            })),
            "1.5"
        );
        assert_eq!(emit_expr(&Expr::Literal(Lit::NullPtr)), "null");
        assert_eq!(emit_expr(&Expr::Literal(Lit::Void)), "void");
    }

    #[test]
    fn variables_write_their_name() {
        assert_eq!(emit_expr(&var("x")), "x");
    }

    #[test]
    fn binary_operators() {
        // Shifts currently render as `<`/`>` rather than `<<`/`>>`.
        let cases = [
            (InstructionOpcode::Add, " + "),
            (InstructionOpcode::FAdd, " + "),
            (InstructionOpcode::Sub, " - "),
            (InstructionOpcode::FSub, " - "),
            (InstructionOpcode::Mul, " * "),
            (InstructionOpcode::FMul, " * "),
            (InstructionOpcode::SDiv, " / "),
            (InstructionOpcode::UDiv, " / "),
            (InstructionOpcode::FDiv, " / "),
            (InstructionOpcode::And, " & "),
            (InstructionOpcode::Shl, " < "),
            (InstructionOpcode::AShr, " > "),
        ];
        for (op, sym) in cases {
            assert_eq!(
                emit_expr(&Expr::binary(op, var("a"), var("b"))),
                format!("a{sym}b"),
                "{op:?}"
            );
        }
    }

    #[test]
    fn calls_end_with_a_semicolon() {
        let single = Expr::Call(CallExpr {
            name: String::from("puts"),
            args: vec![int(1)],
        });
        assert_eq!(emit_expr(&single), "puts(132);");

        let many = Expr::Call(CallExpr {
            name: String::from("add"),
            args: vec![var("a"), var("b"), int(3)],
        });
        assert_eq!(emit_expr(&many), "add(a,b,332);");
    }

    #[test]
    fn index() {
        let expr = Expr::Index(IndexExpr {
            base: Box::new(var("arr")),
            index: Box::new(var("i")),
        });
        assert_eq!(emit_expr(&expr), "arr[i]");
    }

    #[test]
    fn cast_writes_the_dtype_then_the_value() {
        let expr = Expr::Cast(CastExpr {
            data: Box::new(int(1)),
            dtype: Box::new(Expr::var("int", Ty::Void)),
        });
        assert_eq!(emit_expr(&expr), "int 132");
    }

    #[test]
    fn label() {
        assert_eq!(
            emit_stmt(&Stmt::Label(LabelStmt {
                name: String::from("loop"),
            })),
            "loop:\n"
        );
    }

    #[test]
    fn block() {
        let stmts = [binding("a", int(1)), binding("b", var("a"))];
        assert_eq!(emit_block(&stmts), "{\n\ta = 132\n\tb = a\n}\n");
        assert_eq!(emit_block(&[]), "{\n}\n");
    }

    #[test]
    fn if_writes_both_branches_even_when_one_is_empty() {
        let stmt = Stmt::if_then_else(var("a"), vec![binding("a", int(1))], vec![]);
        assert_eq!(emit_stmt(&stmt), "if (a){\n\ta = 132\n}\nelse{\n}\n");
    }

    #[test]
    fn loop_with_and_without_a_condition() {
        let guarded = Stmt::Loop(LoopStmt {
            cond: Some(var("a")),
            body: vec![Stmt::Break],
        });
        assert_eq!(emit_stmt(&guarded), "while(a){\n\tbreak\n}\n");

        let infinite = Stmt::Loop(LoopStmt {
            cond: None,
            body: vec![Stmt::Continue],
        });
        assert_eq!(emit_stmt(&infinite), "while(true){\n\tcontinue\n}\n");
    }

    #[test]
    fn return_with_and_without_a_value() {
        assert_eq!(emit_stmt(&Stmt::ret(int(0))), "return 032\n");
        assert_eq!(emit_stmt(&Stmt::ret_void()), "return \n");
    }

    #[test]
    fn newline_terminated_statements_double_up_inside_blocks() {
        // `emit_return` writes its own newline and `emit_block` adds one per
        // statement, so a `Ret` inside a block leaves a blank line.
        assert_eq!(emit_block(&[Stmt::ret(int(1))]), "{\n\treturn 132\n\n}\n");
    }

    #[test]
    fn break_continue_goto() {
        assert_eq!(emit_stmt(&Stmt::Break), "break");
        assert_eq!(emit_stmt(&Stmt::Continue), "continue");
        assert_eq!(
            emit_stmt(&Stmt::Goto(GotoStmt {
                target: String::from("end"),
            })),
            "goto end"
        );
    }

    #[test]
    fn assignment() {
        assert_eq!(emit_stmt(&binding("x", var("y"))), "x = y");
    }

    #[test]
    fn branch_writes_nothing() {
        let stmt = Stmt::Branch(BranchStmt {
            cond: Some(Expr::bool(true)),
            then_block: String::from("tru"),
            else_block: Some(String::from("fal")),
        });
        assert_eq!(emit_stmt(&stmt), "");
    }

    #[test]
    fn catch_all_clause() {
        let stmt = Stmt::TryCatch(TryCatchStmt {
            try_body: vec![Stmt::Continue],
            catches: vec![CatchClause {
                param: None,
                exception_ty: None,
                body: vec![Stmt::Break],
            }],
            finally_body: None,
        });
        assert_eq!(emit_stmt(&stmt), "try {continue}catch(...){break}");
    }

    #[test]
    fn nested_block_golden() {
        // Indentation is not depth-aware: every block writes one tab per
        // statement, so nested blocks stay flat.
        let stmts = [
            binding("flag", Expr::bool(true)),
            Stmt::Loop(LoopStmt {
                cond: Some(Expr::binary(
                    InstructionOpcode::And,
                    var("flag"),
                    Expr::bool(false),
                )),
                body: vec![Stmt::if_then_else(
                    var("flag"),
                    vec![Stmt::Break],
                    vec![Stmt::Continue],
                )],
            }),
        ];
        assert_eq!(
            emit_block(&stmts),
            "{\n\tflag = true\n\twhile(flag & false){\n\tif (flag){\n\tbreak\n}\nelse{\n\tcontinue\n}\n\n}\n\n}\n"
        );
    }
}
