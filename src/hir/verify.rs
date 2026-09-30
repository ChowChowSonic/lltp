use crate::hir::{cfg::Cfg, flow::reachable, stmt::Stmt};
use std::collections::{BTreeSet, HashMap};

struct ReBuilder {
    succ: HashMap<String, Vec<String>>,
    pred: HashMap<String, Vec<String>>,
    all: Vec<String>,
    next: usize,
}

impl ReBuilder {
    fn fresh(&mut self) -> String {
        let n = format!("b{}", self.next);
        self.next += 1;
        self.all.push(n.clone());
        n
    }
    fn edge(&mut self, from: &str, to: &str) {
        if from == to {
            return;
        }
        let s = self.succ.entry(from.to_string()).or_default();
        if !s.contains(&to.to_string()) {
            s.push(to.to_string());
        }
        let p = self.pred.entry(to.to_string()).or_default();
        if !p.contains(&from.to_string()) {
            p.push(from.to_string());
        }
    }
    fn collect_labels(&mut self, stmts: &[Stmt], labels: &mut HashMap<String, String>) {
        for s in stmts {
            match s {
                Stmt::Label { name } => {
                    labels.insert(name.clone(), self.fresh());
                }
                Stmt::If {
                    then_stmts,
                    else_stmts,
                    ..
                } => {
                    self.collect_labels(then_stmts, labels);
                    self.collect_labels(else_stmts, labels);
                }
                Stmt::Loop { body, .. } => {
                    self.collect_labels(body, labels);
                }
                Stmt::Switch { cases, .. } => {
                    for (_, b) in cases {
                        self.collect_labels(b, labels);
                    }
                }
                _ => {}
            }
        }
    }
    fn region(
        &mut self,
        stmts: &[Stmt],
        brk: Option<&String>,
        cont: Option<&String>,
        labels: &HashMap<String, String>,
    ) -> (String, Option<String>) {
        let entry = self.fresh();
        let mut cur = Some(entry.clone());
        for s in stmts {
            match s {
                Stmt::Let { .. } => {}
                Stmt::Ret { .. } => cur = None,
                Stmt::Break => {
                    if let Some(c) = &cur {
                        let j: &str = brk.expect("Break outside any loop");
                        self.edge(c, j);
                    }
                    cur = None;
                }

                Stmt::Goto { target } => {
                    if let Some(c) = &cur {
                        let t = labels
                            .get(target)
                            .unwrap_or_else(|| panic!("goto unknown label {target}"));
                        self.edge(c, t);
                    }
                    cur = None;
                }
                Stmt::Label { name } => {
                    if let Some(c) = &cur {
                        self.edge(c, &labels[name]);
                    }
                    cur = labels.get(name).cloned();
                }
                Stmt::If {
                    then_stmts,
                    else_stmts,
                    ..
                } => {
                    let Some(c) = cur else { continue }; // dead code: skip
                    let (te, tx) = self.region(then_stmts, brk, cont, labels);
                    self.edge(&c, &te);
                    let mut join: Option<String> = None;
                    if else_stmts.is_empty() {
                        let j = self.fresh(); // false path continues after the If
                        self.edge(&c, &j);
                        join = Some(j);
                    } else {
                        let (ee, ex) = self.region(else_stmts, brk, cont, labels);
                        self.edge(&c, &ee);
                        if let Some(x) = ex {
                            let j = self.fresh();
                            self.edge(&x, &j);
                            join = Some(j);
                        }
                    }
                    if let Some(t) = tx {
                        // then arm falls through
                        let j = join.get_or_insert_with(|| self.fresh());
                        self.edge(&t, j);
                    }
                    cur = join; // None if both arms terminated
                }
                Stmt::Loop {
                    body, cond: lcond, ..
                } => {
                    let Some(c) = cur else { continue };
                    let header = self.fresh();
                    let join = self.fresh(); // false-exit: cond fails → after loop
                    let (be, bx) = self.region(body, Some(&join), Some(&header), labels);
                    self.edge(&c, &header);
                    self.edge(&header, &be); // cond true → body
                    if lcond.is_some() {
                        self.edge(&header, &join); // cond false → after
                    }
                    if let Some(x) = bx {
                        self.edge(&x, &header);
                    } // body falls out → header (back edge)
                    cur = Some(join);
                }

                Stmt::Switch { cases, default, .. } => {
                    let Some(c) = cur else { continue };
                    let mut join: Option<String> = None;
                    for (_, body) in cases {
                        let (ae, ax) = self.region(body, brk, cont, labels);
                        self.edge(&c, &ae);
                        if let Some(x) = ax {
                            let j = join.get_or_insert_with(|| self.fresh());
                            self.edge(&x, j);
                        }
                    }
                    if default.is_empty() {
                        let j = join.get_or_insert_with(|| self.fresh());
                        self.edge(&c, j);
                    } else {
                        let (de, dx) = self.region(default, brk, cont, labels);
                        self.edge(&c, &de);
                        if let Some(x) = dx {
                            let j = join.get_or_insert_with(|| self.fresh());
                            self.edge(&x, j);
                        }
                    }
                    cur = join;
                }
                Stmt::Continue => {
                    if let Some(c) = &cur {
                        let j = cont.expect("Continue outside any loop");
                        self.edge(c, j);
                    }
                    cur = None;
                }

                Stmt::Branch { .. } => unreachable!("structured input must not contain Branch"),
            }
        }
        (entry, cur)
    }
}

