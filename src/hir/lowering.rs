use std::collections::HashMap;

use inkwell::{
    basic_block::BasicBlock,
    module::Module,
    values::{FunctionValue, InstructionOpcode::*, InstructionValue, Operand},
};
use tracing::{debug, warn};

use crate::hir::{
    Lit, cfg::Cfg, error::LowerError, expr::Expr, func::Function, stmt::Stmt, ty::Ty,
};

/// Build a CFG from an LLVM function value.
pub fn build_graph(func: &FunctionValue) -> Result<Cfg, LowerError> {
    let first = func
        .get_first_basic_block()
        .ok_or(LowerError::NoFirstBlock)?;
    let entry_name = first.get_name().to_string_lossy().to_string();

    let basic_blocks = func.get_basic_blocks();
    let mut successors: HashMap<String, Vec<String>> = HashMap::with_capacity(basic_blocks.len());
    let mut preds: HashMap<String, Vec<String>> = HashMap::with_capacity(basic_blocks.len());

    for bb in &basic_blocks {
        let name = bb.get_name().to_string_lossy().to_string();
        successors.entry(name.clone()).or_default();
        preds.entry(name).or_default();
    }

    for bb in &basic_blocks {
        let src_name = bb.get_name().to_string_lossy().to_string();
        for target in get_neighbors(bb) {
            let target_name = target.get_name().to_string_lossy().to_string();
            successors
                .entry(src_name.clone())
                .or_default()
                .push(target_name.clone());
            preds.entry(target_name).or_default().push(src_name.clone());
        }
    }

    let exits: Vec<String> = basic_blocks
        .iter()
        .filter_map(|bb| {
            let name = bb.get_name().to_string_lossy().to_string();
            let has_succ = successors.get(&name).is_some_and(|s| !s.is_empty());
            let has_pred = preds.get(&name).is_some_and(|p| !p.is_empty());
            let is_first = *bb == first;
            if (!has_succ && has_pred) || (!has_succ && !has_pred && is_first) {
                Some(name)
            } else {
                None
            }
        })
        .collect();

    Ok(Cfg::new(entry_name, successors, preds, exits))
}

/// Retrieve the successor basic blocks of a given basic block by inspecting its terminator.
pub fn get_neighbors<'ctx>(bb: &BasicBlock<'ctx>) -> Vec<BasicBlock<'ctx>> {
    let mut ret = Vec::new();
    if let Some(terminator) = bb.get_terminator() {
        for i in 0..terminator.get_num_operands() {
            match terminator.get_operand(i) {
                Some(Operand::Block(op)) => ret.push(op),
                Some(Operand::Value(_)) => {}
                None => {
                    tracing::error!("Malformed BasicBlock operand");
                }
            }
        }
    }
    ret
}

/// Lower an LLVM function into an HIR Function.
pub fn lower_function(val: &FunctionValue) -> Result<Function, LowerError> {
    let name = val.get_name().to_string_lossy().to_string();
    debug!(name = %name, "recovering function");
    let llvm_params = val.get_params();
    let param_types: Vec<Ty> = llvm_params.iter().map(|param| Ty::from(*param)).collect();
    let ret_ty = match val.get_type().get_return_type() {
        Some(x) => Ty::from(x),
        None => {
            warn!(name = %name, "function missing return type; treating recovery as void");
            Ty::Void
        }
    };

    let mut stmt_blocks = Vec::new();
    for block in val.get_basic_block_iter() {
        let bname = block.get_name().to_string_lossy().to_string();
        let last = block
            .get_last_instruction()
            .ok_or_else(|| LowerError::MissingInstruction(bname.clone()))?;
        let stmt = Stmt::try_from(last)?;
        stmt_blocks.push(stmt);
    }

    let mut cfg = build_graph(val)?;
    collect_instructions(val, &mut cfg);

    Ok(Function {
        name,
        params: param_types,
        return_ty: ret_ty,
        cfg,
        body: stmt_blocks,
    })
}

fn collect_instructions(val: &FunctionValue, cfg: &mut Cfg) {
    for bb in val.get_basic_block_iter() {
        let name = bb.get_name().to_string_lossy().to_string();
        let mut stmts = Vec::new();
        for inst in bb.get_instructions() {
            if let Ok(s) = Stmt::try_from(inst) {
                stmts.push(s); // terminators: Branch/Ret (last)
            } else if let Ok(e) = Expr::try_from(inst) {
                let dest: String = inst
                    .get_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                let dtype = Ty::try_from(inst.get_type()).unwrap_or(Ty::Void);
                if !dest.is_empty() {
                    stmts.push(Stmt::Let {
                        dest: Box::new(Expr::Var { name: dest, dtype }),
                        src: Box::new(e),
                    });
                } else {
                    tracing::debug!("Skipping unnamed value");
                }
            } else {
                tracing::debug!(%name, "skipping unrecovered instruction");
            }
        }
        if let Some(block) = cfg.blocks.get_mut(&name) {
            block.stmts = stmts;
        }
    }
}

