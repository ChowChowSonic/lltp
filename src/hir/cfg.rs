use std::collections::{BTreeMap, HashMap};

#[derive(Default, Debug)]
pub struct Cfg {
    pub entry: String,
    pub blocks: BTreeMap<String, Vec<String>>,
    pub successors: HashMap<String, Vec<String>>,
    pub preds: HashMap<String, Vec<String>>,
    pub exits: Vec<String>,
}

impl Cfg {
    pub fn new(
        entry: String,
        successors: HashMap<String, Vec<String>>,
        preds: HashMap<String, Vec<String>>,
        exits: Vec<String>,
    ) -> Self {
        let mut blocks: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (k, v) in &successors {
            blocks.entry(k.clone()).or_default().extend(v.clone());
        }
        Cfg {
            entry,
            blocks,
            successors,
            preds,
            exits,
        }
    }
}
