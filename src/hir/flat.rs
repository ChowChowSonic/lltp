//! Scope (bare bones, `clang -O0 -fno-discard-value-names` output):
//! - supported: alloca, load/store through an alloca, integer/float binops,
//!   icmp/fcmp, scalar casts, direct calls, br, ret, unreachable

use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt;

use inkwell::basic_block::BasicBlock;
use inkwell::module::Module;
use inkwell::types::BasicTypeEnum;
use inkwell::values::{
    BasicValueEnum, FunctionValue, InstructionOpcode as Op, InstructionValue, Operand,
};

use super::op::{BinOp, CastKind, Cmp, FloatCmp, IntCmp};
use super::ty::Ty;

/// An operand: already resolved to a C-friendly name or a constant.
#[derive(Debug, Clone)]
pub enum Val {
    /// A named local
    Var(String, Ty),
    Addr(String),
    Int {
        value: i64,
        bits: usize,
    },
    Float {
        value: f64,
        bits: usize,
    },
    Null,
}

impl Val {
    pub fn ty(&self) -> Ty {
        match self {
            Val::Var(_, t) => t.clone(),
            Val::Addr(_) | Val::Null => {
                Ty::Ptr(Box::new(Ty::Opaque("unresolved pointee".to_string())))
            }
            Val::Int { bits: 1, .. } => Ty::Bool,
            Val::Int { bits, .. } => Ty::Int(*bits, true),
            Val::Float { bits, .. } => Ty::Float(*bits),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Inst {
    /// `dest = *ptr` where `ptr` is an alloca slot.
    Load { dest: String, ptr: String },
    /// `*ptr = value` where `ptr` is an alloca slot.
    Store { ptr: String, value: Val },
    Bin {
        dest: String,
        ty: Ty,
        op: BinOp,
        lhs: Val,
        rhs: Val,
    },
    Cmp {
        dest: String,
        pred: Cmp,
        lhs: Val,
        rhs: Val,
    },
    Cast {
        dest: String,
        ty: Ty,
        kind: CastKind,
        src: Val,
    },
    Call {
        dest: Option<String>,
        callee: String,
        args: Vec<Val>,
    },
}

#[derive(Debug, Clone)]
pub enum Term {
    Br(String),
    CondBr {
        cond: Val,
        then_bb: String,
        else_bb: String,
    },
    Ret(Option<Val>),
    Unreachable,
}

#[derive(Debug, Clone)]
pub struct FlatBlock {
    pub label: String,
    pub insts: Vec<Inst>,
    pub term: Term,
}

#[derive(Debug, Clone)]
pub struct FnSig {
    pub name: String,
    pub params: Vec<Ty>,
    pub ret: Ty,
    pub variadic: bool,
}

#[derive(Debug, Clone)]
pub struct FlatFunction {
    pub sig: FnSig,
    pub param_names: Vec<String>,
    /// Every alloca slot and every instruction result, declared at function
    /// top by the emitter (so `goto` never jumps over an initialization).
    pub locals: Vec<(String, Ty)>,
    pub blocks: Vec<FlatBlock>,
}

#[derive(Debug, Clone)]
pub struct FlatModule {
    /// External declarations
    pub decls: Vec<FnSig>,
    pub funcs: Vec<FlatFunction>,
}

#[derive(Debug)]
pub struct RecoverError {
    pub func: String,
    pub detail: String,
}

impl fmt::Display for RecoverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cannot recover `{}`: {}", self.func, self.detail)
    }
}

impl Error for RecoverError {}

impl FlatModule {
    pub fn from_module(module: &Module<'_>) -> Result<Self, RecoverError> {
        // Function names are reserved so no local gets the same C identifier.
        let reserved: HashSet<String> = module
            .get_functions()
            .map(|f| f.get_name().to_string_lossy().into_owned())
            .collect();
        let mut decls = Vec::new();
        let mut funcs = Vec::new();
        for f in module.get_functions() {
            if f.count_basic_blocks() == 0 {
                decls.push(sig_of(&f));
            } else {
                funcs.push(FlatFunction::from_llvm(&f, &reserved)?);
            }
        }
        Ok(FlatModule { decls, funcs })
    }
}

impl FlatFunction {
    pub fn from_llvm(
        f: &FunctionValue<'_>,
        reserved: &HashSet<String>,
    ) -> Result<Self, RecoverError> {
        build(f, reserved).map_err(|detail| RecoverError {
            func: f.get_name().to_string_lossy().into_owned(),
            detail,
        })
    }
}

// ---------------------------------------------------------------------------
// naming
// ---------------------------------------------------------------------------

const C_KEYWORDS: &[&str] = &[
    "auto", "break", "case", "char", "const", "continue", "default", "do", "double", "else",
    "enum", "extern", "float", "for", "goto", "if", "inline", "int", "long", "register",
    "restrict", "return", "short", "signed", "sizeof", "static", "struct", "switch", "typedef",
    "union", "unsigned", "void", "volatile", "while", "_Bool",
];

fn sanitize(raw: &str) -> String {
    let mut s: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.chars().next().is_none_or(|c| c.is_ascii_digit()) {
        s.insert(0, 'v');
    }
    if C_KEYWORDS.contains(&s.as_str()) {
        s.push('_');
    }
    s
}

struct Namer {
    used: HashSet<String>,
    next_tmp: usize,
}

impl Namer {
    fn new(reserved: &HashSet<String>) -> Self {
        Namer {
            used: reserved.clone(),
            next_tmp: 0,
        }
    }

