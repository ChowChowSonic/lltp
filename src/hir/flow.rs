use crate::errors::StructurizeError;
use crate::hir::cfg::{can_retarget, retarget_edge};
use crate::hir::{
    Block, Cfg, Expr, Lit, Stmt, Ty,
    op::{Cmp, IntCmp},
    stmt::BranchStmt,
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

/// What an exit edge means, so the pass can tell an edge it may merge from one
/// that is real control flow. Every variant carries the same pair, so the kind
/// is a field rather than five copies of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitEdgeKind {
    Canonical,
    Continue,
    OuterBreak,
    Terminates,
    Divergent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExitEdge {
    pub kind: ExitEdgeKind,
    pub src: String,
    pub target: String,
}

impl ExitEdge {
    pub fn is_divergent(&self) -> bool {
        self.kind == ExitEdgeKind::Divergent
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
    ancestors: &[&NaturalLoop],
    reach: &BTreeSet<String>,
    join: Option<&str>,
) -> Vec<ExitEdge> {
    loop_exit_edges(cfg, l, reach)
        .into_iter()
        .map(|(src, target)| {
            let kind = if Some(target.as_str()) == join {
                ExitEdgeKind::Canonical
            } else if ancestors.iter().any(|a| a.header == target) {
                ExitEdgeKind::Continue
            } else if ancestors.iter().any(|a| a.body.contains(&target)) {
                ExitEdgeKind::OuterBreak
            } else if cfg.blocks[&target].succ.is_empty() {
                ExitEdgeKind::Terminates
            } else {
                ExitEdgeKind::Divergent
            };
            ExitEdge { kind, src, target }
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
/// The dispatch entry sits outside the loop bodies: its predecessors are the exit
/// sources and it cannot reach back into a body, so it becomes the loop's single
/// exit target. That is what lets `LoopTree::build` accept a loop that used to
/// report `MultiExitLoop`.
pub fn normalize_loop_exits(
    in_cfg: &Cfg,
    locals: &mut Vec<(String, Ty)>,
    created: &mut HashSet<String>,
    max_blocks: usize,
) -> Cfg {
    let mut out = in_cfg.clone();
    relink(&mut out);
    // Duplication and dispatch blocks grow the graph, so the budget is the caller's
    // cap, not the iteration count.
    let budget = max_blocks;
    for _ in 0..budget {
        let dom = dominators(&out);
        let pdom = PostDomInfo::from(&out);
        let reach = reachable(&out);
        let mut loops = natural_loops(&out, &dom);
        loops.sort_by_key(|l| (l.body.len(), l.header.clone()));
        let mut changed = false;
        for i in 0..loops.len() {
            let ancestors: Vec<&NaturalLoop> = loops
                .iter()
                .filter(|a| {
                    a.body.len() > loops[i].body.len() && a.body.is_superset(&loops[i].body)
                })
                .collect();
            let join = loop_join(&loops[i], &pdom);
            let edges = classify_exit_edges(&out, &loops[i], &ancestors, &reach, join.as_deref());
            let targets: BTreeSet<String> = edges.iter().map(|e| e.target.clone()).collect();
            if targets.len() <= 1 {
                continue;
            }
            if converge_exits(&mut out, &edges, join.as_ref())
                || dispatch_exit(&mut out, locals, &loops[i], &edges, created)
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

/// Route every entry edge of a cyclic region that is entered at more than one
/// node through a single dispatch block, stamping a tag variable per edge.
///
/// This is an ordering rewrite, not a reducibility fix: the guard chain still
/// enters the region at each node it dispatches to, so a multi-entry cycle stays
/// multi-entry and `split_nodes` remains the pass that turns it into a natural
/// loop. What the dispatch carries is the arm identity, which the graph would
/// otherwise lose.
///
/// `created` is the set of blocks this pass made; a region whose entries all
/// come from those blocks has already been dispatched, so it is not dispatched
/// again. `declined` holds the region bodies whose entry blocks cannot be
/// retargeted. Both are per run, so the loop makes progress on every round: it
/// breaks, records a declined region, or adds blocks.
pub fn normalize_loop_entries(
    in_cfg: &Cfg,
    locals: &mut Vec<(String, Ty)>,
    created: &mut HashSet<String>,
    declined: &mut HashSet<BTreeSet<String>>,
    max_blocks: usize,
) -> Result<Cfg, StructurizeError> {
    let mut out = in_cfg.clone();
    relink(&mut out);
    let budget = max_blocks;

    loop {
        if out.len() > budget {
            return Err(StructurizeError::IrreducibleControlFlow);
        }
        // `find_abnormal` takes the scope the region is considered in separately from
        // the reachable set so a caller can look at a loop body; here the region is
        // the whole graph, so the two are the same set.
        let reach = reachable(&out);
        let Some(region) = find_abnormal(&out, &reach, &reach, created, declined) else {
            break;
        };
        let edges = entry_edges(&region);
        // One dispatch head, one guard per entry target, and one stamp block per
        // arm of an entry block that enters the region at several nodes.
        let est = entry_blocks(&region, &edges);
        if out.len() + est > budget {
            let (block, src) = region
                .entries
                .iter()
                .flat_map(|(t, ps)| ps.iter().map(move |p| (t.clone(), p.clone())))
                .next()
                .unwrap_or((region.header.clone(), String::new()));
            return Err(StructurizeError::AbnormalLoopEntry {
                header: region.header.clone(),
                block,
                src,
            });
        }
        if !dispatch_entry(&mut out, locals, &region, &edges, created) {
            declined.insert(region.body.clone());
        }
    }
    Ok(out)
}

#[derive(Debug, Clone)]
struct EntryRegion {
    body: BTreeSet<String>,
    /// Entry node -> the blocks outside the region that enter it.
    entries: BTreeMap<String, BTreeSet<String>>,
    header: String,
}

/// The same edges read the other way: entry block -> the region nodes it enters.
/// An entry block with more than one arm into the region needs a stamp block per
/// arm, because one terminator can only name one target.
fn entry_edges(region: &EntryRegion) -> BTreeMap<String, BTreeSet<String>> {
    let mut edges: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (node, preds) in &region.entries {
        for p in preds {
            edges.entry(p.clone()).or_default().insert(node.clone());
        }
    }
    edges
}

/// The nodes of `body` entered from outside it. An entry is a predecessor inside
/// the considered scope (`within`) that is not part of the region and is reachable
/// from the graph entry. Blocks this pass created are not entries: they are the
/// dispatch it already built, which is what keeps the caller from re-dispatching
/// the same region.
fn external_entries(
    cfg: &Cfg,
    body: &BTreeSet<String>,
    within: &BTreeSet<String>,
    reach: &BTreeSet<String>,
    created: &HashSet<String>,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut entries: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for x in body {
        for p in &cfg.blocks[x].pred {
            if body.contains(p) || !within.contains(p) || !reach.contains(p) || created.contains(p)
            {
                continue;
            }
            entries.entry(x.clone()).or_default().insert(p.clone());
        }
    }
    entries
}

/// How many blocks `dispatch_entry` adds: the head, one guard per entry target
/// beyond the first, and one stamp block per arm of an entry block that enters
/// the region at several nodes.
fn entry_blocks(region: &EntryRegion, edges: &BTreeMap<String, BTreeSet<String>>) -> usize {
    let guards = region.entries.len().saturating_sub(1);
    let stamps: usize = edges
        .values()
        .map(|xs| if xs.len() > 1 { xs.len() } else { 0 })
        .sum();
    1 + guards + stamps
}

fn dispatch_entry(
    cfg: &mut Cfg,
    locals: &mut Vec<(String, Ty)>,
    region: &EntryRegion,
    edges: &BTreeMap<String, BTreeSet<String>>,
    created: &mut HashSet<String>,
) -> bool {
    let mut targets: Vec<String> = region
        .entries
        .keys()
        .filter(|t| t.as_str() != region.header.as_str())
        .cloned()
        .collect();
    targets.sort_by(|a, b| {
        order_index(cfg, a)
            .cmp(&order_index(cfg, b))
            .then_with(|| a.cmp(b))
    });
    targets.insert(0, region.header.clone());

    // `retarget_edge` has to reach whatever names the target.
    for u in edges.keys() {
        if !can_retarget(cfg.get_block(u.as_str()).unwrap()) {
            return false;
        }
    }

    let tag_ty = Ty::Int(32, true);
    let tag = format!("_entry_tag_{}", region.header);
    if !locals.iter().any(|(n, _)| *n == tag) {
        locals.push((tag.clone(), tag_ty.clone()));
    }

    let m = targets.len();
    let dispatch_head = fresh_block_name(cfg, &format!("_entry_header_{}", region.header));
    let guard_names: Vec<String> = (0..m - 1)
        .map(|i| fresh_block_name(cfg, &format!("_entry_dispatch_{}_{}", region.header, i)))
        .collect();

    created.insert(dispatch_head.clone());
    for name in &guard_names {
        created.insert(name.clone());
    }

    cfg.push_block(Block::build(
        dispatch_head.clone(),
        Vec::new(),
        Some(Stmt::Branch(BranchStmt {
            cond: None,
            then_block: guard_names[0].clone(),
            else_block: None,
        })),
    ));

    for i in 0..m - 1 {
        let otherwise = if i + 1 == m - 1 {
            targets[m - 1].clone()
        } else {
            guard_names[i + 1].clone()
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
        cfg.push_block(Block::build(
            guard_names[i].clone(),
            Vec::new(),
            Some(Stmt::Branch(term)),
        ));
    }

    for (u, xs) in edges.iter().map(|(u, xs)| (u.clone(), xs.clone())) {
        if xs.len() == 1 {
            let x = xs.iter().next().unwrap().clone();
            let idx = targets.iter().position(|t| t == &x).unwrap();
            let blk = cfg.get_block_mut(&u).unwrap();
            insert_before_term(blk, stamp(&tag, &tag_ty, idx));
            retarget_edge(blk, &x, &dispatch_head);
        } else {
            for x in xs {
                let idx = targets.iter().position(|t| t == &x).unwrap();
                let edge_name =
                    fresh_block_name(cfg, &format!("_entry_edge_{}_{}", region.header, idx));
                created.insert(edge_name.clone());
                cfg.push_block(Block::build(
                    edge_name.clone(),
                    vec![stamp(&tag, &tag_ty, idx)],
                    Some(Stmt::Branch(BranchStmt {
                        cond: None,
                        then_block: dispatch_head.clone(),
                        else_block: None,
                    })),
                ));
                // The edge to move is `u -> x`: the stamp block replaces `x` as the
                // arm target, so it is `u`'s statement that has to name it.
                let blk = cfg.get_block_mut(&u).unwrap();
                retarget_edge(blk, &x, &edge_name);
            }
        }
    }

    relink(cfg);
    true
}

/// The nodes of `body` entered from outside it. An entry is a reachable
/// predecessor that is not part of the region; blocks this pass created are not
/// entries, they are the dispatch it already built.
fn internal_in_degree(cfg: &Cfg, body: &BTreeSet<String>, node: &str) -> usize {
    cfg.blocks[node]
        .pred
        .iter()
        .filter_map(|x| {
            if body.contains(x) {
                Some(x.as_str())
            } else {
                None
            }
        })
        .collect::<BTreeSet<&str>>()
        .len()
}
fn order_index(cfg: &Cfg, name: &str) -> usize {
    cfg.order
        .iter()
        .position(|x| x.as_str() == name)
        .unwrap_or(usize::MAX)
}
fn choose_header(
    cfg: &Cfg,
    body: &BTreeSet<String>,
    entries: &BTreeMap<String, BTreeSet<String>>,
) -> String {
    let mut best: Option<String> = None;
    for candidate in entries.keys() {
        let better = match &best {
            None => true,
            Some(current) => {
                let dc = internal_in_degree(cfg, body, current);
                let dn = internal_in_degree(cfg, body, candidate);
                dn > dc
                    || (dn == dc
                        && order_index(cfg, candidate) < order_index(cfg, current)
                        && candidate < current)
            }
        };
        if better {
            best = Some(candidate.clone());
        }
    }
    best.expect("entries is non-empty")
}

fn fresh_block_name(cfg: &Cfg, base: &str) -> String {
    let mut n = 1usize;
    loop {
        let name = format!("{base}_{n}");
        if !cfg.contains_block(&name) {
            return name;
        }
        n += 1;
    }
}
/// The smallest cyclic region entered from more than one node. A region entered
/// from a single node is a natural loop, so the recursion on the region without
/// its entry node is what finds a multi-entry region nested inside a larger
/// single-entry one. Regions already dispatched or declined are skipped, which is
/// what makes the caller's loop make progress.
fn find_abnormal(
    cfg: &Cfg,
    within: &BTreeSet<String>,
    reachable: &BTreeSet<String>,
    created: &HashSet<String>,
    declined: &HashSet<BTreeSet<String>>,
) -> Option<EntryRegion> {
    if within.is_empty() {
        return None;
    }

    let mut c = sccs_within(cfg, within);
    c.sort_by(|a, b| (a.len(), a.first()).cmp(&(b.len(), b.first())));
    for cmp in c {
        let body: BTreeSet<String> = cmp.iter().cloned().collect();
        if declined.contains(&body) {
            continue;
        }
        let cyclic = cmp.len() > 1 || cfg.blocks[&cmp[0]].succ.iter().any(|s| s == &cmp[0]);
        if !cyclic {
            continue;
        }
        let entries = external_entries(cfg, &body, within, reachable, created);
        if entries.len() > 1 {
            let header = choose_header(cfg, &body, &entries);
            return Some(EntryRegion {
                body,
                entries,
                header,
            });
        }

        let entry = if let Some(e) = entries.keys().next() {
            e.clone()
        } else if body.contains(&cfg.entry) {
            cfg.entry.clone()
        } else {
            continue;
        };

        let mut inner = body;
        inner.remove(&entry);
        if let Some(rg) = find_abnormal(cfg, &inner, reachable, created, declined) {
            return Some(rg);
        }
    }
    None
}

/// Rewrite 1. Legal only when each divergent target is a statement-free merge
/// node pointing straight at the join: retargeting then skips nothing, and
/// `collapse_chains` already removes such nodes, so the fingerprint is unchanged.
/// A recovered block always carries its terminator statement, so this premise
/// holds only for graph nodes; in the pipeline rewrite 2 is what actually fires.
/// Every src block also has to name its target in a statement `retarget_edge` can
/// reach, so a `Switch` arm is left to rewrite 2.
fn converge_exits(cfg: &mut Cfg, edges: &[ExitEdge], join: Option<&String>) -> bool {
    let Some(j) = join else { return false };
    let divergent: Vec<&ExitEdge> = edges.iter().filter(|e| e.is_divergent()).collect();
    if divergent.is_empty() {
        return false;
    }
    if !divergent.iter().all(|e| {
        let blk = cfg.get_block(&e.target).unwrap();
        is_merge_only(cfg, &e.target) && blk.succ[0] == *j
    }) {
        return false;
    }
    // `retarget_edge` has to reach whatever names the target.
    if !divergent
        .iter()
        .all(|e| can_retarget(cfg.get_block(&e.src).unwrap()))
    {
        return false;
    }
    for e in &divergent {
        let blk = cfg.get_block_mut(&e.src).unwrap();
        retarget_edge(blk, &e.target, j);
    }
    relink(cfg);
    true
}

/// Rewrite 2. Every exit edge is retargeted to one entry block, and each arm
/// writes its own index before control leaves the loop. A `br` block has at most
/// one exit arm — every body node keeps an in-body successor — so the write placed
/// before its terminator is dead on the other arm and stays correct.
///
/// Two exit arms from one block cannot carry two different values in one `Let`,
/// and a block whose terminator does not name its target — a `Switch`, whose arms
/// are statement lists — cannot be retargeted at all, so such loops are left alone
/// until the arms are materialized as blocks.
fn dispatch_exit(
    cfg: &mut Cfg,
    locals: &mut Vec<(String, Ty)>,
    l: &NaturalLoop,
    edges: &[ExitEdge],
    created: &mut HashSet<String>,
) -> bool {
    let tag_ty = Ty::Int(32, true);
    let tag = format!("_exit_tag_{}", l.header);
    let targets: Vec<String> = edges
        .iter()
        .map(|e| e.target.clone())
        .collect::<BTreeSet<String>>()
        .into_iter()
        .collect();
    if targets.len() <= 1 {
        return false;
    }

    // One write per block: two exit arms from one block cannot be told apart.
    let mut arms: BTreeMap<String, usize> = BTreeMap::new();
    for e in edges {
        *arms.entry(e.src.clone()).or_insert(0) += 1;
    }
    if arms.values().any(|c| *c > 1) {
        return false;
    }
    // `retarget_edge` has to reach whatever names the target.
    if !edges
        .iter()
        .all(|e| can_retarget(cfg.get_block(&e.src).unwrap()))
    {
        return false;
    }
    if !locals.iter().any(|(n, _)| *n == tag) {
        locals.push((tag.clone(), tag_ty.clone()));
    }

    let m = targets.len();
    let base = format!("_unified_exit_{}", l.header);
    let names: Vec<String> = (0..m)
        .map(|i| fresh_block_name(cfg, &format!("{base}_{i}")))
        .collect();
    for name in &names {
        created.insert(name.clone());
    }

    for e in edges {
        let idx = targets.iter().position(|t| t == &e.target).unwrap();
        let blk = cfg.get_block_mut(&e.src).unwrap();
        insert_before_term(blk, stamp(&tag, &tag_ty, idx));
        retarget_edge(blk, &e.target, &names[0]);
    }

    // One guard block per arm, each a `Branch`: `successors_of` gives nothing for
    // a `Switch`, so the edges would have to be maintained by hand. The last target
    // needs no guard of its own, so the chain has `m - 1` blocks.
    for i in 0..m - 1 {
        let otherwise = if i + 1 == m - 1 {
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
        cfg.push_block(Block::build(
            names[i].clone(),
            Vec::new(),
            Some(Stmt::Branch(term)),
        ));
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

/// The write that stamps an arm with its index in the guard chain.
fn stamp(tag: &str, tag_ty: &Ty, idx: usize) -> Stmt {
    Stmt::let_binding(Expr::var(tag, tag_ty.clone()), int_lit(idx))
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
pub fn sccs_within(cfg: &Cfg, within: &BTreeSet<String>) -> Vec<Vec<String>> {
    let mut t = Tarjan {
        cfg,
        reach: within,
        index: 0,
        indices: HashMap::new(),
        lowlink: HashMap::new(),
        onstack: HashSet::new(),
        stack: VecDeque::new(),
        out: Vec::new(),
    };
    for n in within {
        if !t.indices.contains_key(n) {
            t.strongconnect(n);
        }
    }
    t.out
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
    use crate::hir::stmt::{GotoStmt, InvokeStmt, SwitchStmt};
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

    fn lit(value: u64) -> Lit {
        Lit::Int {
            value,
            bits: 32,
            signed: true,
        }
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

    /// The growth cap `structurize_with` shares with the passes.
    fn cap(cfg: &Cfg) -> usize {
        cfg.len().saturating_mul(8).max(64)
    }

    /// The block a dispatch pass created, whatever name `fresh_block_name` gave it.
    fn dispatch_block(cfg: &Cfg, base: &str) -> String {
        cfg.blocks
            .keys()
            .find(|n| n.starts_with(base))
            .expect("dispatch block should exist")
            .clone()
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
        let out = normalize_loop_exits(&cfg, &mut locals, &mut HashSet::new(), cap(&cfg));
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
        let out = normalize_loop_exits(&cfg, &mut locals, &mut HashSet::new(), cap(&cfg));
        let entry = dispatch_block(&out, "_unified_exit_h");
        let dom = dominators(&out);
        let loops = natural_loops(&out, &dom);
        assert_eq!(
            exit_targets(&out, &loops[0]),
            BTreeSet::from([entry.clone()])
        );
        // Two targets, so the entry is the only guard block.
        assert_eq!(
            out.blocks
                .keys()
                .filter(|n| n.starts_with("_unified_exit_h"))
                .count(),
            1
        );
        assert_eq!(
            out.blocks[&entry].succ,
            vec!["e1".to_string(), "e2".to_string()]
        );
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
        let out = normalize_loop_exits(&cfg, &mut locals, &mut HashSet::new(), cap(&cfg));
        let dom = dominators(&out);
        let loops = natural_loops(&out, &dom);
        let entry = dispatch_block(&out, "_unified_exit_h");
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
        let out = normalize_loop_exits(&cfg, &mut locals, &mut HashSet::new(), cap(&cfg));
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
        let ancestors: Vec<&NaturalLoop> = loops
            .iter()
            .filter(|a| a.body.len() > inner.body.len() && a.body.is_superset(&inner.body))
            .collect();
        let join = loop_join(inner, &pdom);
        let edges = classify_exit_edges(&cfg, inner, &ancestors, &reachable(&cfg), join.as_deref());
        assert!(
            edges
                .iter()
                .any(|e| e.kind == ExitEdgeKind::Continue && e.src == "C"),
            "{edges:?}"
        );
        assert!(
            edges
                .iter()
                .any(|e| e.kind == ExitEdgeKind::Divergent && e.target == "ij")
        );

        let mut locals = Vec::new();
        let out = normalize_loop_exits(&cfg, &mut locals, &mut HashSet::new(), cap(&cfg));
        let dom = dominators(&out);
        let loops = natural_loops(&out, &dom);
        let inner = loops.iter().find(|l| l.header == "ih").expect("inner loop");
        let all: BTreeSet<String> = out.blocks.keys().cloned().collect();
        let entry = dispatch_block(&out, "_unified_exit_ih");
        assert_eq!(exit_targets(&out, inner), BTreeSet::from([entry.clone()]));
        // The `continue` arm is carried through the dispatch rather than turned
        // into a break: the entry still reaches the enclosing header.
        assert!(reachable_from(&out, &entry, &all).contains("oh"));
        assert!(structurize(&out).is_ok());
    }

    /// A region entered at two nodes, from one fork that branches to both. The
    /// dispatch stamps each arm, so every entry edge now runs through one head.
    #[test]
    fn multi_entry_region_is_dispatched() {
        let cfg = Cfg::cfg_from(
            "entry",
            &[
                ("entry", "a"),
                ("entry", "b"),
                ("a", "b"),
                ("b", "a"),
                ("b", "done"),
            ],
        );
        let mut locals = Vec::new();
        let mut created = HashSet::new();
        let out = normalize_loop_entries(
            &cfg,
            &mut locals,
            &mut created,
            &mut HashSet::new(),
            cap(&cfg),
        )
        .expect("entry dispatch should report no error");

        // `a` has as many internal predecessors as `b` and comes first in the
        // order, so it is the header the chain dispatches first.
        let head = dispatch_block(&out, "_entry_header_");
        let guard = dispatch_block(&out, "_entry_dispatch_");
        assert!(head.starts_with("_entry_header_a"), "{head}");
        // The fork arms are split into one stamp block each, because one terminator
        // cannot carry two different values.
        let stamps: Vec<String> = out
            .blocks
            .keys()
            .filter(|n| n.starts_with("_entry_edge_"))
            .cloned()
            .collect();
        assert_eq!(stamps.len(), 2);
        for s in &stamps {
            assert_eq!(out.blocks[s].succ, vec![head.clone()]);
            assert!(matches!(out.blocks[s].stmts[0], Stmt::Let(_)), "{s}");
        }
        assert_eq!(out.blocks["entry"].succ, stamps);
        assert_eq!(out.blocks[&head].succ, vec![guard.clone()]);
        assert_eq!(
            out.blocks[&guard].succ,
            vec!["a".to_string(), "b".to_string()]
        );
        assert!(locals.contains(&("_entry_tag_a".to_string(), Ty::Int(32, true))));
        // The region is still entered at both nodes: the dispatch adds an entry edge
        // rather than replacing one, which is why it cannot make a multi-entry cycle
        // single-entry.
        assert_eq!(out.blocks["a"].pred, vec!["b".to_string(), guard.clone()]);
        assert_eq!(out.blocks["b"].pred, vec!["a".to_string(), guard.clone()]);
        assert!(!reducible(&out), "the region stays multi-entry");
    }

    /// The dispatch leaves the entry nodes where they are, so a second run over the
    /// same graph has to see the chain it built as the region's entries and stop.
    /// Without that, the pass re-dispatches the same region every round until the
    /// budget is spent.
    #[test]
    fn dispatched_region_is_not_dispatched_again() {
        let cfg = Cfg::cfg_from(
            "entry",
            &[
                ("entry", "a"),
                ("entry", "b"),
                ("a", "b"),
                ("b", "a"),
                ("b", "done"),
            ],
        );
        let mut locals = Vec::new();
        let mut created = HashSet::new();
        let mut declined = HashSet::new();
        let first =
            normalize_loop_entries(&cfg, &mut locals, &mut created, &mut declined, cap(&cfg))
                .unwrap();
        assert!(
            !first
                .blocks
                .keys()
                .all(|n| !n.starts_with("_entry_header_"))
        );
        let second =
            normalize_loop_entries(&first, &mut locals, &mut created, &mut declined, cap(&cfg))
                .unwrap();
        assert_eq!(second, first);
        assert_eq!(second.len(), first.len());
    }

    /// A region that node splitting can already lift is left alone: the dispatch is
    /// an ordering rewrite, not a reducibility fix.
    #[test]
    fn single_entry_region_is_left_alone() {
        let cfg = Cfg::cfg_from(
            "entry",
            &[("entry", "a"), ("a", "b"), ("b", "a"), ("b", "done")],
        );
        let mut locals = Vec::new();
        let out = normalize_loop_entries(
            &cfg,
            &mut locals,
            &mut HashSet::new(),
            &mut HashSet::new(),
            cap(&cfg),
        )
        .expect("entry dispatch should report no error");
        // One entry node, so nothing to dispatch.
        assert!(out.blocks.keys().all(|n| !n.starts_with("_entry_header_")));
        assert!(locals.is_empty());
        assert_eq!(out.len(), cfg.len());
    }

    /// An entry block whose terminator names its arms only as statement lists
    /// cannot be retargeted, so the region is declined rather than looked at again.
    #[test]
    fn switch_entry_block_is_declined() {
        let mut fork = Block::build(
            "entry",
            vec![],
            Some(Stmt::Switch(SwitchStmt {
                value: flag("v"),
                cases: vec![(lit(0), vec![mark("a")]), (lit(1), vec![mark("b")])],
                default: vec![],
            })),
        );
        // The switch arms are statement lists, so the graph edges come from the
        // operand scan rather than from `successors_of`.
        fork.succ = vec!["a".to_string(), "b".to_string()];
        let cfg = Cfg::assemble(
            "entry",
            vec![
                fork,
                Block::build("a", vec![], Some(br(None, "b", None))),
                Block::build("b", vec![], Some(br(None, "a", None))),
            ],
        );
        let mut locals = Vec::new();
        let mut created = HashSet::new();
        let mut declined = HashSet::new();
        let out = normalize_loop_entries(&cfg, &mut locals, &mut created, &mut declined, cap(&cfg))
            .unwrap();
        assert!(declined.contains(&BTreeSet::from(["a".to_string(), "b".to_string()])));
        assert!(out.blocks.keys().all(|n| !n.starts_with("_entry_header_")));
        assert!(locals.is_empty());
    }

    /// The estimate counts the head, one guard per entry target beyond the first,
    /// and one stamp block per arm of a fork that enters the region at several
    /// nodes.
    #[test]
    fn entry_estimate_counts_the_stamp_blocks() {
        let u = "u".to_string();
        let region = EntryRegion {
            body: BTreeSet::from(["a".to_string(), "b".to_string()]),
            entries: BTreeMap::from([
                ("a".to_string(), BTreeSet::from([u.clone()])),
                ("b".to_string(), BTreeSet::from([u.clone()])),
            ]),
            header: "a".to_string(),
        };
        // head + one guard + two stamps, one per arm of `u`
        assert_eq!(entry_blocks(&region, &entry_edges(&region)), 4);

        // One arm per entry node, so the writes can stay on the entry blocks.
        let one_arm = EntryRegion {
            body: BTreeSet::from(["a".to_string(), "b".to_string()]),
            entries: BTreeMap::from([
                ("a".to_string(), BTreeSet::from(["u1".to_string()])),
                ("b".to_string(), BTreeSet::from(["u2".to_string()])),
            ]),
            header: "a".to_string(),
        };
        // head + one guard, no stamps
        assert_eq!(entry_blocks(&one_arm, &entry_edges(&one_arm)), 2);
    }

    /// The entry pass reports the region it cannot afford to dispatch, rather than
    /// returning a half-rewritten graph for the caller to mistake for a reduced one.
    #[test]
    fn entry_budget_exhaustion_is_reported() {
        let cfg = Cfg::cfg_from(
            "entry",
            &[
                ("entry", "a"),
                ("entry", "b"),
                ("a", "b"),
                ("b", "a"),
                ("b", "done"),
            ],
        );
        // head + one guard + two stamps = 4 blocks on top of a 4-block graph.
        assert_eq!(cfg.len(), 4);
        let err = normalize_loop_entries(
            &cfg,
            &mut Vec::new(),
            &mut HashSet::new(),
            &mut HashSet::new(),
            7,
        )
        .expect_err("the dispatch does not fit the cap");
        assert_eq!(
            err,
            StructurizeError::AbnormalLoopEntry {
                header: "a".to_string(),
                block: "a".to_string(),
                src: "entry".to_string(),
            }
        );
    }

    /// `retarget_edge` has to reach whatever names the target, so a terminator that
    /// names its arm keeps the block's statement and its edges in step.
    #[test]
    fn retargeting_moves_the_arm_that_names_the_target() {
        let mut blk = Block::build(
            "x",
            vec![],
            Some(Stmt::Invoke(InvokeStmt {
                callee: "callee".to_string(),
                args: Vec::new(),
                normal: "old".to_string(),
                catches: vec!["lp".to_string()],
            })),
        );
        retarget_edge(&mut blk, "old", "new");
        assert_eq!(blk.succ, vec!["new".to_string()]);
        let Stmt::Invoke(i) = blk.term() else {
            panic!("the invoke arm was replaced");
        };
        assert_eq!(i.normal, "new".to_string());
        // The exception arm is not a control edge, so it stays where recovery put it.
        assert_eq!(i.catches, vec!["lp".to_string()]);
        assert!(can_retarget(&blk));

        let mut got = Block::build(
            "y",
            vec![],
            Some(Stmt::Goto(GotoStmt {
                target: "old".to_string(),
            })),
        );
        // `successors_of` does not report a `goto`, so the edge lives in the
        // statement alone.
        assert!(got.succ.is_empty());
        retarget_edge(&mut got, "old", "new");
        let Stmt::Goto(g) = got.term() else {
            panic!("the goto target was replaced");
        };
        assert_eq!(g.target, "new".to_string());
        assert!(can_retarget(&got));

        // A `Switch` names nothing, so it cannot be retargeted.
        let sw = Block::build(
            "z",
            vec![],
            Some(Stmt::Switch(SwitchStmt {
                value: flag("v"),
                cases: vec![(lit(0), vec![])],
                default: vec![],
            })),
        );
        assert!(!can_retarget(&sw));
    }
}
