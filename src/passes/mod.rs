mod acyclic_regions;
mod switch;

use crate::backend::LowerTarget;
use crate::hir::Function;
use crate::hir::LowerError;

pub use acyclic_regions::AcyclicRegions;
pub use switch::SwitchToIfElse;

/// A lowering pass: a target-agnostic or target-authored transformation of a
/// recovered `Function`'s HIR, run before backend emission. Passes consult
/// the target's capability flags to decide whether a rewrite is needed; the
/// trait stays object-safe so `Language::pipeline` can box passes, and the
/// flags are read through `&dyn LowerTarget`.
pub trait Lowering {
    fn name(&self) -> &'static str;
    fn apply(&self, func: &mut Function, target: &dyn LowerTarget) -> Result<(), LowerError>;

    /// Whether the pass leaves the tree's shape alone. `hir::verify` can only
    /// confirm a pass that neither adds nor drops a node, so the default is false:
    /// a pass that rewrites one node into a chain changes the shape even when it
    /// changes no behaviour. A pass that only reorders or renames within a node can
    /// claim this and be checked.
    fn preserves_shape(&self) -> bool {
        false
    }
}
