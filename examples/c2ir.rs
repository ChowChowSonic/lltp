use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use lltp::frontend::{FrontendConfig, clang_version, compile_c_to_ir};

fn brief(ir: &str) -> &str {
    match ir.find("\nattributes #") {
        Some(idx) => &ir[..idx],
        None => ir,
    }
}

fn main() -> ExitCode {
    let mut cfg = FrontendConfig::default();
    let mut file: Option<PathBuf> = None;
    let mut brief_output = false;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--brief" => brief_output = true,
            "--no-debug" => cfg.debug_info = false,
            _ => file = Some(PathBuf::from(&arg)),
        }
    }

    let Some(file) = file else {
        eprintln!("usage: cargo run --example c2ir -- <file.c> [--no-debug]");
        return ExitCode::from(2);
    };

    if let Some(v) = clang_version(&cfg.clang) {
        eprintln!("[lltp] {v}");
    }

    match compile_c_to_ir(&cfg, &file) {
        Ok(compiled) => {
            if !compiled.diagnostics.is_empty() {
                eprint!("{}", compiled.diagnostics);
            }
            let out = if brief_output { brief(&compiled.ir) } else { &compiled.ir };
            let _ = std::io::stdout().write_all(out.as_bytes());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}