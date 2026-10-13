<p align="center">
  <img src="assets/LLTP_logo_Transparent.png" alt="LLTP logo" width="160">
</p>

<h1 align="center">LLTP — Low-level Transpiler Framework</h1>

<p align="center">
  <strong>Transpilation grounded in LLVM IR: deterministic, high-fidelity language reconstruction.</strong>
</p>

## What it is

LLTP reconstructs idiomatic source in a **target language** from a program written in a **source language**, by grounding every step in **unoptimized LLVM IR** (`-O0 -g`) produced by the source language's **native toolchain**. It is the low-level layer of a push toward *universal transpilers*.

## Current status

- Working today (Sprint 1): the **C → LLVM IR → flat HIR → C (goto baseline)** round trip.
- `clang -O0 -g` produces the IR, inkwell parses it, per-function CFGs are built, and a flat
instruction-level HIR is emitted back to C with one label per basic block and `goto` for every branch.
- The oracle compiles and runs both the original and rebuilt C and requires identical exit code and stdout.

| Area | State |
|---|---|
| Frontend driver, IR ingest, per-function CFGs (SOW 2.1.1-2.1.3) | Done |
| Goto-based C emitter + round-trip tests | Working for int/bool/char/unsigned/float/cast fixtures |
| Control-flow structuring (`hir::structurize`) | Implemented with unit tests; not yet wired into the C emitter |
| `Language` trait, capability flags, `SwitchToIfElse` (2.5.1) | Done |
| Phi lowering, type recovery, aggregates / GEP / globals (strings, structs, arrays) | Not started (fixtures fail cleanly) |
| Rayon parallelism (2.1.4), Rust backend, idiom lifting | Not started |

## Round trip

```powershell
cargo run --example c2ir -- tests/fixtures/alpha_beta.c        # C -> clang -O0 IR
cargo run --example roundtrip -- tests/fixtures/alpha_beta.c   # C -> rebuilt goto C (stdout)
cargo run --example roundtrip -- tests/fixtures/loop_branch.c -o out.c
cargo test --test roundtrip                                    # compile + run both, diff behavior
```

`tests/roundtrip.rs` keeps two lists: `SUPPORTED` fixtures must match the original's
behavior, and `PENDING` fixtures (`string_basic`, `string_loop`, `struct_array`) must
fail with a clean `Err` until GEP/globals/aggregates land, at which point they move to
`SUPPORTED`. Rebuilt sources are written to `target/tmp/roundtrip/<name>_rebuilt.c`.

## Quickstart

> Recovery passes (de-SSA, structured emission, memory mapping) are still landing; the
> seams below are what's public today and are exercised by
> `cargo run --example ingest`, `cargo run --example lowering`, and the round trip above.

**1. Ingest IR into a module.**

```rust
use inkwell::context::Context;
use lltp::build_module;

let ctxt = Context::create();
let module = build_module(&ctxt, IR); // IR: &str of unoptimized LLVM IR
```

**2. Build per-function CFGs and detect loops.**

```rust
use lltp::hir::Cfg;
use lltp::hir::flow::natural_loops;

// One CFG per function; blocks cover all basic blocks,
// entry is the function's first block, exits are its terminators.
let func = module.get_first_function().expect("IR has a function");
let cfg = Cfg::try_from(&func).expect("CFG build");
let loops = natural_loops(&cfg);
```

**3. Recover HIR statements/expressions from instructions.**

```rust
use lltp::hir::{Expr, Stmt};

for inst in bb.get_instructions() {
    if let Ok(stmt) = Stmt::try_from(inst) { /* Branch / Ret */ }
    if let Ok(expr) = Expr::try_from(inst) { /* BinaryOp / Call / … */ }
}
```

**4. Define a target and run its lowering pipeline** — the extension seam
aimed at "users define a language, the library builds the decompiler." A backend
is its capability flags plus a pass list; every `emit_*` method defaults to
"unsupported for this backend" until you implement the nodes you can express.

```rust
use lltp::backend::{ExprEmitter, Language, StmtEmitter};
use lltp::passes::SwitchToIfElse;

struct ToyBackend;
impl ExprEmitter for ToyBackend {} // defaults: unsupported
impl StmtEmitter for ToyBackend {}
impl Language for ToyBackend {
    fn name(&self) -> &str { "toy" }
    fn pipeline(&self) -> Vec<Box<dyn lltp::passes::Lowering>> {
        vec![Box::new(SwitchToIfElse)]
    }
    fn allows_switch(&self) -> bool { false }
    // remaining allows_* flags + emit_* surface…
}

let mut func = /* recovered Function */;
ToyBackend.lower(&mut func)?; // runs pipeline(): Switch -> nested If / else
```

