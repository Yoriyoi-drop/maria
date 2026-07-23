use std::collections::HashMap;
use crate::intern::Symbol;

pub type SignalId = usize;
pub type ClassId = usize;
pub type ObjId = usize;

#[derive(Debug, Clone, PartialEq)]
pub struct IrDesign {
    pub top: IrModule,
    pub modules: HashMap<Symbol, IrModule>,
    pub classes: HashMap<Symbol, IrClassDef>,
    pub covergroups: Vec<IrCovergroup>,
    pub dpi_imports: Vec<IrDpiImport>,
    pub hier_signal_map: HashMap<Symbol, SignalId>,
    pub udp_defs: Vec<crate::ast::types::UdpDef>,
    pub specify_items: Vec<crate::ast::types::SpecifyItem>,
    pub timescale: Option<(String, String)>,
    /// Module-level recursive function declarations — kept for runtime evaluation (not inlined)
    pub module_functions: HashMap<Symbol, crate::ast::types::FunctionDecl>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IrDpiImport {
    pub name: Symbol,
    pub return_width: usize,
    pub arg_widths: Vec<usize>,
    pub is_task: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IrCovergroup {
    pub name: Symbol,
    pub coverpoints: Vec<IrCoverpoint>,
    pub crosses: Vec<IrCross>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IrCoverpoint {
    pub name: Symbol,
    pub expr: IrExpr,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IrCross {
    pub name: Symbol,
    pub coverpoints: Vec<Symbol>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IrTypeParam {
    pub name: Symbol,
    pub default_type: Option<crate::ast::types::DataType>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IrClassDef {
    pub name: Symbol,
    pub extends: Option<Symbol>,
    pub type_params: Vec<IrTypeParam>,
    pub fields: Vec<IrClassField>,
    pub methods: Vec<IrClassMethod>,
    pub constraints: Vec<(Symbol, Vec<crate::ast::types::ConstraintItem>)>,
    pub rand_fields: Vec<Symbol>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ObjectData {
    pub class_name: Symbol,
    pub fields: HashMap<Symbol, LogicVec>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IrClassField {
    pub name: Symbol,
    pub width: usize,
    pub array_depth: usize,
    pub elem_width: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IrClassMethod {
    pub name: Symbol,
    pub is_task: bool,
    pub virtual_flag: bool,
    pub is_static: bool,
    pub ports: Vec<crate::ast::FunctionPort>,
    pub decls: Vec<crate::ast::Decl>,
    pub stmts: Vec<crate::ast::Stmt>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IrModule {
    pub name: Symbol,
    pub signals: Vec<SignalInfo>,
    pub inputs: Vec<SignalId>,
    pub outputs: Vec<SignalId>,
    pub inouts: Vec<SignalId>,
    pub processes: Vec<Process>,
    pub sub_instances: Vec<IrInstance>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NetType {
    Wire,
    Wand,
    Wor,
    Tri,
    Tri0,
    Tri1,
    TriAnd,
    TriOr,
    Supply0,
    Supply1,
}

impl NetType {
    pub fn resolve_bit(&self, current: LogicVal, incoming: LogicVal) -> LogicVal {
        match self {
            NetType::Wand | NetType::TriAnd | NetType::Supply0 => {
                // Wired-AND: Z = transparent, otherwise AND
                match (current, incoming) {
                    (LogicVal::X, _) | (_, LogicVal::X) => LogicVal::X,
                    (LogicVal::Z, v) => v,
                    (v, LogicVal::Z) => v,
                    (LogicVal::Zero, _) | (_, LogicVal::Zero) => LogicVal::Zero,
                    _ => LogicVal::One,
                }
            }
            NetType::Wor | NetType::TriOr | NetType::Supply1 => {
                // Wired-OR: Z = transparent, otherwise OR
                match (current, incoming) {
                    (LogicVal::X, _) | (_, LogicVal::X) => LogicVal::X,
                    (LogicVal::Z, v) => v,
                    (v, LogicVal::Z) => v,
                    (LogicVal::One, _) | (_, LogicVal::One) => LogicVal::One,
                    _ => LogicVal::Zero,
                }
            }
            NetType::Tri | NetType::Tri0 | NetType::Tri1 | NetType::Wire => {
                // Tri-state: exactly one non-Z driver wins; conflict = X
                match (current, incoming) {
                    (LogicVal::Z, v) => v,
                    (v, LogicVal::Z) => v,
                    (LogicVal::X, _) | (_, LogicVal::X) => LogicVal::X,
                    (LogicVal::Zero, LogicVal::One) | (LogicVal::One, LogicVal::Zero) => {
                        LogicVal::X
                    }
                    _ => current, // same value
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct StructFieldInfo {
    pub name: Symbol,
    pub offset: usize,
    pub width: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SignalInfo {
    pub name: Symbol,
    pub width: usize,
    pub kind: SignalKind,
    pub net_type: NetType,
    pub multi_driver: bool,
    pub init_val: LogicVec,
    pub array_depth: usize,
    pub elem_width: usize,
    pub array_dims: Vec<usize>,
    pub class_name: Option<Symbol>,
    pub is_string: bool,
    pub is_real: bool,
    pub is_mailbox: bool,
    pub is_semaphore: bool,
    pub is_2state: bool,
    pub is_dynamic: bool,
    pub is_queue: bool,
    pub is_associative: bool,
    pub is_signed: bool,
    pub is_const: bool,
    pub msb: usize,
    pub lsb: usize,
    pub struct_fields: Vec<StructFieldInfo>,
    pub packed_dims: Vec<usize>,
    pub delay_rise: Option<u64>,
    pub delay_fall: Option<u64>,
    pub iface_type: Option<Symbol>,
    pub iface_modport: Option<Symbol>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SignalKind {
    Wire,
    Reg,
    Logic,
    Input,
    Output,
    Inout,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IrInstance {
    pub module_name: Symbol,
    pub instance_name: Symbol,
    pub port_map: HashMap<Symbol, SignalId>,
    pub param_map: HashMap<Symbol, i64>,
    pub type_param_map: HashMap<Symbol, usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Process {
    Combinational {
        name: Symbol,
        sensitivity: Vec<SignalId>,
        body: Vec<IrStmt>,
    },
    CombReactive {
        name: Symbol,
        sensitivity: Vec<SignalId>,
        body: Vec<IrStmt>,
    },
    Sequential {
        name: Symbol,
        clock: ClockEdge,
        reset: Option<ResetInfo>,
        body: Vec<IrStmt>,
    },
    Initial {
        name: Symbol,
        body: Vec<IrStmt>,
    },
    Final {
        name: Symbol,
        body: Vec<IrStmt>,
    },
    AlwaysWithDelay {
        name: Symbol,
        delay: u64,
        body: Vec<IrStmt>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClockEdge {
    PosEdge(SignalId),
    NegEdge(SignalId),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResetInfo {
    pub signal: SignalId,
    pub polarity: bool,
    pub r#async: bool,
    pub value: LogicVec,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CaseType {
    Normal,
    CaseX,
    CaseZ,
}

#[derive(Debug, Clone, PartialEq)]
pub enum IrStmt {
    Block {
        stmts: Vec<IrStmt>,
    },
    NamedBlock {
        name: Symbol,
        stmts: Vec<IrStmt>,
        decls: Vec<crate::ast::Decl>,
    },
    BlockingAssign {
        lhs: IrLValue,
        rhs: IrExpr,
        delay: Option<u64>,
    },
    NonBlockingAssign {
        lhs: IrLValue,
        rhs: IrExpr,
        delay: Option<u64>,
    },
    If {
        cond: IrExpr,
        true_branch: Vec<IrStmt>,
        false_branch: Vec<IrStmt>,
    },
    Case {
        case_type: CaseType,
        expr: IrExpr,
        items: Vec<IrCaseItem>,
        default: Vec<IrStmt>,
    },
    LoopFor {
        init: Option<Box<IrStmt>>,
        cond: IrExpr,
        step: Option<Box<IrStmt>>,
        body: Vec<IrStmt>,
    },
    LoopWhile {
        cond: IrExpr,
        body: Vec<IrStmt>,
    },
    LoopDoWhile {
        cond: IrExpr,
        body: Vec<IrStmt>,
    },
    Repeat {
        count: IrExpr,
        body: Vec<IrStmt>,
    },
    Foreach {
        array_var: IrExpr,
        index_var: Symbol,
        body: Vec<IrStmt>,
    },
    Delay {
        delay: u64,
        body: Vec<IrStmt>,
    },
    Force {
        lvalue: IrLValue,
        rhs: IrExpr,
    },
    Wait {
        cond: IrExpr,
        body: Vec<IrStmt>,
    },
    SysCall {
        name: Symbol,
        args: Vec<IrExpr>,
    },
    SysFinish,
    Null,
    EventControl {
        sig_id: SignalId,
        edge: Option<ClockEdge>,
        body: Vec<IrStmt>,
    },
    EventTrigger {
        sig_id: SignalId,
    },
    MethodCallStmt {
        obj: IrExpr,
        method: Symbol,
        args: Vec<IrExpr>,
        with_clause: Option<Box<IrExpr>>,
    },
    Break,
    Continue,
    Disable {
        name: Symbol,
    },
    Release {
        lvalue: IrLValue,
    },
    Deassign {
        lvalue: IrLValue,
    },
    Fork {
        processes: Vec<Vec<IrStmt>>,
        join_type: IrJoinType,
    },
    Assert {
        cond: IrExpr,
        pass_stmt: Vec<IrStmt>,
        fail_stmt: Vec<IrStmt>,
        clock_event: Option<crate::ast::types::ClockEvent>,
        disable_iff: Option<Box<IrExpr>>,
        sequence: Option<Box<IrSequence>>,
    },
    Assume {
        cond: IrExpr,
        pass_stmt: Vec<IrStmt>,
        fail_stmt: Vec<IrStmt>,
        clock_event: Option<crate::ast::types::ClockEvent>,
        disable_iff: Option<Box<IrExpr>>,
        sequence: Option<Box<IrSequence>>,
    },
    Cover {
        cond: IrExpr,
        pass_stmt: Vec<IrStmt>,
        clock_event: Option<crate::ast::types::ClockEvent>,
        disable_iff: Option<Box<IrExpr>>,
        sequence: Option<Box<IrSequence>>,
    },
    WaitOrder {
        events: Vec<SignalId>,
        failure_stmts: Vec<IrStmt>,
    },
    RandCase {
        items: Vec<(IrExpr, Vec<IrStmt>)>,
    },
    RandSequence {
        productions: Vec<(Symbol, Vec<(IrExpr, Vec<IrStmt>)>)>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum IrJoinType {
    Join,
    JoinAny,
    JoinNone,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IrCaseItem {
    pub labels: Vec<IrExpr>,
    pub body: Vec<IrStmt>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum IrLValue {
    Signal(SignalId, usize),
    RangeSelect(SignalId, usize, usize),
    BitSelect(SignalId, usize),
    ArrayIndex {
        sig_id: SignalId,
        index: Box<IrExpr>,
        elem_width: usize,
    },
    ArrayRangeSelect {
        sig_id: SignalId,
        index: Box<IrExpr>,
        elem_width: usize,
        msb: usize,
        lsb: usize,
    },
    ArrayBitSelect {
        sig_id: SignalId,
        index: Box<IrExpr>,
        elem_width: usize,
        bit: usize,
    },
    Concat(Vec<IrLValue>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum IrExpr {
    Const(LogicVec),
    FillLit(LogicVal),
    Signal(SignalId, usize),
    RangeSelect(SignalId, usize, usize),
    BitSelect(SignalId, usize),
    ExprRangeSelect(Box<IrExpr>, usize, usize),
    ExprBitSelect(Box<IrExpr>, usize),
    ExprPartSelect(Box<IrExpr>, Box<IrExpr>, Box<IrExpr>),
    ArrayIndex {
        sig_id: SignalId,
        index: Box<IrExpr>,
        elem_width: usize,
    },
    Concat(Vec<IrExpr>),
    Replicate(usize, Box<IrExpr>),
    UnaryOp(UnaryIrOp, Box<IrExpr>),
    BinaryOp(BinaryIrOp, Box<IrExpr>, Box<IrExpr>),
    Cond(Box<IrExpr>, Box<IrExpr>, Box<IrExpr>),
    Signed(Box<IrExpr>),
    String(String),
    SysFunc {
        name: Symbol,
        args: Vec<IrExpr>,
    },
    NewCall {
        class_name: Symbol,
        args: Vec<IrExpr>,
    },
    This,
    MethodCall {
        obj: Box<IrExpr>,
        method: Symbol,
        args: Vec<IrExpr>,
        with_clause: Option<Box<IrExpr>>,
    },
    MemberAccess {
        obj: Box<IrExpr>,
        field: Symbol,
    },
    DpiCall {
        name: Symbol,
        args: Vec<IrExpr>,
        return_width: usize,
    },
    HierRef(Symbol),
    Inside {
        expr: Box<IrExpr>,
        list: Vec<IrExpr>,
    },
    Cast {
        width: usize,
        expr: Box<IrExpr>,
    },
    StreamingConcat {
        op: String,
        slice_size: Option<usize>,
        slices: Vec<IrExpr>,
    },
    Dist {
        expr: Box<IrExpr>,
        items: Vec<IrDistItem>,
    },
    UdpLookup {
        udp_name: Symbol,
        args: Vec<IrExpr>,
    },
    /// Runtime function call (used for recursive functions that can't be inlined)
    FuncCall {
        func_name: Symbol,
        args: Vec<IrExpr>,
    },
    /// Virtual interface binding handle (instance name → binding value)
    VifBinding {
        instance_name: Symbol,
    },
    /// Virtual interface member access (resolved at runtime via bound instance)
    VirtualIfaceAccess {
        vif_name: Symbol,
        field: Symbol,
        field_width: usize,
    },
}

/// Temporal sequence expression for property evaluation
#[derive(Debug, Clone, PartialEq)]
pub enum IrSequence {
    /// Immediate Boolean expression (evaluated each cycle)
    Expr(IrExpr),
    /// ##N — wait N clock cycles
    Delay(u64),
    /// ##[min:max] — wait between min and max clock cycles
    DelayRange(u64, u64),
    /// seq1 ##1 seq2 — concatenation (first then second)
    Concat(Box<IrSequence>, Box<IrSequence>),
    /// seq1 or seq2 — either matches
    Or(Box<IrSequence>, Box<IrSequence>),
    /// seq1 and seq2 — both must match
    And(Box<IrSequence>, Box<IrSequence>),
    /// seq[*N] — repeat seq N times consecutively
    Repeat(Box<IrSequence>, u64),
}

impl IrSequence {
    /// Estimate the minimum number of clock cycles this sequence needs to match
    pub fn min_cycles(&self) -> u64 {
        match self {
            IrSequence::Expr(_) => 0,
            IrSequence::Delay(n) => *n,
            IrSequence::DelayRange(min, _) => *min,
            IrSequence::Concat(a, b) => a.min_cycles() + b.min_cycles() + 1,
            IrSequence::Or(a, b) => a.min_cycles().min(b.min_cycles()),
            IrSequence::And(a, b) => a.min_cycles().max(b.min_cycles()),
            IrSequence::Repeat(seq, n) => seq.min_cycles() * n,
        }
    }
    /// Estimate the maximum number of clock cycles before sequence is determined
    pub fn max_cycles(&self) -> Option<u64> {
        match self {
            IrSequence::Expr(_) => Some(0),
            IrSequence::Delay(n) => Some(*n),
            IrSequence::DelayRange(_, max) => Some(*max),
            IrSequence::Concat(a, b) => a
                .max_cycles()
                .and_then(|am| b.max_cycles().map(|bm| am + bm + 1)),
            IrSequence::Or(a, b) => a
                .max_cycles()
                .and_then(|am| b.max_cycles().map(|bm| am.max(bm))),
            IrSequence::And(a, b) => a
                .max_cycles()
                .and_then(|am| b.max_cycles().map(|bm| am.max(bm))),
            IrSequence::Repeat(seq, n) => seq.max_cycles().map(|m| m * n),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct IrDistItem {
    pub range_lo: Option<i64>,
    pub range_hi: Option<i64>,
    pub weight_type: DistWeightType,
    pub weight: i64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DistWeightType {
    Item,
    Range,
}

#[derive(Debug, Clone, PartialEq)]
pub enum UnaryIrOp {
    Plus,
    Minus,
    Not,
    BitNot,
    RedAnd,
    RedNand,
    RedOr,
    RedNor,
    RedXor,
    RedXnor,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BinaryIrOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Power,
    Eq,
    Neq,
    CaseEq,
    CaseNeq,
    EqWild,
    NeqWild,
    Lt,
    Le,
    Gt,
    Ge,
    BitAnd,
    BitOr,
    BitXor,
    BitXnor,
    Shl,
    Shr,
    Sshl,
    Sshr,
    LogicalAnd,
    LogicalOr,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LogicVec {
    pub bits: Vec<LogicVal>,
    pub width: usize,
}

impl Default for LogicVec {
    fn default() -> Self {
        LogicVec::new(1)
    }
}

impl LogicVec {
    pub fn new(width: usize) -> Self {
        let w = if width > 1_000_000 { 1 } else { width };
        LogicVec {
            bits: vec![LogicVal::X; w],
            width: w,
        }
    }

    pub fn fill(val: LogicVal, width: usize) -> Self {
        LogicVec {
            bits: vec![val; width],
            width,
        }
    }

    pub fn from_u64(val: u64, width: usize) -> Self {
        let mut bits = Vec::with_capacity(width);
        for i in 0..width {
            if i < 64 && (val >> i) & 1 == 1 {
                bits.push(LogicVal::One);
            } else {
                bits.push(LogicVal::Zero);
            }
        }
        LogicVec { bits, width }
    }

    pub fn to_u64(&self) -> u64 {
        let mut result = 0u64;
        for i in 0..self.width.min(64) {
            if self.bits[i] == LogicVal::One {
                result |= 1 << i;
            }
        }
        result
    }

    pub fn to_i64(&self) -> i64 {
        let uval = self.to_u64();
        if self.width < 64 {
            let mask = 1u64 << (self.width - 1);
            if uval & mask != 0 {
                (uval | (!0u64 << self.width)) as i64
            } else {
                uval as i64
            }
        } else {
            uval as i64
        }
    }

    pub fn to_bool(&self) -> Option<bool> {
        if self.width == 0 {
            return Some(false);
        }
        let all_x_or_z = self
            .bits
            .iter()
            .all(|b| *b == LogicVal::X || *b == LogicVal::Z);
        if all_x_or_z {
            return None;
        }
        let any_one = self.bits.iter().any(|b| *b == LogicVal::One);
        // In Verilog, X/Z in a conditional is treated as false
        let any_zero_or_x_or_z = self.bits.iter().any(|b| *b == LogicVal::Zero);
        Some(any_one && (!any_zero_or_x_or_z || any_one))
    }

    pub fn resize(&self, new_width: usize) -> Self {
        if new_width <= self.width {
            let mut bits = self.bits.clone();
            bits.truncate(new_width);
            return LogicVec {
                bits,
                width: new_width,
            };
        }
        let mut bits = self.bits.clone();
        bits.resize(new_width, LogicVal::Zero);
        LogicVec {
            bits,
            width: new_width,
        }
    }

    pub fn extend(&self, other: &LogicVec) -> Self {
        let mut bits = self.bits.clone();
        bits.extend_from_slice(&other.bits);
        LogicVec {
            bits,
            width: self.width + other.width,
        }
    }

    pub fn from_hex(hex_str: &str) -> Result<Self, String> {
        let hex = hex_str.trim_start_matches("0x").trim_start_matches("0X");
        let num_bits = hex.len() * 4;
        let val = u64::from_str_radix(hex, 16)
            .map_err(|e| format!("invalid hex '{}': {}", hex_str, e))?;
        Ok(LogicVec::from_u64(val, num_bits.max(1)))
    }

    pub fn from_bin(bin_str: &str) -> Result<Self, String> {
        let bin = bin_str.trim_start_matches("0b").trim_start_matches("0B");
        let num_bits = bin.len();
        let val = u64::from_str_radix(bin, 2)
            .map_err(|e| format!("invalid binary '{}': {}", bin_str, e))?;
        Ok(LogicVec::from_u64(val, num_bits.max(1)))
    }

    pub fn all_x(&self) -> bool {
        self.bits.iter().all(|b| *b == LogicVal::X)
    }

    pub fn all_z(&self) -> bool {
        self.bits.iter().all(|b| *b == LogicVal::Z)
    }

    pub fn casex_eq(&self, other: &LogicVec) -> bool {
        for i in 0..self.width.max(other.width) {
            let val = self.bits.get(i).copied().unwrap_or(LogicVal::Zero);
            let pat = other.bits.get(i).copied().unwrap_or(LogicVal::Zero);
            // In casex: X or Z in the pattern are don't-care (match anything)
            if pat == LogicVal::X || pat == LogicVal::Z {
                continue;
            }
            if val != pat {
                return false;
            }
        }
        true
    }

    pub fn casez_eq(&self, other: &LogicVec) -> bool {
        for i in 0..self.width.max(other.width) {
            let val = self.bits.get(i).copied().unwrap_or(LogicVal::Zero);
            let pat = other.bits.get(i).copied().unwrap_or(LogicVal::Zero);
            // In casez: Z in the pattern is don't-care (match anything)
            if pat == LogicVal::Z {
                continue;
            }
            if val != pat {
                return false;
            }
        }
        true
    }

    pub fn case_eq(&self, other: &LogicVec) -> LogicVec {
        let eq = self.bits == other.bits;
        LogicVec::from_u64(if eq { 1 } else { 0 }, 1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LogicVal {
    Zero,
    One,
    X,
    Z,
}
