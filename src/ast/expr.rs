#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Value(Value),
    FillLit(crate::ir::LogicVal),
    Ident(String),
    FuncCall {
        name: String,
        args: Vec<Expr>,
    },
    RangeSelect {
        expr: Box<Expr>,
        msb: Box<Expr>,
        lsb: Box<Expr>,
    },
    BitSelect {
        expr: Box<Expr>,
        index: Box<Expr>,
    },
    PartSelect {
        expr: Box<Expr>,
        base: Box<Expr>,
        width: Box<Expr>,
    },
    Concat(Vec<Expr>),
    Replicate {
        count: Box<Expr>,
        expr: Box<Expr>,
    },
    UnaryOp {
        op: UnaryOp,
        expr: Box<Expr>,
    },
    BinaryOp {
        op: BinaryOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    TernaryOp {
        cond: Box<Expr>,
        true_expr: Box<Expr>,
        false_expr: Box<Expr>,
    },
    Paren(Box<Expr>),
    String(String),
    MethodCall {
        obj: Box<Expr>,
        method: String,
        args: Vec<Expr>,
    },
    MemberAccess {
        obj: Box<Expr>,
        field: String,
    },
    Null,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Binary {
        bits: String,
        width: Option<usize>,
    },
    Decimal(i64),
    Hex {
        bits: String,
        width: Option<usize>,
    },
    Octal {
        bits: String,
        width: Option<usize>,
    },
    Real(f64),
}

#[derive(Debug, Clone, PartialEq)]
pub enum UnaryOp {
    Plus,
    Minus,
    BitNot,
    Not,
    ReductionAnd,
    ReductionNand,
    ReductionOr,
    ReductionNor,
    ReductionXor,
    ReductionXnor,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BinaryOp {
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
    Shl,
    Shr,
    Sshl,
    Sshr,
    BitAnd,
    BitOr,
    BitXor,
    BitXnor,
    LogicalAnd,
    LogicalOr,
}