pub fn structured_to_cfg(body: &[Stmt]) -> Cfg {
    let mut b = ReBuilder {
        succ: HashMap::new(),
        pred: HashMap::new(),
        all: Vec::new(),
        next: 0,
    };
    let mut labels = HashMap::new();
    b.collect_labels(body, &mut labels);
    let (entry, _) = b.region(body, None, None, &labels);
    for n in &b.all {
        b.succ.entry(n.clone()).or_default();
    }
    let exits: Vec<String> = b
        .succ
        .iter()
        .filter(|(_, v)| v.is_empty())
        .map(|(k, _)| k.clone())
        .collect();
    Cfg::new(entry, b.succ, b.pred, exits)
}

fn keep_reachable(cfg: &Cfg) -> Cfg {
    let reach = reachable(cfg);
    let succ: HashMap<String, Vec<String>> = reach
        .iter()
        .map(|n| {
            (
                n.clone(),
                cfg.blocks[n]
                    .succ
                    .iter()
                    .filter(|x| reach.contains(*x))
                    .cloned()
                    .collect(),
            )
        })
        .collect();
    let pred: HashMap<String, Vec<String>> = reach
        .iter()
        .map(|n| {
            (
                n.clone(),
                cfg.blocks[n]
                    .pred
                    .iter()
                    .filter(|x| reach.contains(*x))
                    .cloned()
                    .collect(),
            )
        })
        .collect();
    let exits: Vec<String> = succ
        .iter()
        .filter(|(_, v)| v.is_empty())
        .map(|(k, _)| k.clone())
        .collect();
    Cfg::new(cfg.entry.clone(), succ, pred, exits)
}

pub fn collapse_chains(cfg: &Cfg) -> Cfg {
    let mut succ: HashMap<String, Vec<String>> = cfg
        .blocks
        .keys()
        .map(|n| (n.clone(), cfg.blocks[n].succ.clone()))
        .collect();
    let mut pred: HashMap<String, Vec<String>> = cfg
        .blocks
        .keys()
        .map(|n| (n.clone(), cfg.blocks[n].pred.clone()))
        .collect();
    loop {
        let mut merged = false;
        for n in succ.keys().cloned().collect::<Vec<_>>() {
            if n == cfg.entry {
                continue;
            }
            // Fuse n into its single predecessor p, but only when p falls
            // straight through into n (p's only successor is n). Branch arms
            // (p has several successors) must survive as blocks; chain tails
            // (n has no successors) are fine to fuse.
            let Some(ps) = pred.get(&n) else {
                continue;
            };
            if ps.len() != 1 {
                continue;
            }
            let p = ps[0].clone();
            if p == n {
                continue;
            }
            if succ
                .get(&p)
                .map(|s| s.len() != 1 || s[0] != n)
                .unwrap_or(true)
            {
                continue;
            }

            let out: Vec<String> = succ.remove(&n).unwrap_or_default();
            pred.remove(&n);
            for s in &out {
                if s != &p {
                    let sl = succ.entry(p.clone()).or_default();
                    if !sl.contains(s) {
                        sl.push(s.clone());
                    }
                    let ps = pred.entry(s.clone()).or_default();
                    ps.retain(|x| x != &n);
                    if !ps.contains(&p) {
                        ps.push(p.clone());
                    }
                } // s's pred: n → p
            }
            succ.get_mut(&p).unwrap().retain(|x| x != &n); // drop the p→n edge
            merged = true;
        }
        if !merged {
            break;
        }
    }
    let exits: Vec<String> = succ
        .iter()
        .filter(|(_, v)| v.is_empty())
        .map(|(k, _)| k.clone())
        .collect();
    Cfg::new(cfg.entry.clone(), succ, pred, exits)
}

