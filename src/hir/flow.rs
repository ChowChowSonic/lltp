use crate::hir::structurize::retarget_edge;
use crate::hir::{
    Block, Cfg, Expr, Lit, Stmt, Ty,
    expr::VarExpr,
    op::{Cmp, IntCmp},
    stmt::{BranchStmt, LetStmt, SwitchStmt},
};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
pub struct PostDomInfo {
    pub pdom: HashMap<String, BTreeSet<String>>,
    pub ipdom: HashMap<String, String>, //immediate post-dominator
}

const VIRTUAL_SINK_STR: &str = "xXVIRTUAL_SINKXx";
impl From<&Cfg> for PostDomInfo {
    fn from(cfg: &Cfg) -> Self {
        let mut rev_succ: HashMap<String, Vec<String>> = HashMap::new();
        for (name, block) in &cfg.blocks {
            rev_succ.insert(name.clone(), block.pred.clone());
        }
        // The virtual sink is the reverse-graph entry; the exits hang off it so
        // that `idom(exit) == sink` in the reverse graph.
        rev_succ.insert(VIRTUAL_SINK_STR.to_string(), cfg.exits.clone());
        let rev_cfg = Cfg::from_edges(VIRTUAL_SINK_STR.to_string(), rev_succ);
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
    let blocks: Vec<Block> = cfg
        .ordered()
        .into_iter()
        .filter(|b| reach.contains(&b.name))
        .cloned()
        .collect();
    Cfg::assemble(cfg.entry.clone(), blocks)
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
pub fn loop_exit_edges(
    cfg: &Cfg,
    l: &NaturalLoop,
    reach: &BTreeSet<String>,
) -> Vec<(String, String)> {
    let mut edges = Vec::new();
    for n in &l.body {
        for s in &cfg.blocks[n].succ {
            if !l.body.contains(s) && reach.contains(s) {
                edges.push((n.clone(), s.clone()));
            }
        }
    }
    edges
}

/// `n` collapses away under `Cfg::collapse_chains`: no statements and one
/// successor, so an exit edge that reaches the join only through such a node is
/// the one case where retargeting is behaviour-preserving.
fn is_merge_only(cfg: &Cfg, n: &str) -> bool {
    cfg.blocks[n].stmts.is_empty() && cfg.blocks[n].succ.len() == 1
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitEdge {
    Canonical { src: String, target: String },
    Continue { src: String, target: String },
    OuterBreak { src: String, target: String },
    Terminates { src: String, target: String },
    Divergent { src: String, target: String },
}

impl ExitEdge {
    pub fn src(&self) -> &str {
        match self {
            ExitEdge::Canonical { src, .. }
            | ExitEdge::Continue { src, .. }
            | ExitEdge::OuterBreak { src, .. }
            | ExitEdge::Terminates { src, .. }
            | ExitEdge::Divergent { src, .. } => src,
        }
    }

    pub fn target(&self) -> &str {
        match self {
            ExitEdge::Canonical { target, .. }
            | ExitEdge::Continue { target, .. }
            | ExitEdge::OuterBreak { target, .. }
            | ExitEdge::Terminates { target, .. }
            | ExitEdge::Divergent { target, .. } => target,
        }
    }

    pub fn is_divergent(&self) -> bool {
        matches!(self, ExitEdge::Divergent { .. })
    }
}

fn relink(cfg: &mut Cfg) {
    complete_order(cfg);
    cfg.link_edges();
}

/// `pred` and `exits` are rebuilt by walking `order`, so a graph whose `order`
/// does not cover every block — which is what happens when a pass adds blocks
/// straight into `blocks` — must be completed before `link_edges` runs, or the
/// missing blocks' edges are dropped.
fn complete_order(cfg: &mut Cfg) {
    let names: Vec<String> = cfg.blocks.keys().cloned().collect();
    for n in names {
        if !cfg.order.contains(&n) {
            cfg.order.push(n);
        }
    }
}

/// The loop's single-exit target: `ipdom(header)`, or `None` when its paths
/// leave through the virtual sink, which is what a loop containing a `return`
/// looks like (`PostDomInfo` keeps the sink as an `ipdom` value).
pub fn loop_join(l: &NaturalLoop, pdom: &PostDomInfo) -> Option<String> {
    pdom.ipdom
        .get(&l.header)
        .filter(|j| *j != VIRTUAL_SINK_STR)
        .cloned()
}

/// `ancestors` are the loops whose bodies strictly contain `l`'s, and `join` is
/// `l`'s single-exit target when it has one. A `Continue` or `OuterBreak` target
/// is real control flow (`continue`, `break` out of an enclosing loop), so it is
/// never a merge candidate.
pub fn classify_exit_edges(
    cfg: &Cfg,
    l: &NaturalLoop,
    ancestrors: &[&NaturalLoop],
    reach: &BTreeSet<String>,
    join: Option<&str>,
) -> Vec<ExitEdge> {
    loop_exit_edges(cfg, l, reach)
        .into_iter()
        .map(|(src, target)| {
            if Some(target.as_str()) == join {
                ExitEdge::Canonical { src, target }
            } else if ancestrors.iter().any(|a| a.header == target) {
                ExitEdge::Continue { src, target }
            } else if ancestrors.iter().any(|a| a.body.contains(&target)) {
                ExitEdge::OuterBreak { src, target }
            } else if cfg.blocks[&target].succ.is_empty() {
                ExitEdge::Terminates { src, target }
            } else {
                ExitEdge::Divergent { src, target }
            }
        })
        .collect()
}

/// Turn every natural loop in `cfg` into a single-exit (single-successor) loop.
///
/// Two rewrites, in order of preference:
/// 1. `converge_exits` retargets a divergent edge to the join, which is only legal
///    when the target is a statement-free merge node pointing straight at the join.
///    It drops nodes, so `are_equivalent` cannot confirm it: `collapse_chains` only
///    fuses a node into its pred when the pred has a single successor, and the fork
///    that makes an exit arm always has two.
/// 2. `dispatch_exit` keeps every target and adds one entry block plus a chain of
///    guards on a tag variable, so the loop gains exactly one exit edge. Each arm
///    still goes to its own target, so a `continue` or `break` arm is carried
///    through the dispatch rather than dropped.
///
/// The dispatch entry sits inside the loop bodies (its predecessors are the exit
/// sources), which is what lets `LoopTree::build` accept a loop that used to
/// report `MultiExitLoop`.
pub fn normalize_loop_exits(in_cfg: &Cfg, locals: &mut Vec<(String, Ty)>) -> Cfg {
    let mut out = in_cfg.clone();
    relink(&mut out);
    // Duplication and dispatch blocks grow the graph, so the budget is the graph
    // size, not the iteration count.
    let budget = in_cfg.len().saturating_mul(8).max(64);
    for _ in 0..budget {
        let dom = dominators(&out);
        let pdom = PostDomInfo::from(&out);
        let reach = reachable(&out);
        let mut loops = natural_loops(&out, &dom);
        loops.sort_by_key(|l| (l.body.len(), l.header.clone()));
        let mut changed = false;
        for i in 0..loops.len() {
            let ancestrors: Vec<&NaturalLoop> = loops
                .iter()
                .filter(|a| {
                    a.body.len() > loops[i].body.len() && a.body.is_superset(&loops[i].body)
                })
                .collect();
            let join = loop_join(&loops[i], &pdom);
            let edges = classify_exit_edges(&out, &loops[i], &ancestrors, &reach, join.as_deref());
            let targets: BTreeSet<String> = edges.iter().map(|e| e.target().to_string()).collect();
            if targets.len() <= 1 {
                continue;
            }
            if converge_exits(&mut out, &edges, join.as_ref())
                || dispatch_exit(&mut out, locals, &loops[i], &edges)
            {
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    relink(&mut out);
    out
}

/// Rewrite 1. Legal only when each divergent target is a statement-free merge
/// node pointing straight at the join: retargeting then skips nothing, and
/// `collapse_chains` already removes such nodes, so the fingerprint is unchanged.
fn converge_exits(cfg: &mut Cfg, edges: &[ExitEdge], join: Option<&String>) -> bool {
    let Some(j) = join else { return false };
    let divergent: Vec<&ExitEdge> = edges.iter().filter(|e| e.is_divergent()).collect();
    if divergent.is_empty() {
        return false;
    }
    if !divergent.iter().all(|e| {
        let blk = &cfg.blocks[e.target()];
        is_merge_only(cfg, e.target()) && blk.succ[0] == *j
    }) {
        return false;
    }
    for e in &divergent {
        let blk = cfg.get_block_mut(e.src()).unwrap();
        retarget_edge(blk, e.target(), j);
    }
    relink(cfg);
    true
}

/// Rewrite 2. Every exit edge is retargeted to one entry block, and each arm
/// writes its own index before control leaves the loop. A `br` block has at most
/// one exit arm — every body node keeps an in-body successor — so the write placed
/// before its terminator is dead on the other arm and stays correct.
// shortcut: a `switch` block with two arms leaving the loop cannot carry two
// different values in one `Let`, and its arms are statement lists rather than
// blocks, so such loops are left alone until the arms are materialized as blocks.
fn dispatch_exit(
    cfg: &mut Cfg,
    locals: &mut Vec<(String, Ty)>,
    l: &NaturalLoop,
    edges: &[ExitEdge],
) -> bool {
    let tag_ty = Ty::Int(32, true);
    let tag = format!("_exit_tag_{}", l.header);
    let entry = format!("_unified_exit{}", l.header);
    let targets: Vec<String> = edges
        .iter()
        .map(|e| e.target().to_string())
        .collect::<BTreeSet<String>>()
        .into_iter()
        .collect();
    if targets.len() <= 1 {
        return false;
    }

    // One write per block: two exit arms from one block cannot be told apart.
    let mut arms: BTreeMap<String, usize> = BTreeMap::new();
    for e in edges {
        *arms.entry(e.src().to_string()).or_insert(0) += 1;
    }
    if arms.values().any(|c| *c > 1) {
        return false;
    }
    if !locals.iter().any(|(n, _)| *n == tag) {
        locals.push((tag.clone(), tag_ty.clone()));
    }

    for e in edges {
        let idx = targets.iter().position(|t| t == e.target()).unwrap();
        let blk = cfg.get_block_mut(e.src()).unwrap();
        insert_before_term(
            blk,
            Stmt::let_binding(Expr::var(&tag, tag_ty.clone()), int_lit(idx)),
        );
        retarget_edge(blk, e.target(), &entry);
    }

    // One guard block per arm, each a `Branch`: `successors_of` gives nothing for
    // a `Switch`, so the edges would have to be maintained by hand. The last target
    // needs no guard of its own, so the chain has `targets.len() - 1` blocks.
    let names: Vec<String> = (0..targets.len() - 1)
        .map(|i| {
            if i == 0 {
                entry.clone()
            } else {
                format!("{entry}_{i}")
            }
        })
        .collect();
    for (i, name) in names.iter().enumerate() {
        let otherwise = if i + 1 == targets.len() - 1 {
            targets[i + 1].clone()
        } else {
            names[i + 1].clone()
        };
        let term = BranchStmt {
            cond: Some(Expr::cmp(
                Cmp::Int(IntCmp::Eq),
                Expr::var(&tag, tag_ty.clone()),
                int_lit(i),
            )),
            then_block: targets[i].clone(),
            else_block: Some(otherwise),
        };
        cfg.push_block(Block::build(name, Vec::new(), Some(Stmt::Branch(term))));
    }
    relink(cfg);
    true
}

/// Keep the terminator last, as `Block::term()` requires. On a block with no
/// terminator statement the write becomes the whole block, which is the graph-node
/// case (`Block::build` records `succ` from its terminator, so such a node only
/// exists in hand-built graphs).
fn insert_before_term(blk: &mut Block, stmt: Stmt) {
    let pos = if blk.stmts.is_empty() {
        0
    } else {
        blk.stmts.len() - 1
    };
    blk.stmts.insert(pos, stmt);
}

fn int_lit(value: usize) -> Expr {
    Expr::Literal(Lit::Int {
        value: value as u64,
        bits: 32,
        signed: true,
    })
}

pub fn unify_loop_exits(in_cfg: &Cfg) -> Cfg {
    let mut cfg = in_cfg.clone();
    loop {
        let mut changed = false;
        let dom = dominators(&cfg);
        let reach = reachable(&cfg);
        let loops = natural_loops(&cfg, &dom);
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
            if distinct_targets.len() <= 1 {
                continue;
            }
            changed = true;
            let tag_name = format!("_exit_tag_{}", nl.header);
            let unified_name = format!("_unified_exit{}", nl.header);
            let targets: Vec<String> = distinct_targets.into_iter().collect();

            // Create unified exit block
            for (src, og_dst) in exits.iter() {
                let target_idx = targets.iter().position(|t| t == og_dst).unwrap();
                let src_blk = cfg.blocks.get_mut(src).unwrap();

                let insert_pos = src_blk.stmts.len().saturating_sub(1);
                src_blk.stmts.insert(
                    insert_pos,
                    Stmt::Let(LetStmt {
                        dest: Box::new(Expr::Var(VarExpr {
                            name: tag_name.clone(),
                            dtype: Ty::Int(32, true),
                        })),
                        src: Box::new(Expr::Literal(Lit::Int {
                            value: target_idx as u64,
                            bits: 32,
                            signed: true,
                        })),
                    }),
                );
                for s in &mut src_blk.succ {
                    if s == og_dst {
                        *s = unified_name.clone();
                    }
                }
                if let Some(Stmt::Branch(BranchStmt {
                    then_block,
                    else_block,
                    ..
                })) = src_blk.stmts.last_mut()
                {
                    if then_block == og_dst {
                        *then_block = unified_name.clone();
                    }
                    if let Some(e1) = else_block
                        && e1 == og_dst
                    {
                        *else_block = Some(unified_name.clone())
                    }
                }
                if let Some(dest_blk) = cfg.blocks.get_mut(og_dst) {
                    dest_blk.pred.retain(|p| p != src);
                }
            }

            let mut preds = BTreeSet::new();
            for (src, _) in &exits {
                preds.insert(src.clone());
            }
            let mut unified_block = Block::new(&unified_name);
            unified_block.pred = preds.into_iter().collect();

            if targets.len() == 2 {
                let cond = Expr::cmp(
                    Cmp::Int(IntCmp::Eq),
                    Expr::Var(VarExpr {
                        name: tag_name.clone(),
                        dtype: Ty::Int(32, true),
                    }),
                    Expr::Literal(Lit::Int {
                        value: 0,
                        bits: 32,
                        signed: true,
                    }),
                );
                unified_block.stmts.push(Stmt::Branch(BranchStmt {
                    cond: Some(cond),
                    then_block: targets[0].clone(),
                    else_block: Some(targets[1].clone()),
                }));
                unified_block.succ = vec![targets[0].clone(), targets[1].clone()];
            } else {
                unified_block.succ = targets.clone();
                unified_block.stmts.push(Stmt::Switch(SwitchStmt {
                    value: Expr::Var(VarExpr {
                        name: tag_name.clone(),
                        dtype: Ty::Int(32, true),
                    }),
                    cases: targets
                        .clone()
                        .into_iter()
                        .enumerate()
                        .map(|(pos, v)| {
                            (
                                Lit::Int {
                                    value: pos as u64,
                                    bits: 32,
                                    signed: true,
                                },
                                cfg.blocks[&v].stmts.clone(),
                            )
                        })
                        .collect(),
                    default: Vec::new(),
                }));
            }
            for t in &targets {
                if let Some(dest_blk) = cfg.blocks.get_mut(t)
                    && !dest_blk.pred.contains(&unified_name)
                {
                    dest_blk.pred.push(unified_name.clone());
                }
            }
            cfg.push_block(unified_block);
            break;
        }
        if !changed {
            break;
        }
    }
    cfg
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
/// Temporarily deprecated while we get the codebase online
//#[deprecated]
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

/// T1/T2 reduction of `cfg`: repeatedly drop self-edges (T1) and fold nodes
/// with a single predecessor into it (T2), until nothing more reduces.
/// Returns the residual successor map — a single node means the graph is
/// reducible; more than one means cycles remain that no single header
/// dominates (the irreducible residue).
pub(crate) fn t1t2_reduce(cfg: &Cfg) -> HashMap<String, BTreeSet<String>> {
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
    succ
}

pub fn reducible(cfg: &Cfg) -> bool {
    t1t2_reduce(cfg).len() == 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn test_pdom_reverse() {
        // Diamond: entry forks to a and b, both re-joining at the sole exit
        // `join`. join lies on every exit path, so it post-dominates the entry
        // and each branch, yet a and b never post-dominate one another.
        {
            let cfg = Cfg::cfg_from(
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
            let cfg = Cfg::cfg_from(
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
        let cfg = Cfg::cfg_from(
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
        let cfg = Cfg::cfg_from("entry", &[("entry", "a"), ("a", "b"), ("b", "a")]);
        let sccs = sccs(&cfg);
        assert_eq!(sccs.len(), 2);
        assert!(sccs.contains(&vec!["a".to_string(), "b".to_string()]));
        assert!(sccs.contains(&vec!["entry".to_string()]));
    }

    #[test]
    fn reducible_shapes() {
        // self loop: entry → a → a
        assert!(reducible(&Cfg::cfg_from(
            "entry",
            &[("entry", "a"), ("a", "a")]
        )));
        // straight-line chain
        assert!(reducible(&Cfg::cfg_from(
            "entry",
            &[("entry", "a"), ("a", "b")]
        )));
        // while loop: entry → h, h → {body, exit}, body → h
        assert!(reducible(&Cfg::cfg_from(
            "entry",
            &[("entry", "h"), ("h", "body"), ("h", "exit"), ("body", "h")]
        )));
        // irreducible: two headers, each entered from outside the cycle
        assert!(!reducible(&Cfg::cfg_from(
            "entry",
            &[("entry", "a"), ("entry", "b"), ("a", "b"), ("b", "a")]
        )));
    }

    #[test]
    fn natural_loop_of_a_while() {
        let cfg = Cfg::cfg_from(
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
        let cfg = Cfg::cfg_from("entry", &[("entry", "a"), ("a", "a")]);
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
        let cfg = Cfg::cfg_from(
            "entry",
            &[("entry", "a"), ("entry", "b"), ("a", "join"), ("b", "join")],
        );
        let dom = dominators(&cfg);
        assert!(natural_loops(&cfg, &dom).is_empty());
        // irreducible two-header cycle: dominance-based detection finds no
        // single-header loop
        let cfg = Cfg::cfg_from(
            "entry",
            &[("entry", "a"), ("entry", "b"), ("a", "b"), ("b", "a")],
        );
        let dom = dominators(&cfg);
        assert!(natural_loops(&cfg, &dom).is_empty());
    }

    use crate::hir::structurize::structurize;

    fn flag(name: &str) -> Expr {
        Expr::var(name, Ty::Bool)
    }

    fn mark(name: &str) -> Stmt {
        Stmt::Eval(flag(name))
    }

    fn br(cond: Option<Expr>, then: &str, otherwise: Option<&str>) -> Stmt {
        Stmt::Branch(BranchStmt {
            cond,
            then_block: then.to_string(),
            else_block: otherwise.map(str::to_string),
        })
    }

    fn exit_targets(cfg: &Cfg, l: &NaturalLoop) -> BTreeSet<String> {
        loop_exit_edges(cfg, l, &reachable(cfg))
            .iter()
            .map(|(_, t)| t.clone())
            .collect()
    }

    /// `e1` and `e2` are statement-free merge nodes that both meet at `join`, so
    /// retargeting the two exit edges to `join` skips nothing.
    #[test]
    fn merge_only_exits_converge_to_the_join() {
        let cfg = Cfg::cfg_from(
            "entry",
            &[
                ("entry", "h"),
                ("h", "b"),
                ("h", "e1"),
                ("b", "h"),
                ("b", "e2"),
                ("e1", "join"),
                ("e2", "join"),
                ("join", "done"),
            ],
        );
        let mut locals = Vec::new();
        let out = normalize_loop_exits(&cfg, &mut locals);
        let dom = dominators(&out);
        let loops = natural_loops(&out, &dom);
        assert_eq!(
            exit_targets(&out, &loops[0]),
            BTreeSet::from(["join".to_string()])
        );
        let dropped: BTreeSet<String> = reachable(&cfg)
            .difference(&reachable(&out))
            .cloned()
            .collect();
        assert_eq!(
            dropped,
            BTreeSet::from(["e1".to_string(), "e2".to_string()])
        );
        assert!(
            dropped
                .iter()
                .all(|n| is_merge_only(&cfg, n) && cfg.blocks[n].succ[0] == "join")
        );
        // `are_equivalent` cannot confirm this one: `collapse_chains` only fuses a
        // node into its pred when the pred has a single successor, and the fork
        // that creates an exit arm always has two. So the soundness claim is the
        // guard's premise itself — the dropped nodes executed nothing — and the
        // check that remains is that the tree still structures the result.
        assert!(structurize(&out).is_ok());
    }

    /// Both targets carry statements, so nothing can be converged: the exit
    /// becomes one entry block plus a guard chain on the tag, and the loop has a
    /// single exit edge.
    #[test]
    fn statementful_exits_get_a_dispatch_block() {
        let cfg = Cfg::assemble(
            "entry",
            vec![
                Block::build("entry", vec![], Some(br(None, "h", None))),
                Block::build("h", vec![], Some(br(Some(flag("c")), "b", Some("e1")))),
                Block::build("b", vec![], Some(br(Some(flag("d")), "e2", Some("h")))),
                Block::build("e1", vec![mark("e1")], Some(Stmt::ret_void())),
                Block::build("e2", vec![mark("e2")], Some(Stmt::ret_void())),
            ],
        );
        let mut locals = Vec::new();
        let out = normalize_loop_exits(&cfg, &mut locals);
        let entry = "_unified_exith".to_string();
        let dom = dominators(&out);
        let loops = natural_loops(&out, &dom);
        assert_eq!(
            exit_targets(&out, &loops[0]),
            BTreeSet::from([entry.clone()])
        );
        // Two targets, so the entry is the only guard block.
        assert_eq!(
            out.blocks[&entry].succ,
            vec!["e1".to_string(), "e2".to_string()]
        );
        assert!(out.blocks.keys().all(|n| !n.contains("_1")));
        // The write is per arm, so both paths still reach their own target.
        assert!(out.blocks["h"].succ.contains(&entry));
        assert!(out.blocks["b"].succ.contains(&entry));
        assert!(matches!(out.blocks["h"].stmts[0], Stmt::Let(_)));
        assert_eq!(out.blocks["e1"].pred, vec![entry.clone()]);
        assert_eq!(out.blocks["e2"].pred, vec![entry.clone()]);
        assert!(locals.contains(&("_exit_tag_h".to_string(), Ty::Int(32, true))));
    }

    /// `join` carries statements, so nothing can be converged; the `return` arm is
    /// a `Terminates` target and is dispatched alongside it.
    #[test]
    fn loop_with_a_return_gains_one_exit_edge() {
        let cfg = Cfg::assemble(
            "entry",
            vec![
                Block::build("entry", vec![], Some(br(None, "h", None))),
                Block::build("h", vec![], Some(br(Some(flag("c")), "b", Some("join")))),
                Block::build("b", vec![], Some(br(Some(flag("d")), "ret", Some("h")))),
                Block::build("join", vec![mark("join")], Some(br(None, "ret", None))),
                Block::build("ret", vec![], Some(Stmt::ret_void())),
            ],
        );
        let mut locals = Vec::new();
        let out = normalize_loop_exits(&cfg, &mut locals);
        let dom = dominators(&out);
        let loops = natural_loops(&out, &dom);
        let entry = "_unified_exith".to_string();
        assert_eq!(
            exit_targets(&out, &loops[0]),
            BTreeSet::from([entry.clone()])
        );
        assert_eq!(
            out.blocks[&entry].succ,
            vec!["join".to_string(), "ret".to_string()]
        );
        assert!(locals.contains(&("_exit_tag_h".to_string(), Ty::Int(32, true))));
        assert!(structurize(&out).is_ok());
    }

    /// A `switch` inside the loop with two arms leaving it cannot be dispatched:
    /// one `Let` cannot stamp two arms with different values, and the switch arms
    /// are statement lists rather than blocks.
    #[test]
    fn two_exit_arms_from_one_switch_block_are_left_alone() {
        let cfg = Cfg::cfg_from(
            "entry",
            &[
                ("entry", "h"),
                ("h", "b"),
                ("h", "e1"),
                ("h", "e2"),
                ("b", "h"),
                ("e1", "j1"),
                ("e2", "j2"),
                ("j1", "done"),
                ("j2", "done"),
            ],
        );
        let mut locals = Vec::new();
        let out = normalize_loop_exits(&cfg, &mut locals);
        let dom = dominators(&out);
        let loops = natural_loops(&out, &dom);
        assert_eq!(
            exit_targets(&out, &loops[0]),
            BTreeSet::from(["e1".to_string(), "e2".to_string()])
        );
        assert!(out.blocks.keys().all(|n| !n.starts_with("_unified_exit")));
        assert!(locals.is_empty());
    }

    /// A `continue` edge is a frame boundary, so the pass cannot converge it, but
    /// the dispatch still carries it: the arm keeps its own target.
    #[test]
    fn classification_keeps_the_continue_edge() {
        let cfg = Cfg::cfg_from(
            "entry",
            &[
                ("entry", "oh"),
                ("oh", "B"),
                ("oh", "oj"),
                ("B", "ih"),
                ("B", "oh"),
                ("ih", "C"),
                ("ih", "ij"),
                ("C", "ih"),
                ("C", "oh"),
                ("ij", "done"),
                ("oj", "done"),
            ],
        );
        let dom = dominators(&cfg);
        let pdom = PostDomInfo::from(&cfg);
        let loops = natural_loops(&cfg, &dom);
        let inner = loops.iter().find(|l| l.header == "ih").expect("inner loop");
        let ancestrors: Vec<&NaturalLoop> = loops
            .iter()
            .filter(|a| a.body.len() > inner.body.len() && a.body.is_superset(&inner.body))
            .collect();
        let join = loop_join(inner, &pdom);
        let edges =
            classify_exit_edges(&cfg, inner, &ancestrors, &reachable(&cfg), join.as_deref());
        assert!(
            edges
                .iter()
                .any(|e| matches!(e, ExitEdge::Continue { .. }) && e.src() == "C"),
            "{edges:?}"
        );
        assert!(
            edges
                .iter()
                .any(|e| matches!(e, ExitEdge::Divergent { .. }) && e.target() == "ij")
        );

        let mut locals = Vec::new();
        let out = normalize_loop_exits(&cfg, &mut locals);
        let dom = dominators(&out);
        let loops = natural_loops(&out, &dom);
        let inner = loops.iter().find(|l| l.header == "ih").expect("inner loop");
        let all: BTreeSet<String> = out.blocks.keys().cloned().collect();
        let entry = "_unified_exitih".to_string();
        assert_eq!(exit_targets(&out, inner), BTreeSet::from([entry.clone()]));
        // The `continue` arm is carried through the dispatch rather than turned
        // into a break: the entry still reaches the enclosing header.
        assert!(reachable_from(&out, &entry, &all).contains("oh"));
        assert!(structurize(&out).is_ok());
    }
}
