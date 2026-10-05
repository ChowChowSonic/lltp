use crate::hir::{Block, Cfg, Expr, Lit, Stmt, Ty};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
pub struct PostDomInfo {
    pub pdom: HashMap<String, BTreeSet<String>>,
    pub ipdom: HashMap<String, String>, //immediate post-dominator
}

const VIRTUAL_SINK_STR: &str = "xXVIRTUAL_SINKXx";
impl From<&Cfg> for PostDomInfo {
    fn from(cfg: &Cfg) -> Self {
        let mut rev_succ: HashMap<String, Vec<String>> = HashMap::new();
        let mut rev_pred: HashMap<String, Vec<String>> = HashMap::new();
        for (name, block) in &cfg.blocks {
            rev_succ.insert(name.clone(), block.pred.clone());
            rev_pred.insert(name.clone(), block.succ.clone());
        }
        rev_succ.insert(VIRTUAL_SINK_STR.to_string(), cfg.exits.clone());
        rev_pred.insert(VIRTUAL_SINK_STR.to_string(), vec![]);

        for exit in &cfg.exits {
            if let Some(preds) = rev_pred.get_mut(exit) {
                preds.push(VIRTUAL_SINK_STR.to_string());
            }
        }
        let rev_cfg = Cfg::new(
            VIRTUAL_SINK_STR.to_string(),
            rev_succ,
            rev_pred,
            vec![cfg.entry.clone()],
        );
        let rev_dom = dominators(&rev_cfg);
        // The virtual sink stays visible: the last block before the exit
        // reports ipdom == sink and its pdom set contains the sink. Only
        // the sink's own entries are excluded.
        let mut ipdom = HashMap::new();
        for (node, parent) in rev_dom.idom {
            if node != VIRTUAL_SINK_STR {
                ipdom.insert(node, parent);
            }
        }
        let mut pdom = HashMap::new();
        for (node, set) in rev_dom.dom {
            if node != VIRTUAL_SINK_STR {
                pdom.insert(node, set);
            }
        }
        PostDomInfo { pdom, ipdom }
    }
}
impl From<DomInfo> for PostDomInfo {
    fn from(f: DomInfo) -> Self {
        Self {
            pdom: f
                .dom
                .into_iter()
                .filter(|(x, _)| *x != VIRTUAL_SINK_STR)
                .collect(),
            ipdom: f
                .idom
                .into_iter()
                .filter(|x| x.0 != VIRTUAL_SINK_STR)
                .collect(),
        }
    }
}
pub struct DomInfo {
    pub dom: HashMap<String, BTreeSet<String>>, // n -> dominators(n), incl. n itself
    pub idom: HashMap<String, String>,          // n -> immediate dominator
}
pub fn keep_reachable(cfg: &Cfg) -> Cfg {
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
impl From<PostDomInfo> for DomInfo {
    fn from(f: PostDomInfo) -> Self {
        Self {
            dom: f.pdom,
            idom: f.ipdom,
        }
    }
}
/// The set of blocks reachable from `src` following successors that stay
/// inside `within`. Includes `src` itself.
pub fn reachable_from(cfg: &Cfg, src: &str, within: &BTreeSet<String>) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut stack = vec![src.to_string()];
    while let Some(n) = stack.pop() {
        if !seen.insert(n.clone()) {
            continue;
        }
        for s in &cfg.blocks[&n].succ {
            if within.contains(s) {
                stack.push(s.clone());
            }
        }
    }
    seen
}

