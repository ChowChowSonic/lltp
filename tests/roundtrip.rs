//! Round-trip oracle: C (original) -> LLVM IR -> flat HIR -> C (rebuilt).
//! Both programs are compiled and run; exit code and stdout must match.
//!
//! Saved as `target/tmp/roundtrip/<name>_rebuilt.c`

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use inkwell::context::Context;
use lltp::backend::c_goto;
use lltp::build_module;
use lltp::frontend::{FrontendConfig, compile_c_to_ir};
use lltp::hir::Module;
use lltp::hir::verify::{are_equivalent, structured_to_cfg};

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
    "logic_short_circuit",
    "ternary_value",
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
    let module = build_module(&ctxt, &compiled.ir).unwrap();
    let flat = Module::from_module(&module).map_err(|e| e.to_string())?;
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

/// Textual IR -> rebuilt C source text.
/// The merged representation: recovery fills `cfg` (one block per basic block,
/// in LLVM order, with edges taken from each terminator) and `structurize`
/// writes the structured `body` over it. Every SUPPORTED fixture must structure
/// without panicking, and the tree must stay equivalent to the graph as
/// recovered; where an arm had to be duplicated the case is recorded
/// instead, so a change in that set shows up as a failure.
#[test]
fn recovered_fixtures_structure_back_to_their_cfgs() {
    let conf = FrontendConfig::default();
    let mut failures: Vec<String> = Vec::new();
    let mut duplicated: Vec<String> = Vec::new();
    let mut checked = 0;

    for name in SUPPORTED {
        let compiled = match compile_c_to_ir(&conf, &fixture(name)) {
            Ok(c) => c,
            Err(e) => {
                failures.push(format!("[{name}] {e}"));
                continue;
            }
        };
        let ctxt = Context::create();
        let module = build_module(&ctxt, &compiled.ir).expect("IR should parse");
        let recovered = match Module::from_module(&module) {
            Ok(m) => m,
            Err(e) => {
                failures.push(format!("[{name}] recovery: {e}"));
                continue;
            }
        };

        for mut func in recovered.funcs {
            assert!(!func.cfg.order.is_empty(), "[{name}] no recovery order");
            assert_eq!(
                func.cfg.entry, func.cfg.order[0],
                "[{name}] entry is not the first recovered block"
            );

            let graph = func.cfg.clone();
            let body = match func.structurize() {
                Ok(body) => body.to_vec(),
                Err(e) => {
                    failures.push(format!("[{name}] {}: {e}", func.sig.name));
                    continue;
                }
            };
            // Duplicating an arm or inserting a join changes the shape, so the
            // shape comparison only holds where structurize worked on the graph
            // as recovered; a tree that had to duplicate an arm is recorded.
            let structured = structured_to_cfg(&body);
            if structured.len() != graph.len() {
                duplicated.push(format!("[{name}] {}", func.sig.name));
            } else if !are_equivalent(&graph, &structured) {
                failures.push(format!(
                    "[{name}] {}: structured body is not equivalent to its CFG",
                    func.sig.name
                ));
            }
            checked += 1;
        }
    }

    assert!(checked > 0, "no fixture reached structurization");
    // The only fixture whose arms reconverge at different joins, so its fork
    // must be duplicated: `mixed`'s `&&` and `||`.
    assert_eq!(duplicated, ["[logic_short_circuit] mixed".to_string()]);
    assert!(failures.is_empty(), "\n{}", failures.join("\n\n"));
}

fn rebuild_ir(ir: &str) -> Result<String, String> {
    let ctxt = Context::create();
    let module = build_module(&ctxt, ir).map_err(|e| e.to_string())?;
    let flat = Module::from_module(&module).map_err(|e| e.to_string())?;
    c_goto::emit_module(&flat)
}

/// `-O0` never produces phis that read each other, so this is hand-written IR.
const PHI_SWAP_IR: &str = r#"define i32 @swap_loop() {
entry:
  br label %loop
loop:
  %a = phi i32 [ 1, %entry ], [ %b, %loop ]
  %b = phi i32 [ 2, %entry ], [ %a, %loop ]
  %i = phi i32 [ 0, %entry ], [ %i.next, %loop ]
  %i.next = add nsw i32 %i, 1
  %done = icmp eq i32 %i.next, 3
  br i1 %done, label %exit, label %loop
exit:
  ret i32 %a
}

