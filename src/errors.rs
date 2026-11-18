use crate::IrFormat;
use std::io::Error;
use std::path::PathBuf;
use thiserror::Error;

/// Top-level error for HIR operations: the recovery, structuring and pass layer.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum HirError {
    #[error("lowering error: {0}")]
    Lower(#[source] LowerError),
    #[error("structurize error: {0}")]
    Structurize(#[source] StructurizeError),
    /// A pass failed. The pass name is carried so the pipeline can report which
    /// stage of `Language::pipeline` the failure came from.
    #[error("pass {pass:?} failed: {error}")]
    Pass {
        pass: &'static str,
        #[source]
        error: LowerError,
    },
}

impl From<LowerError> for HirError {
    fn from(e: LowerError) -> Self {
        HirError::Lower(e)
    }
}

impl From<StructurizeError> for HirError {
    fn from(e: StructurizeError) -> Self {
        HirError::Structurize(e)
    }
}

/// Errors occurring during lowering from LLVM IR into HIR.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum LowerError {
    #[error("malformed basic block: {0:?}")]
    MalformedBlock(String),
    #[error("block has no instructions: {0:?}")]
    MissingInstruction(String),
    #[error("{op:?}: missing operand at index {index:?}")]
    MissingOperand { op: &'static str, index: usize },
    #[error("unsupported instruction opcode: {0:?}")]
    UnsupportedOpcode(String),
    #[error("unsupported operand kind: {0:?}")]
    UnsupportedOperandKind(String),
    #[error("expected a value operand")]
    ExpectedValueOperand,
    #[error("function has no entry basic block")]
    NoFirstBlock,
    #[error("constant extraction failed: {0:?}")]
    ConstantExtractionFailed(&'static str),
    #[error("cannot recover `{func:?}`: {detail:?}")]
    RecoverError { func: String, detail: String },
    #[error("Unknown error encountered: {0}")]
    Other(String),
}

/// Errors occurring during control-flow structurization.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum StructurizeError {
    #[error("irreducible control flow: every cycle has a single entry after node splitting")]
    IrreducibleControlFlow,
    #[error("multi-exit loop at header {header} with divergent exits {exits:?}")]
    MultiExitLoop { header: String, exits: Vec<String> },
    #[error("abnormal loop exit from {block:?} to {target:?}")]
    AbnormalLoopExit { block: String, target: String },
    #[error("continue from {block:?} returns to the header without passing the guard in {latch:?}")]
    ContinueSkipsGuard { block: String, latch: String },
    #[error("abnormal entry into loop {header:?} at {block:?} from {src:?}")]
    AbnormalLoopEntry {
        header: String,
        block: String,
        src: String,
    },
    #[error("Enested loop {header:?} jumps out of its parent to {exit:?}")]
    NestedLoopEscapedParent { header: String, exit: String },
    #[error("unsupported fan-out from block {block:?} to {successors:?}")]
    UnsupportedFanOut { block: String, successors: usize },
    /// The block whose fan-out arms re-converge at different joins.
    #[error("parallel acyclic joins after fork {fork:?} require node splitting")]
    ParallelJoinsNeedNodeSplitting { fork: String },
    #[error("unknown block in control flow graph: {0:?}")]
    UnknownBlock(String),
    #[error("block {block:?} has no branch condition for the edge to {target:?}")]
    BadBranchEdge { block: String, target: String },
    #[error("region from {entry:?} to {exit:?} is not an acyclic region with a single exit")]
    NotAcyclicRegion { entry: String, exit: String },
}

#[derive(Error, Debug)]
pub enum IngestError {
    #[error("Cannot read {path:?}: {detail:?}")]
    Read { path: PathBuf, detail: String },
    #[error("cannot parse {origin:?} as {format:?}: {detail:?}")]
    Parse {
        origin: String,
        format: IrFormat,
        detail: String,
    },
}

#[derive(Error, Debug)]
pub enum FrontendError {
    /// The compiler executable could not be started.
    #[error(
        "could not run `{clang:?}` due to reason:\n({source:?}); is it installed? set LLTP_CLANG to override"
    )]
    Spawn { clang: String, source: Error },
    /// clang ran but exited non-zero (e.g. a C syntax error).
    #[error("clang failed:\n{stderr}")]
    Compile { stderr: String },
    #[error("clang produced non-UTF-8 output")]
    NotUtf8,
}

#[derive(Error, Debug)]
pub enum PipelineError {
    #[error("Unable to ingest IR due to reason:\n{0}")]
    IngestError(#[source] IngestError),
    /// An HIR failure, reported with the function it happened in. `error` stays
    /// typed so a caller can match `HirError::Structurize` / `HirError::Lower` /
    /// `HirError::Pass` instead of reading a formatted string. `origin` is the
    /// function name at the per-function boundary and the module name at the
    /// module boundary, where recovery has not picked a function yet.
    #[error("Failed to build HIR for {origin:?} due to reason:\n{error}")]
    Hir {
        origin: String,
        #[source]
        error: HirError,
    },
    /// The pipeline reports recovery failures as `Hir`, which keeps the
    /// `LowerError` typed instead of flattening it into a message.
    #[deprecated = "match PipelineError::Hir { error: HirError::Lower(..), .. } instead"]
    #[error("Failed to build HIR due to reason:\n{0}")]
    ModuleParsingError(#[source] LowerError),
    /// The pipeline reports pass failures as `Hir`, which keeps the pass name.
    #[deprecated = "match PipelineError::Hir { error: HirError::Pass { .. }, .. } instead"]
    #[error("Failed to properly apply passes to IR due to reason:\n{0}")]
    PassApplicationError(String),
    #[error(
        "Refusing to convert IR into the {0} language - IR contains disallowed instructions. Please write a pass to remove all {1} statements"
    )]
    DisallowedStmtError(String, String),
    /// A structured body that still contains a `Branch`: structuring left it
    /// unstructured, so no emitter can print it.
    #[error("Structured body of {func:?} still contains a branch at {block:?}")]
    Unstructured { func: String, block: String },
    /// `body` has to describe the same control flow as the `cfg` it was structured
    /// from, which is the contract `hir::func` documents. `detail` carries both
    /// shape fingerprints so the failure names the difference.
    #[error(
        "Structured body of {func:?} is not equivalent to the CFG it was built from:\n{detail}"
    )]
    NotEquivalent { func: String, detail: String },
    /// A pass that claimed to preserve the shape changed it.
    #[error("Pass {pass:?} claimed to preserve the shape of {func:?} and did not")]
    PassChangedShape { func: String, pass: &'static str },
    /// A node that reads its guard at a block the graph does not test at.
    #[error("{func:?} reads a loop guard at {block:?}, which is not a test point of its loop")]
    NotTestPoint { func: String, block: String },
    /// An `invoke` whose unwind arm names the same block as its normal arm: that is
    /// one block, not two arms, so the control view and the exception view agree on
    /// it.
    #[error("{func:?} records {target:?} as both the normal arm and an exception arm of an invoke")]
    ArmConflict { func: String, target: String },
    /// An arm named by a node that the graph does not carry.
    #[error("{func:?} names {block:?} as an invoke arm that the graph does not carry")]
    UnknownArm { func: String, block: String },
    #[error("Failed to emit function {0} due to reason:\n{1}")]
    FunctionEmissionError(String, String),
}
