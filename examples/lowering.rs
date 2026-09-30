//! Quickstart: define a target `Language`, run its lowering pipeline over a
//! recovered `Function`, and emit actual source through the `emit_*` surface.
//! Demonstrates the extension seam — a backend is its capability flags, a pass
//! list, and the emitters for the nodes it can express.
//!
//! Run with: `cargo run --example lowering`

use std::fmt::{self, Formatter};

use inkwell::values::InstructionOpcode;

use lltp::backend::{ExprEmitter, Language, StmtEmitter};
use lltp::hir::{Cfg, Expr, Function, Lit, Stmt, Ty};
use lltp::passes::SwitchToIfElse;

/// A toy C-like backend that can print the small HIR subset this example
/// builds (variables, int literals, `==`, return/if/switch). Everything else
/// falls through to the default emitters, which `unreachable!()`.
struct ToyBackend;

impl ExprEmitter for ToyBackend {
    fn emit_expr(&self, expr: &Expr, out: &mut Formatter) -> fmt::Result {
        match expr {
            Expr::Var { name, .. } => write!(out, "{name}"),
            Expr::Literal(_) => self.emit_literal(expr, out),
            Expr::BinaryOp { .. } => self.emit_binary_op(expr, out),
            other => unreachable!("toy emitter does not support {other:?}"),
        }
    }

    fn emit_literal(&self, expr: &Expr, out: &mut Formatter) -> fmt::Result {
        match expr {
            Expr::Literal(Lit::Int { value, .. }) => write!(out, "{value}"),
            Expr::Literal(Lit::Bool(b)) => write!(out, "{b}"),
            other => unreachable!("toy emitter does not support {other:?}"),
        }
    }

    fn emit_binary_op(&self, expr: &Expr, out: &mut Formatter) -> fmt::Result {
        let Expr::BinaryOp { op, arg1, arg2 } = expr else {
            unreachable!("expected a binary op");
        };
        self.emit_expr(arg1, out)?;
        match op {
            InstructionOpcode::ICmp => write!(out, " == ")?,
            other => unreachable!("toy emitter does not support `{other:?}`"),
        }
        self.emit_expr(arg2, out)
    }
}

impl StmtEmitter for ToyBackend {
    fn emit_stmt(&self, stmt: &Stmt, out: &mut Formatter) -> fmt::Result {
        match stmt {
            Stmt::Ret { .. } => self.emit_return(stmt, out),
            Stmt::If { .. } => self.emit_if(stmt, out),
            Stmt::Switch { .. } => self.emit_switch(stmt, out),
            Stmt::Let { .. } => unreachable!("Assignment statements not supported!"),
            Stmt::Branch { .. } => unreachable!("raw IR branches are not emitted"),
            Stmt::Loop { .. } => unreachable!("Loops are not emitted"),
            Stmt::Break => unreachable!("Breaks are not emitted"),
            Stmt::Continue => unreachable!("Continues are not emitted"),
            Stmt::Label { .. } => unreachable!("Labels are not emitted"),
            Stmt::Goto { .. } => unreachable!("Gotos are not emitted"),
        }
    }

    fn emit_block(&self, stmts: &[Stmt], out: &mut Formatter) -> fmt::Result {
        writeln!(out, "{{")?;
        emit_stmt_lines(self, stmts, out)?;
        write!(out, "}}")
    }

    fn emit_return(&self, stmt: &Stmt, out: &mut Formatter) -> fmt::Result {
        let Stmt::Ret { value } = stmt else {
            unreachable!("expected a return");
        };
        write!(out, "return")?;
        if let Some(value) = value {
            write!(out, " ")?;
            self.emit_expr(value, out)?;
        }
        write!(out, ";")
    }

    fn emit_if(&self, stmt: &Stmt, out: &mut Formatter) -> fmt::Result {
        let Stmt::If {
            cond,
            then_stmts,
            else_stmts,
        } = stmt
        else {
            unreachable!("expected an if");
        };
        write!(out, "if (")?;
        self.emit_expr(cond, out)?;
        writeln!(out, ") {{")?;
        emit_stmt_lines(self, then_stmts, out)?;
        write!(out, "}}")?;
        match else_stmts.as_slice() {
            [nested @ Stmt::If { .. }] => {
                write!(out, " else ")?;
                self.emit_if(nested, out)?;
            }
            [] => {}
            _ => {
                writeln!(out, " else {{")?;
                emit_stmt_lines(self, else_stmts, out)?;
                write!(out, "}}")?;
            }
        }
        Ok(())
    }