/// Collect and lower all functions in an LLVM module.
pub fn collect_functions(module: &Module) -> Result<Vec<Function>, LowerError> {
    let fns = module.get_functions();
    let mut recovered = Vec::new();
    for f in fns {
        recovered.push(lower_function(&f)?);
    }
    debug!(count = recovered.len(), "recovered functions from module");
    Ok(recovered)
}

// ============================================================================
// Conversion Trait Implementations
// ============================================================================

impl<'c> TryFrom<&FunctionValue<'c>> for Function {
    type Error = LowerError;

    fn try_from(val: &FunctionValue<'c>) -> Result<Self, Self::Error> {
        lower_function(val)
    }
}

impl<'c> TryFrom<FunctionValue<'c>> for Function {
    type Error = LowerError;

    fn try_from(val: FunctionValue<'c>) -> Result<Self, Self::Error> {
        Self::try_from(&val)
    }
}

impl<'c> TryFrom<&FunctionValue<'c>> for Cfg {
    type Error = LowerError;

    fn try_from(val: &FunctionValue<'c>) -> Result<Self, Self::Error> {
        build_graph(val)
    }
}

impl<'c> TryFrom<FunctionValue<'c>> for Cfg {
    type Error = LowerError;

    fn try_from(val: FunctionValue<'c>) -> Result<Self, Self::Error> {
        Self::try_from(&val)
    }
}

impl<'c> TryFrom<InstructionValue<'c>> for Stmt {
    type Error = LowerError;

    fn try_from(op: InstructionValue<'c>) -> Result<Self, Self::Error> {
        match op.get_opcode() {
            Br => {
                let mut ops = op.get_operands();
                let op1 = ops
                    .next()
                    .and_then(|o| o)
                    .ok_or(LowerError::MissingOperand { op: "Br", index: 0 })?;
                if op1.is_block()
                    && let blockval = op1.unwrap_block()
                {
                    Ok(Stmt::Branch {
                        cond: None,
                        then_block: blockval.get_name().to_string_lossy().to_string(),
                        else_block: None,
                    })
                } else {
                    let opval = Expr::try_from(op1)?;
                    let else_block = ops
                        .next()
                        .and_then(|o| o)
                        .ok_or(LowerError::MissingOperand { op: "Br", index: 1 })?
                        .block()
                        .ok_or_else(|| {
                            LowerError::UnsupportedOperandKind("expected false block".into())
                        })?;
                    let then_block = ops
                        .next()
                        .and_then(|o| o)
                        .ok_or(LowerError::MissingOperand { op: "Br", index: 2 })?
                        .block()
                        .ok_or_else(|| {
                            LowerError::UnsupportedOperandKind("expected true block".into())
                        })?;
                    Ok(Stmt::Branch {
                        cond: Some(opval),
                        then_block: then_block.get_name().to_string_lossy().to_string(),
                        else_block: Some(else_block.get_name().to_string_lossy().to_string()),
                    })
                }
            }
            Return => {
                let mut ops = op.get_operands();
                let op1 = ops.next().and_then(|o| o);
                if let Some(op1) = op1 {
                    let value = Expr::try_from(op1)?;
                    Ok(Stmt::Ret { value: Some(value) })
                } else {
                    Ok(Stmt::Ret { value: None })
                }
            }
            other => Err(LowerError::UnsupportedOpcode(format!("{other:?}"))),
        }
    }
}

impl<'c> TryFrom<InstructionValue<'c>> for Expr {
    type Error = LowerError;

    fn try_from(op: InstructionValue<'c>) -> Result<Self, Self::Error> {
        match op.get_opcode() {
            Add | FAdd | Sub | FSub | Mul | FMul | SDiv | UDiv | FDiv | SRem | URem | FRem
            | Shl | LShr | AShr | And | Or | Xor | ICmp | FCmp => {
                let mut ops = op.get_operands();
                let op1 = ops
                    .next()
                    .and_then(|o| o)
                    .ok_or(LowerError::MissingOperand {
                        op: "BinaryOp",
                        index: 0,
                    })?;
                let op2 = ops
                    .next()
                    .and_then(|o| o)
                    .ok_or(LowerError::MissingOperand {
                        op: "BinaryOp",
                        index: 1,
                    })?;
                let op1_expr = Expr::try_from(op1)?;
                let op2_expr = Expr::try_from(op2)?;
                Ok(Expr::BinaryOp {
                    op: op.get_opcode(),
                    arg1: Box::new(op1_expr),
                    arg2: Box::new(op2_expr),
                })
            }
            other => Err(LowerError::UnsupportedOpcode(format!("{other:?}"))),
        }
    }
}

