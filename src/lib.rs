use std::path::Path;

use inkwell::{context::Context, module::Module as IrModule};
use tracing::info;

pub mod backend;
pub mod errors;
pub mod frontend;
pub mod hir;
pub mod ingest;
pub mod passes;

use crate::errors::{IngestError, PipelineError};
use crate::hir::verify;
use crate::{
    backend::Language,
    hir::{Function, Stmt},
};
pub use ingest::{IrFormat, load_module};

/// Possible deprecation target. Parse textual IR into an inkwell module. Ingest is the only parse path, so
/// a failure reports the same `IngestError` shape as a file load.
pub fn build_module<'ctx>(ctxt: &'ctx Context, ir: &str) -> Result<IrModule<'ctx>, IngestError> {
    ingest::parse_text(ctxt, ir, "<memory>")
}

/// Run the pipeline over IR text.
pub fn pipeline(lang: &impl Language, ir: &str) -> Result<String, PipelineError> {
    let ctxt = Context::create();
    let module = build_module(&ctxt, ir).map_err(PipelineError::IngestError)?;
    pipeline_module(lang, &module)
}

/// Run the pipeline over a `.ll` or `.bc` file, taking the format from the file's
/// contents rather than its extension.
pub fn pipeline_file(lang: &impl Language, path: &Path) -> Result<String, PipelineError> {
    let ctxt = Context::create();
    let module = load_module(&ctxt, path).map_err(PipelineError::IngestError)?;
    pipeline_module(lang, &module)
}

/// Run the pipeline over an already-parsed module.
pub fn pipeline_module(lang: &impl Language, module: &IrModule) -> Result<String, PipelineError> {
    let mut recovered =
        hir::lowering::collect_functions(module).map_err(|error| PipelineError::Hir {
            origin: module.get_name().to_string_lossy().into_owned(),
            error,
        })?;

    // The tree is what the passes and the capability check read, so structurize has
    // to run before either: recovery fills `cfg` and leaves `body` empty.
    let mut code: String = String::new();
    for func in &mut recovered.funcs {
        if let Err(e) = func.structurize() {
            return Err(PipelineError::Hir {
                origin: func.sig.name.to_string(),
                error: e,
            });
        }
        check_equivalence(func)?;
        if let Err(e) = lang.lower(func) {
            return Err(PipelineError::Hir {
                origin: func.sig.name.to_string(),
                error: e,
            });
        }
        check_capabilities(lang, func)?;
        info!(
            "Successfully verified that no invalid instructions are in {func}",
            func = func.sig.name
        );
        if let Err(e) = lang.emit_function(func, &mut code) {
            return Err(PipelineError::FunctionEmissionError(
                func.sig.name.to_string(),
                e.to_string(),
            ));
        }
        info!("Emitted function {}", func.sig.name);
    }
    Ok(code)
}

/// The tree has to describe the same control flow as the graph it was structured
/// from, which is the contract `hir::func` documents: `body` is never
/// hand-written, always recomputable. `are_equivalent` cannot confirm a pass that
/// drops nodes, so this runs before the passes rather than after them.
#[cfg(debug_assertions)]
fn check_equivalence(func: &Function) -> Result<(), PipelineError> {
    if !verify::are_equivalent(&func.cfg, &verify::structured_to_cfg(&func.body)) {
        return Err(PipelineError::NotEquivalent {
            func: func.sig.name.to_string(),
        });
    }
    Ok(())
}

#[cfg(not(debug_assertions))]
fn check_equivalence(_func: &Function) -> Result<(), PipelineError> {
    Ok(())
}

/// Which nodes this backend can express. A node the target cannot print has to be
/// rewritten away before emission reaches it.
///
/// The return count is checked once, on the whole tree: `check_stmts` sums the
/// arms it walks, so a rule applied at every depth would report the same
/// violation from whichever arm happened to reach it first.
fn check_capabilities(lang: &impl Language, func: &Function) -> Result<(), PipelineError> {
    let returns = check_stmts(lang, &func.sig.name, &func.body)?;
    if !lang.allows_multiple_return() && returns > 1 {
        return Err(PipelineError::DisallowedStmtError(
            lang.name().to_string(),
            "Ret".to_string(),
        ));
    }
    Ok(())
}

