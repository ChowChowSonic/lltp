use tracing::info;

use crate::backend::LowerTarget;
use crate::errors::LowerError;
use crate::hir::op::{Cmp, IntCmp};
use crate::hir::stmt::{CatchClause, IfStmt, SwitchStmt, TryCatchStmt};
use crate::hir::{Expr, Function, Lit, Stmt};
use crate::passes::Lowering;

/// Library-owned structural pass: lowers `Stmt::Switch` into a nested
/// `Stmt::If` chain when the target cannot express `switch`
/// (`LowerTarget::allows_switch() == false`). No-op otherwise.
pub struct SwitchToIfElse;

impl Lowering for SwitchToIfElse {
    fn name(&self) -> &'static str {
        "switch_to_if_else"
    }

    fn apply(&self, func: &mut Function, target: &dyn LowerTarget) -> Result<(), LowerError> {
        if target.allows_switch() {
            return Ok(());
        }
        info!(func = %func.sig.name, "switch -> if/else lowering");
        func.body = lower_stmts(std::mem::take(&mut func.body));
        Ok(())
    }
}

fn lower_stmts(stmts: Vec<Stmt>) -> Vec<Stmt> {
    stmts.into_iter().flat_map(lower_stmt).collect()
}

fn lower_stmt(stmt: Stmt) -> Vec<Stmt> {
    match stmt {
        Stmt::Switch(SwitchStmt {
            value,
            cases,
            default,
        }) if !cases.is_empty() => {
            let mut else_stmts = lower_stmts(default);
            for (case, body) in cases.into_iter().rev() {
                else_stmts = vec![Stmt::If(IfStmt {
                    cond: eq(value.clone(), case),
                    then_stmts: lower_stmts(body),
                    else_stmts,
                })];
            }
            else_stmts
        }
        Stmt::TryCatch(TryCatchStmt {
            try_body,
            catches,
            finally_body,
        }) => vec![Stmt::TryCatch(TryCatchStmt {
            try_body: lower_stmts(try_body),
            catches: catches
                .into_iter()
                .map(|c| CatchClause {
                    param: c.param,
                    exception_ty: c.exception_ty,
                    body: lower_stmts(c.body),
                })
                .collect(),
            finally_body: finally_body.map(lower_stmts),
        })],
        Stmt::Switch(SwitchStmt { default, .. }) => lower_stmts(default),
        Stmt::If(IfStmt {
            cond,
            then_stmts,
            else_stmts,
        }) => vec![Stmt::If(IfStmt {
            cond,
            then_stmts: lower_stmts(then_stmts),
            else_stmts: lower_stmts(else_stmts),
        })],
        other => vec![other],
    }
}

fn eq(value: Expr, case: Lit) -> Expr {
    Expr::cmp(Cmp::Int(IntCmp::Eq), value, Expr::Literal(case))
}

#[cfg(test)]
mod tests {
    use crate::backend::{ExprEmitter, Language, LowerTarget, StmtEmitter};
    use crate::hir::expr::VarExpr;
    use crate::hir::stmt::{IfStmt, RetStmt, SwitchStmt};
    use crate::hir::{Cfg, Expr, FnSig, Function, Lit, Param, Stmt, Ty};
    use crate::passes::{Lowering, SwitchToIfElse};

    struct TestLang {
        switch: bool,
    }

    impl ExprEmitter for TestLang {}
    impl StmtEmitter for TestLang {}

    impl LowerTarget for TestLang {
        fn name(&self) -> &str {
            "test"
        }
        fn allows_switch(&self) -> bool {
            self.switch
        }
    }

    impl Language for TestLang {}

    fn i64(v: u64) -> Lit {
        Lit::Int {
            value: v,
            bits: 64,
            signed: true,
        }
    }

    fn switch_func() -> Function {
        Function {
            sig: FnSig {
                name: "f".to_string(),
                params: vec![Param {
                    name: "x".to_string(),
                    ty: Ty::Int(64, true),
                }],
                ret: Ty::Int(64, true),
                variadic: false,
            },
            locals: Vec::new(),
            cfg: Cfg::default(),
            body: vec![Stmt::Switch(SwitchStmt {
                value: Expr::Var(VarExpr {
                    name: "x".to_string(),
                    dtype: Ty::Int(64, true),
                }),
                cases: vec![
                    (
                        i64(1),
                        vec![Stmt::Ret(RetStmt {
                            value: Some(Expr::Literal(i64(10))),
                        })],
                    ),
                    (
                        i64(2),
                        vec![Stmt::Ret(RetStmt {
                            value: Some(Expr::Literal(i64(20))),
                        })],
                    ),
                ],
                default: vec![Stmt::Ret(RetStmt {
                    value: Some(Expr::Literal(i64(30))),
                })],
            })],
        }
    }

    #[test]
    fn lowering_rewrites_switch_to_nested_if() {
        let mut func = switch_func();
        let target = TestLang { switch: false };

        SwitchToIfElse
            .apply(&mut func, &target)
            .expect("lowering should succeed");

        assert_eq!(func.body.len(), 1);
        let Stmt::If(IfStmt {
            then_stmts,
            else_stmts,
            ..
        }) = &func.body[0]
        else {
            panic!("expected an If, got {:?}", func.body[0]);
        };
        assert_eq!(then_stmts.len(), 1);
        // else branch is a single nested If for case 2
        assert_eq!(else_stmts.len(), 1);
        assert!(matches!(else_stmts[0], Stmt::If { .. }));
    }

    #[test]
    fn noop_when_target_allows_switch() {
        let mut func = switch_func();
        let target = TestLang { switch: true };

        SwitchToIfElse
            .apply(&mut func, &target)
            .expect("lowering should succeed");

        assert_eq!(func.body.len(), 1);
        assert!(matches!(func.body[0], Stmt::Switch { .. }));
    }
}
