use super::expr::Expr;

#[derive(Debug, Clone, PartialEq)]
pub struct AlwaysBlock {
    pub kind: AlwaysKind,
    pub sensitivity: Option<SensitivityList>,
    pub stmts: Vec<Stmt>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InitialBlock {
    pub stmts: Vec<Stmt>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AlwaysKind {
    Always,
    AlwaysComb,
    AlwaysFF,
    AlwaysLatch,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SensitivityList {
    pub events: Vec<SensitivityEvent>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SensitivityEvent {
    PosEdge(Expr),
    NegEdge(Expr),
    Level(Expr),
    Wildcard,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Block {
        stmts: Vec<Stmt>,
    },
    NamedBlock {
        name: String,
        stmts: Vec<Stmt>,
        decls: Vec<super::types::Decl>,
    },
    IfElse {
        cond: Expr,
        true_branch: Box<Stmt>,
        false_branch: Option<Box<Stmt>>,
    },
    Case {
        expr: Expr,
        items: Vec<CaseItem>,
        default: Option<Box<Stmt>>,
    },
    CaseX {
        expr: Expr,
        items: Vec<CaseItem>,
        default: Option<Box<Stmt>>,
    },
    CaseZ {
        expr: Expr,
        items: Vec<CaseItem>,
        default: Option<Box<Stmt>>,
    },
    StmtCase {
        expr: Expr,
        items: Vec<CaseItem>,
        default: Option<Box<Stmt>>,
    },
    LoopForever {
        stmts: Vec<Stmt>,
    },
    LoopWhile {
        cond: Expr,
        stmts: Vec<Stmt>,
    },
    DoWhile {
        cond: Expr,
        stmts: Vec<Stmt>,
    },
    LoopFor {
        init: Option<Box<Stmt>>,
        cond: Option<Expr>,
        step: Option<Box<Stmt>>,
        stmts: Vec<Stmt>,
    },
    Repeat {
        count: Expr,
        stmts: Vec<Stmt>,
    },
    BlockingAssign {
        lhs: Expr,
        rhs: Expr,
        delay: Option<super::types::Delay>,
    },
    NonBlockingAssign {
        lhs: Expr,
        rhs: Expr,
        delay: Option<super::types::Delay>,
    },
    StmtAssign {
        lhs: Expr,
        rhs: Expr,
    },
    Expr {
        expr: Expr,
    },
    SysCall {
        name: String,
        args: Vec<Expr>,
    },
    SysFinish,
    Delay {
        delay: Expr,
        stmt: Box<Stmt>,
    },
    Wait {
        cond: Expr,
        stmt: Option<Box<Stmt>>,
    },
    Disable {
        name: String,
    },
    Force {
        lhs: Expr,
        rhs: Expr,
    },
    Release {
        expr: Expr,
    },
    Deassign {
        expr: Expr,
    },
    Break,
    Continue,
    Return(Option<Box<Expr>>),
    Null,
    EventControl {
        events: Vec<SensitivityEvent>,
        stmt: Option<Box<Stmt>>,
    },
    EventTrigger {
        name: String,
    },
    ForeachLoop {
        array_var: String,
        index_var: String,
        stmts: Vec<Stmt>,
    },
    // Unique/Priority case qualifiers
    UniqueCase {
        expr: Expr,
        items: Vec<CaseItem>,
        default: Option<Box<Stmt>>,
    },
    PriorityCase {
        expr: Expr,
        items: Vec<CaseItem>,
        default: Option<Box<Stmt>>,
    },
    CaseInside {
        expr: Expr,
        items: Vec<CaseItem>,
        default: Option<Box<Stmt>>,
    },
    // Immediate assertions
    Assert {
        cond: Expr,
        pass_stmt: Option<Box<Stmt>>,
        fail_stmt: Option<Box<Stmt>>,
    },
    Assume {
        cond: Expr,
        pass_stmt: Option<Box<Stmt>>,
        fail_stmt: Option<Box<Stmt>>,
    },
    Cover {
        cond: Expr,
        pass_stmt: Option<Box<Stmt>>,
    },
    Expect {
        cond: Expr,
        pass_stmt: Option<Box<Stmt>>,
        fail_stmt: Option<Box<Stmt>>,
    },
    WaitOrder {
        events: Vec<String>,
    },
    /// Unique/priority if
    UniqueIf {
        cond: Expr,
        true_branch: Box<Stmt>,
        false_branch: Option<Box<Stmt>>,
    },
    PriorityIf {
        cond: Expr,
        true_branch: Box<Stmt>,
        false_branch: Option<Box<Stmt>>,
    },
    Fork {
        processes: Vec<Stmt>,
        join_type: JoinType,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum JoinType {
    Join,
    JoinAny,
    JoinNone,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CaseItem {
    pub labels: Vec<Expr>,
    pub stmt: Box<Stmt>,
}
