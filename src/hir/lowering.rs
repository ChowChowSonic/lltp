use std::collections::{HashMap, HashSet};

use inkwell::{
    module::Module,
    values::{FunctionValue, InstructionOpcode::*, InstructionValue, Operand},
};
use tracing::{debug, warn};

use crate::errors::LowerError;
use crate::hir::{
    Block, Cfg, Lit,
    cfg::successors_of,
    expr::{BinaryExpr, CmpExpr, Expr, VarExpr},
    flat::{Namer, block_label},
    func::{FnSig, Function, Param},
    op::{BinOp, Cmp, FloatCmp, IntCmp},
    stmt::{BranchStmt, InvokeStmt, LetStmt, RetStmt, Stmt},
    ty::Ty,
};

/// Build a CFG from an LLVM function value: one block per basic block, named
/// the way `hir::flat` names it, in the order LLVM emitted it, with control
/// edges taken from each terminator. Exception arms are not control edges;
/// `hir::cfg::exception_targets` reports them.
pub fn build_graph(func: &FunctionValue) -> Result<Cfg, LowerError> {
    let blocks = func.get_basic_blocks();
    if blocks.is_empty() {
        return Err(LowerError::NoFirstBlock);
    }

    let mut namer = Namer::new(&HashSet::new());
    let mut raw_to_node: HashMap<String, String> = HashMap::new();
    let mut names: Vec<String> = Vec::new();
    for (i, bb) in blocks.iter().enumerate() {
        let raw = bb.get_name().to_string_lossy().to_string();
        let node = block_label(&mut namer, &raw, i);
        raw_to_node.insert(raw, node.clone());
        names.push(node);
    }

    let mut built: Vec<Block> = Vec::new();
    for (i, bb) in blocks.iter().enumerate() {
        let name = names[i].clone();
        let termin = bb
            .get_terminator()
            .ok_or_else(|| LowerError::MalformedBlock(name.clone()))?;
        let term = Stmt::try_from(termin).ok();

        let mut stmts: Vec<Stmt> = Vec::new();
        for inst in bb.get_instructions() {
            if let Ok(s) = Stmt::try_from(inst) {
                stmts.push(s);
            } else if let Ok(e) = Expr::try_from(inst) {
                let dest: String = inst
                    .get_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                let dtype = Ty::try_from(inst.get_type()).unwrap_or(Ty::Void);
                if !dest.is_empty() {
                    stmts.push(Stmt::Let(LetStmt {
                        dest: Box::new(Expr::Var(VarExpr { name: dest, dtype })),
                        src: Box::new(e),
                    }));
                } else {
                    tracing::debug!("Skipping unnamed value");
                }
            } else {
                tracing::debug!(%name, "skipping unrecovered instruction");
            }
        }

        let mut block = Block::new(name);
        block.stmts = stmts;
        if let Some(t) = &term {
            // `get_instructions` may already have yielded the terminator, so the
            // statement list is not extended twice.
            if block.stmts.last() != Some(t) {
                block.stmts.push(t.clone());
            }
        }
        for s in &mut block.stmts {
            resolve_block_target(s, &raw_to_node);
        }
        let arms = if let Some(t) = &term {
            // Arms come from the resolved statement, not from the raw IR names.
            successors_of(block.stmts.last().unwrap_or(t))
        } else {
            // No AST node for this terminator yet: a `switch`'s arms are bodies,
            // not targets, so its edges come from the IR operand scan until the
            // node carries an arm order.
            let mut resolved: Vec<String> = Vec::new();
            for target in scan_block_operands(bb) {
                resolved.push(raw_to_node.get(&target).cloned().unwrap_or(target));
            }
            tracing::debug!(%block.name, "terminator has no node; edges from the IR scan");
            resolved
        };
        block.succ = arms;
        built.push(block);
    }

    Ok(Cfg::assemble(names[0].clone(), built))
}

