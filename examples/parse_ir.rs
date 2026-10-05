use std::path::PathBuf;
use std::process::ExitCode;

use inkwell::context::Context;
use lltp::build_module;
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
        eprintln!("usage: cargo run --example parse_ir -- <file.c> [--no-debug]");
        return ExitCode::from(2);
    };

    // Stage 1: C -> LLVM IR .
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

    // Stage 2: parse that IR with inkwell .
    let ctxt = Context::create();
    let module = build_module(&ctxt, &compiled.ir);
    eprintln!("[2.1.2] inkwell accepted the module — parse succeeded");

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