    /// Unique C identifier derived from `hint`; empty hint -> `t0`, `t1`, ...
    fn value(&mut self, hint: &str) -> String {
        let base = if hint.is_empty() {
            let t = format!("t{}", self.next_tmp);
            self.next_tmp += 1;
            t
        } else {
            sanitize(hint)
        };
        if self.used.insert(base.clone()) {
            return base;
        }
        let mut n = 1;
        loop {
            let cand = format!("{base}_{n}");
            if self.used.insert(cand.clone()) {
                return cand;
            }
            n += 1;
        }
    }
}

fn value_name(v: &BasicValueEnum<'_>) -> String {
    match v {
        BasicValueEnum::IntValue(x) => x.get_name().to_string_lossy().into_owned(),
        BasicValueEnum::FloatValue(x) => x.get_name().to_string_lossy().into_owned(),
        BasicValueEnum::PointerValue(x) => x.get_name().to_string_lossy().into_owned(),
        _ => String::new(),
    }
}

// ---------------------------------------------------------------------------
// recovery
// ---------------------------------------------------------------------------

fn sig_of(f: &FunctionValue<'_>) -> FnSig {
    let ft = f.get_type();
    FnSig {
        name: f.get_name().to_string_lossy().into_owned(),
        params: f.get_params().iter().map(|p| Ty::from(*p)).collect(),
        ret: ft.get_return_type().map(Ty::from).unwrap_or(Ty::Void),
        variadic: ft.is_var_arg(),
    }
}

fn result_ty(inst: InstructionValue<'_>) -> Result<Ty, String> {
    let basic = BasicTypeEnum::try_from(inst.get_type()).map_err(|_| {
        format!(
            "result type of `{:?}` is not a first-class scalar",
            inst.get_opcode()
        )
    })?;
    Ok(Ty::from(basic))
}

fn alloca_ty(inst: InstructionValue<'_>) -> Result<Ty, String> {
    let t = inst.get_allocated_type().map_err(|e| e.to_string())?;
    Ok(Ty::from(t))
}

#[derive(PartialEq)]
enum CallKind {
    Skip,
    Void,
    Value,
}

fn callee_name(inst: InstructionValue<'_>) -> Result<String, String> {
    let n = inst.get_num_operands();
    if n == 0 {
        return Err("call without operands".to_string());
    }
    match inst.get_operand(n - 1).and_then(|o| o.value()) {
        Some(BasicValueEnum::PointerValue(p)) if p.as_instruction().is_none() => {
            Ok(p.get_name().to_string_lossy().into_owned())
        }
        _ => Err("indirect calls are not supported yet".to_string()),
    }
}

fn call_kind(inst: InstructionValue<'_>) -> Result<CallKind, String> {
    if callee_name(inst)?.starts_with("llvm.dbg.") {
        Ok(CallKind::Skip)
    } else if inst.get_type().is_void_type() {
        Ok(CallKind::Void)
    } else {
        Ok(CallKind::Value)
    }
}

/// Does this instruction produce a named local?
fn defines_value(inst: InstructionValue<'_>) -> Result<bool, String> {
    match inst.get_opcode() {
        Op::Alloca
        | Op::Load
        | Op::ICmp
        | Op::FCmp
        | Op::Trunc
        | Op::ZExt
        | Op::SExt
        | Op::FPToUI
        | Op::FPToSI
        | Op::UIToFP
        | Op::SIToFP
        | Op::FPTrunc
        | Op::FPExt => Ok(true),
        Op::Store | Op::Br | Op::Return | Op::Unreachable => Ok(false),
        Op::Call => Ok(call_kind(inst)? == CallKind::Value),
        op if BinOp::from_opcode(op).is_some() => Ok(true),
        other => Err(format!("unsupported instruction `{other:?}`")),
    }
}

struct Cx<'ctx> {
    names: HashMap<InstructionValue<'ctx>, String>,
    params: HashMap<String, String>,
    labels: HashMap<BasicBlock<'ctx>, String>,
}

