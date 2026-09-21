//! First iteration: C file in, unoptimized LLVM IR out.
//!
//!     cargo run --example c2ir -- tests/golden/alpha_beta.c
//!     cargo run --example c2ir -- tests/golden/alpha_beta.c --no-debug
//!     LLTP_CLANG=clang-22 cargo run --example c2ir -- tests/golden/alpha_beta.c
//!
//! IR goes to stdout, status/diagnostics to stderr, so `> out.ll` works.

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use lltp::frontend::{FrontendConfig, clang_version, compile_c_to_ir};

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
            // write_all rather than print!: don't panic on a closed pipe (`| head`).
            let _ = std::io::stdout().write_all(compiled.ir.as_bytes());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}