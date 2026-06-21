use std::collections::HashMap;

use super::expr::{BinaryOp, Expr, UnaryOp, Value};
use super::stmt::{AlwaysBlock, InitialBlock, Stmt};

#[derive(Debug, Clone, PartialEq)]
pub struct Design {
    pub modules: Vec<Module>,
    pub classes: Vec<ClassDecl>,
    pub packages: Vec<PackageDecl>,
    pub interfaces: Vec<Interface>,
    pub top_module: Option<String>,
    pub unit_imports: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClassDecl {
    pub name: String,
    pub extends: Option<String>,
    pub type_params: Vec<TypeParam>,
    pub members: Vec<ClassMember>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypeParam {
    pub name: String,
    pub default_type: Option<DataType>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClassMember {
    Decl(Decl),
    Function(FunctionDecl),
    Task(TaskDecl),
    Constraint {
        name: String,
        body: Vec<ConstraintItem>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskDecl {
    pub name: String,
    pub ports: Vec<FunctionPort>,
    pub decls: Vec<Decl>,
    pub stmts: Vec<Stmt>,
    pub virtual_flag: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConstraintItem {
    Expr(Expr),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Module {
    pub name: String,
    pub ports: Vec<Port>,
    pub params: Vec<ParamDecl>,
    pub decls: Vec<Decl>,
    pub items: Vec<ModuleItem>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModportItem {
    pub name: String,
    pub direction: PortDirection,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Modport {
    pub name: String,
    pub items: Vec<ModportItem>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Interface {
    pub name: String,
    pub params: Vec<ParamDecl>,
    pub decls: Vec<Decl>,
    pub modports: Vec<Modport>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Port {
    pub name: String,
    pub direction: PortDirection,
    pub range: Option<Range>,
    pub expr_range: Option<ExprRange>,
    pub dtype_name: Option<String>,
}

impl Port {
    pub fn resolved_width(&self, param_vals: &HashMap<String, i64>) -> Result<usize, String> {
        if let Some(r) = &self.range {
            Ok(r.width())
        } else if let Some(er) = &self.expr_range {
            let r = resolve_expr_range(er, param_vals)?;
            Ok(r.width())
        } else {
            Ok(1)
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PortDirection {
    Input,
    Output,
    Inout,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Range {
    pub msb: usize,
    pub lsb: usize,
}

impl Range {
    pub fn width(&self) -> usize {
        if self.msb >= self.lsb {
            self.msb - self.lsb + 1
        } else {
            self.lsb - self.msb + 1
        }
    }
}

/// A range whose bounds are expressions (may reference parameters).
/// Resolved during elaboration once parameter values are known.
#[derive(Debug, Clone, PartialEq)]
pub struct ExprRange {
    pub msb: Expr,
    pub lsb: Expr,
}

pub fn resolve_expr_range(er: &ExprRange, param_vals: &HashMap<String, i64>) -> Result<Range, String> {
    let msb = const_eval_with_params(&er.msb, param_vals)?;
    let lsb = const_eval_with_params(&er.lsb, param_vals)?;
    Ok(Range { msb: msb as usize, lsb: lsb as usize })
}

pub fn const_eval_simple(expr: &Expr) -> Result<i64, String> {
    match expr {
        Expr::Value(Value::Decimal(n)) => Ok(*n),
        Expr::Value(Value::Binary { bits, .. }) => {
            i64::from_str_radix(&bits.replace('x', "0").replace('z', "0"), 2).map_err(|_| "bad binary".to_string())
        }
        Expr::Value(Value::Hex { bits, .. }) => {
            i64::from_str_radix(&bits.replace('x', "0").replace('z', "0"), 16).map_err(|_| "bad hex".to_string())
        }
        Expr::Value(Value::Octal { bits, .. }) => {
            i64::from_str_radix(&bits.replace('x', "0").replace('z', "0"), 8).map_err(|_| "bad octal".to_string())
        }
        Expr::Ident(ref s) if s == "1" => Ok(1),
        Expr::MethodCall { .. } => Err("method calls are not simple constants".to_string()),
        Expr::MemberAccess { .. } => Err("member access is not a simple constant".to_string()),
        _ => Err("not a simple constant".to_string()),
    }
}

pub fn const_eval_with_params(expr: &Expr, param_vals: &HashMap<String, i64>) -> Result<i64, String> {
    match expr {
        Expr::Value(Value::Decimal(n)) => Ok(*n),
        Expr::Value(Value::Binary { bits, .. }) => {
            i64::from_str_radix(&bits.replace('x', "0").replace('z', "0"), 2).map_err(|_| "bad binary".to_string())
        }
        Expr::Value(Value::Hex { bits, .. }) => {
            i64::from_str_radix(&bits.replace('x', "0").replace('z', "0"), 16).map_err(|_| "bad hex".to_string())
        }
        Expr::Value(Value::Octal { bits, .. }) => {
            i64::from_str_radix(&bits.replace('x', "0").replace('z', "0"), 8).map_err(|_| "bad octal".to_string())
        }
        Expr::Ident(name) => {
            if let Some(&val) = param_vals.get(name) {
                Ok(val)
            } else if name == "1" {
                Ok(1)
            } else {
                Err(format!("cannot evaluate parameter '{}'", name))
            }
        }
        Expr::UnaryOp { op: UnaryOp::Minus, expr: inner } => {
            Ok(-const_eval_with_params(inner, param_vals)?)
        }
        Expr::UnaryOp { op: UnaryOp::Plus, expr: inner } => {
            Ok(const_eval_with_params(inner, param_vals)?)
        }
        Expr::UnaryOp { op: UnaryOp::BitNot, expr: inner } => {
            Ok(!const_eval_with_params(inner, param_vals)?)
        }
        Expr::BinaryOp { op: BinaryOp::Add, lhs, rhs } => {
            Ok(const_eval_with_params(lhs, param_vals)? + const_eval_with_params(rhs, param_vals)?)
        }
        Expr::BinaryOp { op: BinaryOp::Sub, lhs, rhs } => {
            Ok(const_eval_with_params(lhs, param_vals)? - const_eval_with_params(rhs, param_vals)?)
        }
        Expr::BinaryOp { op: BinaryOp::Mul, lhs, rhs } => {
            Ok(const_eval_with_params(lhs, param_vals)? * const_eval_with_params(rhs, param_vals)?)
        }
        Expr::BinaryOp { op: BinaryOp::Div, lhs, rhs } => {
            Ok(const_eval_with_params(lhs, param_vals)? / const_eval_with_params(rhs, param_vals)?)
        }
        Expr::BinaryOp { op: BinaryOp::Power, lhs, rhs } => {
            let base = const_eval_with_params(lhs, param_vals)?;
            let exp = const_eval_with_params(rhs, param_vals)? as u32;
            Ok(base.pow(exp))
        }
        Expr::BinaryOp { op: BinaryOp::Mod, lhs, rhs } => {
            Ok(const_eval_with_params(lhs, param_vals)? % const_eval_with_params(rhs, param_vals)?)
        }
        Expr::BinaryOp { op: BinaryOp::Eq, lhs, rhs } => {
            let l = const_eval_with_params(lhs, param_vals)?;
            let r = const_eval_with_params(rhs, param_vals)?;
            Ok(if l == r { 1 } else { 0 })
        }
        Expr::BinaryOp { op: BinaryOp::Neq, lhs, rhs } => {
            let l = const_eval_with_params(lhs, param_vals)?;
            let r = const_eval_with_params(rhs, param_vals)?;
            Ok(if l != r { 1 } else { 0 })
        }
        Expr::BinaryOp { op: BinaryOp::Lt, lhs, rhs } => {
            let l = const_eval_with_params(lhs, param_vals)?;
            let r = const_eval_with_params(rhs, param_vals)?;
            Ok(if l < r { 1 } else { 0 })
        }
        Expr::BinaryOp { op: BinaryOp::Le, lhs, rhs } => {
            let l = const_eval_with_params(lhs, param_vals)?;
            let r = const_eval_with_params(rhs, param_vals)?;
            Ok(if l <= r { 1 } else { 0 })
        }
        Expr::BinaryOp { op: BinaryOp::Gt, lhs, rhs } => {
            let l = const_eval_with_params(lhs, param_vals)?;
            let r = const_eval_with_params(rhs, param_vals)?;
            Ok(if l > r { 1 } else { 0 })
        }
        Expr::BinaryOp { op: BinaryOp::Ge, lhs, rhs } => {
            let l = const_eval_with_params(lhs, param_vals)?;
            let r = const_eval_with_params(rhs, param_vals)?;
            Ok(if l >= r { 1 } else { 0 })
        }
        Expr::BinaryOp { op: BinaryOp::LogicalAnd, lhs, rhs } => {
            let l = const_eval_with_params(lhs, param_vals)?;
            let r = const_eval_with_params(rhs, param_vals)?;
            Ok(if l != 0 && r != 0 { 1 } else { 0 })
        }
        Expr::BinaryOp { op: BinaryOp::LogicalOr, lhs, rhs } => {
            let l = const_eval_with_params(lhs, param_vals)?;
            let r = const_eval_with_params(rhs, param_vals)?;
            Ok(if l != 0 || r != 0 { 1 } else { 0 })
        }
        Expr::BinaryOp { op: BinaryOp::BitAnd, lhs, rhs } => {
            Ok(const_eval_with_params(lhs, param_vals)? & const_eval_with_params(rhs, param_vals)?)
        }
        Expr::BinaryOp { op: BinaryOp::BitOr, lhs, rhs } => {
            Ok(const_eval_with_params(lhs, param_vals)? | const_eval_with_params(rhs, param_vals)?)
        }
        Expr::BinaryOp { op: BinaryOp::BitXor, lhs, rhs } => {
            Ok(const_eval_with_params(lhs, param_vals)? ^ const_eval_with_params(rhs, param_vals)?)
        }
        Expr::BinaryOp { op: BinaryOp::BitXnor, lhs, rhs } => {
            Ok(!(const_eval_with_params(lhs, param_vals)? ^ const_eval_with_params(rhs, param_vals)?))
        }
        Expr::BinaryOp { op: BinaryOp::Shl, lhs, rhs } => {
            Ok(const_eval_with_params(lhs, param_vals)? << const_eval_with_params(rhs, param_vals)?)
        }
        Expr::BinaryOp { op: BinaryOp::Shr, lhs, rhs } => {
            Ok(const_eval_with_params(lhs, param_vals)? >> const_eval_with_params(rhs, param_vals)?)
        }
        Expr::BinaryOp { op: BinaryOp::Sshl, lhs, rhs } => {
            Ok(const_eval_with_params(lhs, param_vals)? << const_eval_with_params(rhs, param_vals)?)
        }
        Expr::BinaryOp { op: BinaryOp::Sshr, lhs, rhs } => {
            let l = const_eval_with_params(lhs, param_vals)?;
            let r = const_eval_with_params(rhs, param_vals)?;
            Ok(l >> r)
        }
        Expr::BinaryOp { op: BinaryOp::CaseEq, lhs, rhs } => {
            let l = const_eval_with_params(lhs, param_vals)?;
            let r = const_eval_with_params(rhs, param_vals)?;
            Ok(if l == r { 1 } else { 0 })
        }
        Expr::BinaryOp { op: BinaryOp::CaseNeq, lhs, rhs } => {
            let l = const_eval_with_params(lhs, param_vals)?;
            let r = const_eval_with_params(rhs, param_vals)?;
            Ok(if l != r { 1 } else { 0 })
        }
        Expr::BinaryOp { op: BinaryOp::EqWild, lhs, rhs } => {
            let l = const_eval_with_params(lhs, param_vals)?;
            let r = const_eval_with_params(rhs, param_vals)?;
            Ok(if l == r { 1 } else { 0 })
        }
        Expr::BinaryOp { op: BinaryOp::NeqWild, lhs, rhs } => {
            let l = const_eval_with_params(lhs, param_vals)?;
            let r = const_eval_with_params(rhs, param_vals)?;
            Ok(if l != r { 1 } else { 0 })
        }
        Expr::UnaryOp { op: UnaryOp::Not, expr: inner } => {
            let v = const_eval_with_params(inner, param_vals)?;
            Ok(if v == 0 { 1 } else { 0 })
        }
        Expr::UnaryOp { op: UnaryOp::ReductionAnd, expr: inner } => {
            let v = const_eval_with_params(inner, param_vals)?;
            Ok(if v != 0 && v != -1 { 0 } else { 1 })
        }
        Expr::UnaryOp { op: UnaryOp::ReductionNand, expr: inner } => {
            let v = const_eval_with_params(inner, param_vals)?;
            Ok(if v != 0 && v != -1 { 1 } else { 0 })
        }
        Expr::UnaryOp { op: UnaryOp::ReductionOr, expr: inner } => {
            let v = const_eval_with_params(inner, param_vals)?;
            Ok(if v == 0 { 0 } else { 1 })
        }
        Expr::UnaryOp { op: UnaryOp::ReductionNor, expr: inner } => {
            let v = const_eval_with_params(inner, param_vals)?;
            Ok(if v == 0 { 1 } else { 0 })
        }
        Expr::UnaryOp { op: UnaryOp::ReductionXor, expr: inner } => {
            let v = const_eval_with_params(inner, param_vals)?;
            Ok((v.count_ones() & 1) as i64)
        }
        Expr::UnaryOp { op: UnaryOp::ReductionXnor, expr: inner } => {
            let v = const_eval_with_params(inner, param_vals)?;
            Ok(1 - (v.count_ones() & 1) as i64)
        }
        Expr::TernaryOp { cond, true_expr, false_expr } => {
            let cond_val = const_eval_with_params(cond, param_vals)?;
            if cond_val != 0 {
                const_eval_with_params(true_expr, param_vals)
            } else {
                const_eval_with_params(false_expr, param_vals)
            }
        }
        Expr::Paren(inner) => const_eval_with_params(inner, param_vals),
        Expr::MethodCall { .. } => Err("method calls not allowed in constant expression".to_string()),
        Expr::MemberAccess { .. } => Err("member access not allowed in constant expression".to_string()),
        _ => Err(format!("non-constant expression in parameter context: {:?}", expr)),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParamDecl {
    pub name: String,
    pub dtype: Option<DataType>,
    pub range: Option<(Expr, Expr)>,
    pub default: Option<Expr>,
    pub is_localparam: bool,
    pub is_type_param: bool,
    pub type_default: Option<DataType>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Decl {
    pub dtype: DataType,
    pub kind: DeclKind,
    pub names: Vec<DeclVar>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DeclKind {
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
    Reg,
    Logic,
    Int,
    Integer,
}

impl DeclKind {
    pub fn is_net(&self) -> bool {
        matches!(self, DeclKind::Wire | DeclKind::Wand | DeclKind::Wor
            | DeclKind::Tri | DeclKind::Tri0 | DeclKind::Tri1
            | DeclKind::TriAnd | DeclKind::TriOr
            | DeclKind::Supply0 | DeclKind::Supply1)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeclVar {
    pub name: String,
    pub range: Option<Range>,
    pub expr_range: Option<ExprRange>,
    pub array_range: Option<Range>,
    pub is_dynamic: bool,
    pub is_queue: bool,
    pub is_rand: bool,
    pub expr: Option<Expr>,
}

impl DeclVar {
    pub fn resolved_width(&self, param_vals: &HashMap<String, i64>) -> Result<usize, String> {
        if let Some(r) = &self.range {
            Ok(r.width())
        } else if let Some(er) = &self.expr_range {
            let r = resolve_expr_range(er, param_vals)?;
            Ok(r.width())
        } else {
            Ok(1)
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct StructMember {
    pub name: String,
    pub dtype: Box<DataType>,
    pub range: Option<Range>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DataType {
    Bit,
    Logic,
    Int,
    Integer,
    Byte,
    Shortint,
    Longint,
    Time,
    Real,
    Realtime,
    String,
    Signed(Box<DataType>),
    UserDefined(String),
    EnumType {
        base: Option<Box<DataType>>,
        members: Vec<(String, Option<Expr>)>,
    },
    StructType {
        members: Vec<StructMember>,
    },
    UnionType {
        members: Vec<StructMember>,
    },
}

impl std::fmt::Display for DataType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DataType::Bit => write!(f, "bit"),
            DataType::Logic => write!(f, "logic"),
            DataType::Int => write!(f, "int"),
            DataType::Integer => write!(f, "integer"),
            DataType::Byte => write!(f, "byte"),
            DataType::Shortint => write!(f, "shortint"),
            DataType::Longint => write!(f, "longint"),
            DataType::Time => write!(f, "time"),
            DataType::Real => write!(f, "real"),
            DataType::Realtime => write!(f, "realtime"),
            DataType::String => write!(f, "string"),
            DataType::Signed(inner) => write!(f, "signed {}", inner),
            DataType::UserDefined(name) => write!(f, "{}", name),
            DataType::EnumType { .. } => write!(f, "enum"),
            DataType::StructType { .. } => write!(f, "struct"),
            DataType::UnionType { .. } => write!(f, "union"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypedefDecl {
    pub name: String,
    pub dtype: DataType,
    pub range: Option<ExprRange>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GateType {
    And, Or, Nand, Nor, Xor, Xnor, Buf, Not,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GatePrimitive {
    pub gate_type: GateType,
    pub instance_name: Option<String>,
    pub ports: Vec<Expr>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CovergroupDecl {
    pub name: String,
    pub clocking_event: Option<Expr>,
    pub coverpoints: Vec<CoverpointDef>,
    pub crosses: Vec<CrossDef>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CoverpointDef {
    pub name: String,
    pub expr: Expr,
    pub bins: Vec<BinDef>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CrossDef {
    pub name: String,
    pub coverpoints: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BinDef {
    pub name: String,
    pub range_list: Vec<Expr>,
    pub bin_type: BinType,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BinType {
    Normal,
    Illegal,
    Ignore,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DpiImport {
    pub name: String,
    pub return_type: Option<Box<DataType>>,
    pub args: Vec<DpiArg>,
    pub is_task: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DpiArg {
    pub direction: PortDirection,
    pub dtype: DataType,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ModuleItem {
    Always(AlwaysBlock),
    Initial(InitialBlock),
    Final(InitialBlock),
    Assign(ContinuousAssign),
    Instance(ModuleInstance),
    Gate(GatePrimitive),
    Decl(Decl),
    Func(FunctionDecl),
    Generate(GenerateBlock),
    Typedef(TypedefDecl),
    Covergroup(CovergroupDecl),
    // Imported items from packages
    Import { package: String, item: String },
    DpiImport(DpiImport),
    Param(ParamDecl),
}

#[derive(Debug, Clone, PartialEq)]
pub struct GenerateBlock {
    pub items: Vec<GenerateItem>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GenerateItem {
    If {
        cond: Expr,
        true_items: Vec<ModuleItem>,
        false_items: Vec<ModuleItem>,
    },
    For {
        var: String,
        init: Option<Stmt>,
        cond: Option<Expr>,
        step: Option<Stmt>,
        body_items: Vec<ModuleItem>,
    },
    Case {
        case_type: GenerateCaseType,
        expr: Expr,
        items: Vec<CaseGenerateItem>,
        default: Option<Vec<ModuleItem>>,
    },
    Items(Vec<ModuleItem>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum GenerateCaseType {
    Normal,
    CaseX,
    CaseZ,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CaseGenerateItem {
    pub labels: Vec<Expr>,
    pub body: Vec<ModuleItem>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FunctionDecl {
    pub name: String,
    pub range: Option<ExprRange>,
    pub return_type: Option<Box<DataType>>,
    pub ports: Vec<FunctionPort>,
    pub decls: Vec<Decl>,
    pub stmts: Vec<Stmt>,
    pub virtual_flag: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PackageDecl {
    pub name: String,
    pub items: Vec<PackageItem>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PackageItem {
    Decl(Decl),
    Function(FunctionDecl),
    Task(TaskDecl),
    Typedef(TypedefDecl),
    Param(ParamDecl),
    Import { package: String, item: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct FunctionPort {
    pub name: String,
    pub range: Option<Range>,
    pub expr_range: Option<ExprRange>,
}

impl FunctionPort {
    pub fn resolved_width(&self, param_vals: &HashMap<String, i64>) -> Result<usize, String> {
        if let Some(r) = &self.range {
            Ok(r.width())
        } else if let Some(er) = &self.expr_range {
            let r = resolve_expr_range(er, param_vals)?;
            Ok(r.width())
        } else {
            Ok(1)
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContinuousAssign {
    pub lhs: Expr,
    pub rhs: Expr,
    pub delay: Option<Delay>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModuleInstance {
    pub module_name: String,
    pub instance_name: String,
    pub range: Option<ExprRange>,
    pub param_assigns: HashMap<String, Expr>,
    pub type_param_assigns: HashMap<String, DataType>,
    pub port_conns: Vec<PortConnection>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PortConnection {
    Positional(Expr),
    Named { port: String, expr: Expr },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Delay {
    pub rise: Option<Expr>,
    pub fall: Option<Expr>,
    pub turnoff: Option<Expr>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum VarType {
    Reg,
    Logic,
    Int,
    Integer,
}
