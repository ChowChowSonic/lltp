use inkwell::{basic_block::BasicBlock, module::Module, values::Operand};
use std::collections::{HashMap, HashSet, VecDeque};
use tracing::warn;

pub fn build_graph<'ctx>(
    module: &Module<'ctx>,
) -> Result<HashMap<BasicBlock<'ctx>, Vec<BasicBlock<'ctx>>>, &'ctx str> {
    let mut graph: HashMap<BasicBlock, Vec<BasicBlock>> = HashMap::new();
    for x in module.get_functions() {
        let blockslist = x.get_basic_blocks();
        blockslist.iter().for_each(|item: &BasicBlock| {
            graph.entry(*item).or_default().extend(get_neighbors(item));
        });
    }
    Ok(graph)
}

pub fn get_neighbors<'ctx>(bb: &BasicBlock<'ctx>) -> Vec<BasicBlock<'ctx>> {
    let mut ret: Vec<BasicBlock<'ctx>> = Vec::new();
    if let Some(terminator) = bb.get_terminator() {
        for i in 0..terminator.get_num_operands() {
            if let Some(Operand::Block(op)) = terminator.get_operand(i) {
                ret.push(op);
            } else {
                warn!("No terminator operand found (malformed BasicBlock)");
            }
        }
    }
    ret
}

pub fn has_self_referential_loop<'ctx>(
    graph: &HashMap<BasicBlock<'ctx>, Vec<BasicBlock<'ctx>>>,
    start: BasicBlock<'ctx>,
) -> bool {
    let mut visited: HashSet<BasicBlock> = HashSet::new();
    let mut queue: VecDeque<BasicBlock> = VecDeque::new();
    queue.push_back(start);

    while let Some(item) = queue.pop_front() {
        if let Some(neighbors) = graph.get(&item) {
            for &n in neighbors {
                if n == start {
                    return true; // found a path back to start
                }
                if visited.insert(n) {
                    queue.push_back(n);
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use inkwell::basic_block::BasicBlock;

    use crate::hir::graph::{build_graph, has_self_referential_loop};
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
        let ctxt = inkwell::context::Context::create();
        let mut tmp_byte_arr = IR.to_string();
        tmp_byte_arr.push('\0');
        let tmp_byte_arr = tmp_byte_arr.into_bytes();
        let mem_buf = inkwell::memory_buffer::MemoryBuffer::create_from_memory_range(
            &tmp_byte_arr,
            "test_module",
        );
        let res = ctxt
            .create_module_from_ir(mem_buf)
            .expect("Failed to create module from IR");
        let graph_res: Result<HashMap<BasicBlock<'_>, Vec<BasicBlock<'_>>>, &str> =
            build_graph(&res);
        if let Ok(graph) = graph_res {
            let mut loops: Vec<BasicBlock> = Vec::new();
            for (k, _) in graph.clone() {
                if has_self_referential_loop(&graph, k) {
                    loops.push(k);
                }
            }
            assert!(loops.iter().all(|x| {
                ["CF", "CF82", "CF83", "CF84", "CF86", "CF81"]
                    .contains(&x.get_name().to_str().unwrap_or_default())
            }))
        }
    }

    #[test]
    fn test_build_graph() {
        let ctxt = inkwell::context::Context::create();
        let mut tmp_byte_arr = IR.to_string();
        tmp_byte_arr.push('\0');
        let tmp_byte_arr = tmp_byte_arr.into_bytes();
        let mem_buf = inkwell::memory_buffer::MemoryBuffer::create_from_memory_range(
            &tmp_byte_arr,
            "test_module",
        );
        let res = ctxt
            .create_module_from_ir(mem_buf)
            .expect("Failed to create module from IR");
        let graph_res: Result<HashMap<BasicBlock<'_>, Vec<BasicBlock<'_>>>, &str> =
            build_graph(&res);
        if let Ok(graph) = graph_res {
            for (k, v) in graph {
                match k.get_name().to_str().unwrap() {
                    "CF80" => {
                        assert!(v.is_empty());
                    }
                    "CF86" => {
                        let mut tmp: Vec<String> = Vec::new();
                        v.iter().for_each(|item| {
                            tmp.push(item.get_name().to_string_lossy().into());
                        });
                        assert!(tmp.iter().all(|x| ["CF", "CF80"].contains(&x.as_str())))
                    }
                    "BB" => {
                        let mut tmp: Vec<String> = Vec::new();
                        v.iter().for_each(|item| {
                            tmp.push(item.get_name().to_string_lossy().into());
                        });
                        assert!(tmp.iter().all(|x| ["CF85"].contains(&x.as_str())))
                    }
                    "CF85" => {
                        let mut tmp: Vec<String> = Vec::new();
                        v.iter().for_each(|item| {
                            tmp.push(item.get_name().to_string_lossy().into());
                        });
                        assert!(tmp.iter().all(|x| ["CF"].contains(&x.as_str())))
                    }
                    "CF" => {
                        let mut tmp: Vec<String> = Vec::new();
                        v.iter().for_each(|item| {
                            tmp.push(item.get_name().to_string_lossy().into());
                        });
                        assert!(tmp.iter().all(|x| ["CF", "CF82"].contains(&x.as_str())));
                    }
                    "CF82" => {
                        let mut tmp: Vec<String> = Vec::new();
                        v.iter().for_each(|item| {
                            tmp.push(item.get_name().to_string_lossy().into());
                        });
                        assert!(tmp.iter().all(|x| ["CF83", "CF82"].contains(&x.as_str())));
                    }
                    "CF83" => {
                        let mut tmp: Vec<String> = Vec::new();
                        v.iter().for_each(|item| {
                            tmp.push(item.get_name().to_string_lossy().into());
                        });
                        assert!(tmp.iter().all(|x| ["CF83", "CF84"].contains(&x.as_str())));
                    }
                    "CF84" => {
                        let mut tmp: Vec<String> = Vec::new();
                        v.iter().for_each(|item| {
                            tmp.push(item.get_name().to_string_lossy().into());
                        });
                        assert!(tmp.iter().all(|x| ["CF", "CF81"].contains(&x.as_str())));
                    }
                    "CF81" => {
                        let mut tmp: Vec<String> = Vec::new();
                        v.iter().for_each(|item| {
                            tmp.push(item.get_name().to_string_lossy().into());
                        });
                        assert!(tmp.iter().all(|x| ["CF86", "CF81"].contains(&x.as_str())));
                    }
                    _ => {
                        panic!("Unknown basicblock name");
                    }
                }
            }
        }
    }
}
