//! Reaching conditions (SoW 2.3.2): which branch outcomes lead from a source
//! block to a sink block. The graph slice below picks the blocks involved.

use std::collections::BTreeSet;

use crate::hir::Cfg;

/// Edges ignored while slicing, normally the back edges of enclosing loops.
pub type EdgeSet = BTreeSet<(String, String)>;

/// Depth-first walk from `start` along successors (`forward`) or
/// predecessors, never crossing an edge in `skip`. Includes `start`.
fn dfs(cfg: &Cfg, start: &str, forward: bool, skip: &EdgeSet) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut stack = vec![start.to_string()];
    while let Some(n) = stack.pop() {
        if !seen.insert(n.clone()) {
            continue;
        }
        let blk = &cfg.blocks[&n];
        for m in if forward { &blk.succ } else { &blk.pred } {
            let edge = if forward {
                (n.clone(), m.clone())
            } else {
                (m.clone(), n.clone())
            };
            if !skip.contains(&edge) {
                stack.push(m.clone());
            }
        }
    }
    seen
}

/// The blocks that lie on some path from `source` to `sink`: everything
/// reachable from `source` that can also reach `sink`. Empty when either
/// block is unknown or `sink` is unreachable from `source`.
pub fn graph_slice(cfg: &Cfg, source: &str, sink: &str, skip: &EdgeSet) -> BTreeSet<String> {
    if !cfg.contains_block(source) || !cfg.contains_block(sink) {
        return BTreeSet::new();
    }
    let from_source = dfs(cfg, source, true, skip);
    let to_sink = dfs(cfg, sink, false, skip);
    from_source.intersection(&to_sink).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn diamond_slice_covers_both_arms() {
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
        let none = EdgeSet::new();
        assert_eq!(
            graph_slice(&cfg, "entry", "join", &none),
            set(&["entry", "a", "b", "join"])
        );
        assert_eq!(
            graph_slice(&cfg, "a", "exit", &none),
            set(&["a", "join", "exit"])
        );
    }

    #[test]
    fn blocks_off_every_path_are_excluded() {
        let cfg = Cfg::cfg_from("entry", &[("entry", "a"), ("a", "join"), ("entry", "dead")]);
        assert_eq!(
            graph_slice(&cfg, "entry", "join", &EdgeSet::new()),
            set(&["entry", "a", "join"])
        );
    }

    #[test]
    fn skipped_back_edge_drops_the_loop_body() {
        let cfg = Cfg::cfg_from(
            "entry",
            &[("entry", "h"), ("h", "body"), ("body", "h"), ("h", "exit")],
        );
        assert_eq!(
            graph_slice(&cfg, "h", "exit", &EdgeSet::new()),
            set(&["h", "body", "exit"])
        );
        let back: EdgeSet = [("body".to_string(), "h".to_string())].into();
        assert_eq!(graph_slice(&cfg, "h", "exit", &back), set(&["h", "exit"]));
    }

    #[test]
    fn unreachable_or_unknown_sink_gives_empty_slice() {
        let cfg = Cfg::cfg_from("e", &[("e", "a"), ("e", "b")]);
        let none = EdgeSet::new();
        assert!(graph_slice(&cfg, "a", "b", &none).is_empty());
        assert!(graph_slice(&cfg, "e", "nope", &none).is_empty());
        assert_eq!(graph_slice(&cfg, "a", "a", &none), set(&["a"]));
    }
}