pub fn color_fingerprint(cfg: &Cfg) -> String {
    let names: Vec<String> = cfg.blocks.keys().cloned().collect();
    let mut color: HashMap<String, usize> = names.iter().map(|n| (n.clone(), 0)).collect();
    color.insert(cfg.entry.clone(), 1);
    loop {
        let mut sig: HashMap<String, String> = HashMap::new();
        for n in &names {
            let mut in_c: Vec<usize> = cfg.blocks[n].pred.iter().map(|p| color[p]).collect();
            let mut out_c: Vec<usize> = cfg.blocks[n].succ.iter().map(|p| color[p]).collect();
            in_c.sort();
            out_c.sort();
            sig.insert(n.clone(), format!("{}|{:?}|{:?}", color[n], in_c, out_c));
        }
        let uniq: BTreeSet<&String> = sig.values().collect();
        let mut key2color: HashMap<String, usize> = HashMap::new();
        for (i, k) in uniq.iter().enumerate() {
            key2color.insert(k.to_string(), i);
        }
        let next: HashMap<String, usize> =
            sig.iter().map(|(n, k)| (n.clone(), key2color[k])).collect();
        if next == color {
            break;
        }
        color = next;
    }
    let mut classes: HashMap<usize, usize> = HashMap::new();
    for n in &names {
        *classes.entry(color[n]).or_default() += 1;
    }
    let mut v: Vec<(usize, usize)> = classes.into_iter().collect();
    v.sort();
    format!("E{}|{:?}", color[&cfg.entry], v)
}

pub fn are_equivalent(a: &Cfg, b: &Cfg) -> bool {
    let a = collapse_chains(&keep_reachable(a));
    let b = collapse_chains(&keep_reachable(b));
    a.blocks.len() == b.blocks.len() && color_fingerprint(&a) == color_fingerprint(&b)
}

// fresh(): "b0","b1",…
// edge(&mut self, from, to): record both succ and pred, dedupe

// Returns the block the path continues from (Some), or None when the path
// terminated (Ret / Break / Continue / Goto).
#[cfg(test)]
mod tests {
    use super::{are_equivalent, structured_to_cfg};
    use crate::hir::Lit;
    use crate::hir::cfg::Cfg;
    use crate::hir::expr::Expr;
    use crate::hir::stmt::Stmt;
    use std::collections::{BTreeSet, HashMap};

    fn cond() -> Expr {
        Expr::Literal(Lit::Bool(true))
    }

    /// Build a `Cfg` from an entry name and an edge list. Every named node
    /// gets a block; nodes without successors become exits.
    fn cfg_from(entry: &str, edges: &[(&str, &str)]) -> Cfg {
        let mut nodes: BTreeSet<String> = BTreeSet::new();
        nodes.insert(entry.to_string());
        for (a, b) in edges {
            nodes.insert((*a).to_string());
            nodes.insert((*b).to_string());
        }
        let mut succ: HashMap<String, Vec<String>> = HashMap::new();
        let mut pred: HashMap<String, Vec<String>> = HashMap::new();
        for n in &nodes {
            succ.entry(n.clone()).or_default();
            pred.entry(n.clone()).or_default();
        }
        for (a, b) in edges {
            succ.entry((*a).to_string())
                .or_default()
                .push((*b).to_string());
            pred.entry((*b).to_string())
                .or_default()
                .push((*a).to_string());
        }
        let exits: Vec<String> = nodes
            .iter()
            .filter(|n| succ.get(*n).is_some_and(|s| s.is_empty()))
            .cloned()
            .collect();
        Cfg::new(entry.to_string(), succ, pred, exits)
    }

    /// `if c { ret } else { ret }`: header + two arms, nothing after the if,
    /// so no join block is created.
    #[test]
    fn if_with_both_arms_ret_has_no_join() {
        let body = [Stmt::If {
            cond: cond(),
            then_stmts: vec![Stmt::Ret { value: None }],
            else_stmts: vec![Stmt::Ret { value: None }],
        }];
        let cfg = structured_to_cfg(&body);
        assert_eq!(cfg.blocks.len(), 3);
        assert_eq!(cfg.blocks[&cfg.entry].succ.len(), 2);
        // no block has two preds → nothing joined
        assert!(cfg.blocks.values().all(|b| b.pred.len() <= 1));
        // both arms are exits
        assert_eq!(cfg.exits.len(), 2);
    }
    /// A loop body that falls out must return to the header via the bx edge;
    /// only the cond-false edge may leave the loop.
    #[test]
    fn loop_body_fallout_returns_to_the_header() {
        let body = [Stmt::Loop {
            cond: Some(cond()),
            body: vec![Stmt::Let {
                dest: Box::new(Expr::Literal(Lit::Bool(false))),
                src: Box::new(Expr::Literal(Lit::Bool(true))),
            }],
        }];
        let cfg = structured_to_cfg(&body);
        // b0 entry, b1 header, b2 join, b3 body (falls out)
        assert_eq!(cfg.blocks.len(), 4);
        assert_eq!(cfg.blocks[&cfg.entry].succ, vec!["b1".to_string()]);
        assert_eq!(
            cfg.blocks["b1"].succ,
            vec!["b3".to_string(), "b2".to_string()]
        );
        assert_eq!(cfg.blocks["b3"].succ, vec!["b1".to_string()]); // back to header
        assert!(cfg.blocks["b2"].succ.is_empty());
        assert_eq!(cfg.exits.len(), 1);
    }
    /// `while c { break } ret`: header branches to body and exit, break jumps
    /// to the exit, the ret lives in the exit block.
    #[test]
    fn loop_with_break_shape() {
        let body = [
            Stmt::Loop {
                cond: Some(cond()),
                body: vec![Stmt::Break],
            },
            Stmt::Ret { value: None },
        ];
        let cfg = structured_to_cfg(&body);
        // b0 entry, b1 header, b2 join (holds the ret), b3 body entry (break)
        assert_eq!(cfg.blocks.len(), 4);
        assert_eq!(cfg.blocks[&cfg.entry].succ, vec!["b1".to_string()]);
        assert_eq!(
            cfg.blocks["b1"].succ,
            vec!["b3".to_string(), "b2".to_string()]
        );
        assert_eq!(cfg.blocks["b3"].succ, vec!["b2".to_string()]);
        assert!(cfg.blocks["b2"].succ.is_empty());
        assert_eq!(cfg.exits.len(), 1);
    }