define i32 @main() {
entry:
  %r = call i32 @swap_loop()
  ret i32 %r
}
"#;

#[test]
fn phi_swap_at_loop_header() {
    let dir = out_dir();
    let rebuilt_c = dir.join("phi_swap_rebuilt.c");
    fs::write(
        &rebuilt_c,
        rebuild_ir(PHI_SWAP_IR).expect("rebuild swap IR"),
    )
    .expect("write rebuilt C");
    let exe = dir.join(format!("phi_swap{}", std::env::consts::EXE_SUFFIX));
    let (code, _) = build_and_run(&rebuilt_c, &exe).expect("build and run rebuilt C");
    assert_eq!(code, Some(1), "see {}", rebuilt_c.display());
}

#[test]
fn pointer_phi_fails_cleanly() {
    let ir = r#"define ptr @pick(i1 %c, ptr %p, ptr %q) {
entry:
  br i1 %c, label %a, label %b
a:
  br label %m
b:
  br label %m
m:
  %r = phi ptr [ %p, %a ], [ %q, %b ]
  ret ptr %r
}
"#;
    let err = rebuild_ir(ir).expect_err("pointer phi must be rejected");
    assert!(err.contains("phi"), "unexpected error: {err}");
}

/// Three phis rotate `(a, b, c) = (b, c, a)` on the back edge. After 4
/// iterations the exit computes 16a + 4b + c = 27.
const PHI_ROTATE_IR: &str = r#"define i32 @rotate() {
entry:
  br label %loop
loop:
  %a = phi i32 [ 1, %entry ], [ %b, %loop ]
  %b = phi i32 [ 2, %entry ], [ %c, %loop ]
  %c = phi i32 [ 3, %entry ], [ %a, %loop ]
  %i = phi i32 [ 0, %entry ], [ %i.next, %loop ]
  %i.next = add nsw i32 %i, 1
  %done = icmp eq i32 %i.next, 4
  br i1 %done, label %exit, label %loop
exit:
  %m1 = mul nsw i32 %a, 16
  %m2 = mul nsw i32 %b, 4
  %s1 = add nsw i32 %m1, %m2
  %s2 = add nsw i32 %s1, %c
  ret i32 %s2
}

define i32 @main() {
entry:
  %r = call i32 @rotate()
  ret i32 %r
}
"#;

#[test]
fn phi_rotation_survives_coalescing() {
    let dir = out_dir();
    let rebuilt_c = dir.join("phi_rotate_rebuilt.c");
    fs::write(
        &rebuilt_c,
        rebuild_ir(PHI_ROTATE_IR).expect("rebuild rotate IR"),
    )
    .expect("write rebuilt C");
    let exe = dir.join(format!("phi_rotate{}", std::env::consts::EXE_SUFFIX));
    let (code, _) = build_and_run(&rebuilt_c, &exe).expect("build and run rebuilt C");
    assert_eq!(code, Some(27), "see {}", rebuilt_c.display());
}

/// clang spills each parameter into an `x.addr` slot. Recovering variables
/// should fold that slot back into `x` and keep the source-level name.
const PARAM_SLOT_IR: &str = r#"define i32 @bump(i32 %x) {
entry:
  %x.addr = alloca i32
  store i32 %x, ptr %x.addr
  %0 = load i32, ptr %x.addr
  %add = add nsw i32 %0, 1
  store i32 %add, ptr %x.addr
  %1 = load i32, ptr %x.addr
  ret i32 %1
}

define i32 @main() {
entry:
  %r = call i32 @bump(i32 41)
  ret i32 %r
}
"#;

#[test]
fn parameter_slot_folds_into_the_parameter_name() {
    let dir = out_dir();
    let rebuilt = rebuild_ir(PARAM_SLOT_IR).expect("rebuild param IR");
    assert!(
        !rebuilt.contains("x_addr"),
        "slot for `x` was not folded away:\n{rebuilt}"
    );
    let rebuilt_c = dir.join("param_slot_rebuilt.c");
    fs::write(&rebuilt_c, rebuilt).expect("write rebuilt C");
    let exe = dir.join(format!("param_slot{}", std::env::consts::EXE_SUFFIX));
    let (code, _) = build_and_run(&rebuilt_c, &exe).expect("build and run rebuilt C");
    assert_eq!(code, Some(42), "see {}", rebuilt_c.display());
}
