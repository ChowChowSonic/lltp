# LLTP — Low-level Transpiler Framework

**Project:** Towards universal transpilers: a low-level transpiler framework for high-fidelity language reconstruction
**Type:** Portfolio
**Flagship pair:** C → Rust (memory-safety hardening)
**Stance:** Strictly deterministic. No LLMs anywhere in the pipeline — correctness and reproducibility are the selling points over AI-based porting.

## 1. Vision

Reconstruct idiomatic source in a **target language** from a program written in a **source language**, by grounding every step in **unoptimized LLVM-IR** produced by the source language's native toolchain.

Hypothesis to validate: IR-grounded reconstruction yields higher fidelity (compile rate + behavioral equivalence) than syntactic AST-to-AST porters when type systems, memory models, and concurrency primitives diverge.

Primary application domains (proposal):

- Legacy migration (COBOL/FORTRAN → Rust/Go/Python)
- Hot-path extraction (CUDA → Vulkan, Python → C++)
- Memory-safety hardening (C/C++ → Rust) — **flagship**
- Multi-language consolidation in large enterprises

## 2. Pipeline Architecture

```
source ──(native toolchain)──▶ LLVM IR (-O0, -g) ──(ingest)──▶ IR graph ──(recovery passes)──▶ HIR ──(idiom lifting)──▶ target AST ──(Language trait)──▶ target source
```

| Stage | Responsibility | Key techniques |
|---|---|---|
| **Frontend driver** | Invoke native compiler (`clang -O0 -g`, gfortran, …) to emit `.ll`/`.bc` | per-language toolchain config |
| **IR ingest & normalize** | Parse via `inkwell`, build per-function CFGs (sketched in `build_graph`) | parallel over functions with `rayon` |
| **Type recovery** | Recover aggregate/scalar types from alloca/GEP/load/store chains | TIP-style inference; debug-info as hints only |
| **Variable recovery (de-SSA)** | Merge SSA values into named source-level variables | dominance-frontier phi elimination + liveness coalescing |
| **Control-flow structuring** | Reducible CFG → if/else/while ASTs; irreducible → `goto` fallback | back-edge pattern matching, Petabranch-style structuring |
| **Memory/concurrency mapping** | `malloc/free` → `Box`/`Rc`/`Arc`/GC handles; pthread/spinlocks → target primitives | per-target model adapters |
| **Idiom lifting** | Rewrite-rule engine: IR/HIR patterns → idiomatic constructs (iterator loops, `Vec`, RAII scopes) | declarative rule sets per language pair |
| **Backend emission** | Pretty-print HIR through the `Language` trait; post-format with native formatter (`rustfmt`, …) | extensible backend registry |

### Why this beats the alternatives

Brittleness of AST-to-AST porters under: type-system divergence, memory-management differences (GC vs manual vs ownership), concurrency models (green/OS threads vs async/await), metaprogramming (templates, traits, decorators), and generalization beyond a single language pair.

LLM-based transpilation flaws this design avoids: token-cost scaling (sometimes quadratic in project size), non-determinism and inaccuracy, low-quality or unsafe output, and wall-clock slowness versus a deterministic transpiler.

## 3. Repo Layout (target)

```
src/
  cli.rs            # lltp <in> -S <lang>: pipeline driver
  frontend/         # toolchain drivers per source language
  ir/               # inkwell wrappers, CFG, normalization
  hir/              # func.rs, expr.rs, stmt.rs, ty.rs — recovered program model
  passes/           # type recovery, de-SSA, structuring, memory mapping, idiom rules
  backend/          # c.rs, rust.rs, … impl Language
  language.rs       # Language trait (exists; extend with type/stmt emission methods)
tests/
  golden/           # input → expected target source
  differential/     # run original vs transpiled on shared inputs
```

## 4. Milestones

- **M0 — Scaffold repair:** fix `mod` declarations (`hir` undeclared in `lib.rs`, `transpiler` module missing), CLI that ingests `.ll`/`.bc` and dumps a module summary; CI with `cargo fmt` / `clippy` / `test`.
- **M1 — IR → HIR for straight-line code & simple loops:** de-SSA + control-flow structuring; emit back to **C** as an internal sanity stage (transpiled C must compile and match original behavior) before touching Rust.
- **M2 — Type recovery + fidelity:** full round-trip on toy programs; differential harness green.
- **M3 — Rust backend v0 (flagship):** ownership mapping (`Box`/`&mut`), `unsafe` fallback for pointer arithmetic, manual-clean pass.
- **M4 — Idiom lifting & memory-safety hardening mode:** rule engine; C → Rust produces warning-free idiomatic code on a benchmark subset.
- **M5 — Second source language** (Fortran or C++ via clang) to prove the framework generalizes beyond one frontend.
- **M6 — Evaluation write-up:** metrics + results for portfolio presentation.

## 5. Verification & Fidelity Metrics

1. **Compile rate** — % of outputs that build in the target toolchain.
2. **Differential equivalence** — same inputs → same outputs vs original program. The existing test IR is YARPGen-style; continue using **YARPGen-style random program generation** to fuzz the pipeline.
3. **Golden tests** per language pair.
4. **Idiomaticity score** — manual rubric, for the portfolio narrative.

## 6. Risks & Mitigations

- *IR loses high-level structure* → mandate `-O0 -g`; debug-info is auxiliary signal, never load-bearing.
- *Irreducible CFGs* → `goto` fallback behind capability flags already present in the `Language` trait (`allows_goto`, `allows_switch`).
- *Scope explosion (four application domains)* → flagship C → Rust first; other pairs become documented "extension recipes".
- *LLVM churn* → pinned via `inkwell` feature `llvm22-1`.

## 7. Stack

Rust edition 2024 · inkwell / LLVM 22.1 · rayon (function-level parallelism) · tracing (pass observability).