fn check_stmts(lang: &impl Language, func: &str, stmts: &[Stmt]) -> Result<usize, PipelineError> {
    let mut returns = 0;
    for stmt in stmts {
        match stmt {
            Stmt::Ret(_) => {
                returns += 1;
            }
            Stmt::Switch(node) => {
                if !lang.allows_switch() {
                    return Err(PipelineError::DisallowedStmtError(
                        lang.name().to_string(),
                        "Switch".to_string(),
                    ));
                }
                for (_, body) in &node.cases {
                    returns += check_stmts(lang, func, body)?;
                }
                returns += check_stmts(lang, func, &node.default)?;
            }
            Stmt::Branch(node) => {
                return Err(PipelineError::Unstructured {
                    func: func.to_string(),
                    block: node.then_block.clone(),
                });
            }
            Stmt::If(node) => {
                returns += check_stmts(lang, func, &node.then_stmts)?;
                returns += check_stmts(lang, func, &node.else_stmts)?;
            }
            Stmt::Loop(node) => {
                returns += check_stmts(lang, func, &node.body)?;
            }
            Stmt::Goto(_) | Stmt::Label(_) if !lang.allows_goto() => {
                // Labels are only expressible where `goto` is, since they exist
                // solely as `goto` targets.
                return Err(PipelineError::DisallowedStmtError(
                    lang.name().to_string(),
                    "Goto".to_string(),
                ));
            }
            Stmt::TryCatch(_) if !lang.allows_exceptions() => {
                return Err(PipelineError::DisallowedStmtError(
                    lang.name().to_string(),
                    "TryCatch".to_string(),
                ));
            }
            Stmt::TryCatch(stmt) => {
                returns += check_stmts(lang, func, &stmt.try_body)?;
                if let Some(b) = &stmt.finally_body {
                    returns += check_stmts(lang, func, b)?;
                }
                for x in &stmt.catches {
                    returns += check_stmts(lang, func, &x.body)?;
                }
            }
            Stmt::Invoke(_) if !lang.allows_unwind_fork() => {
                // Without unwind/fork the contract is error-code propagation or
                // try/catch, never an `invoke` arm.
                return Err(PipelineError::DisallowedStmtError(
                    lang.name().to_string(),
                    "Invoke".to_string(),
                ));
            }
            // The remaining nodes are core structured control flow with no
            // capability flag, so every backend may see them.
            Stmt::Goto(_)
            | Stmt::Label(_)
            | Stmt::Let(_)
            | Stmt::Eval(_)
            | Stmt::Unreachable
            | Stmt::Break
            | Stmt::Invoke(_)
            | Stmt::Continue => (),
        }
    }
    Ok(returns)
}

#[cfg(test)]
mod tests {
    use crate::backend::{ExprEmitter, Language, LowerTarget, StmtEmitter};
    use crate::errors::{HirError, IngestError, LowerError, PipelineError};
    use crate::hir::lowering::build_graph;
    use crate::hir::{Function, Module, Stmt, Ty};
    use crate::passes::Lowering;
    use crate::{
        IrFormat, build_module, check_capabilities, check_equivalence, hir::cfg::Cfg, pipeline,
        pipeline_file, pipeline_module,
    };
    use inkwell::context::Context;
    use std::collections::HashSet;
    use std::path::PathBuf;

    /// A backend that can only print a stub line, so `pipeline` reaches emission
    /// without needing the real emitters.
    struct EchoLang;
    impl LowerTarget for EchoLang {
        fn name(&self) -> &str {
            "echo"
        }
    }
    impl ExprEmitter for EchoLang {}
    impl StmtEmitter for EchoLang {}
    impl Language for EchoLang {
        fn emit_function(
            &self,
            func: &Function,
            out: &mut impl std::fmt::Write,
        ) -> std::fmt::Result {
            write!(out, "echo {}", func.sig.name)
        }
    }

