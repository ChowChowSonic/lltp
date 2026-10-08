use std::fs;
use std::path::{Path, PathBuf};

use inkwell::context::Context;
use inkwell::module::Module;
use lltp::frontend::{FrontendConfig, compile_c_to_ir};
use lltp::{IngestError, IrFormat, load_module};

fn out_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("ingest");
    fs::create_dir_all(&dir).expect("create tmp dir");
    dir
}

/// (function name, basic-block count) per function, in module order.
fn shape(module: &Module) -> Vec<(String, usize)> {
    module
        .get_functions()
        .map(|f| {
            let name = f.get_name().to_string_lossy().into_owned();
            (name, f.get_basic_blocks().len())
        })
        .collect()
}

/// Load `path`, which must fail; return the error.
fn expect_err(path: &Path) -> IngestError {
    let ctxt = Context::create();
    match load_module(&ctxt, path) {
        Ok(_) => panic!("expected an error loading {}", path.display()),
        Err(e) => e,
    }
}

fn write_ll(name: &str, tag: &str) -> PathBuf {
    let src = PathBuf::from("tests/fixtures").join(format!("{name}.c"));
    let compiled = compile_c_to_ir(&FrontendConfig::default(), &src).expect("compile fixture");
    let path = out_dir().join(format!("{name}_{tag}.ll"));
    fs::write(&path, compiled.ir).expect("write .ll");
    path
}

#[test]
fn loads_text_ll_from_disk() {
    let path = write_ll("loop_branch", "text");
    let ctxt = Context::create();
    let module = load_module(&ctxt, &path).expect("load .ll");
    let shapes = shape(&module);
    assert!(shapes.iter().any(|(n, blocks)| n == "main" && *blocks > 1));
}

#[test]
fn loads_bitcode_and_matches_text() {
    let ll = write_ll("helper_func", "bitcode");
    let ctxt = Context::create();
    let from_text = load_module(&ctxt, &ll).expect("load .ll");

    let bc = out_dir().join("helper_func.bc");
    assert!(from_text.write_bitcode_to_path(&bc), "write bitcode");
    let from_bc = load_module(&ctxt, &bc).expect("load .bc");

    assert_eq!(shape(&from_text), shape(&from_bc));
}

#[test]
fn format_comes_from_contents_not_extension() {
    let ll = write_ll("alpha_beta", "sniff");
    let ctxt = Context::create();
    let module = load_module(&ctxt, &ll).expect("load .ll");

    // Bitcode saved under a misleading `.ll` name still loads.
    let disguised = out_dir().join("disguised_bitcode.ll");
    assert!(module.write_bitcode_to_path(&disguised));
    let again = load_module(&ctxt, &disguised).expect("sniffed as bitcode");
    assert_eq!(shape(&module), shape(&again));
}

#[test]
fn missing_file_is_a_read_error() {
    let err = expect_err(&out_dir().join("does_not_exist.ll"));
    assert!(matches!(err, IngestError::Read { .. }));
}

#[test]
fn empty_file_is_a_read_error() {
    let path = out_dir().join("empty.ll");
    fs::write(&path, "").unwrap();
    assert!(matches!(expect_err(&path), IngestError::Read { .. }));
}

#[test]
fn malformed_text_is_a_parse_error() {
    let path = out_dir().join("malformed.ll");
    fs::write(&path, "define i32 @f( { this is not IR").unwrap();
    let err = expect_err(&path);
    assert!(matches!(
        err,
        IngestError::Parse {
            format: IrFormat::Text,
            ..
        }
    ));
}

#[test]
fn truncated_bitcode_is_a_parse_error() {
    let ll = write_ll("alpha_beta", "trunc");
    let ctxt = Context::create();
    let module = load_module(&ctxt, &ll).expect("load .ll");

    let bc = out_dir().join("full.bc");
    assert!(module.write_bitcode_to_path(&bc));
    let bytes = fs::read(&bc).unwrap();
    let cut = out_dir().join("truncated.bc");
    fs::write(&cut, &bytes[..8]).unwrap();

    let err = expect_err(&cut);
    assert!(matches!(
        err,
        IngestError::Parse {
            format: IrFormat::Bitcode,
            ..
        }
    ));
}
