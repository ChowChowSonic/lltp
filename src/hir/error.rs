use std::fmt;

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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LowerError {
    MalformedBlock(String),
    MissingInstruction(String),
    MissingOperand { op: &'static str, index: usize },
    UnsupportedOpcode(String),
    UnsupportedOperandKind(String),
    ExpectedValueOperand,
    NoFirstBlock,
    ConstantExtractionFailed(&'static str),
    Other(String),
}

impl fmt::Display for LowerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LowerError::MalformedBlock(name) => write!(f, "malformed basic block: {name}"),
            LowerError::MissingInstruction(name) => write!(f, "block has no instructions: {name}"),
            LowerError::MissingOperand { op, index } => {
                write!(f, "{op}: missing operand at index {index}")
            }
            LowerError::UnsupportedOpcode(op) => write!(f, "unsupported instruction opcode: {op}"),
            LowerError::UnsupportedOperandKind(k) => write!(f, "unsupported operand kind: {k}"),
            LowerError::ExpectedValueOperand => write!(f, "expected a value operand"),
            LowerError::NoFirstBlock => write!(f, "function has no entry basic block"),
            LowerError::ConstantExtractionFailed(msg) => {
                write!(f, "constant extraction failed: {msg}")
            }
            LowerError::Other(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for LowerError {}

/// Errors occurring during control-flow structurization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StructurizeError {
    IrreducibleControlFlow,
    MultiExitLoop { header: String, exits: Vec<String> },
    AbnormalLoopExit { block: String, target: String },
    NestedLoopEscapedParent { header: String, exit: String },
    UnsupportedFanOut { block: String, successors: usize },
    ParallelJoinsNeedNodeSplitting,
    UnknownBlock(String),
}

impl fmt::Display for StructurizeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StructurizeError::IrreducibleControlFlow => {
                write!(f, "irreducible control flow (node splitting is Task 5)")
            }
            StructurizeError::MultiExitLoop { header, exits } => {
                write!(
                    f,
                    "multi-exit loop at header {header} with divergent exits {exits:?}"
                )
            }
            StructurizeError::AbnormalLoopExit { block, target } => {
                write!(f, "abnormal loop exit from {block} to {target}")
            }
            StructurizeError::NestedLoopEscapedParent { header, exit } => {
                write!(f, "nested loop {header} jumps out of its parent to {exit}")
            }
            StructurizeError::UnsupportedFanOut { block, successors } => {
                write!(
                    f,
                    "unsupported fan-out from block {block} ({successors} successors; node splitting is Task 5)"
                )
            }
            StructurizeError::ParallelJoinsNeedNodeSplitting => {
                write!(f, "parallel acyclic joins require node splitting")
            }
            StructurizeError::UnknownBlock(name) => {
                write!(f, "unknown block in control flow graph: {name}")
            }
        }
    }
}

impl std::error::Error for StructurizeError {}
