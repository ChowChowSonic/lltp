//! Quickstart: ingest LLVM IR into a module, build per-function CFGs, and
//! recover HIR statements/expressions. Illustrates the library seams that
//! exist today — the frontend/recovery passes land in later milestones.
//!
//! Run with: `cargo run --example ingest`

use inkwell::context::Context;

use lltp::build_module;
use lltp::hir::graph::{build_graph, has_self_referential_loop};
use lltp::hir::{expr::Expr, stmt::Stmt};

/// A small unoptimized-function IR, YARPGen-style: an irreducible jumble of
/// basic blocks with several self-referential loops.
const IR: &str = r#"; ModuleID = '/tmp/autogen.bc'
source_filename = "/tmp/autogen.bc"

define void @autogen_SD0(ptr %0, ptr %1, ptr %2, i32 %3, i64 %4, i8 %5) {
BB:
  %Cmp24 = icmp ugt i64 1, 251161
  br label %CF85

CF85:                                             ; preds = %BB
  %Cmp32 = fcmp ord double 0x18DF23FE11DD527C, 0xAE97BFB633957A34
  br label %CF

CF:                                               ; preds = %CF, %CF84, %CF86, %CF85
  %Cmp40 = icmp slt i16 1, 18437
  br i1 %Cmp40, label %CF, label %CF82

CF82:                                             ; preds = %CF82, %CF
  %Sl47 = select i1 1, i1 1, i1 1
  br i1 %Sl47, label %CF82, label %CF83

CF83:                                             ; preds = %CF83, %CF82
  %Cmp56 = icmp ugt i64 1, 1
  br i1 %Cmp56, label %CF83, label %CF84

CF84:                                             ; preds = %CF83
  %Cmp64 = icmp ule i16 1, 1
  br i1 %Cmp64, label %CF, label %CF81

CF81:                                             ; preds = %CF81, %CF84
  %Cmp72 = icmp ult i16 1, 1
  br i1 %Cmp72, label %CF81, label %CF86

CF86:                                             ; preds = %CF81
  %Cmp79 = icmp ult i1 1, 1
  br i1 %Cmp79, label %CF, label %CF80

CF80:                                             ; preds = %CF86
  ret void
}
"#;

fn main() {
    tracing_subscriber::fmt().init();

    // 1. Ingest IR text into an in-memory LLVM module.
    let ctxt = Context::create();
    let module = build_module(&ctxt, IR);
    println!("module: {}", module.get_name().to_str().unwrap());

    // 2. Enumerate module-level globals.
    let globals = lltp::get_globals(&module);
    println!("{} global(s) in module", globals.len());

    // 3. Build a per-function CFG over basic blocks.
    let graph = build_graph(&module).expect("failed to build CFG");
    println!("{} basic blocks in CFG", graph.len());

    // 4. Find loop headers: blocks reachable from themselves.
    let loop_headers: Vec<_> = graph
        .keys()
        .filter(|bb| has_self_referential_loop(&graph, **bb))
        .map(|bb| bb.get_name().to_str().unwrap_or_default().to_owned())
        .collect();
    println!("loop-header candidates: {loop_headers:?}");

    // 5. Recover HIR statements/expressions from each block's instructions.
    let func = module.get_first_function().unwrap();
    for bb in func.get_basic_blocks() {
        let name = bb.get_name().to_str().unwrap_or_default().to_owned();
        let mut exprs = 0usize;
        let mut stmts = 0usize;
        for inst in bb.get_instructions() {
            if Expr::build(inst).is_ok() {
                exprs += 1;
            }
            if Stmt::build(inst).is_ok() {
                stmts += 1;
            }
        }
        println!("block {name}: {exprs} recoverable expr(s), {stmts} stmt(s)");
    }
}
