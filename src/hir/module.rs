use std::collections::HashSet;

use inkwell::module::Module as IrModule;

use crate::errors::LowerError;
use crate::hir::{FnSig, Function, flat};

/// A recovered HIR module: the function definitions plus the external
/// declarations they call.
#[derive(Debug, Clone, PartialEq)]
pub struct Module {
    /// External declarations
    pub decls: Vec<FnSig>,
    pub funcs: Vec<Function>,
}

impl Module {
    pub fn from_module(module: &IrModule) -> Result<Self, LowerError> {
        // Function names are reserved so no local gets the same target identifier.
        let reserved: HashSet<String> = module
            .get_functions()
            .map(|f| f.get_name().to_string_lossy().into_owned())
            .collect();
        let mut decls = Vec::new();
        let mut funcs = Vec::new();
        for f in module.get_functions() {
            if f.count_basic_blocks() == 0 {
                decls.push(flat::sig_of(&f));
            } else {
                funcs.push(flat::recover_function(&f, &reserved)?);
            }
        }
        Ok(Module { decls, funcs })
    }
}

impl<'c> TryFrom<&IrModule<'c>> for Module {
    type Error = LowerError;

    fn try_from(val: &IrModule<'c>) -> Result<Self, Self::Error> {
        Self::from_module(val)
    }
}