impl<'ctx> Cx<'ctx> {
    fn name(&self, inst: InstructionValue<'ctx>) -> Result<String, String> {
        self.names
            .get(&inst)
            .cloned()
            .ok_or_else(|| format!("no name recorded for `{:?}`", inst.get_opcode()))
    }

    fn param(&self, raw: &str) -> Result<String, String> {
        self.params.get(raw).cloned().ok_or_else(|| {
            format!("operand `{raw}` is not an instruction, constant or parameter (a global?)")
        })
    }

    fn label(&self, op: Option<Operand<'ctx>>) -> Result<String, String> {
        let bb = op
            .and_then(|o| o.block())
            .ok_or("expected a block operand")?;
        self.labels
            .get(&bb)
            .cloned()
            .ok_or_else(|| "branch to an unknown block".to_string())
    }

    /// Pointer operand of a load/store: must be an alloca slot.
    fn slot(&self, op: Option<Operand<'ctx>>) -> Result<String, String> {
        if let Some(BasicValueEnum::PointerValue(p)) = op.and_then(|o| o.value())
            && let Some(inst) = p.as_instruction()
            && inst.get_opcode() == Op::Alloca
        {
            return self.name(inst);
        }
        Err(
            "memory access through a non-alloca pointer (GEP, global, loaded pointer) \
             is not supported yet"
                .to_string(),
        )
    }

    fn val(&self, op: Option<Operand<'ctx>>) -> Result<Val, String> {
        let bve = op
            .and_then(|o| o.value())
            .ok_or("missing or non-value operand")?;
        match bve {
            BasicValueEnum::IntValue(i) => {
                if let Some(inst) = i.as_instruction() {
                    Ok(Val::Var(self.name(inst)?, Ty::from(bve)))
                } else if i.is_const() {
                    let bits = i.get_type().get_bit_width() as usize;
                    let value = i
                        .get_sign_extended_constant()
                        .ok_or("integer constant wider than 64 bits")?;
                    Ok(Val::Int { value, bits })
                } else {
                    let raw = i.get_name().to_string_lossy().into_owned();
                    Ok(Val::Var(self.param(&raw)?, Ty::from(bve)))
                }
            }
            BasicValueEnum::FloatValue(f) => {
                if let Some(inst) = f.as_instruction() {
                    Ok(Val::Var(self.name(inst)?, Ty::from(bve)))
                } else if f.is_const() {
                    let bits = f.get_type().get_bit_width() as usize;
                    let (value, _lossy) = f.get_constant().ok_or("unreadable float constant")?;
                    Ok(Val::Float { value, bits })
                } else {
                    let raw = f.get_name().to_string_lossy().into_owned();
                    Ok(Val::Var(self.param(&raw)?, Ty::from(bve)))
                }
            }
            BasicValueEnum::PointerValue(p) => {
                if let Some(inst) = p.as_instruction() {
                    let n = self.name(inst)?;
                    if inst.get_opcode() == Op::Alloca {
                        Ok(Val::Addr(n))
                    } else {
                        Ok(Val::Var(n, Ty::from(bve)))
                    }
                } else if p.is_null() {
                    Ok(Val::Null)
                } else {
                    let raw = p.get_name().to_string_lossy().into_owned();
                    Ok(Val::Var(self.param(&raw)?, Ty::from(bve)))
                }
            }
            _ => Err("unsupported operand kind (struct/vector/array value)".to_string()),
        }
    }
}