    /// `while c { if d { continue } ret }`: the continue block must jump back
    /// to the loop header (re-evaluating the condition), not to the exit.
    #[test]
    fn continue_targets_the_loop_header() {
        let body = [Stmt::Loop {
            cond: Some(cond()),
            body: vec![
                Stmt::If {
                    cond: cond(),
                    then_stmts: vec![Stmt::Continue],
                    else_stmts: vec![],
                },
                Stmt::Ret { value: None },
            ],
        }];
        let cfg = structured_to_cfg(&body);
        // b0 entry, b1 header, b2 join, b3 body entry, b4 then arm (continue),
        // b5 after the if (ret)
        assert_eq!(cfg.blocks.len(), 6);
        // the continue must target the header, re-evaluating the condition
        assert_eq!(cfg.blocks["b4"].succ, vec!["b1".to_string()]);
        // the if dispatches from the body entry to the then arm and false path
        assert_eq!(
            cfg.blocks["b3"].succ,
            vec!["b4".to_string(), "b5".to_string()]
        );
        // the entry has a single unconditional edge into the header
        assert_eq!(cfg.blocks[&cfg.entry].succ, vec!["b1".to_string()]);
        // header branches to body and exit
        assert_eq!(
            cfg.blocks["b1"].succ,
            vec!["b3".to_string(), "b2".to_string()]
        );
        assert_eq!(cfg.exits.len(), 2);
    }

    /// `loop { break }` (cond: None): the header has no false exit, the join
    /// is reachable only through the break.
    #[test]
    fn infinite_loop_has_no_false_exit() {
        let body = [Stmt::Loop {
            cond: None,
            body: vec![Stmt::Break],
        }];
        let cfg = structured_to_cfg(&body);
        // b0 entry, b1 header, b2 join, b3 body entry (break)
        assert_eq!(cfg.blocks.len(), 4);
        assert_eq!(cfg.blocks[&cfg.entry].succ, vec!["b1".to_string()]);
        assert_eq!(cfg.blocks["b1"].succ, vec!["b3".to_string()]);
        assert_eq!(cfg.blocks["b3"].succ, vec!["b2".to_string()]);
        // no false edge from the header means the join is only reachable
        // through the break
        assert_eq!(cfg.blocks["b2"].pred, vec!["b3".to_string()]);
        assert_eq!(cfg.exits.len(), 1);
    }

    /// Same shapes with different block names must compare equal; different
    /// shapes must not.
    #[test]
    fn are_equivalent_ignores_block_names() {
        let d1 = cfg_from("E", &[("E", "x"), ("E", "y"), ("x", "J"), ("y", "J")]);
        let d2 = cfg_from("e", &[("e", "a"), ("e", "b"), ("a", "j"), ("b", "j")]);
        assert!(are_equivalent(&d1, &d2));
        // diamond where both arms terminate vs diamond with a real join
        let d3 = cfg_from("e", &[("e", "a"), ("e", "b"), ("a", "ret1"), ("b", "ret2")]);
        assert!(!are_equivalent(&d2, &d3));
    }

    /// A straight-line chain collapses down to a single block: the fused
    /// version must compare equal to the un-fused one.
    #[test]
    fn are_equivalent_tolerates_chain_fusion() {
        let line = cfg_from("e", &[("e", "a"), ("a", "b"), ("b", "c"), ("c", "d")]);
        let fused = cfg_from("z", &[]);
        assert!(are_equivalent(&line, &fused));
    }
}
