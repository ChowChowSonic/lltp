use inkwell::context::Context;
use lltp::frontend::{FrontendConfig, compile_c_to_ir};
use lltp::{build_module, hir::lowering::build_graph};
use std::fs;

#[test]
fn all_fixtures_ingest_and_build_cfgs() {
    let cfg = FrontendConfig::default();
    let mut count = 0;

    for entry in fs::read_dir("tests/fixtures").unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "c") {
            let compiled = compile_c_to_ir(&cfg, &path)
                .unwrap_or_else(|e| panic!("clang failed on {path:?}: {e}"));
            let ctxt = Context::create();
            let module = build_module(&ctxt, &compiled.ir);

            // build_graph is per function; declarations have no body.
            for func in module.get_functions() {
                if func.count_basic_blocks() == 0 {
                    continue;
                }
                build_graph(&func).unwrap_or_else(|e| {
                    panic!(
                        "CFG build failed for {path:?} in `{}`: {e}",
                        func.get_name().to_string_lossy()
                    )
                });
            }

            count += 1;
        }
    }

    assert!(count > 0, "no fixtures found in tests/fixtures");
    println!("ingested {count} fixtures");
}
