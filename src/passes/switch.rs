use inkwell::values::InstructionOpcode::ICmp;
use tracing::info;

use crate::backend::Language;
use crate::hir::{Function, Lit, expr::Expr, stmt::Stmt};
use crate::passes::{LowerError, Lowering};

/// Library-owned structural pass: lowers `Stmt::Switch` into a nested
/// `Stmt::If` chain when the target cannot express `switch`
/// (`Language::allows_switch() == false`). No-op otherwise.
pub struct SwitchToIfElse;

impl Lowering for SwitchToIfElse {
    fn name(&self) -> &'static str {
        "switch_to_if_else"
    }

    fn apply(&self, func: &mut Function, target: &dyn Language) -> Result<(), LowerError> {
        if target.allows_switch() {
            return Ok(());
        }
        info!(func = %func.name, "switch -> if/else lowering");
        func.body = lower_stmts(std::mem::take(&mut func.body));
        Ok(())
    }
}

fn lower_stmts(stmts: Vec<Stmt>) -> Vec<Stmt> {
    stmts.into_iter().flat_map(lower_stmt).collect()
}

fn lower_stmt(stmt: Stmt) -> Vec<Stmt> {
    match stmt {
        Stmt::Switch {
            value,
            cases,
            default,
        } if !cases.is_empty() => {
            let mut else_stmts = lower_stmts(default);
            for (case, body) in cases.into_iter().rev() {
                else_stmts = vec![Stmt::If {
                    cond: eq(value.clone(), case),
                    then_stmts: lower_stmts(body),
                    else_stmts,
                }];
            }
            else_stmts
        }
        Stmt::Switch { default, .. } => lower_stmts(default),
        Stmt::If {
            cond,
            then_stmts,
            else_stmts,
        } => vec![Stmt::If {
            cond,
            then_stmts: lower_stmts(then_stmts),
            else_stmts: lower_stmts(else_stmts),
        }],
        other => vec![other],
    }
}

fn eq(value: Expr, case: Lit) -> Expr {
    Expr::BinaryOp {
        op: ICmp,
        arg1: Box::new(value),
        arg2: Box::new(Expr::Literal(case)),
    }
}

#[cfg(test)]
mod tests {
    use crate::backend::{ExprEmitter, Language, StmtEmitter};
    use crate::hir::{Function, Lit, expr::Expr, stmt::Stmt, ty::Ty};
    use crate::passes::{Lowering, SwitchToIfElse};

    struct TestLang {
        switch: bool,
    }

    impl ExprEmitter for TestLang {}
    impl StmtEmitter for TestLang {}

    impl Language for TestLang {
        fn name(&self) -> &str {
            "test"
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
        fn allows_switch(&self) -> bool {
            self.switch
        }
    }

    fn i64(v: u64) -> Lit {
        Lit::Int {
            value: v,
            bits: 64,
            signed: true,
        }
    }

    fn switch_func() -> Function {
        Function {
            name: "f".to_string(),
            params: vec![Ty::Int(64, true)],
            return_ty: Ty::Int(64, true),
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

    #[test]
    fn lowering_rewrites_switch_to_nested_if() {
        let mut func = switch_func();
        let target = TestLang { switch: false };

        SwitchToIfElse
            .apply(&mut func, &target)
            .expect("lowering should succeed");

        assert_eq!(func.body.len(), 1);
        let Stmt::If {
            then_stmts,
            else_stmts,
            ..
        } = &func.body[0]
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