pub fn unify_loop_exits(in_cfg: &Cfg) -> Cfg {
    let mut cfg = in_cfg.clone();
    loop {
        let dom = dominators(&cfg);
        let reach = reachable(&cfg);
        let loops = natural_loops(&cfg, &dom);
        let mut changed = false;
        // collect exit targets not in the body but are reachable
        for nl in loops {
            let mut exits: Vec<(String, String)> = Vec::new();
            let mut distinct_targets: BTreeSet<String> = BTreeSet::new();
            for node in &nl.body {
                for s in &cfg.blocks[node].succ {
                    if !nl.body.contains(s) && reach.contains(s) {
                        exits.push((node.clone(), s.clone()));
                        distinct_targets.insert(s.clone());
                    }
                }
            }
            if exits.len() <= 1 {
                continue;
            }
            changed = true;
            let header = &nl.header;
            let tag_name = format!("_exit_tag_{}", nl.header);
            let unified_name = format!("_unified_exit{}", nl.header);
            let targets: Vec<String> = distinct_targets.into_iter().collect();

            // Create unified exit block
            for (edge_idx, (src, og_dst)) in exits.iter().enumerate() {
                let target_idx = targets.iter().position(|t| t == og_dst).unwrap();
                let pad_name = format!("_exit_pad_{header}_{edge_idx}");
                let pad_stmts = [
                    Stmt::Let {
                        dest: Box::new(Expr::Var {
                            name: tag_name.clone(),
                            dtype: Ty::Int(32, true),
                        }),
                        src: Box::new(Expr::Literal(Lit::Int {
                            value: target_idx as u64,
                            bits: 32,
                            signed: true,
                        })),
                    },
                    Stmt::Branch {
                        cond: None,
                        then_block: unified_name.clone(),
                        else_block: None,
                    },
                ];
                let pad_block = Block {
                    name: pad_name.clone(),
                    stmts: pad_stmts.to_vec(),
                    succ: vec![unified_name.clone()],
                    pred: vec![src.clone()],
                };
                cfg.blocks.insert(pad_name.clone(), pad_block);

                let src_blk = cfg.blocks.get_mut(src).unwrap();
                for s in &mut src_blk.succ {
                    if s == og_dst {
                        *s = pad_name.clone();
                    }
                }
                if let Some(Stmt::Branch {
                    then_block,
                    else_block,
                    ..
                }) = src_blk.stmts.last_mut()
                {
                    if then_block == og_dst {
                        *then_block = pad_name.clone();
                    }
                    if let Some(e1) = else_block
                        && e1 == og_dst
                    {
                        *else_block = Some(pad_name.clone())
                    }
                }
                if let Some(dest_blk) = cfg.blocks.get_mut(og_dst) {
                    dest_blk.pred.retain(|p| p != src);
                }
            }

            let pad_names: Vec<String> = (0..exits.len())
                .map(|i| format!("_exit_pad_{header}_{i}"))
                .collect();
            let mut unified_block = Block::new(&unified_name);
            unified_block.pred = pad_names;

            if targets.len() == 2 {
                let cond = Expr::BinaryOp {
                    op: inkwell::values::InstructionOpcode::ICmp,
                    arg1: Box::new(Expr::Var {
                        name: tag_name.clone(),
                        dtype: Ty::Int(32, true),
                    }),
                    arg2: Box::new(Expr::Literal(Lit::Int {
                        value: 0,
                        bits: 32,
                        signed: true,
                    })),
                };
                unified_block.stmts.push(Stmt::Branch {
                    cond: Some(cond),
                    then_block: targets[0].clone(),
                    else_block: Some(targets[1].clone()),
                });
                unified_block.succ = vec![targets[0].clone(), targets[1].clone()];
            } else {
                let first = targets[0].clone();
                let second = targets[1].clone();
                unified_block.succ = targets.clone();
                unified_block.stmts.push(Stmt::Branch {
                    cond: Some(Expr::Var {
                        name: tag_name.clone(),
                        dtype: Ty::Int(32, true),
                    }),
                    then_block: first,
                    else_block: Some(second),
                });
            }
            for t in &targets {
                if let Some(dest_blk) = cfg.blocks.get_mut(t)
                    && !dest_blk.pred.contains(&unified_name)
                {
                    dest_blk.pred.push(unified_name.clone());
                }
            }
            cfg.blocks.insert(unified_name, unified_block);
        }
        if !changed {
            break;
        }
    }
    cfg.clone()
}

