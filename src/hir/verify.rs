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
                Stmt::Continue => {
                    if let Some(c) = &cur {
                        let j = brk.expect("Continue outside any loop");
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
                    let (te, tx) = self.region(then_stmts, brk, labels);
                    self.edge(&c, &te);
                    let mut join: Option<String> = None;
                    if else_stmts.is_empty() {
                        let j = self.fresh(); // false path continues after the If
                        self.edge(&c, &j);
                        join = Some(j);
                    } else {
                        let (ee, ex) = self.region(else_stmts, brk, labels);
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
                Stmt::Loop { body, .. } => {
                    let Some(c) = cur else { continue };
                    let join = self.fresh(); // false-exit: cond fails → after loop
                    let (be, bx) = self.region(body, Some(&join), labels);
                    self.edge(&c, &be); // cond true → body
                    self.edge(&c, &join); // cond false → after
                    if let Some(x) = bx {
                        self.edge(&x, &c);
                    } // body falls out → header (back edge)
                    cur = Some(join);
                }

                Stmt::Switch { cases, default, .. } => {
                    let Some(c) = cur else { continue };
                    let mut join: Option<String> = None;
                    for (_, body) in cases {
                        let (ae, ax) = self.region(body, brk, labels);
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
                        let (de, dx) = self.region(default, brk, labels);
                        self.edge(&c, &de);
                        if let Some(x) = dx {
                            let j = join.get_or_insert_with(|| self.fresh());
                            self.edge(&x, j);
                        }
                    }
                    cur = join;
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
    let (entry, _) = b.region(body, None, &labels);
    for n in &b.all {
        b.succ.entry(n.clone()).or_default();
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
            if pred.get(&n).map(|p| p.len() != 1).unwrap_or(true) {
                continue; //Can we collapse this down into a .filter() over the inner loop? 
            }
            if succ.get(&n).map(|p| p.len() != 1).unwrap_or(true) {
                continue;
            }

            let p = pred[&n][0].clone();
            let s = succ[&n][0].clone();
            if p == n || s == n || p == s {
                continue;
            } // dont fuse cycles 

            succ.remove(&n);
            pred.remove(&n);
            let sl = succ.entry(p.clone()).or_default();
            if !sl.contains(&s) {
                sl.push(s.clone());
            }
            let ps = pred.entry(s.clone()).or_default();
            ps.retain(|x| x != &n);
            if p != s && !ps.contains(&p) {
                ps.push(p.clone());
            } // s's pred: n → p
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
/*
fn walk(
    stmts: &[Stmt],
    b: &mut ReBuilder,
    mut cur: String,
    brk: Option<&str>,
    cont: Option<&str>,
    labels: &HashMap<String, String>,
) -> Option<String> {
    None
}*/
