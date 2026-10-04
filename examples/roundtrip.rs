//! C -> LLVM IR -> flat HIR -> goto-based C (the round-trip baseline).
//!
//! Run with:
//!   cargo run --example roundtrip -- tests/fixtures/alpha_beta.c
//!   cargo run --example roundtrip -- tests/fixtures/loop_branch.c -o out.c

use std::path::PathBuf;
use std::process::ExitCode;

use inkwell::context::Context;
use lltp::backend::c_goto;
use lltp::build_module;
use lltp::frontend::{FrontendConfig, compile_c_to_ir};
use lltp::hir::flat::FlatModule;

fn main() -> ExitCode {
    let mut file: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-o" => out = args.next().map(PathBuf::from),
            _ => file = Some(PathBuf::from(a)),
        }
    }
    let Some(file) = file else {
        eprintln!("usage: cargo run --example roundtrip -- <file.c> [-o out.c]");
        return ExitCode::from(2);
    };

    let cfg = FrontendConfig::default();
    let compiled = match compile_c_to_ir(&cfg, &file) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error compiling {}: {e}", file.display());
            return ExitCode::FAILURE;
        }
    };

    let ctxt = Context::create();
    let module = build_module(&ctxt, &compiled.ir);

    let flat = match FlatModule::from_module(&module) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("recovery failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    eprintln!(
        "recovered {} function(s), {} external declaration(s)",
        flat.funcs.len(),
        flat.decls.len()
    );

    let c = match c_goto::emit_module(&flat) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("emission failed: {e}");
            return ExitCode::FAILURE;
        }
    };

    match out {
        Some(path) => {
            if let Err(e) = std::fs::write(&path, &c) {
                eprintln!("could not write {}: {e}", path.display());
                return ExitCode::FAILURE;
            }
            eprintln!("wrote {}", path.display());
        }
        None => print!("{c}"),
    }
    ExitCode::SUCCESS
}
