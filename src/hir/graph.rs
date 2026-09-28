#![allow(dead_code)]
use inkwell::{
    basic_block::BasicBlock,
    values::{FunctionValue, Operand},
};
use std::collections::HashMap;

use crate::hir::cfg::Cfg;

enum BlockClass<'c> {
    Exit(BasicBlock<'c>),
    Entry(BasicBlock<'c>),
    Unreachable(BasicBlock<'c>),
    Intermediate(BasicBlock<'c>),
}

pub fn build_graph(func: &FunctionValue) -> Result<Cfg, String> {
    let mut res: Vec<BlockClass> = Vec::new();
    let blockslist = func.get_basic_blocks();
    let mut successors: HashMap<String, Vec<String>> = HashMap::new();
    for &x in &blockslist {
        let successor_list: Vec<String> = get_neighbors(&x)
            .iter()
            .map(|x| x.get_name().to_string_lossy().to_string())
            .collect();
        successors
            .entry(x.get_name().to_string_lossy().to_string())
            .or_default()
            .extend(successor_list);
    }
    let mut preds: HashMap<String, Vec<String>> = HashMap::new();
    for x in &blockslist {
        let name = x.get_name().to_string_lossy().to_string();
        let preds_list: Vec<String> = successors.iter().fold(Vec::new(), |mut a, (k, v)| {
            if v.contains(&name) {
                a.push(k.clone());
            }
            a
        });
        preds.entry(name).or_default().extend(preds_list);
    }
    // The entry block is ALWAYS the first-listed block in LLVM IR. Any other
    // block without predecessors is unreachable code, regardless of whether it
    // has successors. Only the first block may be classified `Entry`.
    let first = func
        .get_first_basic_block()
        .expect("Failed to get first basic block of function");
    blockslist.iter().for_each(|item: &BasicBlock| {
        let name = item.get_name().to_string_lossy().to_string();
        let has_pred = preds.get(&name).is_some_and(|p| !p.is_empty());
        let has_succ = successors.get(&name).is_some_and(|s| !s.is_empty());
        let is_first = *item == first;
        match (has_pred, has_succ) {
            // Unreachable code that happens to branch somewhere.
            (false, true) if !is_first => res.push(BlockClass::Unreachable(*item)),
            (false, true) => res.push(BlockClass::Entry(*item)),
            // Terminates the function (ret, unreachable, ...): no successors.
            (true, false) => res.push(BlockClass::Exit(*item)),
            // A single-block function is both entry and exit.
            (false, false) if is_first => {
                res.push(BlockClass::Entry(*item));
                res.push(BlockClass::Exit(*item));
            }
            (false, false) => res.push(BlockClass::Unreachable(*item)),
            // Has both predecessors and successors: an interior block.
            (true, true) => res.push(BlockClass::Intermediate(*item)),
        }
    });
    let entry_block = res
        .iter()
        .filter_map(|x| match x {
            BlockClass::Entry(y) => Some(y),
            _ => None,
        })
        .next()
        .ok_or("Failed to retrieve first block of function")?;
    let exits: Vec<String> = res
        .iter()
        .filter_map(|x| match x {
            BlockClass::Exit(i) => Some(i.get_name().to_string_lossy().to_string()),
            _ => None,
        })
        .collect();
    Ok(Cfg::new(
        entry_block.get_name().to_string_lossy().to_string(),
        successors,
        preds,
        exits,
    ))
}

pub fn get_neighbors<'ctx>(bb: &BasicBlock<'ctx>) -> Vec<BasicBlock<'ctx>> {
    let mut ret: Vec<BasicBlock<'ctx>> = Vec::new();
    if let Some(terminator) = bb.get_terminator() {
        for i in 0..terminator.get_num_operands() {
            match terminator.get_operand(i) {
                Some(Operand::Block(op)) => ret.push(op),
                Some(Operand::Value(_)) => {}
                None => {
                    tracing::error!("Malformed BasicBlock")
                }
            }
        }
    }
    ret
}

#[cfg(test)]
mod tests {
    use crate::hir::{
        cfg::Cfg,
        flow::{dominators, natural_loops},
        graph::build_graph,
    };

    /// Parse module and build a Cfg per function.
    fn build_cfgs(ir: &str) -> Vec<Cfg> {
        let mut bytes = ir.as_bytes().to_vec();
        bytes.push(b'\0');
        let mem =
            inkwell::memory_buffer::MemoryBuffer::create_from_memory_range(&bytes, "test_module");
        let ctxt = inkwell::context::Context::create();
        let module = ctxt
            .create_module_from_ir(mem)
            .expect("Failed to create module from IR");
        module
            .get_functions()
            .map(|f| build_graph(&f).expect("CFG build should succeed"))
            .collect()
    }

    /// Build the Cfg of the (single) function in `ir`.
    fn build_cfg(ir: &str) -> Cfg {
        build_cfgs(ir)
            .into_iter()
            .next()
            .expect("module has no functions")
    }

    const IR: &str = r#"; ModuleID = '/tmp/autogen.bc'
source_filename = "/tmp/autogen.bc"

define void @autogen_SD0(ptr %0, ptr %1, ptr %2, i32 %3, i64 %4, i8 %5) {
BB:
  %Cmp24 = icmp ugt i64 1, 251161
  br label %CF85

CF85:                                             ; preds = %BB
  %Cmp32 = fcmp ord double 0x18DF23FE11DD527C, 0xAE97BFB633957A34
  br label %CF

CF:                                               ; preds = %CF, %CF84, %CF86, %CF85
  %Cmp40 = icmp slt i16 1, 18437
  br i1 %Cmp40, label %CF, label %CF82

CF82:                                             ; preds = %CF82, %CF
  %Sl47 = select i1 1, i1 1, i1 1
  br i1 %Sl47, label %CF82, label %CF83

CF83:                                             ; preds = %CF83, %CF82
  %Cmp56 = icmp ugt i64 1, 1
  br i1 %Cmp56, label %CF83, label %CF84

CF84:                                             ; preds = %CF83
  %Cmp64 = icmp ule i16 1, 1
  br i1 %Cmp64, label %CF, label %CF81

CF81:                                             ; preds = %CF81, %CF84
  %Cmp72 = icmp ult i16 1, 1
  br i1 %Cmp72, label %CF81, label %CF86

CF86:                                             ; preds = %CF81
  %Cmp79 = icmp ult i1 1, 1
  br i1 %Cmp79, label %CF, label %CF80

CF80:                                             ; preds = %CF86
  ret void
}
"#;

    #[test]
    fn test_find_loop() {
        let graph_res = build_cfgs(IR);
        for graph in &graph_res {
            let dom = dominators(graph);
            let mut headers: Vec<String> = natural_loops(graph, &dom)
                .into_iter()
                .map(|l| l.header)
                .collect();
            headers.sort();
            // Dominance-based loop detection: these are the natural-loop
            // headers of the IR. CF84/CF86 lie on cycles but never dominate
            // one, so they are loop members, not headers.
            assert_eq!(
                headers,
                ["CF", "CF81", "CF82", "CF83"]
                    .iter()
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn test_build_graph() {
        let graph_res = build_cfgs(IR);
        for graph in graph_res {
            for (k, v) in graph.blocks {
                match k.as_str() {
                    "CF80" => {
                        assert!(v.succ.is_empty());
                    }
                    "CF86" => assert!(v.succ.iter().all(|x| ["CF", "CF80"].contains(&x.as_str()))),
                    "BB" => assert!(v.succ.iter().all(|x| ["CF85"].contains(&x.as_str()))),
                    "CF85" => assert!(v.succ.iter().all(|x| ["CF"].contains(&x.as_str()))),
                    "CF" => assert!(v.succ.iter().all(|x| ["CF", "CF82"].contains(&x.as_str()))),
                    "CF82" => assert!(
                        v.succ
                            .iter()
                            .all(|x| ["CF83", "CF82"].contains(&x.as_str()))
                    ),
                    "CF83" => assert!(
                        v.succ
                            .iter()
                            .all(|x| ["CF83", "CF84"].contains(&x.as_str()))
                    ),
                    "CF84" => assert!(v.succ.iter().all(|x| ["CF", "CF81"].contains(&x.as_str()))),
                    "CF81" => assert!(
                        v.succ
                            .iter()
                            .all(|x| ["CF86", "CF81"].contains(&x.as_str()))
                    ),
                    _ => panic!("Unknown basicblock name"),
                }
            }
        }
    }

    /// Spec: per-function CFGs. Two functions sharing block names must yield two
    /// isolated graphs — no edge or block leaks across functions.
    #[test]
    fn per_function_cfgs_are_isolated() {
        let ir = r#"
define void @alpha() {
entry:
  br label %body
body:
  br label %exit
exit:
  ret void
}
define void @beta() {
entry:
  br label %done
done:
  ret void
}
"#;
        let cfgs = build_cfgs(ir);
        assert_eq!(cfgs.len(), 2);

        let alpha = cfgs
            .iter()
            .find(|c| c.exits == vec!["exit".to_string()])
            .expect("alpha cfg");
        let beta = cfgs
            .iter()
            .find(|c| c.exits == vec!["done".to_string()])
            .expect("beta cfg");

        assert_eq!(alpha.entry, "entry");
        assert_eq!(alpha.blocks["entry"].succ, vec!["body".to_string()]);
        assert_eq!(alpha.blocks["body"].succ, vec!["exit".to_string()]);
        assert!(alpha.blocks["exit"].succ.is_empty());
        assert_eq!(alpha.blocks["body"].pred, vec!["entry".to_string()]);
        assert_eq!(alpha.blocks["exit"].pred, vec!["body".to_string()]);

        assert_eq!(beta.entry, "entry");
        assert_eq!(beta.blocks["entry"].succ, vec!["done".to_string()]);
        assert!(beta.blocks["done"].succ.is_empty());

        // No cross-function bleed.
        assert!(!alpha.blocks.contains_key("done"));
        assert!(!beta.blocks.contains_key("body"));
    }

    /// A function whose exit lives mid-list must classify that block as Exit.
    #[test]
    fn exit_in_middle_of_function() {
        let ir = r#"
define void @f() {
entry:
  br label %mid
mid:
  ret void
tail:
  unreachable
}
"#;
        let cfg = build_cfg(ir);
        assert_eq!(cfg.entry, "entry");
        assert_eq!(cfg.exits, vec!["mid".to_string()]);
        assert_eq!(cfg.blocks["entry"].succ, vec!["mid".to_string()]);
        assert!(cfg.blocks["mid"].succ.is_empty());
        assert_eq!(cfg.blocks["mid"].pred, vec!["entry".to_string()]);
    }

    /// A single block is both entry and exit.
    #[test]
    fn single_block_function_is_entry_and_exit() {
        let ir = "define void @f() {\nentry:\n  ret void\n}\n";
        let cfg = build_cfg(ir);
        assert_eq!(cfg.entry, "entry");
        assert_eq!(cfg.exits, vec!["entry".to_string()]);
        assert!(cfg.blocks["entry"].succ.is_empty());
        assert!(cfg.blocks["entry"].pred.is_empty());
    }

    /// Switch terminators: every case label plus the default becomes a successor.
    /// Note: LLVM assembly rejects a comma after `label %dest` between cases —
    /// entries separate on whitespace only.
    #[test]
    fn switch_terminator_edges() {
        let ir = r#"
define void @f(i32 %x) {
entry:
  switch i32 %x, label %dflt [ i32 0, label %zero
    i32 1, label %one ]
zero:
  ret void
one:
  ret void
dflt:
  ret void
}
"#;
        let cfg = build_cfg(ir);
        let succs = &cfg.blocks["entry"].succ;
        assert_eq!(succs.len(), 3);
        assert!(succs.contains(&"zero".to_string()));
        assert!(succs.contains(&"one".to_string()));
        assert!(succs.contains(&"dflt".to_string()));
        for target in ["zero", "one", "dflt"] {
            assert_eq!(cfg.blocks[target].pred, vec!["entry".to_string()]);
            assert!(cfg.blocks[target].succ.is_empty());
        }
    }

    /// Conditional branch: condition operand is a value, not a block — must not
    /// appear in successor list or trigger the old malformed-block warning.
    /// inkwell exposes `br` operands in order [cond, iffalse, iftrue], so the
    /// false target comes first in the successor list.
    #[test]
    fn conditional_branch_edges() {
        let ir = r#"
define void @f(i1 %c) {
entry:
  br i1 %c, label %t, label %f
t:
  ret void
f:
  ret void
}
"#;
        let cfg = build_cfg(ir);
        assert_eq!(
            cfg.blocks["entry"].succ,
            vec!["f".to_string(), "t".to_string()]
        );
        assert_eq!(cfg.blocks["t"].pred, vec!["entry".to_string()]);
        assert_eq!(cfg.blocks["f"].pred, vec!["entry".to_string()]);
    }

    /// Unreachable blocks (no preds, not entry) still appear as nodes.
    #[test]
    fn unreachable_block_is_a_node() {
        let ir = r#"
define void @f() {
entry:
  ret void
ghost:
  ret void
}
"#;
        let cfg = build_cfg(ir);
        assert!(cfg.blocks.contains_key("ghost"));
        assert!(cfg.blocks["ghost"].succ.is_empty());
        assert!(cfg.blocks["ghost"].pred.is_empty());
        assert_eq!(cfg.exits, vec!["entry".to_string()]);
    }
}