///dom(n) = {n} ∪ ⋂ { dom(p) : p ∈ preds(n) }, iterate until fixpoint
pub fn dominators(cfg: &Cfg) -> DomInfo {
    let reach = reachable(cfg);
    // Seed every node with the full reachable set and iterate downward: the
    // *least* fixed point of the equations (seeding from ∅) is not the
    // dominator relation on cyclic graphs — the intersection over the
    // back-edge preds cuts the path from the entry into the loop, so the
    // header's dominators silently lose `entry` and its loop never appears.
    let mut dom: HashMap<String, BTreeSet<String>> =
        reach.iter().map(|n| (n.clone(), reach.clone())).collect();
    dom.insert(cfg.entry.clone(), BTreeSet::from([cfg.entry.clone()]));
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
    stack.push_back(cfg.entry.clone());
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
            if dom.dom.get(name).is_some_and(|d| d.contains(s)) {
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
        // T1: drop self-edges (T2 merges can create new ones)
        for n in &reach {
            if let Some(s) = succ.get_mut(n) {
                s.remove(n);
            }
        }
        // T2: fold one single-predecessor node into its predecessor
        let mut single: Option<(String, String)> = None;
        for n in &reach {
            if *n == cfg.entry || !succ.contains_key(n) {
                continue;
            }
            let preds: Vec<String> = succ
                .iter()
                .filter(|(k, ss)| k.as_str() != n && ss.contains(n))
                .map(|(k, _)| k.clone())
                .collect();
            if preds.len() == 1 {
                single = Some((preds[0].clone(), n.clone()));
                break;
            }
        }
        let Some((p, n)) = single else { break };
        let out: BTreeSet<String> = succ.remove(&n).unwrap_or_default();
        for s in &out {
            if s != &p && s != &n {
                succ.get_mut(&p).unwrap().insert(s.clone());
            }
        }
        for k in &reach {
            if let Some(set) = succ.get_mut(k) {
                set.remove(&n);
            }
        }
    }
    succ.len() == 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeSet, HashMap};

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

    #[test]
    fn test_pdom_reverse() {
        // Diamond: entry forks to a and b, both re-joining at the sole exit
        // `join`. join lies on every exit path, so it post-dominates the entry
        // and each branch, yet a and b never post-dominate one another.
        {
            let cfg = cfg_from(
                "entry",
                &[("entry", "a"), ("entry", "b"), ("a", "join"), ("b", "join")],
            );
            let info = PostDomInfo::from(&cfg);
            for n in ["entry", "a", "b"] {
                assert!(info.pdom[n].contains("join"));
            }
            assert!(!info.pdom[&"a".to_string()].contains("b"));
            assert!(!info.pdom[&"b".to_string()].contains("a"));
            for n in ["entry", "a", "b"] {
                assert_eq!(info.ipdom.get(n), Some(&"join".to_string()));
            }
            // join is the sole exit, so it post-dominates itself and the sink only.
            assert_eq!(
                *info.pdom.get("join").unwrap(),
                BTreeSet::from(["join".to_string(), "xXVIRTUAL_SINKXx".to_string()])
            );
            assert_eq!(
                info.ipdom.get("join"),
                Some(&"xXVIRTUAL_SINKXx".to_string())
            );
        }

        // Multi-branch: a three-way switch (entry -> a|b|c) that all converge on
        // `mid` then fall through to `done`. Both mid and done post-dominate all
        // branches and the entry; every branch's ipdom is mid, and the ipdom
        // chain is mid -> done -> virtual sink.
        {
            let cfg = cfg_from(
                "entry",
                &[
                    ("entry", "a"),
                    ("entry", "b"),
                    ("entry", "c"),
                    ("a", "mid"),
                    ("b", "mid"),
                    ("c", "mid"),
                    ("mid", "done"),
                ],
            );
            let info = PostDomInfo::from(&cfg);
            for n in ["entry", "a", "b", "c"] {
                assert!(info.pdom[n].contains("mid"));
                assert!(info.pdom[n].contains("done"));
            }
            for n in ["entry", "a", "b", "c"] {
                assert_eq!(info.ipdom.get(n), Some(&"mid".to_string()));
            }
            assert_eq!(info.ipdom.get("mid"), Some(&"done".to_string()));
            assert!(!info.pdom[&"a".to_string()].contains("b"));
            assert!(!info.pdom[&"b".to_string()].contains("c"));
            assert_eq!(
                info.ipdom.get("done"),
                Some(&"xXVIRTUAL_SINKXx".to_string())
            );
        }
    }

    #[test]
    fn idom_in_a_diamond() {
        let cfg = cfg_from(
            "entry",
            &[("entry", "a"), ("entry", "b"), ("a", "join"), ("b", "join")],
        );
        let info = dominators(&cfg);
        // every non-entry node's immediate dominator is the entry
        for n in ["a", "b", "join"] {
            assert_eq!(info.idom.get(n), Some(&"entry".to_string()));
        }
        // every node dominates itself
        for n in ["entry", "a", "b", "join"] {
            assert!(info.dom[n].contains(n));
        }
    }

    #[test]
    fn sccs_on_a_two_node_cycle() {
        let cfg = cfg_from("entry", &[("entry", "a"), ("a", "b"), ("b", "a")]);
        let sccs = sccs(&cfg);
        assert_eq!(sccs.len(), 2);
        assert!(sccs.contains(&vec!["a".to_string(), "b".to_string()]));
        assert!(sccs.contains(&vec!["entry".to_string()]));
    }

    #[test]
    fn reducible_shapes() {
        // self loop: entry → a → a
        assert!(reducible(&cfg_from("entry", &[("entry", "a"), ("a", "a")])));
        // straight-line chain
        assert!(reducible(&cfg_from("entry", &[("entry", "a"), ("a", "b")])));
        // while loop: entry → h, h → {body, exit}, body → h
        assert!(reducible(&cfg_from(
            "entry",
            &[("entry", "h"), ("h", "body"), ("h", "exit"), ("body", "h")]
        )));
        // irreducible: two headers, each entered from outside the cycle
        assert!(!reducible(&cfg_from(
            "entry",
            &[("entry", "a"), ("entry", "b"), ("a", "b"), ("b", "a")]
        )));
    }

    #[test]
    fn natural_loop_of_a_while() {
        let cfg = cfg_from(
            "entry",
            &[("entry", "h"), ("h", "body"), ("h", "exit"), ("body", "h")],
        );
        let dom = dominators(&cfg);
        let loops = natural_loops(&cfg, &dom);
        assert_eq!(loops.len(), 1);
        let l = &loops[0];
        assert_eq!(l.header, "h");
        assert!(
            l.back_edges
                .contains(&("body".to_string(), "h".to_string()))
        );
        assert_eq!(
            l.body,
            BTreeSet::from(["h".to_string(), "body".to_string()])
        );
    }

    #[test]
    fn natural_loop_of_a_self_loop() {
        let cfg = cfg_from("entry", &[("entry", "a"), ("a", "a")]);
        let dom = dominators(&cfg);
        let loops = natural_loops(&cfg, &dom);
        assert_eq!(loops.len(), 1);
        let l = &loops[0];
        assert_eq!(l.header, "a");
        assert!(l.back_edges.contains(&("a".to_string(), "a".to_string())));
        assert_eq!(l.body, BTreeSet::from(["a".to_string()]));
    }

    #[test]
    fn natural_loops_absent_without_cycles() {
        // diamond: no cycle, no loops
        let cfg = cfg_from(
            "entry",
            &[("entry", "a"), ("entry", "b"), ("a", "join"), ("b", "join")],
        );
        let dom = dominators(&cfg);
        assert!(natural_loops(&cfg, &dom).is_empty());
        // irreducible two-header cycle: dominance-based detection finds no
        // single-header loop
        let cfg = cfg_from(
            "entry",
            &[("entry", "a"), ("entry", "b"), ("a", "b"), ("b", "a")],
        );
        let dom = dominators(&cfg);
        assert!(natural_loops(&cfg, &dom).is_empty());
    }
}
