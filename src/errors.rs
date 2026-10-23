use crate::IrFormat;
use std::fmt;
use std::io::Error;
use std::path::PathBuf;
use thiserror::Error;

/// Top-level error for HIR operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HirError {
    Lower(LowerError),
    Structurize(StructurizeError),
}

impl fmt::Display for HirError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HirError::Lower(e) => write!(f, "lowering error: {e}"),
            HirError::Structurize(e) => write!(f, "structurize error: {e}"),
        }
    }
}

impl std::error::Error for HirError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            HirError::Lower(e) => Some(e),
            HirError::Structurize(e) => Some(e),
        }
    }
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
    #[error("irreducible control flow (node splitting is Task 5)")]
    IrreducibleControlFlow,
    #[error("multi-exit loop at header {header} with divergent exits {exits:?}")]
    MultiExitLoop { header: String, exits: Vec<String> },
    #[error("abnormal loop exit from {block:?} to {target:?}")]
    AbnormalLoopExit { block: String, target: String },
    #[error("Enested loop {header:?} jumps out of its parent to {exit:?}")]
    NestedLoopEscapedParent { header: String, exit: String },
    #[error("unsupported fan-out from block {block:?} to {successors:?}")]
    UnsupportedFanOut { block: String, successors: usize },
    /// The block whose fan-out arms re-converge at different joins.
    #[error("parallel acyclic joins after fork {fork:?} require node splitting")]
    ParallelJoinsNeedNodeSplitting { fork: String },
    #[error("unknown block in control flow graph: {0:?}")]
    UnknownBlock(String),
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