fn build(f: &FunctionValue<'_>, reserved: &HashSet<String>) -> Result<FlatFunction, String> {
    let sig = sig_of(f);
    let mut vnames = Namer::new(reserved);
    let mut lnames = Namer::new(&HashSet::new());

    // parameters
    let mut param_names = Vec::new();
    let mut params = HashMap::new();
    for (i, p) in f.get_params().iter().enumerate() {
        let raw = value_name(p);
        if raw.is_empty() {
            return Err(format!(
                "parameter {i} has no name; compile with -fno-discard-value-names \
                 (the lltp frontend does by default)"
            ));
        }
        let c = vnames.value(&raw);
        params.insert(raw, c.clone());
        param_names.push(c);
    }

    // block labels
    let blocks = f.get_basic_blocks();
    let mut labels = HashMap::new();
    for (i, bb) in blocks.iter().enumerate() {
        let raw = bb.get_name().to_string_lossy().into_owned();
        let hint = if raw.is_empty() {
            format!("bb{i}")
        } else {
            raw
        };
        labels.insert(*bb, lnames.value(&hint));
    }

    // pass 1: name every value-producing instruction (uses may precede
    // defs in block layout order, so this must be a separate pass)
    let mut names = HashMap::new();
    let mut locals = Vec::new();
    for bb in &blocks {
        for inst in bb.get_instructions() {
            if !defines_value(inst)? {
                continue;
            }
            let hint = inst
                .get_name()
                .map(|c| c.to_string_lossy().into_owned())
                .unwrap_or_default();
            let n = vnames.value(&hint);
            let ty = if inst.get_opcode() == Op::Alloca {
                alloca_ty(inst)?
            } else {
                result_ty(inst)?
            };
            locals.push((n.clone(), ty));
            names.insert(inst, n);
        }
    }

    let cx = Cx {
        names,
        params,
        labels,
    };

    // pass 2: translate
    let mut out_blocks = Vec::new();
    for bb in &blocks {
        let mut insts = Vec::new();
        let mut term = None;
        for inst in bb.get_instructions() {
            let opc = inst.get_opcode();
            let opnd = |i: u32| inst.get_operand(i);
            match opc {
                Op::Alloca => {}
                Op::Load => insts.push(Inst::Load {
                    dest: cx.name(inst)?,
                    ptr: cx.slot(opnd(0))?,
                }),
                Op::Store => insts.push(Inst::Store {
                    value: cx.val(opnd(0))?,
                    ptr: cx.slot(opnd(1))?,
                }),
                Op::ICmp => {
                    let p = inst.get_icmp_predicate().ok_or("icmp without predicate")?;
                    insts.push(Inst::Cmp {
                        dest: cx.name(inst)?,
                        pred: Cmp::Int(IntCmp::from(p)),
                        lhs: cx.val(opnd(0))?,
                        rhs: cx.val(opnd(1))?,
                    });
                }
                Op::FCmp => {
                    let p = inst.get_fcmp_predicate().ok_or("fcmp without predicate")?;
                    insts.push(Inst::Cmp {
                        dest: cx.name(inst)?,
                        pred: Cmp::Float(FloatCmp::from(p)),
                        lhs: cx.val(opnd(0))?,
                        rhs: cx.val(opnd(1))?,
                    });
                }
                Op::Call => match call_kind(inst)? {
                    CallKind::Skip => {}
                    kind => {
                        let n = inst.get_num_operands();
                        let args = (0..n - 1)
                            .map(|i| cx.val(opnd(i)))
                            .collect::<Result<Vec<_>, _>>()?;
                        insts.push(Inst::Call {
                            dest: if kind == CallKind::Value {
                                Some(cx.name(inst)?)
                            } else {
                                None
                            },
                            callee: callee_name(inst)?,
                            args,
                        });
                    }
                },
                Op::Br => {
                    term = Some(match inst.get_num_operands() {
                        1 => Term::Br(cx.label(opnd(0))?),
                        // LLVM operand order for a conditional br: cond, false, true
                        3 => Term::CondBr {
                            cond: cx.val(opnd(0))?,
                            then_bb: cx.label(opnd(2))?,
                            else_bb: cx.label(opnd(1))?,
                        },
                        n => return Err(format!("br with {n} operands")),
                    });
                }
                Op::Return => {
                    term = Some(if inst.get_num_operands() == 0 {
                        Term::Ret(None)
                    } else {
                        Term::Ret(Some(cx.val(opnd(0))?))
                    });
                }
                Op::Unreachable => term = Some(Term::Unreachable),
                other => {
                    if let Some(kind) = CastKind::from_opcode(other) {
                        insts.push(Inst::Cast {
                            dest: cx.name(inst)?,
                            ty: result_ty(inst)?,
                            kind,
                            src: cx.val(opnd(0))?,
                        });
                    } else if let Some(op) = BinOp::from_opcode(other) {
                        insts.push(Inst::Bin {
                            dest: cx.name(inst)?,
                            ty: result_ty(inst)?,
                            op,
                            lhs: cx.val(opnd(0))?,
                            rhs: cx.val(opnd(1))?,
                        });
                    } else {
                        return Err(format!("unsupported instruction `{other:?}`"));
                    }
                }
            }
        }
        let label = cx.labels[bb].clone();
        out_blocks.push(FlatBlock {
            label,
            insts,
            term: term.ok_or("basic block without a supported terminator")?,
        });
    }

    Ok(FlatFunction {
        sig,
        param_names,
        locals,
        blocks: out_blocks,
    })
}
