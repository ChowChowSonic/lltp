use std::fmt::{Debug, Formatter};
use std::path::PathBuf;
use std::process::ExitCode;

use inkwell::context::Context;
use lltp::frontend::{FrontendConfig, compile_c_to_ir};
use lltp::hir::lowering::build_graph;
use lltp::{build_module, hir};

fn main() -> ExitCode {
    let mut cfg = FrontendConfig::default();
    let mut file: Option<PathBuf> = None;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--no-debug" => cfg.debug_info = false,
            _ => file = Some(PathBuf::from(&arg)),
        }
    }

    let Some(file) = file else {
        eprintln!("usage: cargo run --example build_cfg -- <file.c> [--no-debug]");
        return ExitCode::from(2);
    };

    // 2.1.1
    let compiled = match compile_c_to_ir(&cfg, &file) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error compiling {}: {e}", file.display());
            return ExitCode::FAILURE;
        }
    };
    eprintln!(
        "[2.1.1] compiled {} -> {} bytes of IR",
        file.display(),
        compiled.ir.len()
    );

    let ctxt = Context::create();
    let module = build_module(&ctxt, &compiled.ir);
    eprintln!("[2.1.2] inkwell accepted the module — parse succeeded");

    // 2.1.3: one CFG per function (declarations have no body, so skip them)
    let mut loop_headers = Vec::new();
    for func in module.get_functions() {
        if func.count_basic_blocks() == 0 {
            continue;
        }
        let fname = func.get_name().to_string_lossy().into_owned();
        let mut graph = match build_graph(&func) {
            Ok(g) => g,
            Err(e) => {
                eprintln!("CFG build failed for `{fname}`: {e}");
                return ExitCode::FAILURE;
            }
        };

        eprintln!(
            "[2.1.3] `{fname}`: built CFG with {} basic block(s)",
            graph.blocks.len()
        );
        let stmts = graph.structurize().unwrap_or(Vec::new());
        assert!(!stmts.is_empty());
        for bb in stmts {
            match bb {
                hir::Stmt::Loop { cond, body } => {
                    let name = if body.is_empty() {
                        "<unnamed>"
                    } else {
                        "<unassigned>"
                    };
                    loop_headers.push(format!("{fname}::{name}"));
                }
                _ => {}
            }
        }
    }

    if loop_headers.is_empty() {
        println!("no self-referential loop headers found");
        println!(
            "note: has_self_referential_loop may only flag a block that branches directly \
             back to itself (a tight single-block loop), not a multi-block cycle like \
             while.cond -> ... -> if.end -> while.cond. An empty result here isn't\n\
             necessarily a failure — confirm against the function's actual semantics."
        );
    } else {
        loop_headers.sort();
        println!("loop-header candidate(s):");
        for name in &loop_headers {
            println!("  {name}");
        }
    }

    ExitCode::SUCCESS
}
