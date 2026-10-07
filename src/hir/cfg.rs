use std::collections::{BTreeMap, HashMap};

use crate::hir::{BranchInfo, Stmt, StructurizeError};

#[derive(Default, Debug, Clone, PartialEq)]
pub struct Block {
    pub name: String,
    pub stmts: Vec<Stmt>,
    pub succ: Vec<String>,
    pub pred: Vec<String>,
}

impl Block {
    pub fn new(name: impl Into<String>) -> Self {
        Block {
            name: name.into(),
            stmts: Vec::new(),
            succ: Vec::new(),
            pred: Vec::new(),
        }
    }
}

#[derive(Default, Debug, Clone, PartialEq)]
pub struct Cfg {
    pub entry: String,
    pub blocks: BTreeMap<String, Block>,
    pub exits: Vec<String>,
}

impl Cfg {
    pub fn new(
        entry: String,
        successors: HashMap<String, Vec<String>>,
        preds: HashMap<String, Vec<String>>,
        exits: Vec<String>,
    ) -> Self {
        let mut blocks: BTreeMap<String, Block> = BTreeMap::new();
        for (k, v) in successors {
            let blk = blocks.entry(k.clone()).or_insert_with(|| Block::new(&k));
            blk.succ.extend(v);
        }
        for (k, v) in preds {
            let blk = blocks.entry(k.clone()).or_insert_with(|| Block::new(&k));
            blk.pred.extend(v);
        }
        Cfg {
            entry,
            blocks,
            exits,
        }
    }

    /// Construct an empty CFG with the given entry block name.
    pub fn empty(entry: impl Into<String>) -> Self {
        let entry_str = entry.into();
        let mut blocks = BTreeMap::new();
        blocks.insert(entry_str.clone(), Block::new(&entry_str));
        Cfg {
            entry: entry_str,
            blocks,
            exits: Vec::new(),
        }
    }

    /// Number of blocks in this CFG.
    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    /// Whether this CFG is empty.
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// Check if a block with `name` exists in this CFG.
    pub fn contains_block(&self, name: &str) -> bool {
        self.blocks.contains_key(name)
    }

    /// Get a block by name.
    pub fn get_block(&self, name: &str) -> Option<&Block> {
        self.blocks.get(name)
    }

    /// Get a mutable block by name.
    pub fn get_block_mut(&mut self, name: &str) -> Option<&mut Block> {
        self.blocks.get_mut(name)
    }

    /// Keep only blocks reachable from the entry block.
    pub fn keep_reachable(&self) -> Cfg {
        crate::hir::flow::keep_reachable(self)
    }

    /// Check whether this control flow graph is reducible.
    pub fn is_reducible(&self) -> bool {
        crate::hir::flow::reducible(self)
    }

    /// Attempt to structure this CFG into a goto-free HIR statement tree.
    pub fn structurize(&self) -> Result<Vec<Stmt>, StructurizeError> {
        crate::hir::structurize::structurize(self)
    }

    /// Fuse straight-through chains into single blocks.
    pub fn collapse_chains(&self) -> Cfg {
        let mut succ: HashMap<String, Vec<String>> = self
            .blocks
            .iter()
            .map(|(n, b)| (n.clone(), b.succ.clone()))
            .collect();
        let mut pred: HashMap<String, Vec<String>> = self
            .blocks
            .iter()
            .map(|(n, b)| (n.clone(), b.pred.clone()))
            .collect();
        loop {
            let mut merged = false;
            for n in succ.keys().cloned().collect::<Vec<_>>() {
                if n == self.entry {
                    continue;
                }
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
                    }
                }
                succ.get_mut(&p).unwrap().retain(|x| x != &n);
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
        Cfg::new(self.entry.clone(), succ, pred, exits)
    }

    /// Split a block into its prefix statements and its terminating branch.
    pub fn split_block(
        &self,
        name: &str,
    ) -> Result<(Vec<Stmt>, Option<BranchInfo>), StructurizeError> {
        let block = self
            .blocks
            .get(name)
            .ok_or_else(|| StructurizeError::UnknownBlock(name.to_string()))?;
        let mut body = block.stmts.clone();
        match body.last() {
            Some(Stmt::Branch {
                cond,
                then_block,
                else_block,
            }) => {
                let br = BranchInfo {
                    cond: cond.clone(),
                    then_block: then_block.clone(),
                    else_block: else_block.clone(),
                };
                body.pop();
                Ok((body, Some(br)))
            }
            _ => Ok((body, None)),
        }
    }
}
