//! Reaching conditions (SoW 2.3.2): which branch outcomes lead from a source
//! block to a sink block. The graph slice picks the blocks involved and
//! `Cond` is the boolean algebra the conditions are written in.

use std::collections::BTreeSet;
use std::ops::Not;

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

/// A boolean condition over CFG branch outcomes. Build it through `!`,
/// `and` and `or` so it stays simplified and in one canonical form.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Cond {
    True,
    False,
    /// The branch at `from` goes to `to`.
    Edge {
        from: String,
        to: String,
    },
    Not(Box<Cond>),
    /// Two or more distinct terms, none of them `And` or constant.
    And(Vec<Cond>),
    /// Two or more distinct terms, none of them `Or` or constant.
    Or(Vec<Cond>),
}

impl Cond {
    pub fn edge(from: &str, to: &str) -> Cond {
        Cond::Edge {
            from: from.to_string(),
            to: to.to_string(),
        }
    }

    pub fn and(self, other: Cond) -> Cond {
        join(self, other, true)
    }

    pub fn or(self, other: Cond) -> Cond {
        join(self, other, false)
    }
}

impl Not for Cond {
    type Output = Cond;

    fn not(self) -> Cond {
        match self {
            Cond::True => Cond::False,
            Cond::False => Cond::True,
            Cond::Not(inner) => *inner,
            other => Cond::Not(Box::new(other)),
        }
    }
}

/// `a AND b` (`is_and`) or `a OR b`: flatten same-kind terms, drop the unit,
/// let the absorbing constant win, dedupe, and sort so equal conditions
/// compare equal.
// shortcut: only a term next to its exact negation is caught (x & !x); no
// absorption or De Morgan, upgrade if if/else grouping (2.3.4) misses merges.
fn join(a: Cond, b: Cond, is_and: bool) -> Cond {
    let (unit, zero) = if is_and {
        (Cond::True, Cond::False)
    } else {
        (Cond::False, Cond::True)
    };
    let mut terms = BTreeSet::new();
    for t in [a, b] {
        match t {
            Cond::And(v) if is_and => terms.extend(v),
            Cond::Or(v) if !is_and => terms.extend(v),
            t => {
                terms.insert(t);
            }
        }
    }
    if terms.contains(&zero) {
        return zero;
    }
    terms.remove(&unit);
    if terms.iter().any(|t| terms.contains(&!t.clone())) {
        return zero;
    }
    match terms.len() {
        0 => unit,
        1 => terms.into_iter().next().unwrap(),
        _ => {
            let v = terms.into_iter().collect();
            if is_and { Cond::And(v) } else { Cond::Or(v) }
        }
    }
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

    fn atom(name: &str) -> Cond {
        Cond::edge(name, "next")
    }

    #[test]
    fn constants_fold() {
        let x = atom("a");
        assert_eq!(Cond::True.and(x.clone()), x);
        assert_eq!(Cond::False.and(x.clone()), Cond::False);
        assert_eq!(Cond::False.or(x.clone()), x);
        assert_eq!(Cond::True.or(x), Cond::True);
    }

    #[test]
    fn double_negation_and_complements_collapse() {
        let x = atom("a");
        assert_eq!(!!x.clone(), x);
        assert_eq!(!Cond::True, Cond::False);
        assert_eq!(x.clone().and(!x.clone()), Cond::False);
        assert_eq!(x.clone().or(!x), Cond::True);
    }

    #[test]
    fn terms_are_flattened_deduped_and_ordered() {
        let (a, b, c) = (atom("a"), atom("b"), atom("c"));
        assert_eq!(a.clone().and(a.clone()), a);
        assert_eq!(a.clone().and(b.clone()), b.clone().and(a.clone()));
        assert_eq!(
            a.clone().and(b.clone()).and(c.clone()),
            a.clone().and(b.clone().and(c.clone()))
        );
        assert_eq!(a.clone().or(b.clone()).or(c.clone()), a.or(b.or(c)));
    }
}
