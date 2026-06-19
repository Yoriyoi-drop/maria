use std::collections::HashMap;

use super::expr::{BinaryOp, Expr, UnaryOp, Value};
use super::stmt::{AlwaysBlock, InitialBlock, Stmt};

#[derive(Debug, Clone, PartialEq)]
pub struct Design {
    pub modules: Vec<Module>,
    pub classes: Vec<ClassDecl>,
    pub packages: Vec<PackageDecl>,
    pub top_module: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClassDecl {
    pub name: String,
    pub extends: Option<String>,
    pub members: Vec<ClassMember>,
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
pub struct Port {
    pub name: String,
    pub direction: PortDirection,
    pub range: Option<Range>,
    pub expr_range: Option<ExprRange>,
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
    pub default: Option<Expr>,
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
    Reg,
    Logic,
    Int,
    Integer,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeclVar {
    pub name: String,
    pub range: Option<Range>,
    pub expr_range: Option<ExprRange>,
    pub array_range: Option<Range>,
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

#[derive(Debug, Clone, PartialEq)]
pub struct TypedefDecl {
    pub name: String,
    pub dtype: DataType,
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
pub enum ModuleItem {
    Always(AlwaysBlock),
    Initial(InitialBlock),
    Assign(ContinuousAssign),
    Instance(ModuleInstance),
    Gate(GatePrimitive),
    Decl(Decl),
    Func(FunctionDecl),
    Generate(GenerateBlock),
    Typedef(TypedefDecl),
    // Imported items from packages
    Import { package: String, item: String },
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
    Items(Vec<ModuleItem>),
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
    pub param_assigns: HashMap<String, Expr>,
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
pub enum NetType {
    Wire,
    Wand,
    Wor,
    Tri,
}

#[derive(Debug, Clone, PartialEq)]
pub enum VarType {
    Reg,
    Logic,
    Int,
    Integer,
}