impl<'c> TryFrom<Operand<'c>> for Expr {
    type Error = LowerError;

    fn try_from(operand: Operand<'c>) -> Result<Self, Self::Error> {
        let bve = operand.value().ok_or(LowerError::ExpectedValueOperand)?;

        if bve.is_int_value() {
            let int = bve.into_int_value();
            if let Some(inst) = int.as_instruction() {
                Expr::try_from(inst)
            } else if int.is_const() {
                let bits = int.get_type().get_bit_width() as usize;
                let value = int
                    .get_zero_extended_constant()
                    .ok_or(LowerError::ConstantExtractionFailed("integer"))?;
                if bits == 1 {
                    Ok(Expr::Literal(Lit::Bool(value != 0)))
                } else {
                    Ok(Expr::Literal(Lit::Int {
                        value,
                        bits,
                        signed: true,
                    }))
                }
            } else {
                let name = int.get_name().to_string_lossy().into_owned();
                Ok(Expr::Var {
                    name,
                    dtype: Ty::from(bve),
                })
            }
        } else if bve.is_float_value() {
            let float = bve.into_float_value();
            if let Some(inst) = float.as_instruction() {
                Expr::try_from(inst)
            } else if float.is_const() {
                let (value, _lossy) = float
                    .get_constant()
                    .ok_or(LowerError::ConstantExtractionFailed("float"))?;
                let bits = float.get_type().get_bit_width() as usize;
                Ok(Expr::Literal(Lit::Float { bits, value }))
            } else {
                let name = float.get_name().to_string_lossy().into_owned();
                Ok(Expr::Var {
                    name,
                    dtype: Ty::from(bve),
                })
            }
        } else if bve.is_pointer_value() {
            let ptr = bve.into_pointer_value();
            if let Some(inst) = ptr.as_instruction() {
                Expr::try_from(inst)
            } else if ptr.is_null() {
                Ok(Expr::Literal(Lit::NullPtr))
            } else {
                let name = ptr.get_name().to_string_lossy().into_owned();
                Ok(Expr::Var {
                    name,
                    dtype: Ty::from(bve),
                })
            }
        } else {
            Err(LowerError::UnsupportedOperandKind(format!("{bve:?}")))
        }
    }
}

impl<'c> TryFrom<&Operand<'c>> for Expr {
    type Error = LowerError;

    fn try_from(operand: &Operand<'c>) -> Result<Self, Self::Error> {
        Self::try_from(*operand)
    }
}

#[cfg(test)]
mod tests {
    use crate::hir::{
        cfg::Cfg,
        flow::{dominators, natural_loops},
        func::Function,
        stmt::Stmt,
    };

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
            .map(|f| Cfg::try_from(&f).expect("CFG build should succeed"))
            .collect()
    }

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

        assert!(!alpha.blocks.contains_key("done"));
        assert!(!beta.blocks.contains_key("body"));
    }

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

    #[test]
    fn single_block_function_is_entry_and_exit() {
        let ir = "define void @f() {\nentry:\n  ret void\n}\n";
        let cfg = build_cfg(ir);
        assert_eq!(cfg.entry, "entry");
        assert_eq!(cfg.exits, vec!["entry".to_string()]);
        assert!(cfg.blocks["entry"].succ.is_empty());
        assert!(cfg.blocks["entry"].pred.is_empty());
    }

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

    const IR2: &str = r#"define void @f(i32 %x) {
entry:
  %c = icmp sgt i32 %x, 0
  br i1 %c, label %pos, label %nonpos
pos:
  ret void
nonpos:
  ret void
}
"#;

    #[test]
    fn block_statements_are_recovered() {
        let ctxt = inkwell::context::Context::create();
        let mut ir_str = IR2.to_string();
        ir_str.push('\0');
        let module = crate::build_module(&ctxt, &ir_str);
        let func = module.get_first_function().unwrap();
        let f = Function::try_from(&func).expect("Function lowering failed");
        assert_eq!(f.cfg.blocks["entry"].stmts.len(), 2);
        assert!(matches!(&f.cfg.blocks["entry"].stmts[0], Stmt::Let { .. }));
        assert!(matches!(
            &f.cfg.blocks["entry"].stmts[1],
            Stmt::Branch { .. }
        ));
    }
}
