use std::collections::{BTreeMap, HashMap};

use crate::hir::stmt::Stmt;

#[derive(Default, Debug)]
pub struct Block {
    pub name: String,
    pub stmts: Vec<Stmt>,
    pub succ: Vec<String>,
    pub pred: Vec<String>,
}

#[derive(Default, Debug)]
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
            let blk = blocks.entry(k.to_string()).or_default();
            blk.succ.extend(v);
            blk.name = k;
        }
        for (k, v) in preds {
            let blk = blocks.entry(k.to_string()).or_default();
            blk.pred.extend(v);
            blk.name = k;
        }
        Cfg {
            entry,
            blocks,
            exits,
        }
    }
}