/// Point a terminator's arms at the graph's block names rather than the IR's,
/// so edges and statements name the same block.
fn resolve_block_target(stmt: &mut Stmt, map: &HashMap<String, String>) {
    match stmt {
        Stmt::Branch(b) => {
            if let Some(n) = map.get(&b.then_block) {
                b.then_block = n.clone();
            }
            if let Some(e) = &mut b.else_block
                && let Some(n) = map.get(e)
            {
                *e = n.clone();
            }
        }
        Stmt::Goto(g) => {
            if let Some(n) = map.get(&g.target) {
                g.target = n.clone();
            }
        }
        Stmt::Invoke(i) => {
            if let Some(n) = map.get(&i.normal) {
                i.normal = n.clone();
            }
            for c in &mut i.catches {
                if let Some(n) = map.get(c) {
                    *c = n.clone();
                }
            }
        }
        _ => {}
    }
}

/// The blocks a terminator's operand list names. Used only for terminators
/// that have no HIR node; once a node carries its arms, `successors_of` is the
/// definition.
fn scan_block_operands(bb: &inkwell::basic_block::BasicBlock<'_>) -> Vec<String> {
    let mut ret = Vec::new();
    if let Some(termin) = bb.get_terminator() {
        for i in 0..termin.get_num_operands() {
            if let Some(Operand::Block(op)) = termin.get_operand(i) {
                let name = op.get_name().to_string_lossy().to_string();
                if !ret.contains(&name) {
                    ret.push(name);
                }
            }
        }
    }
    ret
}

/// Instruction-level recovery: one statement per instruction, with no value
/// naming, slot promotion, or de-SSA. Exception-bearing IR still takes this
/// path, because `invoke`/`landingpad` are not recovered by the canonical
/// value-naming recovery in `hir::flat`.
pub fn lower_function(val: &FunctionValue) -> Result<Function, LowerError> {
    let name = val.get_name().to_string_lossy().to_string();
    debug!(name = %name, "recovering function at instruction level");
    let params: Vec<Param> = val
        .get_params()
        .iter()
        .map(|param| Param {
            name: crate::hir::flat::value_name(param),
            ty: Ty::from(*param),
        })
        .collect();
    let ret_ty = match val.get_type().get_return_type() {
        Some(x) => Ty::from(x),
        None => {
            warn!(name = %name, "function missing return type; treating recovery as void");
            Ty::Void
        }
    };

    let mut func = Function {
        sig: FnSig {
            name,
            params,
            ret: ret_ty,
            variadic: val.get_type().is_var_arg(),
        },
        locals: Vec::new(),
        cfg: build_graph(val)?,
        body: Vec::new(),
    };
    func.structurize()
        .map_err(|e| LowerError::Other(e.to_string()))?;
    Ok(func)
}

/// Collect and lower all functions in an LLVM module, through the canonical
/// recovery in `hir::flat`.
pub fn collect_functions(module: &Module) -> Result<Vec<Function>, LowerError> {
    let recovered = crate::hir::module::Module::from_module(module)?;
    debug!(
        count = recovered.funcs.len(),
        "recovered functions from module"
    );
    Ok(recovered.funcs)
}

// ============================================================================
// Conversion Trait Implementations
// ============================================================================

