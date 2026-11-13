//! Reachability-based refinement (SoW 2.3.6): neighbouring guarded nodes that
//! can never both run are written as one cascading `if` / `else if` / `else`
//! instead of separate `if`s.

use crate::hir::Expr;
use crate::hir::acyclic::negate;
use crate::hir::op::BinOp;
use crate::hir::refine::conjuncts;

/// Drop from a conjunction every operand that `facts` already settle: an
/// operand `p` is dropped when `!p` is one of the expressions known to be
/// false. In a cascade this turns the guard `!c1 & c2` of the second arm into
/// `c2`, and `!c1 & !c2` of the last into `true`, a plain `else`. A guard with
/// nothing to drop comes back unchanged.
pub fn drop_known_false(guard: Expr, facts: &[Expr]) -> Expr {
    let mut parts = Vec::new();
    conjuncts(guard.clone(), &mut parts);
    let before = parts.len();
    parts.retain(|p| !facts.contains(&negate(p.clone())));
    if parts.len() == before {
        return guard;
    }
    let mut rest = parts.into_iter();
    let first = rest.next().unwrap_or_else(|| Expr::bool(true));
    rest.fold(first, |acc, p| Expr::binary(BinOp::And, acc, p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::Ty;

    fn flag(name: &str) -> Expr {
        Expr::var(name, Ty::Bool)
    }

    fn and(a: Expr, b: Expr) -> Expr {
        Expr::binary(BinOp::And, a, b)
    }

    #[test]
    fn operands_the_earlier_arms_settle_are_dropped() {
        let second = and(negate(flag("c1")), flag("c2"));
        assert_eq!(drop_known_false(second, &[flag("c1")]), flag("c2"));

        let third = and(negate(flag("c1")), negate(flag("c2")));
        let facts = [flag("c1"), flag("c2")];
        assert_eq!(drop_known_false(third, &facts), Expr::bool(true));
    }

    #[test]
    fn a_guard_the_facts_do_not_settle_is_unchanged() {
        let guard = and(negate(flag("c1")), flag("c2"));
        for facts in [vec![], vec![flag("c2")], vec![flag("other")]] {
            assert_eq!(drop_known_false(guard.clone(), &facts), guard);
        }
    }
}
