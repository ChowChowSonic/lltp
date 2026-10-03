use std::{fs, path::Path, process::Command};
use inkwell::context::Context;
use lltp::{build_module, hir::graph::build_graph};

fn compile_to_ir(c_file: &Path) -> String {
    let out = Command::new("clang")
        .args(["-O0", "-g", "-S", "-emit-llvm", "-o", "-"])
        .arg(c_file)
        .output()
        .expect("clang not found");
    assert!(out.status.success(), "clang failed on {c_file:?}");
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn all_fixtures_ingest_and_build_cfgs() {
    let mut count = 0;

    for entry in fs::read_dir("tests/fixtures").unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "c") {
            let ir = compile_to_ir(&path);
            let ctxt = Context::create();
            let module = build_module(&ctxt, &ir);
            build_graph(&module)
                .unwrap_or_else(|e| panic!("CFG build failed for {path:?}: {e:?}"));

            count += 1;
        }
    }

    assert!(count > 0, "no fixtures found in tests/fixtures");
    println!("ingested {count} fixtures");
}