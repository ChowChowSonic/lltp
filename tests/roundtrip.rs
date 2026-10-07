//! Round-trip oracle: C (original) -> LLVM IR -> flat HIR -> C (rebuilt).
//! Both programs are compiled and run; exit code and stdout must match.
//!
//! Saved as `target/tmp/roundtrip/'<name>_rebuilt.c'

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use inkwell::context::Context;
use lltp::backend::c_goto;
use lltp::build_module;
use lltp::frontend::{FrontendConfig, compile_c_to_ir};
use lltp::hir::flat::FlatModule;

/// Fixtures the baseline is expected to round-trip.
const SUPPORTED: &[&str] = &[
    "alpha_beta",
    "loop_branch",
    "self_loop",
    "helper_func",
    "counting_func",
    "char_basic",
    "unsigned_ops",
    "bool_basic",
    "casts",
    "float_basic",
];

/// Fixtures that need GEP / globals / aggregates: must fail *cleanly* (an
/// `Err`, not a panic) until those are implemented. If one starts working,
/// move it to SUPPORTED.
const PENDING: &[&str] = &["string_basic", "string_loop", "struct_array"];

fn out_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("roundtrip");
    fs::create_dir_all(&dir).expect("create output dir");
    dir
}

fn fixture(name: &str) -> PathBuf {
    Path::new("tests/fixtures").join(format!("{name}.c"))
}

/// C file -> rebuilt C source text.
fn rebuild(c_file: &Path) -> Result<String, String> {
    let cfg = FrontendConfig::default();
    let compiled = compile_c_to_ir(&cfg, c_file).map_err(|e| e.to_string())?;
    let ctxt = Context::create();
    let module = build_module(&ctxt, &compiled.ir);
    let flat = FlatModule::from_module(&module).map_err(|e| e.to_string())?;
    c_goto::emit_module(&flat)
}

/// Compile a C file to an executable, run it, return (exit code, stdout).
fn build_and_run(c_file: &Path, exe: &Path) -> Result<(Option<i32>, String), String> {
    let clang = FrontendConfig::default().clang;
    let cc = Command::new(&clang)
        .args(["-O0", "-w", "-fwrapv"])
        .arg(c_file)
        .arg("-o")
        .arg(exe)
        .output()
        .map_err(|e| format!("could not run `{clang}`: {e}"))?;
    if !cc.status.success() {
        return Err(format!(
            "clang failed on {}:\n{}",
            c_file.display(),
            String::from_utf8_lossy(&cc.stderr)
        ));
    }
    let run = Command::new(exe)
        .output()
        .map_err(|e| format!("could not run {}: {e}", exe.display()))?;
    Ok((
        run.status.code(),
        String::from_utf8_lossy(&run.stdout).into_owned(),
    ))
}

fn check(name: &str) -> Result<(), String> {
    let dir = out_dir();
    let exe_suffix = std::env::consts::EXE_SUFFIX;

    let rebuilt_c = dir.join(format!("{name}_rebuilt.c"));
    fs::write(&rebuilt_c, rebuild(&fixture(name))?).map_err(|e| e.to_string())?;

    let orig = build_and_run(
        &fixture(name),
        &dir.join(format!("{name}_orig{exe_suffix}")),
    )?;
    let new = build_and_run(&rebuilt_c, &dir.join(format!("{name}_new{exe_suffix}")))?;

    if orig == new {
        Ok(())
    } else {
        Err(format!(
            "behavior differs\n  original: {orig:?}\n  rebuilt:  {new:?}\n  see {}",
            rebuilt_c.display()
        ))
    }
}

#[test]
fn supported_fixtures_round_trip() {
    let failures: Vec<String> = SUPPORTED
        .iter()
        .filter_map(|n| check(n).err().map(|e| format!("[{n}] {e}")))
        .collect();
    assert!(failures.is_empty(), "\n{}", failures.join("\n\n"));
}

#[test]
fn pending_fixtures_fail_cleanly() {
    for name in PENDING {
        assert!(
            rebuild(&fixture(name)).is_err(),
            "{name} now round-trips; move it to SUPPORTED"
        );
    }
}