    struct FailingPass;
    impl Lowering for FailingPass {
        fn name(&self) -> &'static str {
            "failing_pass"
        }
        fn apply(&self, _func: &mut Function, _target: &dyn LowerTarget) -> Result<(), LowerError> {
            Err(LowerError::Other("the pass failed".to_string()))
        }
    }

    struct PassLang;
    impl LowerTarget for PassLang {
        fn name(&self) -> &str {
            "pass"
        }
    }
    impl ExprEmitter for PassLang {}
    impl StmtEmitter for PassLang {}
    impl Language for PassLang {
        fn pipeline(&self) -> Vec<Box<dyn Lowering>> {
            vec![Box::new(FailingPass)]
        }
    }

    #[test]
    fn build_module_and_load_module_report_the_same_ingest_error() {
        let ctxt = Context::create();
        // `build_module` is a thin wrapper over `ingest::parse_text`, so the origin
        // it reports is the one it was handed.
        let err = build_module(&ctxt, "this is not llvm ir")
            .expect_err("garbage should be reported as a parse error");
        let IngestError::Parse { origin, format, .. } = err else {
            panic!("expected a parse error, got {err}");
        };
        assert_eq!(origin, "<memory>");
        assert_eq!(format, IrFormat::Text);

        // A missing file is still an ingest error, and `pipeline_file` carries it
        // unchanged rather than flattening it to a string.
        let missing = PathBuf::from("definitely-not-here.ll");
        let err = pipeline_file(&PassLang, &missing).expect_err("the missing file should error");
        assert!(matches!(err, PipelineError::IngestError(_)), "{err}");
    }

    #[test]
    #[cfg(debug_assertions)]
    fn the_equivalence_check_rejects_a_tree_that_is_not_the_graph() {
        // A fork the tree does not carry: `are_equivalent` compares node counts, so
        // the two arms have to be missing, not merely fused into their pred.
        let mut func = Function::new(
            "f",
            Vec::new(),
            Ty::Void,
            Cfg::cfg_from("entry", &[("entry", "a"), ("entry", "b")]),
        );
        func.body = vec![Stmt::ret_void()];
        let err = check_equivalence(&func).expect_err("the tree is not the graph");
        assert!(matches!(err, PipelineError::NotEquivalent { .. }), "{err}");

        // A straight-line chain is the same shape as its fused form, so the check
        // accepts the tree that carries only the exit.
        let mut line = Function::new("g", Vec::new(), Ty::Void, Cfg::cfg_from("e", &[("e", "x")]));
        line.body = vec![Stmt::ret_void()];
        assert!(
            matches!(check_equivalence(&line), Ok(())),
            "the chain should pass"
        );
    }

    #[test]
    fn a_pass_failure_keeps_its_type_and_its_pass_name() {
        let err = pipeline(&PassLang, IR).expect_err("the pass should fail");
        let PipelineError::Hir { origin, error } = err else {
            panic!("expected a typed HIR error, got {err}");
        };
        assert_eq!(origin, "f");
        let HirError::Pass { pass, .. } = error else {
            panic!("expected the pass to be named, got {error}");
        };
        assert_eq!(pass, "failing_pass");
    }

    #[test]
    fn pipeline_runs_over_a_module_that_ingest_handed_it() {
        let ctxt = Context::create();
        let module = build_module(&ctxt, SIMPLE_IR).expect("IR should parse");
        assert_eq!(
            pipeline_module(&EchoLang, &module).expect("echo backend should emit"),
            "echo g".to_string()
        );
    }

    #[test]
    fn a_single_return_is_not_a_multiple_return_violation() {
        let ctxt = Context::create();
        let single = build_module(&ctxt, SIMPLE_IR).expect("IR should parse");
        let two = build_module(&ctxt, IR).expect("IR should parse");
        let recovered = Module::from_module(&single).expect("recovery should succeed");
        // `EchoLang` takes the default `allows_multiple_return() == false`, so one
        // `Ret` has to pass where two do not.
        assert!(matches!(
            check_capabilities(&EchoLang, &recovered.funcs[0]),
            Ok(())
        ));
        let err = pipeline_module(&EchoLang, &two).expect_err("two returns should be refused");
        assert!(
            matches!(err, PipelineError::DisallowedStmtError(_, _)),
            "{err}"
        );
    }

    const IR: &str = r#"define void @f(i32 %x) {
entry:
  %c = icmp sgt i32 %x, 0
  br i1 %c, label %pos, label %nonpos
pos:
  ret void
nonpos:
  ret void
}
"#;

    const SIMPLE_IR: &str = r#"define void @g(i32 %x) {
entry:
  ret void
}
"#;

    #[test]
    fn cfgs_build_from_ingested_module() {
        let ctxt = Context::create();
        let module = build_module(&ctxt, IR).unwrap();
        let func = module.get_first_function().expect("IR contains a function");
        let cfg: Cfg = build_graph(&func, &HashSet::new()).expect("CFG build should succeed");

        assert_eq!(cfg.entry, "entry");
        assert_eq!(
            cfg.blocks["entry"].succ,
            vec!["pos".to_string(), "nonpos".to_string()]
        );

        assert_eq!(cfg.blocks["pos"].pred, vec!["entry".to_string()]);
        assert_eq!(cfg.blocks["nonpos"].pred, vec!["entry".to_string()]);
        assert!(cfg.blocks["pos"].succ.is_empty());
        assert!(cfg.blocks["nonpos"].succ.is_empty());
        assert_eq!(cfg.exits, vec!["pos".to_string(), "nonpos".to_string()]);
    }
}
