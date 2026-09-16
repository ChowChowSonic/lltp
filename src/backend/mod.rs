mod expr;
mod stmt;

use std::fmt::{self, Formatter};

use crate::hir::{Function, Ty};
use crate::passes::{LowerError, Lowering, run_pipeline};

pub use expr::ExprEmitter;
pub use stmt::StmtEmitter;

pub trait Language: ExprEmitter + StmtEmitter {
    fn name(&self) -> &str;

    /// Default: unsupported for this backend. Panics on reach (see
    /// `ExprEmitter`/`StmtEmitter` defaults).
    fn emit_function(&self, func: &Function, out: &mut Formatter) -> fmt::Result {
        let _ = (func, out);
        unreachable!(
            "`emit_function` has no override for this backend; set the matching `allows_*()` flag to false or implement the emitter"
        )
    }

    /// Default: unsupported for this backend. Panics on reach (see
    /// `ExprEmitter`/`StmtEmitter` defaults).
    fn emit_ty(&self, ty: &Ty, out: &mut Formatter) -> fmt::Result {
        let _ = (ty, out);
        unreachable!(
            "`emit_ty` has no override for this backend; set the matching `allows_*()` flag to false or implement the emitter"
        )
    }

    /// Default: unsupported for this backend. Panics on reach (see
    /// `ExprEmitter`/`StmtEmitter` defaults).
    fn emit_module(&self, id: i32, out: &mut Formatter) -> fmt::Result {
        let _ = (id, out);
        unreachable!(
            "`emit_module` has no override for this backend; set the matching `allows_*()` flag to false or implement the emitter"
        )
    }

    /// Lowering passes contributed by a backend, run by `lower` (the B-core).
    /// Structural passes are library-owned and flags-driven; semantic passes
    /// (memory mapping, error propagation) are authored per target.
    fn pipeline(&self) -> Vec<Box<dyn Lowering>> {
        Vec::new()
    }

    /// A-veneer entry point: runs `pipeline()` via `run_pipeline`. Backends
    /// that need bespoke lowering override this method (instead of or in
    /// addition to supplying `pipeline()`).
    fn lower(&self, func: &mut Function) -> Result<(), LowerError>
    where
        Self: Sized,
    {
        run_pipeline(func, self)
    }

    /// If false, the middle-end MUST rewrite all `Match` HIR nodes into
    /// if/else chains before reaching the backend.
    fn allows_match(&self) -> bool;
    /// If false, the middle-end MUST rewrite all `Closure` HIR nodes into
    /// deferred records before reaching the backend.
    fn allows_closures(&self) -> bool;
    /// If false, the middle-end MUST monomorphize generic HIR nodes before
    /// reaching the backend.
    fn allows_generics(&self) -> bool;
    /// If false, the middle-end MUST erase async/await into an explicit state
    /// machine before reaching the backend.
    fn allows_async(&self) -> bool;
    /// If false, the middle-end MUST map threads onto the target's threading
    /// primitives before reaching the backend.
    fn allows_threads(&self) -> bool;
    /// If false, the middle-end MUST lower locks to a target-compatible
    /// primitive before reaching the backend.
    fn allows_locks(&self) -> bool;
    /// If false, the middle-end MUST rewrite raw-pointer HIR nodes into the
    /// target's safe reference types, or gate them behind `unsafe`.
    fn allows_raw_pointers(&self) -> bool;
    /// If false, the middle-end MUST rewrite all `Try`/`Catch` HIR nodes into
    /// error-code propagation before reaching the backend.
    fn allows_exceptions(&self) -> bool;
    /// The backend may assume it is only called for single-value returns when
    /// this is false.
    fn allows_multiple_return(&self) -> bool;
    /// If false, the middle-end MUST flatten inheritance hierarchies into
    /// composition before reaching the backend.
    fn allows_inheritance(&self) -> bool;
    /// The backend may assume only required arguments are present when this is
    /// false.
    fn allows_default_args(&self) -> bool;
    /// If false, the middle-end MUST lower unions to tagged buffers before
    /// reaching the backend.
    fn allows_union(&self) -> bool;
    /// Ignored by the backend; reserved for the frontend to signal whether
    /// macro-style codegen is permitted.
    fn allows_macros(&self) -> bool;
    /// If false, the middle-end MUST rewrite all `Goto` HIR nodes into
    /// structured control flow before reaching the backend. The backend may
    /// panic if `emit_goto` is called and this returns false.
    fn allows_goto(&self) -> bool;
    /// If false, the middle-end MUST rewrite all `Switch` HIR nodes into
    /// if/else chains before reaching the backend. The backend may panic if
    /// `emit_switch` is called and this returns false.
    fn allows_switch(&self) -> bool;
}
