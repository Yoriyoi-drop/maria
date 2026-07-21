use std::collections::HashMap;

use super::expr::Expr;
use super::stmt::{AlwaysBlock, InitialBlock, Stmt};

// Re-export constant evaluation functions
pub use crate::ast::const_eval::{const_eval_simple, const_eval_with_params, string_to_i64};

#[derive(Debug, Clone, PartialEq)]
pub struct Design {
    pub modules: Vec<Module>,
    pub classes: Vec<ClassDecl>,
    pub packages: Vec<PackageDecl>,
    pub interfaces: Vec<Interface>,
    pub binds: Vec<BindDecl>,
    pub clocking_blocks: Vec<ClockingBlock>,
    pub configs: Vec<ConfigDecl>,
    pub udp_defs: Vec<UdpDef>,
    pub top_module: Option<String>,
    pub unit_imports: Vec<(String, String)>,
    pub unit_decls: Vec<Decl>,
    pub unit_funcs: Vec<FunctionDecl>,
    pub unit_tasks: Vec<TaskDecl>,
    pub unit_typedefs: Vec<TypedefDecl>,
    pub unit_params: Vec<ParamDecl>,
    pub timescale: Option<(String, String)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConfigDecl {
    pub name: String,
    pub design_top: Option<String>,
    pub default_liblist: Option<String>,
    pub rules: Vec<ConfigRule>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConfigRule {
    InstanceLiblist { instance: String, liblist: String },
    CellLiblist { cell: String, liblist: String },
    UseLiblist { liblist: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct BindDecl {
    pub target: String,
    pub instance: ModuleInstance,
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
    pub is_static: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConstraintItem {
    Expr(Expr),
    SolveBefore {
        vars: Vec<String>,
    },
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PortDirection {
    Input,
    Output,
    Inout,
    Ref,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Range {
    pub msb: usize,
    pub lsb: usize,
}

impl Range {
    pub fn width(&self) -> usize {
        if self.msb >= self.lsb {
            self.msb.saturating_sub(self.lsb).saturating_add(1)
        } else {
            self.lsb.saturating_sub(self.msb).saturating_add(1)
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
    pub extra_packed_dims: Vec<(ExprRange, Option<Range>)>,
    pub is_dynamic: bool,
    pub is_queue: bool,
    pub is_associative: bool,
    pub assoc_key_type: Option<DataType>,
    pub is_rand: bool,
    pub is_const: bool,
    pub expr: Option<Expr>,
}

impl DeclVar {
    pub fn resolved_width(&self, param_vals: &HashMap<String, i64>) -> Result<usize, String> {
        let base_width = if let Some(r) = &self.range {
            r.width()
        } else if let Some(er) = &self.expr_range {
            let r = resolve_expr_range(er, param_vals)?;
            r.width()
        } else {
            1
        };
        // Multiply by all extra packed dim widths
        let mut total = base_width;
        for (er, _) in &self.extra_packed_dims {
            let r = resolve_expr_range(er, param_vals)?;
            total *= r.width();
        }
        Ok(total)
    }

    /// Returns all packed dimension widths from outermost to innermost.
    pub fn packed_dim_widths(&self, param_vals: &HashMap<String, i64>) -> Result<Vec<usize>, String> {
        let first_width = if let Some(er) = &self.expr_range {
            let r = resolve_expr_range(er, param_vals)?;
            r.width()
        } else if let Some(r) = &self.range {
            r.width()
        } else {
            1usize
        };
        let mut dims = vec![first_width];
        for (er, _) in &self.extra_packed_dims {
            let r = resolve_expr_range(er, param_vals)?;
            dims.push(r.width());
        }
        Ok(dims)
    }

    /// Returns the width of the innermost element (last packed dim).
    pub fn innermost_width(&self, param_vals: &HashMap<String, i64>) -> Result<usize, String> {
        if let Some((er, _)) = self.extra_packed_dims.last() {
            let r = resolve_expr_range(er, param_vals)?;
            Ok(r.width())
        } else if let Some(r) = &self.range {
            Ok(r.width())
        } else if let Some(er) = &self.expr_range {
            let r = resolve_expr_range(er, param_vals)?;
            Ok(r.width())
        } else {
            Ok(1)
        }
    }

    /// Returns the number of elements at the outermost packed dimension.
    pub fn outer_depth(&self, param_vals: &HashMap<String, i64>) -> Result<usize, String> {
        if let Some(er) = &self.expr_range {
            let r = resolve_expr_range(er, param_vals)?;
            Ok(r.width())
        } else if let Some(r) = &self.range {
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
    Void,
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
            DataType::Void => write!(f, "void"),
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
    pub drive_strength: Option<(String, String)>,
    pub delay: Option<Delay>,
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
    DpiExport(DpiImport),
    Param(ParamDecl),
    Clocking(ClockingBlock),
    Specify(SpecifyBlock),
    VirtualInterface {
        iface_type: String,
        modport: Option<String>,
        vif_name: String,
    },
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
    pub is_static: bool,
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
    Export { package: String, item: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClockingBlock {
    pub name: String,
    pub clock_event: ClockEvent,
    pub default_input_skew: Option<u64>,
    pub default_output_skew: Option<u64>,
    pub items: Vec<ClockingItem>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClockEvent {
    Posedge(String),
    Negedge(String),
    Edge(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClockingItem {
    Input {
        signals: Vec<String>,
        skew: Option<u64>,
    },
    Output {
        signals: Vec<String>,
        skew: Option<u64>,
    },
    InputOutput {
        signals: Vec<String>,
    },
    DefaultInputSkew(u64),
    DefaultOutputSkew(u64),
}

#[derive(Debug, Clone, PartialEq)]
pub struct FunctionPort {
    pub name: String,
    pub range: Option<Range>,
    pub expr_range: Option<ExprRange>,
    pub direction: Option<PortDirection>,
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
pub struct UdpPort {
    pub direction: PortDirection,
    pub name: String,
    pub is_reg: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum UdpSymbol {
    Zero,
    One,
    X,
    DontCare,
    Edge(String),
    NoChange,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UdpTableEntry {
    pub inputs: Vec<UdpSymbol>,
    pub output: UdpSymbol,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UdpDef {
    pub name: String,
    pub ports: Vec<UdpPort>,
    pub table: Vec<UdpTableEntry>,
    pub is_sequential: bool,
    pub initial_output: Option<UdpSymbol>,
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
pub enum SpecifyItem {
    PathDelay {
        src: String,
        dst: String,
        rise: Option<Expr>,
        fall: Option<Expr>,
    },
    SpecParam {
        name: String,
        value: Expr,
    },
    SetupCheck {
        data: Expr,
        ref_event: Expr,
        limit: Expr,
    },
    HoldCheck {
        ref_event: Expr,
        data: Expr,
        limit: Expr,
    },
    SetupHoldCheck {
        ref_event: Expr,
        data: Expr,
        setup_limit: Expr,
        hold_limit: Expr,
    },
    RecoveryCheck {
        data: Expr,
        ref_event: Expr,
        limit: Expr,
    },
    RemovalCheck {
        ref_event: Expr,
        data: Expr,
        limit: Expr,
    },
    RecoveryRemovalCheck {
        ref_event: Expr,
        data: Expr,
        recovery_limit: Expr,
        removal_limit: Expr,
    },
    PeriodCheck {
        ref_event: Expr,
        limit: Expr,
    },
    WidthCheck {
        ref_event: Expr,
        limit: Expr,
        threshold: Option<Expr>,
    },
    NochangeCheck {
        ref_event: Expr,
        data: Expr,
        start_limit: Expr,
        end_limit: Expr,
    },
    SkewCheck {
        ref_event: Expr,
        data: Expr,
        limit: Expr,
    },
    TimeskewCheck {
        ref_event: Expr,
        data: Expr,
        limit: Expr,
        threshold: Option<Expr>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpecifyBlock {
    pub items: Vec<SpecifyItem>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum VarType {
    Reg,
    Logic,
    Int,
    Integer,
}