Full walkthroughs live in [`examples/ingest.rs`](examples/ingest.rs) and
[`examples/lowering.rs`](examples/lowering.rs).

## Why IR-grounded transpilation

The hypothesis: IR-grounded reconstruction yields higher fidelity (compile rate + behavioral equivalence) than syntactic AST-to-AST porters when type systems, memory models, and concurrency primitives diverge. Most AST-to-AST porters break down under:

- Type-system divergence (ownership, linear types, traits)
- Memory-management differences (GC vs manual vs ownership)
- Concurrency models (OS threads vs green threads vs async/await)
- Metaprogramming (templates, traits, decorators)
- Generalization beyond a single language pair

LLM-based transpilation flips the pipeline's selling points: quadratic token-cost scaling in project size, non-determinism and inaccuracy, low-quality or unsafe output, and wall-clock slowness.

## Windows toolchain setup

Building against `inkwell`/`llvm-sys` on Windows needs a real LLVM *development*
archive, not the LLVM installer (which only ships the toolchain, no
`llvm-config.exe` or static libs).

1. Download `clang+llvm-22.1.x-x86_64-pc-windows-msvc.tar.xz` from the LLVM
   releases page and extract it somewhere like `C:\LLVM-22.1.6`.
2. Set environment variables (new terminal/VS Code window required after this):
```powershell
   setx LLVM_SYS_221_PREFIX "C:\LLVM-22.1.6"
   setx LLTP_CLANG "C:\LLVM-22.1.6\bin\clang.exe"
```
3. Verify before building:
```powershell
   $env:LLVM_SYS_221_PREFIX
   & "C:\LLVM-22.1.6\bin\llvm-config.exe" --version   # expect 22.1.x
```

If another LLVM install (e.g. the official installer, or a newer major
version) is also on the machine, double-check `LLVM_SYS_221_PREFIX` still
points at the 22.1.x dev archive — a second install can silently overwrite it.

`Cargo.toml` defaults to the `target-all` feature (what CI on Linux uses). On Windows,
linking every LLVM target backend (Mips, Sparc, PowerPC, etc.) fails with unresolved
`LLVMInitialize*` symbols, so build with only the x86 backend:

```powershell
cargo test --no-default-features --features target-x86
```

## Pipeline

| Stage | Responsibility | Key techniques |
|---|---|---|
| **Frontend driver** | Invoke native compiler (`clang -O0 -g`, …) to emit `.ll`/`.bc` | per-language toolchain config |
| **IR ingest & normalize** | Parse via inkwell, build per-function CFGs | parallel over functions with rayon |
| **Type recovery** | Recover aggregate/scalar types from alloca/GEP/load/store chains | TIP-style inference; debug-info as hints only |
| **Variable recovery (de-SSA)** | Merge SSA values into named source-level variables | dominance-frontier phi elimination + liveness coalescing |
| **Control-flow structuring** | Reducible CFG → if/else/while ASTs; irreducible → `goto` fallback | back-edge pattern matching, Petabranch-style structuring |
| **Memory/concurrency mapping** | `malloc/free` → `Box`/`Rc`/`Arc`/GC handles; pthread/spinlocks → target primitives | per-target model adapters |
| **Idiom lifting** | Rewrite-rule engine: IR/HIR patterns → idiomatic constructs (iterator loops, `Vec`, RAII scopes) | declarative rule sets per language pair |
| **Backend emission** | Pretty-print HIR through the `Language` trait; post-format with native formatter | extensible backend registry |

## Application domains

- Legacy migration (COBOL/FORTRAN → Rust/Go/Python)
- Hot-path extraction (CUDA → Vulkan, Python → C++)
- Memory-safety hardening (C/C++ → Rust) — *flagship*
- Multi-language consolidation in large enterprises

## Verification & fidelity metrics

1. **Compile rate** — % of outputs that build in the target toolchain.
2. **Differential equivalence** — same inputs → same outputs vs the original program; YARPGen-style random program generation fuzzes the pipeline.
3. **Golden tests** per language pair.
4. **Idiomaticity score** — manual rubric for the portfolio narrative.

## Local checks

`./.github/scripts/pre-push.sh` mirrors CI (`fmt`, `clippy -D warnings`, `test`, example
smoke runs); pass a name to run one check, e.g. `./.github/scripts/pre-push.sh fmt`.
The round-trip tests need `clang` on `PATH` or `LLTP_CLANG` set.

## Stack

Rust edition 2024 · inkwell / LLVM 22.1 (pinned via `llvm22-1` feature) · rayon (function-level parallelism) · tracing (pass observability).
