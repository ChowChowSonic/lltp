use crate::hir::cfg::Cfg;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
pub struct DomInfo {
    pub dom: HashMap<String, BTreeSet<String>>, // n -> dominators(n), incl. n itself
    pub idom: HashMap<String, String>,          // n -> immediate dominator
}
///dom(n) = {n} ∪ ⋂ { dom(p) : p ∈ preds(n) }, iterate until fixpoint
pub fn dominators(cfg: &Cfg) -> DomInfo {
    let reach = reachable(cfg);
    let mut dom: HashMap<String, BTreeSet<String>> =
        reach.iter().map(|n| (n.clone(), BTreeSet::new())).collect();
    dom.get_mut(&cfg.entry).unwrap().insert(cfg.entry.clone());
    loop {
        let mut changed = false;
        for n in &reach {
            if *n == cfg.entry {
                continue;
            }
            let mut new: BTreeSet<String> = BTreeSet::new();
            new.insert(n.clone());
            let blk = &cfg.blocks[n];
            let mut inter: Option<BTreeSet<String>> = None;
            for p in &blk.pred {
                if let Some(d) = dom.get(p) {
                    inter = Some(match inter {
                        None => d.clone(),
                        Some(i) => i.intersection(d).cloned().collect(),
                    });
                }
            }
            if let Some(i) = inter {
                new.extend(i);
            }
            let slot = dom.get_mut(n).unwrap();
            if *slot != new {
                changed = true;
                *slot = new;
            }
        }
        if !changed {
            break;
        }
    }
    let mut idom = HashMap::new();
    for n in &reach {
        if *n == cfg.entry {
            continue;
        }
        let dset = &dom[n];
        let mut best: Option<String> = None;
        for c in dset.iter().filter(|x| *x != n) {
            let deeper = match &best {
                None => true,
                Some(b) => dom[c].len() > dom[b].len(),
            };
            if deeper {
                best = Some(c.clone());
            }
        }
        if let Some(b) = best {
            idom.insert(n.clone(), b);
        }
    }
    DomInfo { dom, idom }
}

struct Tarjan<'a> {
    cfg: &'a Cfg,
    reach: &'a BTreeSet<String>,
    index: usize,
    indices: HashMap<String, usize>,
    lowlink: HashMap<String, usize>,
    onstack: HashSet<String>,
    stack: VecDeque<String>,
    out: Vec<Vec<String>>,
}
impl Tarjan<'_> {
    fn strongconnect(&mut self, n: &str) {
        self.indices.insert(n.to_string(), self.index);
        self.lowlink.insert(n.to_string(), self.index);
        self.index += 1;
        self.stack.push_back(n.to_string());
        self.onstack.insert(n.to_string());
        for s in &self.cfg.blocks[n].succ {
            if !self.reach.contains(s) {
                continue;
            }
            if !self.indices.contains_key(s) {
                self.strongconnect(s);
                let l = self.lowlink[n].min(self.lowlink[s]);
                self.lowlink.insert(n.to_string(), l);
            } else if self.onstack.contains(s) {
                let l = self.lowlink[n].min(self.indices[s]);
                self.lowlink.insert(n.to_string(), l);
            }
        }
        if self.lowlink[n] == self.indices[n] {
            let mut comp = Vec::new();
            loop {
                let w = self.stack.pop_back().unwrap();
                self.onstack.remove(&w);
                if w == n {
                    comp.push(w);
                    break;
                };
                comp.push(w);
            }
            comp.sort();
            self.out.push(comp);
        }
    }
}
pub fn sccs(cfg: &Cfg) -> Vec<Vec<String>> {
    let reach = reachable(cfg);
    let mut t = Tarjan {
        cfg,
        reach: &reach,
        index: 0,
        indices: HashMap::new(),
        lowlink: HashMap::new(),
        onstack: HashSet::new(),
        stack: VecDeque::new(),
        out: Vec::new(),
    };
    for n in &reach {
        if !t.indices.contains_key(n) {
            t.strongconnect(n);
        }
    }
    t.out
}

pub fn reachable(cfg: &Cfg) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut stack: VecDeque<String> = VecDeque::new();
    while let Some(n) = stack.pop_front() {
        if seen.insert(n.clone()) {
            for s in &cfg.blocks[&n].succ {
                if !seen.contains(s) {
                    stack.push_back(s.clone());
                }
            }
        }
    }
    seen
}

pub struct NaturalLoop {
    pub header: String,
    pub back_edges: Vec<(String, String)>, // (source, header), self-loop included
    pub body: BTreeSet<String>,
}
pub fn natural_loops(cfg: &Cfg, dom: &DomInfo) -> Vec<NaturalLoop> {
    #[allow(clippy::type_complexity)]
    let mut by_header: BTreeMap<String, (Vec<(String, String)>, BTreeSet<String>)> =
        BTreeMap::new();
    for (name, blk) in &cfg.blocks {
        if !dom.dom.contains_key(name) {
            continue;
        }
        for s in &blk.succ {
            if dom.dom.get(s).is_some_and(|d| d.contains(name)) {
                by_header
                    .entry(s.clone())
                    .or_default()
                    .0
                    .push((name.clone(), s.clone()));
            }
        }
    }
    for (header, (edges, body)) in &mut by_header {
        body.insert(header.clone());
        for (a, _h) in edges {
            let mut seen = HashSet::new();
            let mut stack: VecDeque<String> = VecDeque::new();
            stack.push_back(a.clone());
            while let Some(x) = stack.pop_back() {
                if x == *header {
                    continue;
                }
                if seen.insert(x.clone()) {
                    body.insert(x.clone());
                    for p in &cfg.blocks[&x].pred {
                        if !seen.contains(p) {
                            stack.push_back(p.clone());
                        }
                    }
                }
            }
        }
    }
    by_header
        .into_iter()
        .map(|(header, (back_edges, body))| NaturalLoop {
            header,
            back_edges,
            body,
        })
        .collect()
}
pub fn reducible(cfg: &Cfg) -> bool {
    let reach = reachable(cfg);
    let mut succ: HashMap<String, BTreeSet<String>> =
        reach.iter().map(|n| (n.clone(), BTreeSet::new())).collect();
    for n in &reach {
        for s in &cfg.blocks[n].succ {
            if reach.contains(s) {
                succ.get_mut(n).unwrap().insert(s.clone());
            }
        }
    }
    loop {
        let mut changed = false;
        for n in &reach {
            if succ.get_mut(n).unwrap().remove(n) {
                changed = true;
            }
        }
        let mut merges: Vec<(String, String)> = Vec::new();
        for n in &reach {
            if *n == cfg.entry {
                continue;
            }
            let preds: Vec<String> = succ
                .iter()
                .filter(|(k, ss)| k.as_str() != n && ss.contains(n))
                .map(|(k, _)| k.clone())
                .collect();
            if preds.len() == 1 {
                merges.push((preds[0].clone(), n.clone()));
            }
        }
        for (p, n) in merges {
            let out: Vec<String> = succ.get(&n).unwrap().iter().cloned().collect();
            succ.remove(&n);
            for s in out {
                if s != p && n != s {
                    succ.get_mut(&p).unwrap().insert(s);
                }
            }
            for k in &reach {
                if let Some(set) = succ.get_mut(k) {
                    set.remove(&n);
                }
            }
            changed = true;
        }
        if !changed {
            break;
        }
    }
    false
}
