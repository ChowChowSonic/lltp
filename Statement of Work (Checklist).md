# Statement of Work: Low-level Transpiler Framework (LLTP)

## 1. Project Objective and Scope
The objective of this project is to develop the Low-level Transpiler Framework (LLTP), a modular transpiler framework that leverages unoptimized LLVM-IR (-O0 -g) as a canonical intermediate representation to faithfully reconstruct source code in a target programming language.

The framework aims to solve the problem of semantic-preserving code regeneration across languages with divergent type systems, memory models, and concurrency primitives. The flagship implementation will focus on memory-safety hardening by transpiling C to Rust. The system will rely strictly on deterministic algorithms, avoiding the use of Large Language Models (LLMs) to ensure accuracy, safety, and reproducibility.

## 2. Detailed Requirements
The following requirements define the functional and non-functional capabilities the system shall deliver.

### 2.1 Frontend and Ingestion
- [X] **2.1.1** The system shall invoke or consume output from the source language's native toolchain compiler to generate unoptimized LLVM Intermediate Representation (IR) with debug symbols enabled. `[Priority = High]`
- [X] **2.1.2** The system shall parse the generated LLVM IR files (.ll or .bc formats) using the inkwell library. `[Priority = High]` (Note:pending additional tests to fully confirm)
- [ ] **2.1.3** The system shall build per-function control-flow graphs (CFGs) from the ingested LLVM IR. `[Priority = High]`
- [ ] **2.1.4** The system shall process functions in parallel utilizing the rayon library. `[Priority = Low]`

### 2.2 Type and Variable Recovery (De-SSA)
- [ ] **2.2.1** The system shall recover aggregate and scalar types from alloca, GEP, load, and store instruction chains using Type-Based Alias Analysis (TIP-style) inference. `[Priority = Medium]`
- [ ] **2.2.2** The system shall utilize debug information exclusively as hints during the type recovery process. `[Priority = Medium]`
- [ ] **2.2.3** The system shall merge Static Single Assignment (SSA) values into named source-level variables. `[Priority = High]`
- [ ] **2.2.4** The system shall eliminate phi nodes by executing dominance-frontier phi elimination and liveness coalescing. `[Priority = High]`

### 2.3 Pattern-Independent Control-Flow Structuring (HIR)
- [ ] **2.3.1** The system shall generate structured, goto-free High-Level Representation (HIR) by executing a pattern-independent control-flow structuring algorithm. `[Priority = High]`
- [ ] **2.3.2** The system shall compute reaching conditions from a source node to a sink node utilizing a depth-first Graph Slice algorithm. `[Priority = High]`
- [ ] **2.3.3** The system shall structure acyclic regions by generating an initial abstract syntax tree (AST) sequence ordered by reverse postordering (topological order). `[Priority = High]`
- [ ] **2.3.4** The system shall apply condition-based refinement to group AST sequence nodes with complementary reaching conditions into if-then-else constructs. `[Priority = High]`
- [ ] **2.3.5** The system shall apply condition-aware refinement to group AST sequence nodes sharing equality checks against a single variable into switch constructs. `[Priority = Medium]`
- [ ] **2.3.6** The system shall apply reachability-based refinement to represent mutually unreachable nodes as cascading if-else constructs. `[Priority = Medium]`
- [ ] **2.3.7** The system shall identify the final unique loop successor of a cyclic region by executing a loop successor refinement algorithm. `[Priority = High]`
- [ ] **2.3.8** The system shall infer loop types and continuation conditions by initially representing cyclic regions as endless loops with conditional break statements and applying structuring inference rules. `[Priority = High]`
- [ ] **2.3.9** The system shall transform abnormal loop entries (cyclic regions with multiple entries) into semantically equivalent single-entry loops by inserting a structuring variable and a centralized condition check. `[Priority = High]`
- [ ] **2.3.10** The system shall transform abnormal loop exits (cyclic regions with multiple successors) into semantically equivalent single-successor loops by redirecting exit edges to cascading condition nodes. `[Priority = High]`
- [ ] **2.3.11** The system shall evaluate reaching conditions for side effects and insert Boolean state variables when modified variables compromise semantic equivalence. `[Priority = High]`
- [ ] **2.3.12** The system shall map source memory management operations into target-specific memory models using per-target model adapters. `[Priority = Medium]`
- [ ] **2.3.13** The system shall map source concurrency primitives into target language concurrency primitives using per-target model adapters. `[Priority = Low]`
- [ ] **2.3.14** The system shall execute a rewrite-rule engine to transform IR and HIR patterns into idiomatic target language constructs. `[Priority = Medium]`