// `TryFrom<&FunctionValue> for Function` lives with the canonical recovery in
// `hir::flat`; these cover the graph and the instruction-level nodes.

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
                    Ok(Stmt::Branch(BranchStmt {
                        cond: None,
                        then_block: blockval.get_name().to_string_lossy().to_string(),
                        else_block: None,
                    }))
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
                    Ok(Stmt::Branch(BranchStmt {
                        cond: Some(opval),
                        then_block: then_block.get_name().to_string_lossy().to_string(),
                        else_block: Some(else_block.get_name().to_string_lossy().to_string()),
                    }))
                }
            }
            Invoke => {
                let num_ops = op.get_num_operands();
                let mut blocks = Vec::new();
                let mut value_operands = Vec::new();

                for i in 0..num_ops {
                    match op.get_operand(i) {
                        Some(inkwell::values::Operand::Block(bb)) => {
                            blocks.push(bb);
                        }
                        Some(inkwell::values::Operand::Value(vv)) => {
                            value_operands.push(vv);
                        }
                        None => {}
                    }
                }

                if blocks.is_empty() {
                    return Err(LowerError::MissingOperand {
                        op: "Invoke",
                        index: 0,
                    });
                }
                // InvokeInst operands are [args..., normal dest, unwind dests,
                // callee]: the first block is the control arm, the rest are the
                // exception arms in operand order.
                let normal = blocks[0].get_name().to_string_lossy().to_string();
                let catches: Vec<String> = blocks
                    .iter()
                    .skip(1)
                    .map(|b| b.get_name().to_string_lossy().to_string())
                    .collect();
                let callee_val = value_operands
                    .pop()
                    .ok_or(LowerError::ExpectedValueOperand)?;
                let mut args = Vec::new();
                for v in value_operands {
                    let oprnd = Expr::try_from(inkwell::values::Operand::Value(v))?;
                    args.push(oprnd);
                }
                let callee_name = callee_val.get_name().to_string_lossy().to_string();
                Ok(Stmt::Invoke(InvokeStmt {
                    callee: callee_name,
                    args,
                    normal,
                    catches,
                }))
            }
            Return => {
                let mut ops = op.get_operands();
                let op1 = ops.next().and_then(|o| o);
                if let Some(op1) = op1 {
                    let value = Expr::try_from(op1)?;
                    Ok(Stmt::Ret(RetStmt { value: Some(value) }))
                } else {
                    Ok(Stmt::Ret(RetStmt { value: None }))
                }
            }
            Unreachable => Ok(Stmt::Unreachable),
            other => Err(LowerError::UnsupportedOpcode(format!("{other:?}"))),
        }
    }
}

impl<'c> TryFrom<InstructionValue<'c>> for Expr {
    type Error = LowerError;

