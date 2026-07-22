//! High-Level IR — immutable, cacheable intermediate representation.
//!
//! HIR dibangun dari AST via builder, dan di-cache per module.
//! Tipe data di-resolve, parameter di-substitute, generate di-unroll.

use std::collections::HashMap;
use std::sync::Arc;

use crate::intern::Symbol;

// ─── HIR Types ───

/// Resolved type in HIR.
#[derive(Debug, Clone, PartialEq)]
pub enum HirType {
    /// Bit vector: logic [MSB:LSB]
    BitVec { width: usize },
    /// Signed integer
    Int { width: usize },
    /// Unsigned integer
    UInt { width: usize },
    /// Real (double precision)
    Real,
    /// String
    String,
    /// Void (for tasks)
    Void,
    /// User-defined type (resolved)
    Named { name: Symbol, width: usize },
    /// Packed array
    PackedArray {
        elem: Box<HirType>,
        dims: Vec<(usize, usize)>,
    },
    /// Unpacked array
    UnpackedArray {
        elem: Box<HirType>,
        dims: Vec<(usize, usize)>,
    },
    /// Struct
    Struct { fields: Vec<HirStructField> },
    /// Enum
    Enum {
        base: Box<HirType>,
        variants: Vec<(Symbol, Option<u64>)>,
    },
}

impl HirType {
    /// Bit width of this type.
    pub fn width(&self) -> usize {
        match self {
            HirType::BitVec { width } => *width,
            HirType::Int { width } => *width,
            HirType::UInt { width } => *width,
            HirType::Real => 64,
            HirType::String => 0, // dynamic
            HirType::Void => 0,
            HirType::Named { width, .. } => *width,
            HirType::PackedArray { elem, dims } => {
                dims.iter().map(|(hi, lo)| hi - lo + 1).product::<usize>() * elem.width()
            }
            HirType::UnpackedArray { .. } => 0, // runtime-sized
            HirType::Struct { fields } => fields.iter().map(|f| f.width).sum(),
            HirType::Enum { base, .. } => base.width(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirStructField {
    pub name: Symbol,
    pub dtype: HirType,
    pub width: usize,
}

// ─── HIR Expression ───

#[derive(Debug, Clone, PartialEq)]
pub enum HirExpr {
    IntLiteral(u64, usize),
    RealLiteral(f64),
    StringLiteral(Symbol),
    Ident(Symbol),
    Binary {
        op: HirBinOp,
        lhs: Box<HirExpr>,
        rhs: Box<HirExpr>,
        width: usize,
    },
    Unary {
        op: HirUnOp,
        operand: Box<HirExpr>,
        width: usize,
    },
    Ternary {
        cond: Box<HirExpr>,
        then: Box<HirExpr>,
        else_: Box<HirExpr>,
        width: usize,
    },
    BitSelect {
        base: Box<HirExpr>,
        index: Box<HirExpr>,
        width: usize,
    },
    PartSelect {
        base: Box<HirExpr>,
        msb: Box<HirExpr>,
        lsb: Box<HirExpr>,
        width: usize,
    },
    Concat {
        parts: Vec<HirExpr>,
        width: usize,
    },
    Call {
        func: Symbol,
        args: Vec<HirExpr>,
        width: usize,
    },
    FillLit {
        val: u8,
        width: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HirBinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    BitAnd,
    BitOr,
    BitXor,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    LogicAnd,
    LogicOr,
    Shl,
    Shr,
    Sar,
    Power,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HirUnOp {
    Neg,
    BitNot,
    LogicNot,
}

// ─── HIR Statement ───

#[derive(Debug, Clone, PartialEq)]
pub enum HirStmt {
    Block {
        stmts: Vec<HirStmt>,
        name: Option<Symbol>,
    },
    NonBlockingAssign {
        lhs: Box<HirExpr>,
        rhs: Box<HirExpr>,
    },
    BlockingAssign {
        lhs: Box<HirExpr>,
        rhs: Box<HirExpr>,
    },
    If {
        cond: Box<HirExpr>,
        then: Box<HirStmt>,
        else_: Option<Box<HirStmt>>,
    },
    Case {
        expr: Box<HirExpr>,
        items: Vec<HirCaseItem>,
    },
    For {
        init: Box<HirStmt>,
        cond: Box<HirExpr>,
        step: Box<HirStmt>,
        body: Box<HirStmt>,
    },
    While {
        cond: Box<HirExpr>,
        body: Box<HirStmt>,
    },
    Repeat {
        count: Box<HirExpr>,
        body: Box<HirStmt>,
    },
    Forever {
        body: Box<HirStmt>,
    },
    Display {
        args: Vec<HirExpr>,
    },
    Finish(i64),
    Return {
        value: Option<Box<HirExpr>>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirCaseItem {
    pub exprs: Vec<HirExpr>,
    pub stmt: Box<HirStmt>,
}

// ─── HIR Module ───

/// Elaborated module in HIR.
#[derive(Debug, Clone)]
pub struct HirModule {
    pub name: Symbol,
    pub signals: Vec<HirSignal>,
    pub inputs: Vec<usize>,
    pub outputs: Vec<usize>,
    pub params: Vec<HirParam>,
    pub stmts: Vec<HirStmt>,
    pub sub_instances: Vec<HirInstance>,
    pub checksum: u64,
}

#[derive(Debug, Clone)]
pub struct HirSignal {
    pub name: Symbol,
    pub dtype: HirType,
    pub width: usize,
    pub is_input: bool,
    pub is_output: bool,
}

#[derive(Debug, Clone)]
pub struct HirParam {
    pub name: Symbol,
    pub dtype: HirType,
    pub default: Option<HirExpr>,
    pub is_local: bool,
}

#[derive(Debug, Clone)]
pub struct HirInstance {
    pub module_name: Symbol,
    pub instance_name: Symbol,
    pub connections: Vec<(Symbol, HirExpr)>,
    pub params: Vec<(Symbol, HirExpr)>,
}

// ─── HIR Design ───

/// Complete HIR design (all modules).
#[derive(Debug, Clone)]
pub struct HirDesign {
    pub modules: HashMap<Symbol, Arc<HirModule>>,
    pub top: Arc<HirModule>,
}