### 2.4 Post-Structuring Optimizations
- [ ] **2.4.1** The system shall simplify control constructs by transforming while loops into for loops when the continuation condition and loop body share an iterator variable. `[Priority = Low]`
- [ ] **2.4.2** The system shall transform branching assignments of a single variable into ternary operator expressions. `[Priority = Low]`
- [ ] **2.4.3** The system shall outline inline string functions into distinct, equivalent function calls. `[Priority = Low]`
- [ ] **2.4.4** The system shall rename variables based on the parameter signatures of corresponding Application Programming Interface (API) calls. `[Priority = Low]`

### 2.5 Backend Capability and Emission
- [ ] **2.5.1** The backend component shall define target language capabilities using explicit boolean flags via the Language trait. `[Priority = High]`
- [ ] **2.5.2** When a target capability flag evaluates to false, the system shall rewrite the corresponding HIR node into a structurally expressible form before emission. `[Priority = High]`
- [ ] **2.5.3** The system shall output the generated source code by pretty-printing the HIR through the Language trait. `[Priority = High]`
- [ ] **2.5.4** The system shall format the emitted output using Rust's native formatting tool. `[Priority = Low]`

### 2.6 System Constraints
- [ ] **2.6.1** The system shall generate all outputs using strictly deterministic algorithms. `[Priority = High]`

## 3. Project Milestones and Deliverables
The project will be executed and delivered according to the following milestones:
- [ ] **M0 — Scaffold Repair:** Setup hir/backend/passes modules, Language trait capabilities, structural passes, and CLI ingestion for .ll/.bc files.
- [ ] **M1 — IR to HIR for Straight-line Code & Simple Loops:** Implement De-SSA and control-flow structuring. Emit back to C as an internal sanity stage to verify behavioral equivalence.
- [ ] **M2 — Type Recovery & Fidelity:** Achieve full round-trip execution on toy programs with a green differential testing harness.
- [ ] **M3 — Rust Backend v0 (Flagship):** Implement ownership mapping (Box/&mut), unsafe fallback for pointer arithmetic, and manual-clean pass.
- [ ] **M4 — Idiom Lifting & Memory-Safety Hardening Mode:** Integrate rule engine to produce warning-free, idiomatic Rust code from C on a benchmark subset.
- [ ] **M5 — Second Source Language Integration:** Integrate Fortran or C++ (via Clang) to prove framework generalization beyond a single frontend.
- [ ] **M6 — Evaluation & Reporting:** Finalize metrics and results write-up for portfolio presentation.

## 4. Acceptance Criteria
To be considered complete and successful, the LLTP deliverables must meet the following verification and fidelity metrics:
1. **Compile Rate:** All outputs LLTP produces must successfully build in the target toolchain.
2. **Completeness of requirements:** All high-priority requirements listed above in section (2) must be completed and accompanied by a passing suite of test cases
3. **Golden Tests:** The system must consistently pass golden tests verifying expected output source structures for each configured language pair.
4. **Sample Frontend:** At minimum ONE (1) sample frontend must be built for an arbitrary programming language; this frontend will be used to test the outputs of LLTP & verify other criteria