    fn emit_switch(&self, stmt: &Stmt, out: &mut Formatter) -> fmt::Result {
        let Stmt::Switch {
            value,
            cases,
            default,
        } = stmt
        else {
            unreachable!("expected a switch");
        };
        write!(out, "switch (")?;
        self.emit_expr(value, out)?;
        writeln!(out, ") {{")?;
        for (case, body) in cases {
            write!(out, "case ")?;
            self.emit_literal(&Expr::Literal(case.clone()), out)?;
            writeln!(out, ":")?;
            emit_stmt_lines(self, body, out)?;
        }
        if !default.is_empty() {
            writeln!(out, "default:")?;
            emit_stmt_lines(self, default, out)?;
        }
        write!(out, "}}")
    }
}

/// One statement per line; newline handling lives here and only here.
fn emit_stmt_lines(backend: &ToyBackend, stmts: &[Stmt], out: &mut Formatter) -> fmt::Result {
    for stmt in stmts {
        backend.emit_stmt(stmt, out)?;
        writeln!(out)?;
    }
    Ok(())
}

impl Language for ToyBackend {
    fn name(&self) -> &str {
        "toy-c"
    }

    /// Entry point of the emit chain: function header, then its body block.
    fn emit_function(&self, func: &Function, out: &mut Formatter) -> fmt::Result {
        write!(out, "int {}(", func.name)?;
        for (i, _) in func.params.iter().enumerate() {
            if i > 0 {
                write!(out, ", ")?;
            }
            write!(out, "int")?;
        }
        write!(out, ") ")?;
        self.emit_block(&func.body, out)
    }

    // Structural lowering is library-owned and flags-driven: this target has no
    // `switch`, so SwitchToIfElse rewrites it before emission.
    fn pipeline(&self) -> Vec<Box<dyn lltp::passes::Lowering>> {
        vec![Box::new(SwitchToIfElse)]
    }

    fn allows_match(&self) -> bool {
        false
    }
    fn allows_closures(&self) -> bool {
        false
    }
    fn allows_generics(&self) -> bool {
        false
    }
    fn allows_async(&self) -> bool {
        false
    }
    fn allows_threads(&self) -> bool {
        false
    }
    fn allows_locks(&self) -> bool {
        false
    }
    fn allows_raw_pointers(&self) -> bool {
        false
    }
    fn allows_exceptions(&self) -> bool {
        false
    }
    fn allows_multiple_return(&self) -> bool {
        false
    }
    fn allows_inheritance(&self) -> bool {
        false
    }
    fn allows_default_args(&self) -> bool {
        false
    }
    fn allows_union(&self) -> bool {
        false
    }
    fn allows_macros(&self) -> bool {
        false
    }
    fn allows_goto(&self) -> bool {
        false
    }
    // No `switch` in the target: the structural lowering pass rewrites it.
    fn allows_switch(&self) -> bool {
        false
    }
}

fn i64(v: u64) -> Lit {
    Lit::Int {
        value: v,
        bits: 64,
        signed: true,
    }
}

/// A `Function` built by hand — until the recovery passes (de-SSA,
/// structuring) land in M1, this is the direct route to lowering/emission.
fn recovered_switch() -> Function {
    Function {
        name: "classify".to_string(),
        params: vec![Ty::Int(64, true)],
        return_ty: Ty::Int(64, true),
        cfg: Cfg::default(),
        body: vec![Stmt::Switch {
            value: Expr::Var {
                name: "x".to_string(),
                dtype: Ty::Int(64, true),
            },
            cases: vec![
                (
                    i64(1),
                    vec![Stmt::Ret {
                        value: Some(Expr::Literal(i64(10))),
                    }],
                ),
                (
                    i64(2),
                    vec![Stmt::Ret {
                        value: Some(Expr::Literal(i64(20))),
                    }],
                ),
            ],
            default: vec![Stmt::Ret {
                value: Some(Expr::Literal(i64(30))),
            }],
        }],
    }
}

/// Render a function to a string through the backend's `emit_*` chain. The
/// `Display` impl hands its `Formatter` straight to `emit_function`, so the
/// demo exercises the exact emission interface a pipeline driver would use.
struct Rendered<'a>(&'a ToyBackend, &'a Function);

impl fmt::Display for Rendered<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.emit_function(self.1, f)
    }
}

fn render(backend: &ToyBackend, func: &Function) -> String {
    format!("{}", Rendered(backend, func))
}

fn main() {
    tracing_subscriber::fmt().init();

    let backend = ToyBackend;

    let mut func = recovered_switch();
    println!(
        "== recovered HIR, emitted as-is ==\n{}",
        render(&backend, &func)
    );

    // `lower` runs the backend's pipeline (the B-core) via its `Language`
    // default (the A-veneer): SwitchToIfElse replaces the switch with a nested
    // if/else chain because `allows_switch()` is false. The same emitters then
    // produce structured source.
    backend.lower(&mut func).expect("lowering should succeed");

    println!("== after lowering ==\n{}", render(&backend, &func));
}
