//! Acyclic structuring (SoW 2.3.3): turn a source-to-sink region into an
//! ordered sequence of guarded nodes. This part gives the order.

use std::collections::BTreeSet;

use crate::hir::Cfg;
use crate::hir::reaching::EdgeSet;

/// Blocks of `region` reachable from `source`, in reverse postorder over the
/// edges not in `skip`. On an acyclic region every edge points forward in
/// the result, and a branch's first successor comes before its later ones.
/// Empty when `source` is not in `region`.
pub fn reverse_postorder(
    cfg: &Cfg,
    source: &str,
    region: &BTreeSet<String>,
    skip: &EdgeSet,
) -> Vec<String> {
    let mut order = Vec::new();
    if !region.contains(source) {
        return order;
    }
    let mut seen = BTreeSet::from([source.to_string()]);
    // Each frame holds a block and its successors still to visit; popping
    // from the end visits the first successor last, so it ends up first.
    let mut stack = vec![(source.to_string(), cfg.blocks[source].succ.clone())];
    while let Some((n, pending)) = stack.last_mut() {
        match pending.pop() {
            Some(m) => {
                if region.contains(&m)
                    && !skip.contains(&(n.clone(), m.clone()))
                    && seen.insert(m.clone())
                {
                    let next = cfg.blocks[&m].succ.clone();
                    stack.push((m, next));
                }
            }
            None => {
                order.push(n.clone());
                stack.pop();
            }
        }
    }
    order.reverse();
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    fn names(order: &[String]) -> Vec<&str> {
        order.iter().map(String::as_str).collect()
    }

    #[test]
    fn diamond_puts_the_first_arm_first() {
        let cfg = Cfg::cfg_from(
            "entry",
            &[
                ("entry", "a"),
                ("entry", "b"),
                ("a", "join"),
                ("b", "join"),
                ("join", "exit"),
            ],
        );
        let region = set(&["entry", "a", "b", "join", "exit"]);
        let order = reverse_postorder(&cfg, "entry", &region, &EdgeSet::new());
        assert_eq!(names(&order), ["entry", "a", "b", "join", "exit"]);
    }

    #[test]
    fn every_edge_points_forward() {
        let edges = [
            ("entry", "a"),
            ("entry", "b"),
            ("a", "c"),
            ("b", "c"),
            ("a", "d"),
            ("c", "exit"),
            ("d", "exit"),
        ];
        let cfg = Cfg::cfg_from("entry", &edges);
        let region = set(&["entry", "a", "b", "c", "d", "exit"]);
        let order = reverse_postorder(&cfg, "entry", &region, &EdgeSet::new());
        assert_eq!(order.len(), region.len());
        let pos = |n: &str| order.iter().position(|x| x == n).unwrap();
        for (from, to) in edges {
            assert!(pos(from) < pos(to), "{from} should come before {to}");
        }
    }

    #[test]
    fn blocks_outside_the_region_are_left_out() {
        let cfg = Cfg::cfg_from("entry", &[("entry", "a"), ("a", "join"), ("entry", "dead")]);
        let region = set(&["entry", "a", "join"]);
        let order = reverse_postorder(&cfg, "entry", &region, &EdgeSet::new());
        assert_eq!(names(&order), ["entry", "a", "join"]);
    }

    #[test]
    fn skipped_back_edge_still_gives_a_forward_order() {
        let cfg = Cfg::cfg_from(
            "entry",
            &[("entry", "h"), ("h", "body"), ("body", "h"), ("h", "exit")],
        );
        let region = set(&["h", "body", "exit"]);
        let back: EdgeSet = [("body".to_string(), "h".to_string())].into();
        let order = reverse_postorder(&cfg, "h", &region, &back);
        assert_eq!(names(&order), ["h", "body", "exit"]);
    }

    #[test]
    fn source_outside_the_region_gives_nothing() {
        let cfg = Cfg::cfg_from("entry", &[("entry", "a")]);
        let order = reverse_postorder(&cfg, "entry", &set(&["a"]), &EdgeSet::new());
        assert!(order.is_empty());
    }
}
