use std::collections::HashMap;

pub type SignalId = usize;
pub type ClassId = usize;
pub type ObjId = usize;

#[derive(Debug, Clone, PartialEq)]
pub struct IrDesign {
    pub top: IrModule,
    pub modules: HashMap<String, IrModule>,
    pub classes: HashMap<String, IrClassDef>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IrClassDef {
    pub name: String,
    pub extends: Option<String>,
    pub fields: Vec<IrClassField>,
    pub methods: Vec<IrClassMethod>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ObjectData {
    pub class_name: String,
    pub fields: HashMap<String, LogicVec>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IrClassField {
    pub name: String,
    pub width: usize,
    pub array_depth: usize,
    pub elem_width: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IrClassMethod {
    pub name: String,
    pub virtual_flag: bool,
    pub ports: Vec<crate::ast::FunctionPort>,
    pub decls: Vec<crate::ast::Decl>,
    pub stmts: Vec<crate::ast::Stmt>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IrModule {
    pub name: String,
    pub signals: Vec<SignalInfo>,
    pub inputs: Vec<SignalId>,
    pub outputs: Vec<SignalId>,
    pub inouts: Vec<SignalId>,
    pub processes: Vec<Process>,
    pub sub_instances: Vec<IrInstance>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SignalInfo {
    pub name: String,
    pub width: usize,
    pub kind: SignalKind,
    pub init_val: LogicVec,
    pub array_depth: usize,
    pub elem_width: usize,
    pub class_name: Option<String>,
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
    pub module_name: String,
    pub instance_name: String,
    pub port_map: HashMap<String, SignalId>,
    pub param_map: HashMap<String, i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Process {
    Combinational {
        name: String,
        sensitivity: Vec<SignalId>,
        body: Vec<IrStmt>,
    },
    Sequential {
        name: String,
        clock: ClockEdge,
        reset: Option<ResetInfo>,
        body: Vec<IrStmt>,
    },
    Initial {
        name: String,
        body: Vec<IrStmt>,
    },
    AlwaysWithDelay {
        name: String,
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
        name: String,
        stmts: Vec<IrStmt>,
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
    Delay {
        delay: u64,
        body: Vec<IrStmt>,
    },
    Wait {
        cond: IrExpr,
        body: Vec<IrStmt>,
    },
    SysCall {
        name: String,
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
        method: String,
        args: Vec<IrExpr>,
    },
    Break,
    Continue,
    Disable {
        name: String,
    },
    Release {
        lvalue: IrLValue,
    },
    Deassign {
        lvalue: IrLValue,
    },
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
        name: String,
        args: Vec<IrExpr>,
    },
    NewCall {
        class_name: String,
        args: Vec<IrExpr>,
    },
    This,
    MethodCall {
        obj: Box<IrExpr>,
        method: String,
        args: Vec<IrExpr>,
    },
    MemberAccess {
        obj: Box<IrExpr>,
        field: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum UnaryIrOp {
    Plus, Minus, Not, BitNot,
    RedAnd, RedNand, RedOr, RedNor, RedXor, RedXnor,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BinaryIrOp {
    Add, Sub, Mul, Div, Mod, Power,
    Eq, Neq, CaseEq, CaseNeq, EqWild, NeqWild,
    Lt, Le, Gt, Ge,
    BitAnd, BitOr, BitXor, BitXnor,
    Shl, Shr, Sshl, Sshr,
    LogicalAnd, LogicalOr,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LogicVec {
    pub bits: Vec<LogicVal>,
    pub width: usize,
}

impl LogicVec {
    pub fn new(width: usize) -> Self {
        LogicVec {
            bits: vec![LogicVal::X; width],
            width,
        }
    }

    pub fn fill(val: LogicVal, width: usize) -> Self {
        LogicVec { bits: vec![val; width], width }
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

    pub fn to_bool(&self) -> Option<bool> {
        if self.width == 0 {
            return Some(false);
        }
        let all_x_or_z = self.bits.iter().all(|b| *b == LogicVal::X || *b == LogicVal::Z);
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
            return LogicVec { bits, width: new_width };
        }
        let mut bits = self.bits.clone();
        bits.resize(new_width, LogicVal::Zero);
        LogicVec { bits, width: new_width }
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
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LogicVal {
    Zero,
    One,
    X,
    Z,
}
