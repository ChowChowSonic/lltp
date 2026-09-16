mod switch;

use std::error::Error;
use std::fmt;

use crate::backend::Language;
use crate::hir::Function;

pub use switch::SwitchToIfElse;

/// A lowering pass: a target-agnostic or target-authored transformation of a
/// recovered `Function`'s HIR, run before backend emission. Passes consult
/// the target's capability flags to decide whether a rewrite is needed.
pub trait Lowering {
    fn name(&self) -> &'static str;
    fn apply(&self, func: &mut Function, target: &dyn Language) -> Result<(), LowerError>;
}

#[derive(Debug)]
pub struct LowerError {
    pub pass: &'static str,
    pub detail: String,
}

impl fmt::Display for LowerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "pass `{}` failed: {}", self.pass, self.detail)
    }
}

impl Error for LowerError {}

/// Runs the backend's registered lowering pipeline (the B-core). Structural
/// passes are library-owned and flags-driven; semantic passes are authored per
/// target. The `Language::lower` default wraps this driver (the A-veneer).
pub fn run_pipeline(func: &mut Function, target: &dyn Language) -> Result<(), LowerError> {
    for pass in target.pipeline() {
        pass.apply(func, target)?;
    }
    Ok(())
}