    fn try_from(op: InstructionValue<'c>) -> Result<Self, Self::Error> {
        match op.get_opcode() {
            Add | FAdd | Sub | FSub | Mul | FMul | SDiv | UDiv | FDiv | SRem | URem | FRem
            | Shl | LShr | AShr | And | Or | Xor => {
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
                let binop =
                    BinOp::from_opcode(op.get_opcode()).ok_or(LowerError::UnsupportedOpcode(
                        format!("{:?} without a BinOp mapping", op.get_opcode()),
                    ))?;
                Ok(Expr::BinaryOp(BinaryExpr {
                    op: binop,
                    arg1: Box::new(op1_expr),
                    arg2: Box::new(op2_expr),
                }))
            }
            ICmp | FCmp => {
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
                let pred = match op.get_opcode() {
                    ICmp => Cmp::Int(IntCmp::from(op.get_icmp_predicate().ok_or(
                        LowerError::UnsupportedOpcode("icmp without predicate".into()),
                    )?)),
                    _ => Cmp::Float(FloatCmp::from(op.get_fcmp_predicate().ok_or(
                        LowerError::UnsupportedOpcode("fcmp without predicate".into()),
                    )?)),
                };
                Ok(Expr::Cmp(CmpExpr {
                    pred,
                    arg1: Box::new(op1_expr),
                    arg2: Box::new(op2_expr),
                }))
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
                Ok(Expr::Var(VarExpr {
                    name,
                    dtype: Ty::from(bve),
                }))
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
                Ok(Expr::Var(VarExpr {
                    name,
                    dtype: Ty::from(bve),
                }))
            }
        } else if bve.is_pointer_value() {
            let ptr = bve.into_pointer_value();
            if let Some(inst) = ptr.as_instruction() {
                Expr::try_from(inst)
            } else if ptr.is_null() {
                Ok(Expr::Literal(Lit::NullPtr))
            } else {
                let name = ptr.get_name().to_string_lossy().into_owned();
                Ok(Expr::Var(VarExpr {
                    name,
                    dtype: Ty::from(bve),
                }))
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
    use crate::{
        build_module,
        hir::{
            cfg::Cfg,
            flow::{dominators, natural_loops},
            func::Function,
            stmt::Stmt,
            verify::{are_equivalent, structured_to_cfg},
        },
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

    /// The graph keeps LLVM's emission order, which is what the goto baseline
    /// falls through along.
    #[test]
    fn graph_order_follows_llvm_emission_order() {
        let ir = r#"
define void @f() {
entry:
  br label %second
second:
  br label %third
third:
  ret void
}
"#;
        let cfg = build_cfg(ir);
        assert_eq!(
            cfg.order,
            vec![
                "entry".to_string(),
                "second".to_string(),
                "third".to_string()
            ]
        );
    }

    /// A dotted label is named the same way by the graph key and by the
    /// statement that points at it, so the two paths cannot name one function
    /// two different ways.
    #[test]
    fn dotted_labels_name_the_graph_and_the_statement_the_same_way() {
        let ir = r#"
define void @f(i1 %c) {
entry:
  br i1 %c, label %if.then, label %if.end
if.then:
  br label %if.end
if.end:
  ret void
}
"#;
        let cfg = build_cfg(ir);
        assert!(cfg.blocks.contains_key("if_then"));
        assert_eq!(
            cfg.blocks["entry"].succ,
            vec!["if_then".to_string(), "if_end".to_string()]
        );
        assert_eq!(
            cfg.blocks["if_end"].pred,
            vec!["entry".to_string(), "if_then".to_string()]
        );
    }

    /// An edge is recorded once, even when a terminator names the same block
    /// twice, so `collapse_chains` and the colour fingerprint see the same
    /// multiplicity as the other builder.
    #[test]
    fn a_duplicate_target_is_recorded_once() {
        let ir = r#"
define void @f(i32 %x) {
entry:
  switch i32 %x, label %join [ i32 0, label %join
    i32 1, label %join ]
join:
  ret void
}
"#;
        let cfg = build_cfg(ir);
        assert_eq!(cfg.blocks["entry"].succ, vec!["join".to_string()]);
        assert_eq!(cfg.blocks["join"].pred, vec!["entry".to_string()]);
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
        // Arm order is the node's: the then arm, then the else arm. LLVM stores
        // a conditional branch as [cond, else, then], so this is not the operand
        // order.
        assert_eq!(
            cfg.blocks["entry"].succ,
            vec!["t".to_string(), "f".to_string()]
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
        let module = crate::build_module(&ctxt, &ir_str).unwrap();
        let func = module.get_first_function().unwrap();
        let f = Function::try_from(&func).expect("Function lowering failed");
        assert_eq!(f.cfg.blocks["entry"].stmts.len(), 2);
        assert!(matches!(&f.cfg.blocks["entry"].stmts[0], Stmt::Let { .. }));
        assert!(matches!(
            &f.cfg.blocks["entry"].stmts[1],
            Stmt::Branch { .. }
        ));
    }

    /* //TODO: Re-add this test when support for Phi instructions is working
        #[test]
        fn lower_and_structurize_simple_loop() {
            let ir = r#"define void @f(i32 %n) {
            entry: 
              br label %header
            header: 
              %i = phi i32 [0, %entry], [%i2, %body]
              %c = icmp slt i32 %i, %n
              br i1 %c, label %body, label %exit 
            body: 
              %i2 = add i32 %i, 1 
              br label %header
            exit: 
              ret void 
            }"#;
            let ctxt = inkwell::context::Context::create();
            let module = build_module(&ctxt, ir).unwrap();
            let func_val = module.get_first_function().unwrap();
            let mut func = Function::try_from(&func_val).unwrap();
            let func_imut = func.clone();
            let body = func.structurize().unwrap();

            assert!(matches!(body[0], Stmt::Loop { .. }));
            assert!(are_equivalent(&func_imut.cfg, &structured_to_cfg(body)));
        }
    */
    #[test]
    fn lower_and_structurize_simple_if() {
        let ir = r#"define i32 @f(i32 %x) {
            entry: 
              %c = icmp sgt i32 %x, 0
              br i1 %c, label %pos, label %neg
            pos: 
              ret i32 1
            neg: 
              ret i32 0
            }"#;
        let ctxt = inkwell::context::Context::create();
        let module = build_module(&ctxt, ir).unwrap();
        let func_val = module.get_first_function().unwrap();
        let mut func = Function::try_from(&func_val).unwrap();
        let func_imut = func.clone();
        let body = func.structurize().unwrap();

        // The structured body carries the statements, so the comparison is the
        // assignment then the fork.
        assert!(matches!(body[0], Stmt::Let { .. }), "got {body:?}");
        assert!(matches!(body[1], Stmt::If { .. }), "got {body:?}");
        assert!(are_equivalent(&func_imut.cfg, &structured_to_cfg(body)));
    }
}
