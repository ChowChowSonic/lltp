//! Condition-based refinement (SoW 2.3.4): group guarded nodes with
//! complementary reaching conditions into if/else.

use crate::hir::Expr;
use crate::hir::acyclic::negate;

/// True when `a` and `b` are exact opposites: one is the other's negation,
/// in the form `negate` produces (`e` and `e ^ true`, or `true` and `false`).
// shortcut: syntactic match only, so `x < y` and `x >= y` are not seen as
// complementary; upgrade if refinement needs to group raw comparison guards.
pub fn complementary(a: &Expr, b: &Expr) -> bool {
    negate(a.clone()) == *b || negate(b.clone()) == *a
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::Ty;

    fn flag(name: &str) -> Expr {
        Expr::var(name, Ty::Bool)
    }

    #[test]
    fn a_flag_and_its_negation_are_complementary() {
        let c = flag("entry.cond");
        assert!(complementary(&c, &negate(c.clone())));
        assert!(complementary(&negate(c.clone()), &c));
    }

    #[test]
    fn boolean_constants_are_complementary() {
        assert!(complementary(&Expr::bool(true), &Expr::bool(false)));
    }

    #[test]
    fn unrelated_or_identical_guards_are_not() {
        let (a, b) = (flag("a.cond"), flag("b.cond"));
        assert!(!complementary(&a, &b));
        assert!(!complementary(&a, &a));
        assert!(!complementary(&a, &negate(b)));
    }
}
