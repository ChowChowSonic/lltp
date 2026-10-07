use std::path::PathBuf;
use std::process::ExitCode;

use inkwell::context::Context;
use lltp::{load_module, parse_ir};
use lltp::frontend::{FrontendConfig, compile_c_to_ir};

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
        eprintln!("usage: cargo run --example parse_ir -- <file.c|file.ll|file.bc> [--no-debug]");
        return ExitCode::from(2);
    };

    let ctxt = Context::create();
    let is_ir = matches!(file.extension().and_then(|e| e.to_str()), Some("ll" | "bc"));

    let module = if is_ir {
        // 2.1.2: load IR (.ll text or .bc bitcode) directly, no clang needed.
        match load_module(&ctxt, &file) {
            Ok(m) => {
                eprintln!("[2.1.2] loaded {} directly", file.display());
                m
            }
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        // 2.1.1: C -> LLVM IR, then 2.1.2: parse that IR.
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
        match parse_ir(&ctxt, &compiled.ir) {
            Ok(m) => {
                eprintln!("[2.1.2] inkwell accepted the module — parse succeeded");
                m
            }
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::FAILURE;
            }
        }
    };

    let functions: Vec<_> = module.get_functions().collect();
    println!("module has {} function(s):", functions.len());
    for func in &functions {
        let name = func.get_name().to_string_lossy().into_owned();
        let blocks: Vec<_> = func.get_basic_blocks();
        println!("  fn {name}: {} basic block(s)", blocks.len());
        for bb in &blocks {
            let bb_name = bb.get_name().to_string_lossy().into_owned();
            let bb_name = if bb_name.is_empty() {
                "<entry>".to_string()
            } else {
                bb_name
            };
            let inst_count = bb.get_instructions().count();
            println!("    {bb_name}: {inst_count} instruction(s)");
        }
    }

    ExitCode::SUCCESS
}
