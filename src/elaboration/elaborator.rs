use std::collections::{HashMap, HashSet};

use crate::ast::*;
use crate::ast::types::const_eval_simple;
use crate::ast::types::const_eval_with_params;
use crate::ast::types::string_to_i64;
use crate::ir::*;

const BUILTIN_UVM_CLASSES: &[&str] = &[
    "uvm_object", "uvm_component", "uvm_sequence_item", "uvm_sequence",
    "uvm_sequencer", "uvm_driver", "uvm_monitor", "uvm_scoreboard",
    "uvm_analysis_port", "uvm_analysis_imp", "uvm_test", "uvm_config_db", "uvm_report_object", "uvm_factory", "uvm_resource_db",
];

fn is_2state_type(dtype: &DataType) -> bool {
    matches!(dtype, DataType::Bit | DataType::Byte | DataType::Shortint | DataType::Int | DataType::Longint | DataType::Time)
}

fn is_signed_type(dtype: &DataType) -> bool {
    matches!(dtype, DataType::Signed(_))
}

pub struct Elaborator {
    pub design: Design,
    pub modules: HashMap<String, IrModule>,
    pub param_vals: HashMap<String, i64>,
    pub typedef_map: HashMap<String, usize>,
    pub typedef_field_map: HashMap<String, Vec<StructFieldInfo>>,
    pub package_symbols: HashMap<String, HashMap<String, PackageItem>>,
    pub specialized_classes: std::cell::RefCell<Vec<ClassDecl>>,
}

impl Elaborator {
    pub fn new(design: Design) -> Self {
        let mut package_symbols: HashMap<String, HashMap<String, PackageItem>> = HashMap::new();
        // First pass: collect directly declared items
        for pkg in &design.packages {
            let mut items = HashMap::new();
            for item in &pkg.items {
                let name = match item {
                    PackageItem::Param(p) => p.name.clone(),
                    PackageItem::Typedef(t) => t.name.clone(),
                    PackageItem::Function(f) => f.name.clone(),
                    PackageItem::Task(t) => t.name.clone(),
                    PackageItem::Decl(d) => {
                        d.names.first().map(|v| v.name.clone()).unwrap_or_default()
                    }
                    PackageItem::Import { .. } => continue,
                };
                items.insert(name, item.clone());
            }
            package_symbols.insert(pkg.name.clone(), items);
        }
        // Second pass: resolve imports within packages
        let imports: Vec<(String, String, String)> = design.packages.iter()
            .flat_map(|pkg| {
                pkg.items.iter().filter_map(|item| {
                    if let PackageItem::Import { package, item: import_item } = item {
                        Some((pkg.name.clone(), package.clone(), import_item.clone()))
                    } else {
                        None
                    }
                })
            })
            .collect();
        for (pkg_name, source_pkg_name, import_item) in imports {
            let source_items = package_symbols.get(&source_pkg_name).cloned();
            if let Some(source_items) = source_items {
                if let Some(pkg_items) = package_symbols.get_mut(&pkg_name) {
                    let names: Vec<String> = if import_item == "*" {
                        source_items.keys().cloned().collect()
                    } else {
                        vec![import_item]
                    };
                    for name in names {
                        if let Some(source_item) = source_items.get(&name) {
                            if !pkg_items.contains_key(&name) {
                                pkg_items.insert(name, source_item.clone());
                            }
                        }
                    }
                }
            }
        }
        Elaborator {
            design,
            modules: HashMap::new(),
            param_vals: HashMap::new(),
            typedef_map: HashMap::new(),
            typedef_field_map: HashMap::new(),
            package_symbols,
            specialized_classes: std::cell::RefCell::new(Vec::new()),
        }
    }

    pub fn elaborate(&mut self, top_module: Option<&str>) -> Result<IrDesign, String> {
        // Inline function calls in all modules
        for module in &mut self.design.modules {
            let temps = crate::ast::inline::inline_func_calls_in_module(module)?;
            for (name, width) in temps {
                module.decls.push(Decl {
                    dtype: DataType::Logic,
                    kind: DeclKind::Reg,
                    names: vec![DeclVar {
                        name,
                        range: None,
                        expr_range: if width > 1 {
                            Some(ExprRange {
                                msb: Expr::Value(crate::ast::expr::Value::Decimal((width - 1) as i64)),
                                lsb: Expr::Value(crate::ast::expr::Value::Decimal(0)),
                            })
                        } else { None },
                        array_range: None,
                        is_dynamic: false,
                        is_queue: false,
                        is_rand: false,
                        expr: None,
                    }],
                });
            }
        }

        // Expand generates in all modules (with resolved params)
        // Use index-based iteration to avoid borrow conflicts
        for i in 0..self.design.modules.len() {
            let param_vals = resolve_param_values_fn(&self.design.modules[i], &HashMap::new())?;
            if let Some(module) = self.design.modules.get_mut(i) {
                expand_all_generates(module, &param_vals)?;
            }
        }

        // First pass: elaborate all modules
        let module_names: Vec<String> = self.design.modules.iter().map(|m| m.name.clone()).collect();

        let modules_snapshot: Vec<Module> = self.design.modules.clone();
        for module in &modules_snapshot {
            let ir = self.elaborate_module(module, &module_names)?;
            self.modules.insert(module.name.clone(), ir);
        }

        // Elaborate interfaces as signal-only modules
        for iface in &self.design.interfaces {
            let mut signals = Vec::new();
            let mut signal_map: HashMap<String, SignalId> = HashMap::new();
            let mut next_id = 0usize;
            for decl in &iface.decls {
                let decl_is_2state = is_2state_type(&decl.dtype);
                for var in &decl.names {
                    let is_real = decl.dtype == DataType::Real || decl.dtype == DataType::Realtime;
                    if is_real || decl.dtype == DataType::String {
                        let sid = next_id;
                        next_id += 1;
                        signal_map.insert(var.name.clone(), sid);
                    signals.push(SignalInfo {
                        name: var.name.clone(),
                        width: if is_real { 64 } else { 0 },
                        kind: SignalKind::Wire,
                        net_type: NetType::Wire,
                        multi_driver: false,
                        init_val: if is_real { LogicVec::new(64) } else { LogicVec::fill(LogicVal::Z, 0) },
                        array_depth: 1,
                        elem_width: if is_real { 64 } else { 0 },
                        array_dims: vec![],
                        class_name: None,
                        is_string: decl.dtype == DataType::String,
                        is_mailbox: false,
                        is_semaphore: false,
                        is_real,
                        is_2state: false,
                        is_dynamic: false,
                        is_queue: false,
                        is_signed: false,
                        msb: if is_real { 63 } else { 0 },
                        lsb: 0,
                        struct_fields: vec![],
                    });
                    continue;
                    }
                    let width = self.resolve_type_width(&decl.dtype)?;
                    let elem_width = width.max(
                        var.resolved_width(&HashMap::new()).unwrap_or(width)
                    ).max(decl.kind.default_width());
                    let sid = next_id;
                    next_id += 1;
                    signal_map.insert(var.name.clone(), sid);
                    signals.push(SignalInfo {
                        name: var.name.clone(),
                        width: elem_width,
                        kind: SignalKind::Wire,
                        net_type: NetType::Wire,
                        multi_driver: false,
                        init_val: LogicVec::fill(LogicVal::Z, elem_width),
                        array_depth: 1,
                        elem_width,
                        array_dims: vec![],
                        class_name: None,
                        is_string: false,
                        is_mailbox: false,
                        is_semaphore: false,
                        is_real: false,
                        is_2state: decl_is_2state,
                        is_dynamic: false,
                        is_queue: false,
                        is_signed: is_signed_type(&decl.dtype),
                        msb: if elem_width > 0 { elem_width - 1 } else { 0 },
                        lsb: 0,
                        struct_fields: vec![],
                    });
                }
            }
            self.modules.insert(iface.name.clone(), IrModule {
                name: iface.name.clone(),
                signals,
                inputs: vec![],
                outputs: vec![],
                inouts: vec![],
                processes: vec![],
                sub_instances: vec![],
            });
        }

        // Find top module
        let top_name = match top_module {
            Some(name) => name.to_string(),
            None => {
                self.design.modules.first()
                    .map(|m| m.name.clone())
                    .ok_or_else(|| "no modules in design".to_string())?
            }
        };

        let mut top = self.modules.remove(&top_name)
            .ok_or_else(|| format!("top module '{}' not found", top_name))?;

        // Flatten instances: merge child module processes into the top module
        let hier_signal_map = self.flatten_instances(&mut top)?;

        // Merge specialized parameterized classes into design classes before elaboration
        {
            let mut specialized = self.specialized_classes.borrow_mut();
            for spec in specialized.drain(..) {
                if !self.design.classes.iter().any(|c| c.name == spec.name) {
                    self.design.classes.push(spec);
                }
            }
        }

        let mut classes = self.elaborate_classes()?;

        // Inject built-in __uvm_object and __uvm_component classes
        if !classes.contains_key("__uvm_object") {
            for (_, cls) in classes.iter_mut() {
                match cls.extends.as_deref() {
                    Some("uvm_object") => cls.extends = Some("__uvm_object".to_string()),
                    Some("uvm_component") => cls.extends = Some("__uvm_component".to_string()),
                    Some("uvm_sequence_item") => cls.extends = Some("__uvm_sequence_item".to_string()),
                    Some("uvm_sequence") => cls.extends = Some("__uvm_sequence".to_string()),
                    Some("uvm_sequencer") => cls.extends = Some("__uvm_sequencer".to_string()),
                    Some("uvm_driver") => cls.extends = Some("__uvm_driver".to_string()),
                    Some("uvm_monitor") => cls.extends = Some("__uvm_monitor".to_string()),
                    Some("uvm_scoreboard") => cls.extends = Some("__uvm_scoreboard".to_string()),
                    Some("uvm_analysis_port") => cls.extends = Some("__uvm_analysis_port".to_string()),
                    Some("uvm_analysis_imp") => cls.extends = Some("__uvm_analysis_imp".to_string()),
                    Some("uvm_test") => cls.extends = Some("__uvm_test".to_string()),
                    Some("uvm_config_db") => cls.extends = Some("__uvm_config_db".to_string()),
                    Some("uvm_report_object") => cls.extends = Some("__uvm_report_object".to_string()),
                    Some("uvm_factory") => cls.extends = Some("__uvm_factory".to_string()),
                    Some("uvm_resource_db") => cls.extends = Some("__uvm_resource_db".to_string()),
                    _ => {}
                }
            }
            classes.insert("__uvm_object".to_string(), IrClassDef {
                name: "__uvm_object".to_string(), extends: None, type_params: vec![],
                fields: vec![], methods: vec![], constraints: vec![], rand_fields: vec![],
            });
            classes.insert("__uvm_report_object".to_string(), IrClassDef {
                name: "__uvm_report_object".to_string(), extends: Some("__uvm_object".to_string()), type_params: vec![],
                fields: vec![], methods: vec![], constraints: vec![], rand_fields: vec![],
            });
            classes.insert("__uvm_component".to_string(), IrClassDef {
                name: "__uvm_component".to_string(), extends: Some("__uvm_report_object".to_string()), type_params: vec![],
                fields: vec![], methods: vec![], constraints: vec![], rand_fields: vec![],
            });
            classes.insert("__uvm_sequence_item".to_string(), IrClassDef {
                name: "__uvm_sequence_item".to_string(), extends: Some("__uvm_object".to_string()), type_params: vec![],
                fields: vec![], methods: vec![], constraints: vec![], rand_fields: vec![],
            });
            classes.insert("__uvm_sequence".to_string(), IrClassDef {
                name: "__uvm_sequence".to_string(), extends: Some("__uvm_sequence_item".to_string()), type_params: vec![],
                fields: vec![], methods: vec![], constraints: vec![], rand_fields: vec![],
            });
            classes.insert("__uvm_sequencer".to_string(), IrClassDef {
                name: "__uvm_sequencer".to_string(), extends: Some("__uvm_component".to_string()), type_params: vec![],
                fields: vec![], methods: vec![], constraints: vec![], rand_fields: vec![],
            });
            classes.insert("__uvm_driver".to_string(), IrClassDef {
                name: "__uvm_driver".to_string(), extends: Some("__uvm_component".to_string()), type_params: vec![],
                fields: vec![], methods: vec![], constraints: vec![], rand_fields: vec![],
            });
            classes.insert("__uvm_monitor".to_string(), IrClassDef {
                name: "__uvm_monitor".to_string(), extends: Some("__uvm_component".to_string()), type_params: vec![],
                fields: vec![], methods: vec![], constraints: vec![], rand_fields: vec![],
            });
            classes.insert("__uvm_scoreboard".to_string(), IrClassDef {
                name: "__uvm_scoreboard".to_string(), extends: Some("__uvm_component".to_string()), type_params: vec![],
                fields: vec![], methods: vec![], constraints: vec![], rand_fields: vec![],
            });
            classes.insert("__uvm_analysis_port".to_string(), IrClassDef {
                name: "__uvm_analysis_port".to_string(), extends: Some("__uvm_object".to_string()), type_params: vec![],
                fields: vec![], methods: vec![], constraints: vec![], rand_fields: vec![],
            });
            classes.insert("__uvm_analysis_imp".to_string(), IrClassDef {
                name: "__uvm_analysis_imp".to_string(), extends: Some("__uvm_object".to_string()), type_params: vec![],
                fields: vec![], methods: vec![], constraints: vec![], rand_fields: vec![],
            });
            classes.insert("__uvm_test".to_string(), IrClassDef {
                name: "__uvm_test".to_string(), extends: Some("__uvm_component".to_string()), type_params: vec![],
                fields: vec![], methods: vec![], constraints: vec![], rand_fields: vec![],
            });
            classes.insert("__uvm_config_db".to_string(), IrClassDef {
                name: "__uvm_config_db".to_string(), extends: Some("__uvm_object".to_string()), type_params: vec![],
                fields: vec![], methods: vec![], constraints: vec![], rand_fields: vec![],
            });
            classes.insert("__uvm_resource_db".to_string(), IrClassDef {
                name: "__uvm_resource_db".to_string(), extends: Some("__uvm_object".to_string()), type_params: vec![],
                fields: vec![], methods: vec![], constraints: vec![], rand_fields: vec![],
            });
            classes.insert("__uvm_factory".to_string(), IrClassDef {
                name: "__uvm_factory".to_string(), extends: Some("__uvm_object".to_string()), type_params: vec![],
                fields: vec![], methods: vec![], constraints: vec![], rand_fields: vec![],
            });
        }

        self.detect_multi_driver_signals(&mut top)?;

        let top_signal_map: HashMap<String, SignalId> = top.signals.iter().enumerate()
            .map(|(i, s)| (s.name.clone(), i)).collect();
        let covergroups = self.elaborate_covergroups(&top_name, &top_signal_map, &top.signals)?;
        let dpi_imports = self.elaborate_dpi_imports()?;

        Ok(IrDesign {
            top,
            modules: self.modules.clone(),
            classes,
            covergroups,
            dpi_imports,
            hier_signal_map,
        })
    }

    fn resolve_param_values(&self, module: &Module, instance_overrides: &HashMap<String, i64>) -> Result<HashMap<String, i64>, String> {
        resolve_param_values_fn(module, instance_overrides)
    }

    fn store_typedef_fields(&mut self, name: &str, dtype: &DataType) {
        let fields = Self::compute_struct_fields(dtype);
        if !fields.is_empty() {
            self.typedef_field_map.insert(name.to_string(), fields);
        }
    }

    fn resolve_type_width(&self, dtype: &DataType) -> Result<usize, String> {
        match dtype {
            DataType::UserDefined(name) if name == "__mailbox" || name == "__semaphore" => Ok(64),
            DataType::UserDefined(name) if name == "process" => Ok(64),
            DataType::UserDefined(name) if BUILTIN_UVM_CLASSES.contains(&name.as_str()) => Ok(64),
            DataType::UserDefined(name) => {
                if self.design.classes.iter().any(|c| c.name == *name) {
                    return Ok(64);
                }
                if self.design.modules.iter().any(|m|
                    m.items.iter().any(|item| matches!(item, ModuleItem::Covergroup(cg) if cg.name == *name))
                ) {
                    return Ok(64);
                }
                self.typedef_map.get(name)
                    .copied()
                    .ok_or_else(|| format!("unknown type '{}' is not defined in this scope", name))
            }
            DataType::Signed(inner) => self.resolve_type_width(inner),
            _ => Ok(dtype.width()),
        }
    }

    fn compute_struct_fields(dtype: &DataType) -> Vec<StructFieldInfo> {
        match dtype {
            DataType::UnionType { members } => {
                members.iter().map(|m| {
                    let w = m.range.as_ref().map(|r| r.width()).unwrap_or(1);
                    StructFieldInfo { name: m.name.clone(), offset: 0, width: w }
                }).collect()
            }
            DataType::StructType { members } => {
                let mut fields = Vec::new();
                let mut offset = 0usize;
                let members_rev: Vec<_> = members.iter().rev().collect();
                for m in &members_rev {
                    let w = m.range.as_ref().map(|r| r.width()).unwrap_or(1);
                    fields.push(StructFieldInfo { name: m.name.clone(), offset, width: w });
                    offset += w;
                }
                fields.reverse();
                fields
            }
            _ => vec![],
        }
    }
}

fn collect_body_params(module: &Module) -> Vec<ParamDecl> {
    let mut params = Vec::new();
    for item in &module.items {
        match item {
            ModuleItem::Param(p) => params.push(p.clone()),
            ModuleItem::Generate(gen) => {
                for gi in &gen.items {
                    if let GenerateItem::Items(items) = gi {
                        for i in items {
                            if let ModuleItem::Param(p) = i {
                                params.push(p.clone());
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    params
}

fn resolve_param_values_fn(module: &Module, instance_overrides: &HashMap<String, i64>) -> Result<HashMap<String, i64>, String> {
    let mut vals = HashMap::new();
    // First, collect positional overrides indexed by position in module's param list
    let mut positional_overrides: Vec<i64> = Vec::new();
    for (name, val) in instance_overrides {
        if name.starts_with("__param") {
            let idx: usize = name.trim_start_matches("__param").parse().unwrap_or(0);
            if idx >= positional_overrides.len() {
                positional_overrides.resize(idx + 1, 0);
            }
            positional_overrides[idx] = *val;
        }
    }

    // Collect body-level parameter declarations and add them to vals first
    // so they can be referenced by module.params expressions
    // Helper to evaluate a parameter's default value, handling string defaults
    let eval_param_default = |e: &Expr, existing_vals: &HashMap<String, i64>| -> i64 {
        match e {
            Expr::String(s) => string_to_i64(s),
            _ => const_eval_with_params(e, existing_vals).unwrap_or(0),
        }
    };

    // Collect body-level parameter declarations and add them to vals first
    // so they can be referenced by module.params expressions
    for param in collect_body_params(module) {
        if !vals.contains_key(&param.name) {
            match &param.default {
                Some(e) => {
                    let v = eval_param_default(e, &vals);
                    vals.insert(param.name.clone(), v);
                }
                None => {
                    vals.insert(param.name.clone(), 0);
                }
            }
        }
    }

    for (i, param) in module.params.iter().enumerate() {
        if param.is_localparam {
            if let Some(e) = &param.default {
                vals.insert(param.name.clone(), eval_param_default(e, &vals));
            } else {
                vals.insert(param.name.clone(), 0);
            }
            continue;
        }
        let val = if i < positional_overrides.len() {
            positional_overrides[i]
        } else if let Some(override_val) = instance_overrides.get(&param.name) {
            *override_val
        } else {
            match &param.default {
                Some(e) => eval_param_default(e, &vals),
                None => 0,
            }
        };
        vals.insert(param.name.clone(), val);
    }
    Ok(vals)
}

fn detect_sync_reset(body: &[IrStmt]) -> Option<ResetInfo> {
    if let Some(IrStmt::If { cond: IrExpr::Signal(sig_id, _), .. }) = body.first() {
        return Some(ResetInfo {
            signal: *sig_id,
            polarity: true,
            r#async: false,
            value: LogicVec::new(1),
        });
    }
    None
}

impl Elaborator {
    fn resolve_class_field_width(&self, dtype: &DataType, type_params: &[TypeParam]) -> usize {
        if let DataType::UserDefined(name) = dtype {
            if let Some(tp) = type_params.iter().find(|tp| tp.name == *name) {
                if let Some(ref default_dt) = tp.default_type {
                    return default_dt.width();
                }
            }
        }
        dtype.width()
    }

    fn elaborate_module(&mut self, module: &Module, known_modules: &[String]) -> Result<IrModule, String> {
        let param_vals = self.resolve_param_values(module, &HashMap::new())?;
        self.elaborate_module_with_params(module, known_modules, &param_vals)
    }

    fn elaborate_module_with_params(&mut self, module: &Module, known_modules: &[String],
                                    param_vals: &HashMap<String, i64>) -> Result<IrModule, String> {
        self.elaborate_module_with_params_and_type(module, known_modules, param_vals, &HashMap::new())
    }

    fn elaborate_module_with_params_and_type(&mut self, module: &Module, known_modules: &[String],
                                    param_vals: &HashMap<String, i64>,
                                    type_param_overrides: &HashMap<String, usize>) -> Result<IrModule, String> {
        let mut effective_params = param_vals.clone();

        // Process $unit imports
        for (package, import_item) in &self.design.unit_imports {
            if let Some(pkg_items) = self.package_symbols.get(package) {
                let names: Vec<&str> = if import_item == "*" {
                    pkg_items.keys().map(|s| s.as_str()).collect()
                } else {
                    vec![import_item.as_str()]
                };
                for name in names {
                    if let Some(pkg_item) = pkg_items.get(name) {
                        if let PackageItem::Param(p) = pkg_item {
                            if !effective_params.contains_key(&p.name) {
                                if let Some(expr) = &p.default {
                                    if let Ok(val) = const_eval_with_params(expr, &effective_params) {
                                        effective_params.insert(p.name.clone(), val);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Process package imports: add package parameters to effective_params
        for item in &module.items {
            if let ModuleItem::Import { package, item: import_item } = item {
                if let Some(pkg_items) = self.package_symbols.get(package) {
                    let names: Vec<&str> = if import_item == "*" {
                        pkg_items.keys().map(|s| s.as_str()).collect()
                    } else {
                        vec![import_item.as_str()]
                    };
                    for name in names {
                        if let Some(pkg_item) = pkg_items.get(name) {
                            if let PackageItem::Param(p) = pkg_item {
                                if !effective_params.contains_key(&p.name) {
                                    if let Some(expr) = &p.default {
                                        if let Ok(val) = const_eval_with_params(expr, &effective_params) {
                                            effective_params.insert(p.name.clone(), val);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                // Process module-level imports for typedefs
                if let Some(pkg_items) = self.package_symbols.get(package) {
                    let names: Vec<&str> = if import_item == "*" {
                        pkg_items.keys().map(|s| s.as_str()).collect()
                    } else {
                        vec![import_item.as_str()]
                    };
                    let mut struct_imports: Vec<(String, DataType)> = Vec::new();
                    for name in names {
                        if let Some(pkg_item) = pkg_items.get(name) {
                            if let PackageItem::Typedef(td) = pkg_item {
                                if !self.typedef_map.contains_key(&td.name) {
                                    let width = self.resolve_typedef_width(&td.dtype, td.range.as_ref());
                                    self.typedef_map.insert(td.name.clone(), width);
                                }
                                if matches!(&td.dtype, DataType::StructType { .. } | DataType::UnionType { .. }) {
                                    struct_imports.push((td.name.clone(), td.dtype.clone()));
                                }
                            }
                        }
                    }
                    for (name, dtype) in struct_imports {
                        self.store_typedef_fields(&name, &dtype);
                    }
                }
            }
        }
        // Resolve type parameter widths from module's param declarations and overrides
        let mut type_param_widths: HashMap<String, usize> = HashMap::new();
        for param in &module.params {
            if param.is_type_param {
                let width = if let Some(w) = type_param_overrides.get(&param.name) {
                    *w
                } else {
                    match &param.default {
                        Some(_) => 8,
                        None => 1,
                    }
                };
                type_param_widths.insert(param.name.clone(), width);
            }
        }

        // Pre-pass: collect in-module typedefs before declaration processing
        for item in &module.items {
            if let ModuleItem::Typedef(td) = item {
                let width = self.resolve_typedef_width(&td.dtype, td.range.as_ref());
                self.typedef_map.insert(td.name.clone(), width);
                // Store struct/union field info for member access
                if matches!(&td.dtype, DataType::StructType { .. } | DataType::UnionType { .. }) {
                    self.store_typedef_fields(&td.name, &td.dtype);
                }
            }
        }
        // Pre-pass: process $unit imports for typedefs
        for (package, import_item) in &self.design.unit_imports {
            if let Some(pkg_items) = self.package_symbols.get(package) {
                let names: Vec<&str> = if import_item == "*" {
                    pkg_items.keys().map(|s| s.as_str()).collect()
                } else {
                    vec![import_item.as_str()]
                };
                for name in names {
                    if let Some(pkg_item) = pkg_items.get(name) {
                        if let PackageItem::Typedef(td) = pkg_item {
                            let width = self.resolve_typedef_width(&td.dtype, td.range.as_ref());
                            self.typedef_map.insert(td.name.clone(), width);
                        }
                    }
                }
            }
        }
        // Pre-pass: store struct/union fields for $unit import typedefs
        let unit_imports = self.design.unit_imports.clone();
        let typedef_imports: Vec<(String, DataType)> = unit_imports.iter().filter_map(|(package, import_item)| {
            self.package_symbols.get(package).and_then(|pkg_items| {
                let names: Vec<String> = if import_item == "*" {
                    pkg_items.keys().cloned().collect()
                } else {
                    vec![import_item.clone()]
                };
                names.iter().find_map(|name| {
                    if let Some(PackageItem::Typedef(td)) = pkg_items.get(name.as_str()) {
                        if matches!(&td.dtype, DataType::StructType { .. } | DataType::UnionType { .. }) {
                            Some((td.name.clone(), td.dtype.clone()))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
            })
        }).collect();
        for (name, dtype) in &typedef_imports {
            self.store_typedef_fields(name, dtype);
        }
        // Pre-pass: process package imports for typedefs before declaration processing
        // Pre-pass: process package imports for struct/union typedef fields
        let import_typedefs: Vec<(String, DataType)> = module.items.iter().filter_map(|item| {
            if let ModuleItem::Import { package, item: import_item } = item {
                self.package_symbols.get(package).and_then(|pkg_items| {
                    let names: Vec<String> = if import_item == "*" {
                        pkg_items.keys().cloned().collect()
                    } else {
                        vec![import_item.clone()]
                    };
                    names.iter().find_map(|name| {
                        if let Some(PackageItem::Typedef(td)) = pkg_items.get(name.as_str()) {
                            if matches!(&td.dtype, DataType::StructType { .. } | DataType::UnionType { .. }) {
                                Some((td.name.clone(), td.dtype.clone()))
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    })
                })
            } else {
                None
            }
        }).collect();
        for (name, dtype) in &import_typedefs {
            let fields = Self::compute_struct_fields(dtype);
            if !fields.is_empty() {
                self.typedef_field_map.entry(name.clone()).or_insert(fields);
            }
        }

        let mut signals = Vec::new();
        let mut signal_map: HashMap<String, SignalId> = HashMap::new();
        let mut inputs = Vec::new();
        let mut outputs = Vec::new();
        let mut inouts = Vec::new();
        let mut processes = Vec::new();
        let mut sub_instances = Vec::new();
        let mut next_id = 0usize;

        // Helper to get or create signal
        let get_or_create_signal = |name: &str,
                                         width: usize,
                                         kind: SignalKind,
                                         net_type: NetType,
                                         signals: &mut Vec<SignalInfo>,
                                         signal_map: &mut HashMap<String, SignalId>,
                                         id: &mut SignalId,
                                         array_depth: usize,
                                         elem_width: usize,
                                         msb: usize,
                                         lsb: usize,
                                         is_2state: bool,
                                         is_signed: bool|
         -> SignalId {
            if let Some(&sid) = signal_map.get(name) {
                sid
            } else {
                let sid = *id;
                *id += 1;
                signal_map.insert(name.to_string(), sid);
                let init_val = match kind {
                    SignalKind::Wire | SignalKind::Inout => LogicVec::fill(LogicVal::Z, width),
                    _ => LogicVec::new(width),
                };
                signals.push(SignalInfo {
                    name: name.to_string(),
                    width,
                    kind,
                    net_type,
                    multi_driver: false,
                    init_val,
                    array_depth,
                    elem_width,
                    array_dims: vec![],
                    class_name: None,
                    is_string: false,
                    is_mailbox: false,
                    is_semaphore: false,
                    is_real: false,
                    is_2state,
                    is_dynamic: false,
                    is_queue: false,
                    is_signed,
                    msb,
                    lsb,
                    struct_fields: vec![],
                });
                sid
            }
        };

        // Process ports with parameter-aware width resolution
        for port in &module.ports {
            let width = if let Some(tn) = &port.dtype_name {
                if let Some(tw) = type_param_widths.get(tn) {
                    if port.expr_range.is_some() || port.range.is_some() {
                        port.resolved_width(&effective_params)?
                    } else {
                        *tw
                    }
                } else {
                    port.resolved_width(&effective_params)?
                }
            } else {
                port.resolved_width(&effective_params)?
            };
            let kind = match port.direction {
                PortDirection::Input => SignalKind::Input,
                PortDirection::Output => SignalKind::Output,
                PortDirection::Inout => SignalKind::Inout,
            };
            let (p_msb, p_lsb) = if let Some(r) = &port.range {
                (r.msb, r.lsb)
            } else if let Some(er) = &port.expr_range {
                if let Ok(r) = resolve_expr_range(er, &effective_params) {
                    (r.msb, r.lsb)
                } else {
                    (width - 1, 0)
                }
            } else {
                (width - 1, 0)
            };
            let net_type = match port.direction {
                PortDirection::Inout => NetType::Tri,
                _ => NetType::Wire,
            };
            let sid = get_or_create_signal(&port.name, width, kind.clone(), net_type, &mut signals, &mut signal_map, &mut next_id, 1, width, p_msb, p_lsb, false, false);
            match port.direction {
                PortDirection::Input => inputs.push(sid),
                PortDirection::Output => outputs.push(sid),
                PortDirection::Inout => inouts.push(sid),
            }
        }

        // Process declarations with parameter-aware width resolution
        for decl in &module.decls {
            let class_name = match &decl.dtype {
                DataType::UserDefined(cn) if cn == "process" => Some("__process".to_string()),
                DataType::UserDefined(cn) => Some(cn.clone()),
                _ => None,
            };
            let decl_is_2state = is_2state_type(&decl.dtype);
            for var in &decl.names {
                let is_real = decl.dtype == DataType::Real || decl.dtype == DataType::Realtime;
                if is_real || decl.dtype == DataType::String {
                    let sid = next_id;
                    next_id += 1;
                    signal_map.insert(var.name.clone(), sid);
                    signals.push(SignalInfo {
                        name: var.name.clone(),
                        width: if is_real { 64 } else { 0 },
                        kind: SignalKind::Reg,
                        net_type: NetType::Wire,
                        multi_driver: false,
                        init_val: LogicVec::new(if is_real { 64 } else { 0 }),
                        array_depth: 1,
                        elem_width: if is_real { 64 } else { 0 },
                        array_dims: vec![],
                        class_name: None,
                        is_string: decl.dtype == DataType::String,
                        is_mailbox: false,
                        is_semaphore: false,
                        is_real,
                        is_2state: false,
                        is_dynamic: false,
                        is_queue: false,
                        is_signed: false,
                        msb: if is_real { 63 } else { 0 },
                        lsb: 0,
                        struct_fields: vec![],
                    });
                    continue;
                }
                if var.is_dynamic || var.is_queue {
                    let dtype_width = self.resolve_type_width(&decl.dtype)?;
                    let elem_width = dtype_width.max(
                        var.resolved_width(&effective_params)?
                    ).max(decl.kind.default_width());
                    let sid = next_id;
                    next_id += 1;
                    signal_map.insert(var.name.clone(), sid);
                    signals.push(SignalInfo {
                        name: var.name.clone(),
                        width: 0,
                        kind: SignalKind::Reg,
                        net_type: NetType::Wire,
                        multi_driver: false,
                        init_val: LogicVec::new(0),
                        array_depth: 0,
                        elem_width,
                        array_dims: vec![],
                        class_name: None,
                        is_string: false,
                        is_mailbox: false,
                        is_semaphore: false,
                        is_real: false,
                        is_2state: decl_is_2state,
                        is_dynamic: var.is_dynamic,
                        is_queue: var.is_queue,
                        is_signed: is_signed_type(&decl.dtype),
                    msb: 0,
                    lsb: 0,
                    struct_fields: vec![],
                });
            }
                let dtype_width = self.resolve_type_width(&decl.dtype)?;
                let elem_width = dtype_width.max(
                    var.resolved_width(&effective_params)?
                ).max(
                    decl.kind.default_width()
                );
                let (kind, net_type) = match decl.kind {
                    DeclKind::Wire => (SignalKind::Wire, NetType::Wire),
                    DeclKind::Wand => (SignalKind::Wire, NetType::Wand),
                    DeclKind::Wor => (SignalKind::Wire, NetType::Wor),
                    DeclKind::Tri => (SignalKind::Wire, NetType::Tri),
                    DeclKind::Tri0 => (SignalKind::Wire, NetType::Tri0),
                    DeclKind::Tri1 => (SignalKind::Wire, NetType::Tri1),
                    DeclKind::TriAnd => (SignalKind::Wire, NetType::TriAnd),
                    DeclKind::TriOr => (SignalKind::Wire, NetType::TriOr),
                    DeclKind::Supply0 => (SignalKind::Wire, NetType::Supply0),
                    DeclKind::Supply1 => (SignalKind::Wire, NetType::Supply1),
                    DeclKind::Reg | DeclKind::Logic | DeclKind::Int | DeclKind::Integer => (SignalKind::Reg, NetType::Wire),
                };
                let (d_msb, d_lsb) = if let Some(r) = &var.range {
                    (r.msb, r.lsb)
                } else if let Some(er) = &var.expr_range {
                    if let Ok(r) = resolve_expr_range(er, &effective_params) {
                        (r.msb, r.lsb)
                    } else {
                        (elem_width - 1, 0)
                    }
                } else {
                    (elem_width - 1, 0)
                };
                if let Some(ar) = &var.array_range {
                    let depth = if ar.msb >= ar.lsb { ar.msb - ar.lsb + 1 } else { ar.lsb - ar.msb + 1 };
                    let total_width = elem_width * depth;
                    let _sid = get_or_create_signal(&var.name, total_width, kind.clone(), net_type, &mut signals, &mut signal_map, &mut next_id, depth, elem_width, total_width - 1, 0, decl_is_2state, is_signed_type(&decl.dtype));
                    if let Some(sig) = signals.iter_mut().find(|s| s.name == var.name) {
                        sig.is_2state = decl_is_2state;
                        let elem_init = if kind == SignalKind::Wire {
                            LogicVec::fill(LogicVal::Z, elem_width)
                        } else {
                            LogicVec::new(elem_width)
                        };
                        let mut full_init = if kind == SignalKind::Wire {
                            LogicVec::fill(LogicVal::Z, total_width)
                        } else {
                            LogicVec::new(total_width)
                        };
                        for i in 0..depth {
                            for j in 0..elem_width {
                                full_init.bits[i * elem_width + j] = elem_init.bits[j].clone();
                            }
                        }
                        sig.init_val = full_init;
                        if let Some(ref class) = class_name {
                            sig.class_name = Some(class.clone());
                            if class == "__mailbox" { sig.is_mailbox = true; }
                            if class == "__semaphore" { sig.is_semaphore = true; }
                        }
                    }
                } else {
                    let _sid = get_or_create_signal(&var.name, elem_width, kind, net_type, &mut signals, &mut signal_map, &mut next_id, 1, elem_width, d_msb, d_lsb, decl_is_2state, is_signed_type(&decl.dtype));
                    if let Some(class) = &class_name {
                        if let Some(sig) = signals.iter_mut().find(|s| s.name == var.name) {
                            sig.class_name = Some(class.clone());
                            if class == "__mailbox" { sig.is_mailbox = true; }
                            if class == "__semaphore" { sig.is_semaphore = true; }
                        }
                    }
                    if let Some(sig) = signals.iter_mut().find(|s| s.name == var.name) {
                        sig.is_2state = decl_is_2state;
                    }
                }
                // Compute struct/union field offsets for member access
                match &decl.dtype {
                    DataType::StructType { members } | DataType::UnionType { members } => {
                        if let Some(sig) = signals.iter_mut().find(|s| s.name == var.name) {
                            match &decl.dtype {
                                DataType::UnionType { members } => {
                                    for m in members {
                                        let w = m.range.as_ref().map(|r| r.width()).unwrap_or(1);
                                        sig.struct_fields.push(StructFieldInfo {
                                            name: m.name.clone(),
                                            offset: 0,
                                            width: w,
                                        });
                                    }
                                }
                                _ => {
                                    let mut offset = 0usize;
                                    let members_rev: Vec<_> = members.iter().rev().collect();
                                    for m in &members_rev {
                                        let w = m.range.as_ref().map(|r| r.width()).unwrap_or(1);
                                        sig.struct_fields.push(StructFieldInfo {
                                            name: m.name.clone(),
                                            offset,
                                            width: w,
                                        });
                                        offset += w;
                                    }
                                    sig.struct_fields.reverse();
                                }
                            }
                        }
                    }
                    DataType::UserDefined(name) => {
                        if let Some(fields) = self.typedef_field_map.get(name) {
                            if !fields.is_empty() {
                                if let Some(sig) = signals.iter_mut().find(|s| s.name == var.name) {
                                    sig.struct_fields = fields.clone();
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        // Expand generate blocks in module items
        // Collect body-level params (localparam, parameter) into effective_params
        for item in &module.items {
            if let ModuleItem::Param(p) = item {
                if !effective_params.contains_key(&p.name) {
                    if let Some(expr) = &p.default {
                        if let Ok(val) = const_eval_with_params(expr, &effective_params) {
                            effective_params.insert(p.name.clone(), val);
                        }
                    }
                }
            }
        }
        self.param_vals = effective_params.clone();

        let expanded_items: Vec<ModuleItem> = {
            let mut items = Vec::new();
            for item in &module.items {
                match item {
                    ModuleItem::Generate(gen) => {
                        let expanded = expand_generate_block(gen, &effective_params)?;
                        // Collect params from expanded generate items too
                        for ei in &expanded {
                            if let ModuleItem::Param(p) = ei {
                                if !effective_params.contains_key(&p.name) {
                                    if let Some(expr) = &p.default {
                                        if let Ok(val) = const_eval_with_params(expr, &effective_params) {
                                            effective_params.insert(p.name.clone(), val);
                                        }
                                    }
                                }
                            }
                        }
                        items.extend(expanded);
                    }
                    other => items.push(other.clone()),
                }
            }
            items
        };

        // Update param_vals after generate expansion which may add body-level params
        self.param_vals = effective_params.clone();

        // Process module items
        for item in &expanded_items {
            match item {
                ModuleItem::Always(always) => {
                    let process = self.elaborate_always(&always, &signal_map, &signals)?;
                    processes.push(process);
                }
                ModuleItem::Initial(initial) => {
                    let body = self.elaborate_stmt_block(&initial.stmts, &signal_map, &known_modules, &signals)?;
                    processes.push(Process::Initial {
                        name: format!("initial_{}", processes.len()),
                        body,
                    });
                }
                ModuleItem::Final(final_block) => {
                    let body = self.elaborate_stmt_block(&final_block.stmts, &signal_map, &known_modules, &signals)?;
                    processes.push(Process::Final {
                        name: format!("final_{}", processes.len()),
                        body,
                    });
                }
                ModuleItem::Assign(assign) => {
                    // Convert to a combinational process
                    let lhs = self.elaborate_lvalue(&assign.lhs, &signal_map, &signals)?;
                    let rhs = self.elaborate_expr(&assign.rhs, &signal_map, &signals)?;
                    let stmts = vec![IrStmt::BlockingAssign { lhs, rhs, delay: None }];
                    let sensitivity = collect_sensitivity(&assign.rhs, &signal_map);
                    processes.push(Process::Combinational {
                        name: format!("assign_{}", processes.len()),
                        sensitivity,
                        body: stmts,
                    });
                }
                ModuleItem::Typedef(td) => {
                    // Already collected in pre-pass; register for UserDefined resolution
                    let width = self.typedef_map.get(&td.name).copied().unwrap_or_else(|| self.resolve_typedef_width(&td.dtype, td.range.as_ref()));
                    self.typedef_map.insert(td.name.clone(), width);
                    // Store struct/union field info for member access
                    match &td.dtype {
                        DataType::StructType { members } | DataType::UnionType { members } => {
                            let mut fields = Vec::new();
                            match &td.dtype {
                                DataType::UnionType { members } => {
                                    for m in members {
                                        let w = m.range.as_ref().map(|r| r.width()).unwrap_or(1);
                                        fields.push(StructFieldInfo { name: m.name.clone(), offset: 0, width: w });
                                    }
                                }
                                _ => {
                                    let mut offset = 0usize;
                                    let members_rev: Vec<_> = members.iter().rev().collect();
                                    for m in &members_rev {
                                        let w = m.range.as_ref().map(|r| r.width()).unwrap_or(1);
                                        fields.push(StructFieldInfo { name: m.name.clone(), offset, width: w });
                                        offset += w;
                                    }
                                    fields.reverse();
                                }
                            }
                            self.typedef_field_map.insert(td.name.clone(), fields);
                        }
                        _ => {}
                    }
                }
                ModuleItem::Instance(inst) => {
                    let mut port_map = HashMap::new();
                    // Look up target module to get port order for positional connections
                    let target_module: Option<&Module> = self.design.modules.iter()
                        .find(|m| m.name == inst.module_name);
                    for (i, conn) in inst.port_conns.iter().enumerate() {
                        match conn {
                            PortConnection::Positional(expr) => {
                                if let Some(tm) = target_module {
                                    if let Some(port) = tm.ports.get(i) {
                                        let sig_id = self.elaborate_expr_to_signal(expr, &signal_map)?;
                                        port_map.insert(port.name.clone(), sig_id);
                                    }
                                }
                            }
                            PortConnection::Named { port, expr } => {
                                let sig_id = self.elaborate_expr_to_signal(expr, &signal_map)?;
                                port_map.insert(port.clone(), sig_id);
                            }
                        }
                    }
                    // Resolve parameter overrides to integer values
                    let mut param_map = HashMap::new();
                    for (pname, pexpr) in &inst.param_assigns {
                        let val = const_eval_with_params(pexpr, &effective_params).unwrap_or(0);
                        param_map.insert(pname.clone(), val);
                    }
                    let mut type_param_map: HashMap<String, usize> = HashMap::new();
                    for (pname, dt) in &inst.type_param_assigns {
                        type_param_map.insert(pname.clone(), dt.width());
                    }

                    if let Some(range) = &inst.range {
                        let msb = const_eval_with_params(&range.msb, &effective_params)?;
                        let lsb = const_eval_with_params(&range.lsb, &effective_params)?;
                        let (start, end) = if msb >= lsb { (lsb, msb) } else { (msb, lsb) };
                        for idx in start..=end {
                            let inst_name = format!("{}[{}]", inst.instance_name, idx);
                            sub_instances.push(IrInstance {
                                module_name: inst.module_name.clone(),
                                instance_name: inst_name,
                                port_map: port_map.clone(),
                                param_map: param_map.clone(),
                                type_param_map: type_param_map.clone(),
                            });
                        }
                    } else {
                        sub_instances.push(IrInstance {
                            module_name: inst.module_name.clone(),
                            instance_name: inst.instance_name.clone(),
                            port_map,
                            param_map,
                            type_param_map,
                        });
                    }
                }
                ModuleItem::Gate(gate) => {
                    let mut sig_ids = Vec::new();
                    for port in &gate.ports {
                        let sid = match port {
                            Expr::Ident(name) => signal_map.get(name).copied()
                                .ok_or_else(|| format!("signal '{}' not found for gate", name))?,
                            _ => return Err(format!("gate port must be a simple signal")),
                        };
                        sig_ids.push(sid);
                    }
                    if sig_ids.len() < 2 {
                        return Err(format!("gate requires at least 2 ports"));
                    }
                    let (out_ids, in_ids) = match gate.gate_type {
                        GateType::And | GateType::Or | GateType::Nand | GateType::Nor | GateType::Xor | GateType::Xnor => {
                            (vec![sig_ids[0]], sig_ids[1..].to_vec())
                        }
                        GateType::Buf | GateType::Not => {
                            let in_id = sig_ids[sig_ids.len() - 1];
                            let outs = sig_ids[..sig_ids.len() - 1].to_vec();
                            (outs, vec![in_id])
                        }
                    };
                    let in_exprs: Vec<IrExpr> = in_ids.iter().map(|id| IrExpr::Signal(*id, 0)).collect();
                    let gate_expr = build_gate_expr(&gate.gate_type, &in_exprs);
                    for &out_id in &out_ids {
                        let process = Process::Combinational {
                            name: format!("gate_{}", out_id),
                            sensitivity: in_ids.clone(),
                            body: vec![IrStmt::BlockingAssign {
                                lhs: IrLValue::Signal(out_id, 0),
                                rhs: gate_expr.clone(),
                                delay: None,
                            }],
                        };
                        processes.push(process);
                    }
                }
                _ => {}
            }
        }

        // Process declaration initializers (wire a = 1; reg b = 0; etc.)
        for decl in &module.decls {
            for var in &decl.names {
                if let Some(init_expr) = &var.expr {
                    let lhs = self.elaborate_lvalue(
                        &Expr::Ident(var.name.clone()),
                        &signal_map, &signals,
                    )?;
                    let rhs = self.elaborate_expr(init_expr, &signal_map, &signals)?;
                    if decl.kind.is_net() {
                        let sensitivity = collect_sensitivity(init_expr, &signal_map);
                        processes.push(Process::Combinational {
                            name: format!("decl_assign_{}", processes.len()),
                            sensitivity,
                            body: vec![IrStmt::BlockingAssign { lhs, rhs, delay: None }],
                        });
                    } else {
                        processes.push(Process::Initial {
                            name: format!("decl_init_{}", processes.len()),
                            body: vec![IrStmt::BlockingAssign { lhs, rhs, delay: None }],
                        });
                    }
                }
            }
        }

        Ok(IrModule {
            name: module.name.clone(),
            signals,
            inputs,
            outputs,
            inouts,
            processes,
            sub_instances,
        })
    }

    fn elaborate_classes(&self) -> Result<HashMap<String, IrClassDef>, String> {
        let mut classes = HashMap::new();
        for cd in &self.design.classes {
            let mut fields = Vec::new();
            for member in &cd.members {
                if let ClassMember::Decl(decl) = member {
                    for dv in &decl.names {
                        let decl_width = self.resolve_class_field_width(&decl.dtype, &cd.type_params);
                        let var_width = dv.resolved_width(&HashMap::new()).unwrap_or(1);
                        let elem_width = decl_width.max(var_width).max(1);
                        let (array_depth, actual_elem_width) = if let Some(ar) = &dv.array_range {
                            let depth = if ar.msb >= ar.lsb { ar.msb - ar.lsb + 1 } else { ar.lsb - ar.msb + 1 };
                            (depth, elem_width)
                        } else {
                            (1, elem_width)
                        };
                        let total_width = array_depth * actual_elem_width;
                        fields.push(IrClassField {
                            name: dv.name.clone(),
                            width: total_width,
                            array_depth,
                            elem_width: actual_elem_width,
                        });
                    }
                }
            }
            let methods = cd.members.iter().filter_map(|m| {
                match m {
                    ClassMember::Function(fd) => Some(IrClassMethod {
                        name: fd.name.clone(),
                        virtual_flag: fd.virtual_flag,
                        ports: fd.ports.clone(),
                        decls: fd.decls.clone(),
                        stmts: fd.stmts.clone(),
                    }),
                    ClassMember::Task(td) => Some(IrClassMethod {
                        name: td.name.clone(),
                        virtual_flag: td.virtual_flag,
                        ports: td.ports.clone(),
                        decls: td.decls.clone(),
                        stmts: td.stmts.clone(),
                    }),
                    _ => None,
                }
            }).collect();
            let constraints: Vec<(String, Vec<crate::ast::types::ConstraintItem>)> = cd.members.iter()
                .filter_map(|m| {
                    if let ClassMember::Constraint { name, body } = m {
                        Some((name.clone(), body.clone()))
                    } else { None }
                }).collect();
            let rand_fields: Vec<String> = cd.members.iter()
                .flat_map(|m| {
                    if let ClassMember::Decl(decl) = m {
                        decl.names.iter().filter(|dv| dv.is_rand).map(|dv| dv.name.clone()).collect::<Vec<_>>()
                    } else { vec![] }
                }).collect();
            // Merge parent class fields (recursively) — parent fields come before child fields
            let all_fields = if let Some(ref parent_name) = cd.extends {
                let parent_key = parent_name.split("::").last().unwrap_or(parent_name).to_string();
                let mut merged = Vec::new();
                let mut seen = std::collections::HashSet::new();
                if let Some(parent_cd) = classes.get(&parent_key) {
                    let mut ancestors: Vec<&IrClassDef> = vec![parent_cd];
                    loop {
                        let current = ancestors.last().unwrap();
                        if let Some(ref gp) = current.extends {
                            let gp_key = gp.split("::").last().unwrap_or(gp);
                            if let Some(gp_cd) = classes.get(gp_key) {
                                ancestors.push(gp_cd);
                            } else { break; }
                        } else { break; }
                    }
                    for anc in ancestors.iter().rev() {
                        for f in &anc.fields {
                            if seen.insert(f.name.clone()) {
                                merged.push(f.clone());
                            }
                        }
                    }
                }
                for f in &fields {
                    if seen.insert(f.name.clone()) {
                        merged.push(f.clone());
                    } else if let Some(pos) = merged.iter().position(|pf| pf.name == f.name) {
                        merged[pos] = f.clone();
                    }
                }
                merged
            } else {
                fields
            };

            classes.insert(cd.name.clone(), IrClassDef {
                name: cd.name.clone(),
                extends: cd.extends.clone(),
                type_params: cd.type_params.iter().map(|tp| IrTypeParam {
                    name: tp.name.clone(),
                    default_type: tp.default_type.clone(),
                }).collect(),
                fields: all_fields,
                methods,
                constraints,
                rand_fields,
            });
        }
        Ok(classes)
    }

    fn elaborate_covergroups(&self, top_name: &str, signal_map: &HashMap<String, SignalId>,
                              signals: &[SignalInfo]) -> Result<Vec<IrCovergroup>, String> {
        let mut covergroups = Vec::new();
        let top_module = if let Some(m) = self.design.modules.iter().find(|m| m.name == top_name) {
            m
        } else {
            return Ok(covergroups);
        };
        for item in &top_module.items {
            if let ModuleItem::Covergroup(cg) = item {
                let mut ir_cps = Vec::new();
                for cp in &cg.coverpoints {
                    let ir_expr = self.elaborate_expr(&cp.expr, signal_map, signals)?;
                    ir_cps.push(IrCoverpoint { name: cp.name.clone(), expr: ir_expr });
                }
                let ir_crosses = cg.crosses.iter().map(|c| {
                    IrCross { name: c.name.clone(), coverpoints: c.coverpoints.clone() }
                }).collect();
                covergroups.push(IrCovergroup { name: cg.name.clone(), coverpoints: ir_cps, crosses: ir_crosses });
            }
        }
        Ok(covergroups)
    }

    fn elaborate_dpi_imports(&self) -> Result<Vec<IrDpiImport>, String> {
        let mut dpi_imports = Vec::new();
        for module in &self.design.modules {
            for item in &module.items {
                if let ModuleItem::DpiImport(dpi) = item {
                    let return_width = dpi.return_type.as_ref()
                        .map(|dt| dt.width()).unwrap_or(1);
                    let arg_widths: Vec<usize> = dpi.args.iter()
                        .map(|a| a.dtype.width()).collect();
                    dpi_imports.push(IrDpiImport {
                        name: dpi.name.clone(),
                        return_width,
                        arg_widths,
                        is_task: dpi.is_task,
                    });
                }
            }
        }
        Ok(dpi_imports)
    }

    fn detect_multi_driver_signals(&self, top: &mut IrModule) -> Result<(), String> {
        let mut driver_count: Vec<usize> = vec![0; top.signals.len()];
        for process in &top.processes {
            match process {
                Process::Combinational { body, .. } | Process::CombReactive { body, .. }
                    | Process::Sequential { body, .. } => {
                    let mut driven = HashSet::new();
                    Self::collect_driven_signals(body, &mut driven);
                    for id in driven {
                        if id < driver_count.len() {
                            driver_count[id] += 1;
                        }
                    }
                }
                _ => {}
            }
        }
        for (id, count) in driver_count.iter().enumerate() {
            if *count > 1 {
                if let Some(sig) = top.signals.get_mut(id) {
                    if sig.kind == SignalKind::Wire || sig.kind == SignalKind::Reg || sig.kind == SignalKind::Inout {
                        sig.multi_driver = true;
                    }
                }
            }
        }
        Ok(())
    }

    fn collect_driven_signals(stmts: &[IrStmt], driven: &mut HashSet<usize>) {
        for stmt in stmts {
            match stmt {
                IrStmt::BlockingAssign { lhs, .. } | IrStmt::NonBlockingAssign { lhs, .. } => {
                    if let IrLValue::Signal(id, _) = lhs {
                        driven.insert(*id);
                    }
                }
                IrStmt::Block { stmts: body } | IrStmt::NamedBlock { stmts: body, .. } => {
                    Self::collect_driven_signals(body, driven);
                }
                IrStmt::If { true_branch, false_branch, .. } => {
                    Self::collect_driven_signals(true_branch, driven);
                    Self::collect_driven_signals(false_branch, driven);
                }
                IrStmt::Case { items, default, .. } => {
                    for item in items {
                        Self::collect_driven_signals(&item.body, driven);
                    }
                    Self::collect_driven_signals(default, driven);
                }
                IrStmt::LoopFor { init, body, .. } => {
                    if let Some(init) = init {
                        Self::collect_driven_signals(&[init.as_ref().clone()], driven);
                    }
                    Self::collect_driven_signals(body, driven);
                }
                IrStmt::LoopWhile { body, .. } | IrStmt::LoopDoWhile { body, .. } | IrStmt::Repeat { body, .. } => {
                    Self::collect_driven_signals(body, driven);
                }
                IrStmt::Delay { body, .. } | IrStmt::Wait { body, .. } => {
                    Self::collect_driven_signals(body, driven);
                }
                _ => {}
            }
        }
    }

    fn flatten_instances(&mut self, top: &mut IrModule) -> Result<HashMap<String, SignalId>, String> {
        let mut hier_signal_map: HashMap<String, SignalId> = HashMap::new();
        let instances = std::mem::take(&mut top.sub_instances);
        for inst in &instances {
            let ast_module_clone: Module = if let Some(m) = self.design.modules.iter()
                .find(|m| m.name == inst.module_name) {
                m.clone()
            } else if let Some(iface) = self.design.interfaces.iter()
                .find(|i| i.name == inst.module_name) {
                Module {
                    name: iface.name.clone(),
                    ports: vec![],
                    params: vec![],
                    decls: iface.decls.clone(),
                    items: vec![],
                }
            } else {
                return Err(format!("module or interface '{}' not found for instance '{}'",
                    inst.module_name, inst.instance_name));
            };

            let needs_custom_params = !ast_module_clone.params.is_empty() && !inst.param_map.is_empty();
            let needs_type_params = !inst.type_param_map.is_empty();
            let mut child = if needs_custom_params || needs_type_params {
                let known_mods: Vec<String> = self.design.modules.iter().map(|m| m.name.clone()).collect();
                let param_vals = self.resolve_param_values(&ast_module_clone, &inst.param_map)?;
                self.elaborate_module_with_params_and_type(&ast_module_clone, &known_mods, &param_vals, &inst.type_param_map)?
            } else {
                // Use pre-elaborated module (default params)
                self.modules.get(&inst.module_name)
                    .ok_or_else(|| format!("module '{}' not found", inst.module_name))?
                    .clone()
            };

            // Recursively flatten child's own instances
            let child_hier_map = self.flatten_instances(&mut child)?;
            hier_signal_map.extend(child_hier_map);

            // Build signal remapping: child_signal_id -> parent_signal_id
            let mut sig_remap: Vec<Option<SignalId>> = vec![None; child.signals.len()];
            let mut next_parent_id = top.signals.len();

            // Map port connections
            for (port_name, &parent_sig) in &inst.port_map {
                if let Some(child_sig) = child.signals.iter().position(|s| s.name == *port_name) {
                    let child_width = child.signals[child_sig].width;
                    let parent_width = top.signals[parent_sig].elem_width;
                    if child_width != parent_width {
                        return Err(format!(
                            "port width mismatch on instance '{}': port '{}' expects width {}, connected signal '{}' has width {}",
                            inst.instance_name, port_name, child_width,
                            top.signals[parent_sig].name, parent_width
                        ));
                    }
                    // Port type checking: inout must connect to tri
                    if child.signals[child_sig].kind == SignalKind::Inout
                        && top.signals[parent_sig].net_type != NetType::Tri
                    {
                        return Err(format!(
                            "port type mismatch on instance '{}': inout port '{}' must connect to a tri signal, but '{}' has net type {:?}",
                            inst.instance_name, port_name,
                            top.signals[parent_sig].name,
                            top.signals[parent_sig].net_type
                        ));
                    }
                    sig_remap[child_sig] = Some(parent_sig);
                    // Add hierarchical alias: inst_name.port_name -> parent signal ID
                    hier_signal_map.insert(
                        format!("{}.{}", inst.instance_name, port_name),
                        parent_sig,
                    );
                }
            }

            // Allocate parent signal IDs for unmapped child signals (internal signals)
            for (i, sig) in child.signals.iter().enumerate() {
                if sig_remap[i].is_none() {
                    let new_id = next_parent_id;
                    next_parent_id += 1;
                    sig_remap[i] = Some(new_id);
                    top.signals.push(SignalInfo {
                        name: format!("{}.{}", inst.instance_name, sig.name),
                        width: sig.width,
                        kind: sig.kind.clone(),
                        net_type: sig.net_type,
                        multi_driver: sig.multi_driver,
                        init_val: sig.init_val.clone(),
                        array_depth: sig.array_depth,
                        elem_width: sig.elem_width,
                        array_dims: sig.array_dims.clone(),
                        class_name: sig.class_name.clone(),
                        is_string: sig.is_string,
                        is_mailbox: sig.is_mailbox,
                        is_semaphore: sig.is_semaphore,
                        is_real: sig.is_real,
                        is_2state: sig.is_2state,
                        is_dynamic: sig.is_dynamic,
                        is_queue: sig.is_queue,
                        is_signed: sig.is_signed,
                    msb: sig.msb,
                        lsb: sig.lsb,
                    struct_fields: sig.struct_fields.clone(),
                });
                    // Also add to hier_signal_map: internal signals already have the right name in flat list
                    hier_signal_map.insert(
                        format!("{}.{}", inst.instance_name, sig.name),
                        new_id,
                    );
                }
            }

            let map_sig = |child_id: SignalId| -> SignalId {
                sig_remap.get(child_id).and_then(|&v| v).unwrap_or(child_id)
            };

            for process in &child.processes {
                let translated = self.translate_process(process, &map_sig)?;
                top.processes.push(translated);
            }
        }
        Ok(hier_signal_map)
    }

    fn translate_process(&self, process: &Process, map_sig: &dyn Fn(SignalId) -> SignalId) -> Result<Process, String> {
        match process {
            Process::Combinational { name, sensitivity, body } => {
                let new_sens = sensitivity.iter().map(|s| map_sig(*s)).collect();
                let new_body = self.translate_stmts(body, map_sig)?;
                Ok(Process::Combinational { name: name.clone(), sensitivity: new_sens, body: new_body })
            }
            Process::CombReactive { name, sensitivity, body } => {
                let new_sens = sensitivity.iter().map(|s| map_sig(*s)).collect();
                let new_body = self.translate_stmts(body, map_sig)?;
                Ok(Process::CombReactive { name: name.clone(), sensitivity: new_sens, body: new_body })
            }
            Process::Sequential { name, clock, reset, body } => {
                let new_clock = match clock {
                    ClockEdge::PosEdge(id) => ClockEdge::PosEdge(map_sig(*id)),
                    ClockEdge::NegEdge(id) => ClockEdge::NegEdge(map_sig(*id)),
                };
                let new_reset = reset.as_ref().map(|r| ResetInfo {
                    signal: map_sig(r.signal),
                    polarity: r.polarity,
                    r#async: r.r#async,
                    value: r.value.clone(),
                });
                let new_body = self.translate_stmts(body, map_sig)?;
                Ok(Process::Sequential { name: name.clone(), clock: new_clock, reset: new_reset, body: new_body })
            }
            Process::Initial { name, body } => {
                let new_body = self.translate_stmts(body, map_sig)?;
                Ok(Process::Initial { name: name.clone(), body: new_body })
            }
            Process::AlwaysWithDelay { name, delay, body } => {
                let new_body = self.translate_stmts(body, map_sig)?;
                Ok(Process::AlwaysWithDelay { name: name.clone(), delay: *delay, body: new_body })
            }
            Process::Final { name, body } => {
                let new_body = self.translate_stmts(body, map_sig)?;
                Ok(Process::Final { name: name.clone(), body: new_body })
            }
        }
    }

    fn translate_stmts(&self, stmts: &[IrStmt], map_sig: &dyn Fn(SignalId) -> SignalId) -> Result<Vec<IrStmt>, String> {
        stmts.iter().map(|s| self.translate_stmt(s, map_sig)).collect()
    }

    fn translate_stmt(&self, stmt: &IrStmt, map_sig: &dyn Fn(SignalId) -> SignalId) -> Result<IrStmt, String> {
        match stmt {
            IrStmt::Block { stmts } => {
                let new = self.translate_stmts(stmts, map_sig)?;
                Ok(IrStmt::Block { stmts: new })
            }
            IrStmt::NamedBlock { name, stmts } => {
                let new = self.translate_stmts(stmts, map_sig)?;
                Ok(IrStmt::NamedBlock { name: name.clone(), stmts: new })
            }
            IrStmt::BlockingAssign { lhs, rhs, delay } => {
                let new_lhs = self.translate_lvalue(lhs, map_sig);
                let new_rhs = self.translate_expr(rhs, map_sig);
                Ok(IrStmt::BlockingAssign { lhs: new_lhs, rhs: new_rhs, delay: *delay })
            }
            IrStmt::NonBlockingAssign { lhs, rhs, delay } => {
                let new_lhs = self.translate_lvalue(lhs, map_sig);
                let new_rhs = self.translate_expr(rhs, map_sig);
                Ok(IrStmt::NonBlockingAssign { lhs: new_lhs, rhs: new_rhs, delay: *delay })
            }
            IrStmt::If { cond, true_branch, false_branch } => {
                let new_cond = self.translate_expr(cond, map_sig);
                let new_true = self.translate_stmts(true_branch, map_sig)?;
                let new_false = self.translate_stmts(false_branch, map_sig)?;
                Ok(IrStmt::If { cond: new_cond, true_branch: new_true, false_branch: new_false })
            }
            IrStmt::Case { case_type, expr, items, default } => {
                let new_expr = self.translate_expr(expr, map_sig);
                let new_items = items.iter().map(|item| {
                    let labels = item.labels.iter().map(|l| self.translate_expr(l, map_sig)).collect();
                    let body = self.translate_stmts(&item.body, map_sig)?;
                    Ok(IrCaseItem { labels, body })
                }).collect::<Result<Vec<_>, String>>()?;
                let new_default = self.translate_stmts(default, map_sig)?;
                Ok(IrStmt::Case { case_type: case_type.clone(), expr: new_expr, items: new_items, default: new_default })
            }
            IrStmt::LoopFor { init, cond, step, body } => {
                let new_init = init.as_ref().map(|i| Box::new(self.translate_stmt(i, map_sig).unwrap_or(IrStmt::Null)));
                let new_cond = self.translate_expr(cond, map_sig);
                let new_step = step.as_ref().map(|s| Box::new(self.translate_stmt(s, map_sig).unwrap_or(IrStmt::Null)));
                let new_body = self.translate_stmts(body, map_sig)?;
                Ok(IrStmt::LoopFor { init: new_init, cond: new_cond, step: new_step, body: new_body })
            }
            IrStmt::LoopWhile { cond, body } => {
                let new_cond = self.translate_expr(cond, map_sig);
                let new_body = self.translate_stmts(body, map_sig)?;
                Ok(IrStmt::LoopWhile { cond: new_cond, body: new_body })
            }
            IrStmt::LoopDoWhile { cond, body } => {
                let new_cond = self.translate_expr(cond, map_sig);
                let new_body = self.translate_stmts(body, map_sig)?;
                Ok(IrStmt::LoopDoWhile { cond: new_cond, body: new_body })
            }
            IrStmt::Repeat { count, body } => {
                let new_count = self.translate_expr(count, map_sig);
                let new_body = self.translate_stmts(body, map_sig)?;
                Ok(IrStmt::Repeat { count: new_count, body: new_body })
            }
            IrStmt::Foreach { array_var, index_var, body } => {
                let new_var = self.translate_expr(array_var, map_sig);
                let new_body = self.translate_stmts(body, map_sig)?;
                Ok(IrStmt::Foreach { array_var: new_var, index_var: index_var.clone(), body: new_body })
            }
            IrStmt::Delay { delay, body } => {
                let new_body = self.translate_stmts(body, map_sig)?;
                Ok(IrStmt::Delay { delay: *delay, body: new_body })
            }
            IrStmt::Wait { cond, body } => {
                let new_cond = self.translate_expr(cond, map_sig);
                let new_body = self.translate_stmts(body, map_sig)?;
                Ok(IrStmt::Wait { cond: new_cond, body: new_body })
            }
            IrStmt::SysCall { name, args } => {
                let new_args = args.iter().map(|a| self.translate_expr(a, map_sig)).collect();
                Ok(IrStmt::SysCall { name: name.clone(), args: new_args })
            }
            IrStmt::EventControl { sig_id, edge, body } => {
                let new_body = self.translate_stmts(body, map_sig)?;
                Ok(IrStmt::EventControl { sig_id: map_sig(*sig_id), edge: edge.clone(), body: new_body })
            }
            IrStmt::EventTrigger { sig_id } => {
                Ok(IrStmt::EventTrigger { sig_id: map_sig(*sig_id) })
            }
            IrStmt::SysFinish => Ok(IrStmt::SysFinish),
            IrStmt::Null => Ok(IrStmt::Null),
            IrStmt::MethodCallStmt { obj, method, args } => {
                Ok(IrStmt::MethodCallStmt {
                    obj: self.translate_expr(obj, map_sig),
                    method: method.clone(),
                    args: args.iter().map(|a| self.translate_expr(a, map_sig)).collect(),
                })
            }
            IrStmt::Break => Ok(IrStmt::Break),
            IrStmt::Continue => Ok(IrStmt::Continue),
            IrStmt::Disable { name } => {
                Ok(IrStmt::Disable { name: name.clone() })
            }
            IrStmt::Force { lvalue, rhs } => {
                Ok(IrStmt::Force { lvalue: self.translate_lvalue(lvalue, map_sig), rhs: self.translate_expr(rhs, map_sig) })
            }
            IrStmt::Release { lvalue } => {
                Ok(IrStmt::Release { lvalue: self.translate_lvalue(lvalue, map_sig) })
            }
            IrStmt::Deassign { lvalue } => {
                Ok(IrStmt::Deassign { lvalue: self.translate_lvalue(lvalue, map_sig) })
            }
            IrStmt::Fork { processes, join_type } => {
                let new_proc = processes.iter().map(|p| self.translate_stmts(p, map_sig)).collect::<Result<Vec<_>, String>>()?;
                Ok(IrStmt::Fork { processes: new_proc, join_type: join_type.clone() })
            }
            IrStmt::Assert { cond, pass_stmt, fail_stmt } => {
                let new_cond = self.translate_expr(cond, map_sig);
                let new_pass = self.translate_stmts(pass_stmt, map_sig)?;
                let new_fail = self.translate_stmts(fail_stmt, map_sig)?;
                Ok(IrStmt::Assert { cond: new_cond, pass_stmt: new_pass, fail_stmt: new_fail })
            }
            IrStmt::Assume { cond, pass_stmt, fail_stmt } => {
                let new_cond = self.translate_expr(cond, map_sig);
                let new_pass = self.translate_stmts(pass_stmt, map_sig)?;
                let new_fail = self.translate_stmts(fail_stmt, map_sig)?;
                Ok(IrStmt::Assume { cond: new_cond, pass_stmt: new_pass, fail_stmt: new_fail })
            }
            IrStmt::Cover { cond, pass_stmt } => {
                let new_cond = self.translate_expr(cond, map_sig);
                let new_pass = self.translate_stmts(pass_stmt, map_sig)?;
                Ok(IrStmt::Cover { cond: new_cond, pass_stmt: new_pass })
            }
            IrStmt::WaitOrder { events, failure_stmts } => {
                let new_events = events.iter().map(|id| map_sig(*id)).collect();
                let new_failure = self.translate_stmts(failure_stmts, map_sig)?;
                Ok(IrStmt::WaitOrder { events: new_events, failure_stmts: new_failure })
            }
        }
    }

    fn translate_lvalue(&self, lv: &IrLValue, map_sig: &dyn Fn(SignalId) -> SignalId) -> IrLValue {
        match lv {
            IrLValue::Signal(id, w) => IrLValue::Signal(map_sig(*id), *w),
            IrLValue::RangeSelect(id, msb, lsb) => IrLValue::RangeSelect(map_sig(*id), *msb, *lsb),
            IrLValue::BitSelect(id, idx) => IrLValue::BitSelect(map_sig(*id), *idx),
            IrLValue::ArrayIndex { sig_id, index, elem_width } => IrLValue::ArrayIndex {
                sig_id: map_sig(*sig_id),
                index: Box::new(self.translate_expr(index, map_sig)),
                elem_width: *elem_width,
            },
            IrLValue::ArrayRangeSelect { sig_id, index, elem_width, msb, lsb } => IrLValue::ArrayRangeSelect {
                sig_id: map_sig(*sig_id),
                index: Box::new(self.translate_expr(index, map_sig)),
                elem_width: *elem_width,
                msb: *msb,
                lsb: *lsb,
            },
            IrLValue::ArrayBitSelect { sig_id, index, elem_width, bit } => IrLValue::ArrayBitSelect {
                sig_id: map_sig(*sig_id),
                index: Box::new(self.translate_expr(index, map_sig)),
                elem_width: *elem_width,
                bit: *bit,
            },
            IrLValue::Concat(parts) => IrLValue::Concat(parts.iter().map(|p| self.translate_lvalue(p, map_sig)).collect()),
        }
    }

    fn translate_expr(&self, expr: &IrExpr, map_sig: &dyn Fn(SignalId) -> SignalId) -> IrExpr {
        match expr {
            IrExpr::Const(v) => IrExpr::Const(v.clone()),
            IrExpr::FillLit(val) => IrExpr::FillLit(*val),
            IrExpr::Signal(id, w) => IrExpr::Signal(map_sig(*id), *w),
            IrExpr::RangeSelect(id, msb, lsb) => IrExpr::RangeSelect(map_sig(*id), *msb, *lsb),
            IrExpr::BitSelect(id, idx) => IrExpr::BitSelect(map_sig(*id), *idx),
            IrExpr::ArrayIndex { sig_id, index, elem_width } => IrExpr::ArrayIndex {
                sig_id: map_sig(*sig_id),
                index: Box::new(self.translate_expr(index, map_sig)),
                elem_width: *elem_width,
            },
            IrExpr::Concat(exprs) => IrExpr::Concat(exprs.iter().map(|e| self.translate_expr(e, map_sig)).collect()),
            IrExpr::Replicate(n, inner) => IrExpr::Replicate(*n, Box::new(self.translate_expr(inner, map_sig))),
            IrExpr::UnaryOp(op, inner) => IrExpr::UnaryOp(op.clone(), Box::new(self.translate_expr(inner, map_sig))),
            IrExpr::BinaryOp(op, l, r) => IrExpr::BinaryOp(
                op.clone(),
                Box::new(self.translate_expr(l, map_sig)),
                Box::new(self.translate_expr(r, map_sig)),
            ),
            IrExpr::Cond(c, t, f) => IrExpr::Cond(
                Box::new(self.translate_expr(c, map_sig)),
                Box::new(self.translate_expr(t, map_sig)),
                Box::new(self.translate_expr(f, map_sig)),
            ),
            IrExpr::Signed(inner) => IrExpr::Signed(Box::new(self.translate_expr(inner, map_sig))),
            IrExpr::String(s) => IrExpr::String(s.clone()),
            IrExpr::SysFunc { name, args } => IrExpr::SysFunc {
                name: name.clone(),
                args: args.iter().map(|a| self.translate_expr(a, map_sig)).collect(),
            },
            IrExpr::NewCall { class_name, args } => IrExpr::NewCall {
                class_name: class_name.clone(),
                args: args.iter().map(|a| self.translate_expr(a, map_sig)).collect(),
            },
            IrExpr::This => IrExpr::This,
            IrExpr::MethodCall { obj, method, args } => IrExpr::MethodCall {
                obj: Box::new(self.translate_expr(obj, map_sig)),
                method: method.clone(),
                args: args.iter().map(|a| self.translate_expr(a, map_sig)).collect(),
            },
            IrExpr::MemberAccess { obj, field } => IrExpr::MemberAccess {
                obj: Box::new(self.translate_expr(obj, map_sig)),
                field: field.clone(),
            },
            IrExpr::ExprRangeSelect(inner, msb, lsb) => IrExpr::ExprRangeSelect(
                Box::new(self.translate_expr(inner, map_sig)),
                *msb,
                *lsb,
            ),
            IrExpr::ExprBitSelect(inner, idx) => IrExpr::ExprBitSelect(
                Box::new(self.translate_expr(inner, map_sig)),
                *idx,
            ),
            IrExpr::ExprPartSelect(inner, base_expr, width_expr) => IrExpr::ExprPartSelect(
                Box::new(self.translate_expr(inner, map_sig)),
                Box::new(self.translate_expr(base_expr, map_sig)),
                Box::new(self.translate_expr(width_expr, map_sig)),
            ),
            IrExpr::DpiCall { name, args, return_width } => IrExpr::DpiCall {
                name: name.clone(),
                args: args.iter().map(|a| self.translate_expr(a, map_sig)).collect(),
                return_width: *return_width,
            },
            IrExpr::HierRef(name) => IrExpr::HierRef(name.clone()),
            IrExpr::Inside { expr, list } => IrExpr::Inside {
                expr: Box::new(self.translate_expr(expr, map_sig)),
                list: list.iter().map(|e| self.translate_expr(e, map_sig)).collect(),
            },
            IrExpr::Cast { width, expr } => IrExpr::Cast {
                width: *width,
                expr: Box::new(self.translate_expr(expr, map_sig)),
            },
        }
    }

    fn elaborate_always(&self, always: &AlwaysBlock, signal_map: &HashMap<String, SignalId>,
                         signals: &[SignalInfo])
        -> Result<Process, String>
    {
        let name = format!("always_{}", 0);

        match always.kind {
            AlwaysKind::AlwaysComb | AlwaysKind::AlwaysLatch => {
                let body = self.elaborate_stmt_block(&always.stmts, signal_map, &[], signals)?;
                let sensitivity = infer_comb_sensitivity(&body);
                Ok(Process::CombReactive { name, sensitivity, body })
            }
            AlwaysKind::AlwaysFF => {
                let (clock, reset) = self.extract_clock_reset(&always.sensitivity, signal_map)?;
                let body = self.elaborate_stmt_block(&always.stmts, signal_map, &[], signals)?;
                let reset = reset.or_else(|| detect_sync_reset(&body));
                Ok(Process::Sequential { name, clock, reset, body })
            }
            AlwaysKind::Always => {
                // Check if body starts with a delay (always #N pattern)
                if always.sensitivity.is_none()
                    && always.stmts.len() == 1
                {
                    if let Stmt::Delay { delay, stmt } = &always.stmts[0] {
                        if let Ok(d) = const_eval_params(delay, &self.param_vals) {
                            let body = self.elaborate_stmt_block(&[stmt.as_ref().clone()], signal_map, &[], signals)?;
                            return Ok(Process::AlwaysWithDelay {
                                name,
                                delay: d as u64,
                                body,
                            });
                        }
                    }
                }
                // Check if sensitivity has clock edges -> Sequential process
                if let Some(sl) = &always.sensitivity {
                    if sl.events.iter().any(|e| matches!(e, SensitivityEvent::PosEdge(_) | SensitivityEvent::NegEdge(_))) {
                        match self.extract_clock_reset(&always.sensitivity, signal_map) {
                            Ok((clock, reset)) => {
                                let body = self.elaborate_stmt_block(&always.stmts, signal_map, &[], signals)?;
                                let reset = reset.or_else(|| detect_sync_reset(&body));
                                return Ok(Process::Sequential { name, clock, reset, body });
                            }
                            Err(_) => {} // fall through to combinational
                        }
                    }
                }
                let body = self.elaborate_stmt_block(&always.stmts, signal_map, &[], signals)?;
                let sensitivity = match &always.sensitivity {
                    Some(sl) => {
                        let has_wildcard = sl.events.iter().any(|e| matches!(e, SensitivityEvent::Wildcard));
                        if has_wildcard {
                            infer_comb_sensitivity(&body)
                        } else {
                            sl.events.iter().filter_map(|e| match e {
                                SensitivityEvent::Level(expr) => resolve_expr_signal(expr, signal_map),
                                _ => None,
                            }).collect()
                        }
                    }
                    None => Vec::new(),
                };
                Ok(Process::Combinational { name, sensitivity, body })
            }
        }
    }

    fn extract_clock_reset(&self, sensitivity: &Option<SensitivityList>,
                           signal_map: &HashMap<String, SignalId>)
        -> Result<(ClockEdge, Option<ResetInfo>), String>
    {
        let events = match sensitivity {
            Some(sl) => &sl.events,
            None => return Err("always_ff requires sensitivity list".to_string()),
        };

        let mut clock_edge = None;
        let mut reset = None;

        for event in events {
            match event {
                SensitivityEvent::PosEdge(expr) | SensitivityEvent::NegEdge(expr) => {
                    let sig_id = resolve_expr_signal(expr, signal_map);
                    let is_pos = matches!(event, SensitivityEvent::PosEdge(_));
                    if let Some(sid) = sig_id {
                        if clock_edge.is_none() {
                            clock_edge = Some(if is_pos {
                                ClockEdge::PosEdge(sid)
                            } else {
                                ClockEdge::NegEdge(sid)
                            });
                        } else if reset.is_none() {
                            reset = Some(ResetInfo {
                                signal: sid,
                                polarity: is_pos,
                                r#async: true,
                                value: LogicVec::new(1),
                            });
                        }
                    }
                }
                _ => {}
            }
        }

        clock_edge.ok_or_else(|| "always_ff must have at least one clock edge".to_string())
            .map(|ce| (ce, reset))
    }

    fn elaborate_stmt_block(&self, stmts: &[Stmt],
                            signal_map: &HashMap<String, SignalId>,
                            _known_modules: &[String],
                            signals: &[SignalInfo])
        -> Result<Vec<IrStmt>, String>
    {
        let mut ir_stmts = Vec::new();
        for stmt in stmts {
            ir_stmts.push(self.elaborate_stmt(stmt, signal_map, _known_modules, signals)?);
        }
        Ok(ir_stmts)
    }

    fn elaborate_stmt(&self, stmt: &Stmt,
                      signal_map: &HashMap<String, SignalId>,
                      known_modules: &[String],
                      signals: &[SignalInfo])
        -> Result<IrStmt, String>
    {
        match stmt {
            Stmt::Block { stmts } => {
                let body = self.elaborate_stmt_block(stmts, signal_map, known_modules, signals)?;
                Ok(IrStmt::Block { stmts: body })
            }
            Stmt::BlockingAssign { lhs, rhs, .. } => {
                let ir_lhs = self.elaborate_lvalue(lhs, signal_map, signals)?;
                let mut ir_rhs = self.elaborate_expr(rhs, signal_map, signals)?;
                // Fill in class name for new() calls from LHS signal info
                if let IrExpr::NewCall { ref mut class_name, .. } = ir_rhs {
                    if class_name.is_empty() {
                        if let IrLValue::Signal(sid, _) = ir_lhs {
                            if let Some(sig) = signals.get(sid) {
                                if let Some(cn) = &sig.class_name {
                                    *class_name = cn.clone();
                                }
                            }
                        }
                    }
                }
                Ok(IrStmt::BlockingAssign { lhs: ir_lhs, rhs: ir_rhs, delay: None })
            }
            Stmt::NonBlockingAssign { lhs, rhs, .. } => {
                let ir_lhs = self.elaborate_lvalue(lhs, signal_map, signals)?;
                let mut ir_rhs = self.elaborate_expr(rhs, signal_map, signals)?;
                if let IrExpr::NewCall { ref mut class_name, .. } = ir_rhs {
                    if class_name.is_empty() {
                        if let IrLValue::Signal(sid, _) = ir_lhs {
                            if let Some(sig) = signals.get(sid) {
                                if let Some(cn) = &sig.class_name {
                                    *class_name = cn.clone();
                                }
                            }
                        }
                    }
                }
                Ok(IrStmt::NonBlockingAssign { lhs: ir_lhs, rhs: ir_rhs, delay: None })
            }
            Stmt::IfElse { cond, true_branch, false_branch } => {
                // Constant-fold condition — if known at compile time, eliminate dead branch
                if let Ok(val) = const_eval_with_params(cond, &self.param_vals) {
                    if val != 0 {
                        // Condition is always true — keep only true branch
                        Ok(self.elaborate_stmt(true_branch, signal_map, known_modules, signals)?)
                    } else {
                        // Condition is always false — keep only false branch
                        match false_branch {
                            Some(fb) => self.elaborate_stmt(fb, signal_map, known_modules, signals),
                            None => Ok(IrStmt::Block { stmts: vec![] }),
                        }
                    }
                } else {
                    let ir_cond = self.elaborate_expr(cond, signal_map, signals)?;
                    let true_stmt = vec![self.elaborate_stmt(true_branch, signal_map, known_modules, signals)?];
                    let false_stmt = match false_branch {
                        Some(fb) => vec![self.elaborate_stmt(fb, signal_map, known_modules, signals)?],
                        None => vec![],
                    };
                    Ok(IrStmt::If { cond: ir_cond, true_branch: true_stmt, false_branch: false_stmt })
                }
            }
            Stmt::Case { expr, items, default } => {
                let ir_expr = self.elaborate_expr(expr, signal_map, signals)?;
                let mut ir_items = Vec::new();
                for item in items {
                    let mut labels = Vec::new();
                    for label in &item.labels {
                        labels.push(self.elaborate_expr(label, signal_map, signals)?);
                    }
                    let body = match &*item.stmt {
                        Stmt::Block { stmts } => self.elaborate_stmt_block(stmts, signal_map, known_modules, signals)?,
                        other => self.elaborate_stmt_block(&[other.clone()], signal_map, known_modules, signals)?,
                    };
                    ir_items.push(IrCaseItem { labels, body });
                }
                let ir_default = match default {
                    Some(d) => {
                        vec![self.elaborate_stmt(d, signal_map, known_modules, signals)?]
                    }
                    None => vec![],
                };
                Ok(IrStmt::Case { case_type: CaseType::Normal, expr: ir_expr, items: ir_items, default: ir_default })
            }
            Stmt::StmtAssign { lhs, rhs } => {
                let ir_lhs = self.elaborate_lvalue(lhs, signal_map, signals)?;
                let ir_rhs = self.elaborate_expr(rhs, signal_map, signals)?;
                Ok(IrStmt::BlockingAssign { lhs: ir_lhs, rhs: ir_rhs, delay: None })
            }
            Stmt::Expr { expr } => {
                match expr {
                    Expr::MethodCall { obj, method, args } => {
                        let ir_obj = self.elaborate_expr(obj, signal_map, signals)?;
                        let ir_args: Vec<IrExpr> = args.iter()
                            .map(|a| self.elaborate_expr(a, signal_map, signals))
                            .collect::<Result<_, _>>()?;
                        Ok(IrStmt::MethodCallStmt {
                            obj: ir_obj,
                            method: method.clone(),
                            args: ir_args,
                        })
                    }
                    Expr::FuncCall { name, .. } if name.starts_with('$') => {
                        let ir_expr = self.elaborate_expr(expr, signal_map, signals)?;
                        Ok(IrStmt::SysCall { name: String::new(), args: vec![ir_expr] })
                    }
                    Expr::FuncCall { name, .. } if name.ends_with("::new") => {
                        let ir_expr = self.elaborate_expr(expr, signal_map, signals)?;
                        Ok(IrStmt::SysCall { name: String::new(), args: vec![ir_expr] })
                    }
                    Expr::FuncCall { name, .. } => {
                        // Check if this is a DPI function call used as a statement
                        let is_dpi = self.design.modules.iter().flat_map(|m| m.items.iter())
                            .any(|item| matches!(item, ModuleItem::DpiImport(d) if d.name == *name));
                        if is_dpi {
                            let ir_expr = self.elaborate_expr(expr, signal_map, signals)?;
                            Ok(IrStmt::SysCall { name: "__dpi_stmt".to_string(), args: vec![ir_expr] })
                        } else {
                            // Side-effect-free expression statement — eliminate it
                            Ok(IrStmt::Block { stmts: vec![] })
                        }
                    }
                    _ => {
                        // Side-effect-free expression statement — eliminate it
                        Ok(IrStmt::Block { stmts: vec![] })
                    }
                }
            }
            Stmt::SysCall { name, args } => {
                let ir_args: Vec<IrExpr> = args.iter()
                    .map(|a| self.elaborate_expr(a, signal_map, signals))
                    .collect::<Result<_, _>>()?;
                Ok(IrStmt::SysCall { name: name.clone(), args: ir_args })
            }
            Stmt::SysFinish => Ok(IrStmt::SysFinish),
            Stmt::Null => Ok(IrStmt::Null),
            Stmt::Return(_) => Ok(IrStmt::Null),
            Stmt::EventControl { events, stmt } => {
                if events.is_empty() {
                    return Ok(IrStmt::Null);
                }
                let event = &events[0];
                let body = match stmt {
                    Some(s) => vec![self.elaborate_stmt(s, signal_map, known_modules, signals)?],
                    None => vec![],
                };
                match event {
                    SensitivityEvent::PosEdge(expr) | SensitivityEvent::NegEdge(expr) => {
                        let is_pos = matches!(event, SensitivityEvent::PosEdge(_));
                        if let Some(sig_id) = resolve_expr_signal(expr, signal_map) {
                            let edge = if is_pos {
                                ClockEdge::PosEdge(sig_id)
                            } else {
                                ClockEdge::NegEdge(sig_id)
                            };
                            Ok(IrStmt::EventControl { sig_id, edge: Some(edge), body })
                        } else {
                            Err(format!("cannot resolve signal in @(...)"))
                        }
                    }
                    SensitivityEvent::Level(expr) => {
                        if let Some(sig_id) = resolve_expr_signal(expr, signal_map) {
                            Ok(IrStmt::EventControl { sig_id, edge: None, body })
                        } else {
                            Err(format!("cannot resolve signal in @(...)"))
                        }
                    }
                    SensitivityEvent::Wildcard => {
                        // @(*) in procedural context: wait for any signal change
                        // For now, treat as immediate
                        Ok(IrStmt::Block { stmts: body })
                    }
                }
            }
            Stmt::EventTrigger { name } => {
                if let Some(sig_id) = signal_map.get(name) {
                    Ok(IrStmt::EventTrigger { sig_id: *sig_id })
                } else {
                    Ok(IrStmt::Null)
                }
            }
            Stmt::Force { lhs, rhs } => {
                let ir_lhs = self.elaborate_lvalue(lhs, signal_map, signals)?;
                let ir_rhs = self.elaborate_expr(rhs, signal_map, signals)?;
                Ok(IrStmt::Force { lvalue: ir_lhs, rhs: ir_rhs })
            }
            Stmt::Release { expr } => {
                let ir_lhs = self.elaborate_lvalue(expr, signal_map, signals)?;
                Ok(IrStmt::Release { lvalue: ir_lhs })
            }
            Stmt::Deassign { expr } => {
                let ir_lhs = self.elaborate_lvalue(expr, signal_map, signals)?;
                Ok(IrStmt::Deassign { lvalue: ir_lhs })
            }
            Stmt::CaseX { expr, items, default } => {
                let ir_expr = self.elaborate_expr(expr, signal_map, signals)?;
                let mut ir_items = Vec::new();
                for item in items {
                    let mut labels = Vec::new();
                    for label in &item.labels {
                        labels.push(self.elaborate_expr(label, signal_map, signals)?);
                    }
                    let body = match &*item.stmt {
                        Stmt::Block { stmts } => self.elaborate_stmt_block(stmts, signal_map, known_modules, signals)?,
                        other => self.elaborate_stmt_block(&[other.clone()], signal_map, known_modules, signals)?,
                    };
                    ir_items.push(IrCaseItem { labels, body });
                }
                let ir_default = match default {
                    Some(d) => vec![self.elaborate_stmt(d, signal_map, known_modules, signals)?],
                    None => vec![],
                };
                Ok(IrStmt::Case { case_type: CaseType::CaseX, expr: ir_expr, items: ir_items, default: ir_default })
            }
            Stmt::CaseZ { expr, items, default } => {
                let ir_expr = self.elaborate_expr(expr, signal_map, signals)?;
                let mut ir_items = Vec::new();
                for item in items {
                    let mut labels = Vec::new();
                    for label in &item.labels {
                        labels.push(self.elaborate_expr(label, signal_map, signals)?);
                    }
                    let body = match &*item.stmt {
                        Stmt::Block { stmts } => self.elaborate_stmt_block(stmts, signal_map, known_modules, signals)?,
                        other => self.elaborate_stmt_block(&[other.clone()], signal_map, known_modules, signals)?,
                    };
                    ir_items.push(IrCaseItem { labels, body });
                }
                let ir_default = match default {
                    Some(d) => vec![self.elaborate_stmt(d, signal_map, known_modules, signals)?],
                    None => vec![],
                };
                Ok(IrStmt::Case { case_type: CaseType::CaseZ, expr: ir_expr, items: ir_items, default: ir_default })
            }
            Stmt::NamedBlock { name, stmts, decls: _ } => {
                let body = self.elaborate_stmt_block(stmts, signal_map, known_modules, signals)?;
                Ok(IrStmt::NamedBlock { name: name.clone(), stmts: body })
            }
            Stmt::Delay { delay, stmt } => {
                let d = const_eval_params(delay, &self.param_vals)? as u64;
                let body = vec![self.elaborate_stmt(stmt, signal_map, known_modules, signals)?];
                Ok(IrStmt::Delay { delay: d, body })
            }
            Stmt::Wait { cond, stmt } => {
                let ir_cond = self.elaborate_expr(cond, signal_map, signals)?;
                let body = match stmt {
                    Some(s) => vec![self.elaborate_stmt(s, signal_map, known_modules, signals)?],
                    None => vec![],
                };
                Ok(IrStmt::Wait { cond: ir_cond, body })
            }
            Stmt::LoopFor { init, cond, step, stmts } => {
                // Try to unroll constant-bounded for loops at elaboration time
                if let Ok(Some(unrolled)) = try_unroll_for_loop(
                    init.as_deref(), cond.as_ref(), step.as_deref(), stmts,
                    &|stmts, var_name, iter_val| {
                        let subst_stmts = substitute_loop_var_in_stmts(stmts, var_name, iter_val);
                        self.elaborate_stmt_block(&subst_stmts, signal_map, known_modules, signals)
                    },
                    &self.param_vals,
                ) {
                    return Ok(IrStmt::Block { stmts: unrolled });
                }
                // Fallback: generate runtime LoopFor
                let ir_init = match init {
                    Some(s) => Some(Box::new(self.elaborate_stmt(s, signal_map, known_modules, signals)?)),
                    None => None,
                };
                let ir_cond = if let Some(c) = cond {
                    self.elaborate_expr(c, signal_map, signals)?
                } else {
                    IrExpr::Const(LogicVec::from_u64(1, 1))
                };
                let ir_step = match step {
                    Some(s) => Some(Box::new(self.elaborate_stmt(s, signal_map, known_modules, signals)?)),
                    None => None,
                };
                let ir_body = self.elaborate_stmt_block(stmts, signal_map, known_modules, signals)?;
                Ok(IrStmt::LoopFor { init: ir_init, cond: ir_cond, step: ir_step, body: ir_body })
            }
            Stmt::LoopWhile { cond, stmts } => {
                let ir_cond = self.elaborate_expr(cond, signal_map, signals)?;
                let ir_body = self.elaborate_stmt_block(stmts, signal_map, known_modules, signals)?;
                Ok(IrStmt::LoopWhile { cond: ir_cond, body: ir_body })
            }
            Stmt::DoWhile { cond, stmts } => {
                let ir_cond = self.elaborate_expr(cond, signal_map, signals)?;
                let ir_body = self.elaborate_stmt_block(stmts, signal_map, known_modules, signals)?;
                Ok(IrStmt::LoopDoWhile { cond: ir_cond, body: ir_body })
            }
            Stmt::LoopForever { stmts } => {
                let ir_body = self.elaborate_stmt_block(stmts, signal_map, known_modules, signals)?;
                Ok(IrStmt::LoopWhile {
                    cond: IrExpr::Const(LogicVec::from_u64(1, 1)),
                    body: ir_body,
                })
            }
            Stmt::ForeachLoop { array_var, index_vars, stmts } => {
                let sig_id = signal_map.get(array_var)
                    .ok_or_else(|| format!("array '{}' not found for foreach", array_var))?;
                let sig_info = signals.get(*sig_id)
                    .ok_or_else(|| format!("signal info not found for '{}'", array_var))?;
                if sig_info.is_dynamic || sig_info.is_queue {
                    let ir_body = self.elaborate_stmt_block(stmts, signal_map, known_modules, signals)?;
                    let iv = index_vars.first().cloned().unwrap_or_else(|| "i".to_string());
                    Ok(IrStmt::Foreach {
                        array_var: IrExpr::Signal(*sig_id, sig_info.width),
                        index_var: iv,
                        body: ir_body,
                    })
                } else {
                    let n = sig_info.array_depth;
                    if n == 0 {
                        return Err(format!("'{}' is not an array, cannot use foreach", array_var));
                    }
                    let mut all_stmts = Vec::new();
                    let iv = index_vars.first().cloned().unwrap_or_else(|| "i".to_string());
                    for i in 0..n {
                        let subst_stmts = substitute_loop_var_in_stmts(stmts, &iv, i as i64);
                        all_stmts.extend(self.elaborate_stmt_block(&subst_stmts, signal_map, known_modules, signals)?);
                    }
                    Ok(IrStmt::Block { stmts: all_stmts })
                }
            }
            Stmt::StmtCase { expr, items, default } => {
                let ir_expr = self.elaborate_expr(expr, signal_map, signals)?;
                let mut ir_items = Vec::new();
                for item in items {
                    let mut labels = Vec::new();
                    for label in &item.labels {
                        labels.push(self.elaborate_expr(label, signal_map, signals)?);
                    }
                    let body = match &*item.stmt {
                        Stmt::Block { stmts } => self.elaborate_stmt_block(stmts, signal_map, known_modules, signals)?,
                        other => self.elaborate_stmt_block(&[other.clone()], signal_map, known_modules, signals)?,
                    };
                    ir_items.push(IrCaseItem { labels, body });
                }
                let ir_default = match default {
                    Some(d) => vec![self.elaborate_stmt(d, signal_map, known_modules, signals)?],
                    None => vec![],
                };
                Ok(IrStmt::Case { case_type: CaseType::Normal, expr: ir_expr, items: ir_items, default: ir_default })
            }
            Stmt::Break => Ok(IrStmt::Break),
            Stmt::Continue => Ok(IrStmt::Continue),
            Stmt::Disable { name } => {
                Ok(IrStmt::Disable { name: name.clone() })
            }
            Stmt::Repeat { count, stmts } => {
                if let Ok(n) = const_eval_params(count, &self.param_vals) {
                    let mut all = Vec::new();
                    for _ in 0..n {
                        all.extend(self.elaborate_stmt_block(stmts, signal_map, known_modules, signals)?);
                    }
                    Ok(IrStmt::Block { stmts: all })
                } else {
                    let ir_count = self.elaborate_expr(count, signal_map, signals)?;
                    let ir_body = self.elaborate_stmt_block(stmts, signal_map, known_modules, signals)?;
                    Ok(IrStmt::Repeat { count: ir_count, body: ir_body })
                }
            }
            // New variants: elaborate as comparable constructs
            Stmt::UniqueCase { expr, items, default }
            | Stmt::PriorityCase { expr, items, default } => {
                self.elaborate_stmt(&Stmt::Case { expr: expr.clone(), items: items.clone(), default: default.clone() }, signal_map, known_modules, signals)
            }
            Stmt::CaseInside { expr, items, default } => {
                self.elaborate_stmt(&Stmt::Case { expr: expr.clone(), items: items.clone(), default: default.clone() }, signal_map, known_modules, signals)
            }
            Stmt::UniqueIf { cond, true_branch, false_branch } => {
                self.elaborate_stmt(&Stmt::IfElse { cond: cond.clone(), true_branch: true_branch.clone(), false_branch: false_branch.clone() }, signal_map, known_modules, signals)
            }
            Stmt::PriorityIf { cond, true_branch, false_branch } => {
                self.elaborate_stmt(&Stmt::IfElse { cond: cond.clone(), true_branch: true_branch.clone(), false_branch: false_branch.clone() }, signal_map, known_modules, signals)
            }
            Stmt::Assert { cond, pass_stmt, fail_stmt } => {
                let ir_cond = self.elaborate_expr(cond, signal_map, signals)?;
                let pass = match pass_stmt {
                    Some(s) => vec![self.elaborate_stmt(s, signal_map, known_modules, signals)?],
                    None => vec![],
                };
                let fail = match fail_stmt {
                    Some(s) => vec![self.elaborate_stmt(s, signal_map, known_modules, signals)?],
                    None => vec![],
                };
                Ok(IrStmt::Assert { cond: ir_cond, pass_stmt: pass, fail_stmt: fail })
            }
            Stmt::Assume { cond, pass_stmt, fail_stmt } => {
                let ir_cond = self.elaborate_expr(cond, signal_map, signals)?;
                let pass = match pass_stmt {
                    Some(s) => vec![self.elaborate_stmt(s, signal_map, known_modules, signals)?],
                    None => vec![],
                };
                let fail = match fail_stmt {
                    Some(s) => vec![self.elaborate_stmt(s, signal_map, known_modules, signals)?],
                    None => vec![],
                };
                Ok(IrStmt::Assume { cond: ir_cond, pass_stmt: pass, fail_stmt: fail })
            }
            Stmt::Cover { cond, pass_stmt } => {
                let ir_cond = self.elaborate_expr(cond, signal_map, signals)?;
                let pass = match pass_stmt {
                    Some(s) => vec![self.elaborate_stmt(s, signal_map, known_modules, signals)?],
                    None => vec![],
                };
                Ok(IrStmt::Cover { cond: ir_cond, pass_stmt: pass })
            }
            Stmt::Expect { .. } => {
                Ok(IrStmt::Null)
            }
            Stmt::WaitOrder { events, fail_stmt } => {
                let mut sig_ids = Vec::new();
                for name in events {
                    if let Some(idx) = signal_map.get(name) {
                        sig_ids.push(*idx);
                    } else {
                        return Err(format!("wait_order: signal '{}' not found", name));
                    }
                }
                let failure = match fail_stmt {
                    Some(s) => vec![self.elaborate_stmt(&*s, signal_map, known_modules, signals)?],
                    None => vec![],
                };
                Ok(IrStmt::WaitOrder { events: sig_ids, failure_stmts: failure })
            }
            Stmt::Fork { processes, join_type } => {
                let mut ir_processes = Vec::new();
                for proc_stmt in processes {
                    let ir = self.elaborate_stmt(proc_stmt, signal_map, known_modules, signals)?;
                    ir_processes.push(vec![ir]);
                }
                let ir_join = match join_type {
                    JoinType::Join => IrJoinType::Join,
                    JoinType::JoinAny => IrJoinType::JoinAny,
                    JoinType::JoinNone => IrJoinType::JoinNone,
                };
                Ok(IrStmt::Fork { processes: ir_processes, join_type: ir_join })
            }
        }
    }

    fn elaborate_lvalue(&self, expr: &Expr, signal_map: &HashMap<String, SignalId>, signals: &[SignalInfo]) -> Result<IrLValue, String> {
        match expr {
            Expr::Ident(name) => {
                let sig_id = signal_map.get(name)
                    .ok_or_else(|| format!("signal '{}' not found", name))?;
                Ok(IrLValue::Signal(*sig_id, 0))
            }
            Expr::RangeSelect { expr: inner, msb, lsb } => {
                let inner_lv = self.elaborate_lvalue(inner, signal_map, signals)?;
                let msb_c = const_eval_params(msb, &self.param_vals)? as usize;
                let lsb_c = const_eval_params(lsb, &self.param_vals)? as usize;
                match inner_lv {
                    IrLValue::Signal(sid, _) => {
                        Ok(IrLValue::RangeSelect(sid, msb_c, lsb_c))
                    }
                    IrLValue::RangeSelect(sid, outer_msb, outer_lsb) => {
                        let outer_start = if outer_msb > outer_lsb { outer_lsb } else { outer_msb };
                        let inner_start = outer_start + if msb_c > lsb_c { lsb_c } else { msb_c };
                        let inner_end = outer_start + if msb_c > lsb_c { msb_c } else { lsb_c };
                        Ok(IrLValue::RangeSelect(sid, inner_end, inner_start))
                    }
                    IrLValue::ArrayIndex { sig_id, index, elem_width } => {
                        Ok(IrLValue::ArrayRangeSelect { sig_id, index, elem_width, msb: msb_c, lsb: lsb_c })
                    }
                    _ => Err("nested range select not supported".to_string()),
                }
            }
            Expr::BitSelect { expr: inner, index: bs_index } => {
                let inner_lv = self.elaborate_lvalue(inner, signal_map, signals)?;
                match inner_lv {
                    IrLValue::Signal(sid, _) => {
                        let sig = &signals[sid];
                        if sig.array_depth > 1 || sig.is_dynamic || sig.is_queue {
                            let index_expr = self.elaborate_expr(bs_index, signal_map, signals)?;
                            Ok(IrLValue::ArrayIndex { sig_id: sid, index: Box::new(index_expr), elem_width: sig.elem_width })
                        } else if let Ok(idx) = const_eval_params(bs_index, &self.param_vals) {
                            Ok(IrLValue::BitSelect(sid, idx as usize))
                        } else {
                            // Dynamic index on a flat signal — treat as array index
                            let index_expr = self.elaborate_expr(bs_index, signal_map, signals)?;
                            Ok(IrLValue::ArrayIndex { sig_id: sid, index: Box::new(index_expr), elem_width: sig.elem_width })
                        }
                    }
                    IrLValue::RangeSelect(sid, outer_msb, outer_lsb) => {
                        if let Ok(idx) = const_eval_params(bs_index, &self.param_vals) {
                            let base = if outer_msb > outer_lsb { outer_lsb } else { outer_msb };
                            Ok(IrLValue::BitSelect(sid, base + idx as usize))
                        } else {
                            let index_expr = self.elaborate_expr(bs_index, signal_map, signals)?;
                            Ok(IrLValue::ArrayIndex { sig_id: sid, index: Box::new(index_expr), elem_width: outer_msb.max(outer_lsb) - outer_msb.min(outer_lsb) + 1 })
                        }
                    }
                    IrLValue::ArrayIndex { sig_id, index, elem_width } => {
                        if let Ok(idx) = const_eval_params(bs_index, &self.param_vals) {
                            Ok(IrLValue::ArrayBitSelect { sig_id, index, elem_width, bit: idx as usize })
                        } else {
                            Err("dynamic bit-select on array element not supported".to_string())
                        }
                    }
                    _ => Err("nested bit select not supported".to_string()),
                }
            }
            Expr::PartSelect { expr: inner, base, width } => {
                let inner_lv = self.elaborate_lvalue(inner, signal_map, signals)?;
                let base_r = const_eval_params(base, &self.param_vals);
                let width_r = const_eval_params(width, &self.param_vals);
                let (base_c, width_c) = match (base_r, width_r) {
                    (Ok(b), Ok(w)) => (b as usize, w as usize),
                    _ => return Err("dynamic part-select not supported".to_string()),
                };
                match inner_lv {
                    IrLValue::Signal(sid, _) => {
                        if width_c > 0 {
                            Ok(IrLValue::RangeSelect(sid, base_c + width_c - 1, base_c))
                        } else {
                            Ok(IrLValue::RangeSelect(sid, base_c, base_c))
                        }
                    }
                    IrLValue::RangeSelect(sid, outer_msb, outer_lsb) => {
                        let outer_base = if outer_msb > outer_lsb { outer_lsb } else { outer_msb };
                        let new_base = outer_base + base_c;
                        if width_c > 0 {
                            Ok(IrLValue::RangeSelect(sid, new_base + width_c - 1, new_base))
                        } else {
                            Ok(IrLValue::RangeSelect(sid, new_base, new_base))
                        }
                    }
                    IrLValue::ArrayIndex { sig_id, index, elem_width } => {
                        if width_c > 0 {
                            Ok(IrLValue::ArrayRangeSelect { sig_id, index, elem_width, msb: base_c + width_c - 1, lsb: base_c })
                        } else {
                            Ok(IrLValue::ArrayRangeSelect { sig_id, index, elem_width, msb: base_c, lsb: base_c })
                        }
                    }
                    _ => Err("nested part-select in lvalue not supported".to_string()),
                }
            }
            Expr::Concat(exprs) => {
                let parts: Result<Vec<IrLValue>, String> = exprs.iter()
                    .map(|e| self.elaborate_lvalue(e, signal_map, signals))
                    .collect();
                Ok(IrLValue::Concat(parts?))
            }
            Expr::MethodCall { .. } => Err("method calls cannot be used as lvalues".to_string()),
            Expr::MemberAccess { obj, field } => {
                // Try struct/union field write
                let hier_name = Self::build_hier_name(obj, field);
                if let Some(&sig_id) = signal_map.get(&hier_name) {
                    return Ok(IrLValue::Signal(sig_id, 0));
                }
                match self.elaborate_expr(obj, signal_map, signals) {
                    Ok(IrExpr::Signal(sig_id, _)) => {
                        let sig_info = &signals[sig_id];
                        if !sig_info.struct_fields.is_empty() {
                            if let Some(f) = sig_info.struct_fields.iter().find(|f| f.name == *field) {
                                let lsb = f.offset;
                                let msb = f.offset + f.width - 1;
                                return Ok(IrLValue::RangeSelect(sig_id, lsb, msb));
                            }
                            return Err(format!("field '{}' not found in struct type", field));
                        }
                        Err("member access on non-struct signal cannot be used as lvalue".to_string())
                    }
                    _ => Err("member access cannot be used as lvalues".to_string()),
                }
            }
            _ => Err(format!("invalid lvalue expression: {:?}", expr)),
        }
    }

    fn build_hier_name(obj: &Expr, field: &str) -> String {
        match obj {
            Expr::Ident(prefix) => format!("{}.{}", prefix, field),
            Expr::MemberAccess { obj: inner, field: inner_field } => {
                format!("{}.{}", Self::build_hier_name(inner, inner_field), field)
            }
            _ => String::new(),
        }
    }

    fn elaborate_expr(&self, expr: &Expr, signal_map: &HashMap<String, SignalId>, signals: &[SignalInfo]) -> Result<IrExpr, String> {
        match expr {
            Expr::Ident(name) if name == "this" => Ok(IrExpr::This),
            Expr::Value(v) => {
                let lv = value_to_logicvec(v);
                let is_signed = matches!(v,
                    Value::Binary { is_signed: true, .. }
                    | Value::Hex { is_signed: true, .. }
                    | Value::Octal { is_signed: true, .. });
                if is_signed {
                    Ok(IrExpr::Signed(Box::new(IrExpr::Const(lv))))
                } else {
                    Ok(IrExpr::Const(lv))
                }
            }
            Expr::FillLit(val) => Ok(IrExpr::FillLit(*val)),
            Expr::Ident(name) => {
                if name.starts_with("$") {
                    return Ok(IrExpr::SysFunc { name: name.clone(), args: vec![] });
                }
                // Check if this ident is a parameter (from param_vals or effective_params)
                if let Some(&val) = self.param_vals.get(name) {
                    return Ok(IrExpr::Const(LogicVec::from_u64(val as u64, 64)));
                }
                let sig_id = signal_map.get(name)
                    .ok_or_else(|| format!("signal '{}' not found", name))?;
                Ok(IrExpr::Signal(*sig_id, 0))
            }
            Expr::ScopedIdent { package, item } => {
                if let Some(pkg_items) = self.package_symbols.get(package) {
                    if let Some(pkg_item) = pkg_items.get(item) {
                        match pkg_item {
                            PackageItem::Param(p) => {
                                if let Some(expr) = &p.default {
                                    if let Ok(val) = const_eval_with_params(expr, &self.param_vals) {
                                        return Ok(IrExpr::Const(LogicVec::from_u64(val as u64, 64)));
                                    }
                                }
                                return Err(format!("package param '{}.{}' has no default", package, item));
                            }
                            _ => return Err(format!("'{}' is not a constant in package '{}'", item, package)),
                        }
                    }
                }
                return Err(format!("'{}' not found in package '{}'", item, package));
            }
            Expr::RangeSelect { expr: inner, msb, lsb } => {
                let inner_expr = self.elaborate_expr(inner, signal_map, signals)?;
                if let (Ok(msb_c), Ok(lsb_c)) = (const_eval_params(msb, &self.param_vals), const_eval_params(lsb, &self.param_vals)) {
                    let msb_c = msb_c as usize;
                    let lsb_c = lsb_c as usize;
                    if let IrExpr::Signal(sid, _) = &inner_expr {
                        Ok(IrExpr::RangeSelect(*sid, msb_c, lsb_c))
                    } else {
                        Ok(IrExpr::ExprRangeSelect(Box::new(inner_expr), msb_c, lsb_c))
                    }
                } else {
                    Err("dynamic range select not supported".to_string())
                }
            }
            Expr::BitSelect { expr: inner, index } => {
                let inner_expr = self.elaborate_expr(inner, signal_map, signals)?;
                if let IrExpr::Signal(sid, _) = &inner_expr {
                    let sig = &signals[*sid];
                    if sig.array_depth > 1 || sig.is_dynamic || sig.is_queue {
                        let index_expr = self.elaborate_expr(index, signal_map, signals)?;
                        Ok(IrExpr::ArrayIndex { sig_id: *sid, index: Box::new(index_expr), elem_width: sig.elem_width })
                    } else if let Ok(idx) = const_eval_params(index, &self.param_vals) {
                        Ok(IrExpr::BitSelect(*sid, idx as usize))
                    } else {
                        // Dynamic index on flat signal — treat as array index
                        let index_expr = self.elaborate_expr(index, signal_map, signals)?;
                        Ok(IrExpr::ArrayIndex { sig_id: *sid, index: Box::new(index_expr), elem_width: sig.elem_width })
                    }
                } else if let Ok(idx) = const_eval_params(index, &self.param_vals) {
                    Ok(IrExpr::ExprBitSelect(Box::new(inner_expr), idx as usize))
                } else {
                    Err("dynamic bit-select on non-signal not supported".to_string())
                }
            }
            Expr::Concat(exprs) => {
                if let Some(folded) = try_fold_const(expr, &self.param_vals)? {
                    return Ok(folded);
                }
                let parts: Result<Vec<IrExpr>, String> = exprs.iter()
                    .map(|e| self.elaborate_expr(e, signal_map, signals))
                    .collect();
                Ok(IrExpr::Concat(parts?))
            }
            Expr::Replicate { count, expr: inner } => {
                if let Some(folded) = try_fold_const(expr, &self.param_vals)? {
                    return Ok(folded);
                }
                let c = const_eval_params(count, &self.param_vals)? as usize;
                let inner_expr = self.elaborate_expr(inner, signal_map, signals)?;
                Ok(IrExpr::Replicate(c, Box::new(inner_expr)))
            }
            Expr::UnaryOp { op, expr: inner } => {
                if let Some(folded) = try_fold_const(expr, &self.param_vals)? {
                    return Ok(folded);
                }
                let inner_expr = self.elaborate_expr(inner, signal_map, signals)?;
                let ir_op = map_unary_op(op)?;
                Ok(IrExpr::UnaryOp(ir_op, Box::new(inner_expr)))
            }
            Expr::BinaryOp { op, lhs, rhs } => {
                if let Some(folded) = try_fold_const(expr, &self.param_vals)? {
                    return Ok(folded);
                }
                let lhs_expr = self.elaborate_expr(lhs, signal_map, signals)?;
                let rhs_expr = self.elaborate_expr(rhs, signal_map, signals)?;
                let ir_op = map_binary_op(op)?;
                Ok(IrExpr::BinaryOp(ir_op, Box::new(lhs_expr), Box::new(rhs_expr)))
            }
            Expr::TernaryOp { cond, true_expr, false_expr } => {
                if let Some(folded) = try_fold_const(expr, &self.param_vals)? {
                    return Ok(folded);
                }
                let ir_cond = self.elaborate_expr(cond, signal_map, signals)?;
                let ir_true = self.elaborate_expr(true_expr, signal_map, signals)?;
                let ir_false = self.elaborate_expr(false_expr, signal_map, signals)?;
                Ok(IrExpr::Cond(Box::new(ir_cond), Box::new(ir_true), Box::new(ir_false)))
            }
            Expr::PartSelect { expr: inner, base, width } => {
                let inner_expr = self.elaborate_expr(inner, signal_map, signals)?;
                if let IrExpr::Signal(sid, _) = &inner_expr {
                    if let (Ok(base_c), Ok(width_c)) = (const_eval_params(base, &self.param_vals), const_eval_params(width, &self.param_vals)) {
                        let base = base_c as usize;
                        let width = width_c as usize;
                        if width > 0 {
                            Ok(IrExpr::RangeSelect(*sid, base + width - 1, base))
                        } else {
                            Ok(IrExpr::RangeSelect(*sid, base, base))
                        }
                    } else {
                        let base_expr = self.elaborate_expr(base, signal_map, signals)?;
                        let width_expr = self.elaborate_expr(width, signal_map, signals)?;
                        Ok(IrExpr::ExprPartSelect(Box::new(inner_expr), Box::new(base_expr), Box::new(width_expr)))
                    }
                } else if let (Ok(base_c), Ok(width_c)) = (const_eval_params(base, &self.param_vals), const_eval_params(width, &self.param_vals)) {
                    let base = base_c as usize;
                    let width = width_c as usize;
                    if width > 0 {
                        Ok(IrExpr::ExprRangeSelect(Box::new(inner_expr), base + width - 1, base))
                    } else {
                        Ok(IrExpr::ExprRangeSelect(Box::new(inner_expr), base, base))
                    }
                } else {
                    let base_expr = self.elaborate_expr(base, signal_map, signals)?;
                    let width_expr = self.elaborate_expr(width, signal_map, signals)?;
                    Ok(IrExpr::ExprPartSelect(Box::new(inner_expr), Box::new(base_expr), Box::new(width_expr)))
                }
            }
            Expr::Paren(inner) => self.elaborate_expr(inner, signal_map, signals),
            Expr::FuncCall { name, args } if name.starts_with("$") => {
                match name.as_str() {
                    "$signed" => {
                        if args.len() != 1 {
                            return Err("$signed requires exactly one argument".to_string());
                        }
                        let inner = self.elaborate_expr(&args[0], signal_map, signals)?;
                        Ok(IrExpr::Signed(Box::new(inner)))
                    }
                    "$unsigned" => {
                        if args.len() != 1 {
                            return Err("$unsigned requires exactly one argument".to_string());
                        }
                        self.elaborate_expr(&args[0], signal_map, signals)
                    }
                    "$clog2" => {
                        if let Some(arg) = args.first() {
                            let val = const_eval_params(arg, &self.param_vals)?;
                            if val <= 1 { return Ok(IrExpr::Const(LogicVec::from_u64(0, 32))); }
                            let n = val as u64;
                            let msb = (64 - n.leading_zeros()) as u64;
                            let result = if n.is_power_of_two() { msb - 1 } else { msb };
                            Ok(IrExpr::Const(LogicVec::from_u64(result, 32)))
                        } else {
                            Err("$clog2 requires one argument".to_string())
                        }
                    }
                    "$bits" => {
                        if let Some(arg) = args.first() {
                            let width = resolve_expr_signal(arg, signal_map)
                                .map(|sig_id| {
                                    let info = &signals[sig_id];
                                    info.width * if info.array_depth > 0 { info.array_depth } else { 1 }
                                })
                                .or_else(|| compute_expr_width(arg, signal_map, signals, &self.param_vals).ok())
                                .ok_or_else(|| "$bits argument must resolve to a signal or computable expression".to_string())?;
                            Ok(IrExpr::Const(LogicVec::from_u64(width as u64, 32)))
                        } else {
                            Err("$bits requires one argument".to_string())
                        }
                    }
                    "$high" => {
                        if let Some(arg) = args.first() {
                            let sig_id = resolve_expr_signal(arg, signal_map)
                                .ok_or_else(|| "$high argument must resolve to a signal".to_string())?;
                            let info = &signals[sig_id];
                            let high = info.msb.max(info.lsb);
                            Ok(IrExpr::Const(LogicVec::from_u64(high as u64, 32)))
                        } else {
                            Err("$high requires one argument".to_string())
                        }
                    }
                    "$low" => {
                        if let Some(arg) = args.first() {
                            let sig_id = resolve_expr_signal(arg, signal_map)
                                .ok_or_else(|| "$low argument must resolve to a signal".to_string())?;
                            let info = &signals[sig_id];
                            let low = info.msb.min(info.lsb);
                            Ok(IrExpr::Const(LogicVec::from_u64(low as u64, 32)))
                        } else {
                            Err("$low requires one argument".to_string())
                        }
                    }
                    "$left" => {
                        if let Some(arg) = args.first() {
                            let sig_id = resolve_expr_signal(arg, signal_map)
                                .ok_or_else(|| "$left argument must resolve to a signal".to_string())?;
                            let info = &signals[sig_id];
                            Ok(IrExpr::Const(LogicVec::from_u64(info.msb as u64, 32)))
                        } else {
                            Err("$left requires one argument".to_string())
                        }
                    }
                    "$right" => {
                        if let Some(arg) = args.first() {
                            let sig_id = resolve_expr_signal(arg, signal_map)
                                .ok_or_else(|| "$right argument must resolve to a signal".to_string())?;
                            let info = &signals[sig_id];
                            Ok(IrExpr::Const(LogicVec::from_u64(info.lsb as u64, 32)))
                        } else {
                            Err("$right requires one argument".to_string())
                        }
                    }
                    "$size" => {
                        if let Some(arg) = args.first() {
                            let sig_id = resolve_expr_signal(arg, signal_map)
                                .ok_or_else(|| "$size argument must resolve to a signal".to_string())?;
                            let info = &signals[sig_id];
                            Ok(IrExpr::Const(LogicVec::from_u64(info.width as u64, 32)))
                        } else {
                            Err("$size requires one argument".to_string())
                        }
                    }
                    _ => {
                        let ir_args: Result<Vec<IrExpr>, String> = args.iter()
                            .map(|a| self.elaborate_expr(a, signal_map, signals))
                            .collect();
                        Ok(IrExpr::SysFunc { name: name.to_string(), args: ir_args? })
                    }
                }
            }
            Expr::FuncCall { name, args } if name == "new" => {
                let ir_args: Result<Vec<IrExpr>, String> = args.iter()
                    .map(|a| self.elaborate_expr(a, signal_map, signals))
                    .collect();
                Ok(IrExpr::NewCall { class_name: String::new(), args: ir_args? })
            }
            Expr::String(s) => Ok(IrExpr::String(s.clone())),
            Expr::MethodCall { obj, method, args } => {
                let ir_obj = self.elaborate_expr(obj, signal_map, signals)?;
                let ir_args: Result<Vec<IrExpr>, String> = args.iter()
                    .map(|a| self.elaborate_expr(a, signal_map, signals))
                    .collect();
                Ok(IrExpr::MethodCall {
                    obj: Box::new(ir_obj),
                    method: method.clone(),
                    args: ir_args?,
                })
            }
        Expr::MemberAccess { obj, field } => {
            // Try to resolve as hierarchical signal reference first
            let hier_name = Self::build_hier_name(obj, field);
            if !hier_name.is_empty() {
                if let Some(&sig_id) = signal_map.get(&hier_name) {
                    return Ok(IrExpr::Signal(sig_id, 0));
                }
            }
            // Try struct/union member access: resolve obj signal, check struct_fields
            match self.elaborate_expr(obj, signal_map, signals) {
                Ok(IrExpr::Signal(sig_id, _)) => {
                    let sig_info = &signals[sig_id];
                    if !sig_info.struct_fields.is_empty() {
                        if let Some(f) = sig_info.struct_fields.iter().find(|f| f.name == *field) {
                            let lsb = f.offset;
                            let msb = f.offset + f.width - 1;
                            return Ok(IrExpr::RangeSelect(sig_id, lsb, msb));
                        }
                        return Err(format!("field '{}' not found in struct type (width {})", field, sig_info.width));
                    }
                    Ok(IrExpr::MemberAccess {
                        obj: Box::new(IrExpr::Signal(sig_id, 0)),
                        field: field.clone(),
                    })
                }
                Ok(ir_obj) => Ok(IrExpr::MemberAccess {
                    obj: Box::new(ir_obj),
                    field: field.clone(),
                }),
                Err(_) => {
                    // If obj can't be elaborated (e.g., instance name), emit a HierRef
                    // that the engine can resolve at runtime using the flattened signal list
                    Ok(IrExpr::HierRef(hier_name))
                }
            }
        }
            Expr::Null => Ok(IrExpr::Const(LogicVec::from_u64(0, 64))),
            Expr::Inside { expr: inner, range_list } => {
                let inner_ir = self.elaborate_expr(inner, signal_map, signals)?;
                let mut list_ir = Vec::with_capacity(range_list.len());
                for item in range_list {
                    list_ir.push(self.elaborate_expr(&item, signal_map, signals)?);
                }
                Ok(IrExpr::Inside { expr: Box::new(inner_ir), list: list_ir })
            }
            Expr::StreamingConcat { slices, .. } => {
                if let Some(first) = slices.first() {
                    self.elaborate_expr(first, signal_map, signals)
                } else {
                    Ok(IrExpr::Const(LogicVec::new(0)))
                }
            }
            Expr::Cast { dtype, expr: inner } => {
                let inner_ir = self.elaborate_expr(inner, signal_map, signals)?;
                let cast_width = match parse_type_spec_str(dtype) {
                    Some(dt) => self.resolve_type_width(&dt).unwrap_or(1),
                    None => 1,
                };
                Ok(IrExpr::Cast { width: cast_width, expr: Box::new(inner_ir) })
            }
            Expr::FuncCall { name, args } if name.starts_with("process::") => {
                let ir_args: Result<Vec<IrExpr>, String> = args.iter()
                    .map(|a| self.elaborate_expr(a, signal_map, signals))
                    .collect();
                Ok(IrExpr::SysFunc { name: name.clone(), args: ir_args? })
            }
            Expr::FuncCall { name, args } if name.ends_with("::new") && (self.design.classes.iter().any(|c| *name == format!("{}::new", c.name))
                || BUILTIN_UVM_CLASSES.iter().any(|c| *name == format!("{}::new", c))
                || name.contains('#')
            ) => {
                let raw_name = name.strip_suffix("::new").unwrap().to_string();
                let class_name = if let Some(hash_pos) = raw_name.find('#') {
                    let base = &raw_name[..hash_pos];
                    let type_spec = &raw_name[hash_pos+1..];
                    let specialized = format!("{}__param_{}", base, type_spec.replace(',', "_"));
                    let exists_in_design = self.design.classes.iter().any(|c| c.name == specialized);
                    let exists_in_spec = self.specialized_classes.borrow().iter().any(|c| c.name == specialized);
                    if !exists_in_design && !exists_in_spec {
                        let orig = self.design.classes.iter().find(|c| c.name == base).cloned();
                        if let Some(mut spec) = orig {
                            let tp_name = spec.type_params.first().map(|tp| tp.name.clone());
                            spec.name = specialized.clone();
                            if let Some(ref param_name) = tp_name {
                                let type_dt = parse_type_spec_str(type_spec);
                                if let Some(ref dt) = type_dt {
                                    spec = substitute_class_types(spec, param_name, dt);
                                }
                            }
                            self.specialized_classes.borrow_mut().push(spec);
                        }
                    }
                    specialized
                } else if BUILTIN_UVM_CLASSES.contains(&raw_name.as_str()) {
                    format!("__{}", raw_name)
                } else {
                    raw_name.clone()
                };
                let ir_args: Result<Vec<IrExpr>, String> = args.iter()
                    .map(|a| self.elaborate_expr(a, signal_map, signals))
                    .collect();
                Ok(IrExpr::NewCall { class_name, args: ir_args? })
            }
            Expr::FuncCall { name, args } if name == "uvm_factory::set_type_override_by_type" => {
                let ir_args: Result<Vec<IrExpr>, String> = args.iter()
                    .map(|a| self.elaborate_expr(a, signal_map, signals))
                    .collect();
                Ok(IrExpr::SysFunc { name: name.clone(), args: ir_args? })
            }
            Expr::FuncCall { name, args } if name == "uvm_config_db::set" || name == "uvm_config_db::get"
                || name == "uvm_resource_db::set" || name == "uvm_resource_db::get" => {
                let ir_args: Result<Vec<IrExpr>, String> = args.iter()
                    .map(|a| self.elaborate_expr(a, signal_map, signals))
                    .collect();
                // Use SysFunc variant for engine dispatch
                Ok(IrExpr::SysFunc { name: name.clone(), args: ir_args? })
            }
            Expr::FuncCall { name, args } if name != "new" => {
                let is_dpi = self.design.modules.iter().flat_map(|m| m.items.iter())
                    .any(|item| matches!(item, ModuleItem::DpiImport(d) if d.name == *name));
                if is_dpi {
                    let ir_args: Result<Vec<IrExpr>, String> = args.iter()
                        .map(|a| self.elaborate_expr(a, signal_map, signals))
                        .collect();
                    let return_width = self.design.modules.iter().flat_map(|m| m.items.iter())
                        .filter_map(|item| if let ModuleItem::DpiImport(d) = item { Some(d) } else { None })
                        .find(|d| d.name == *name)
                        .and_then(|d| d.return_type.as_ref())
                        .map(|dt| dt.width())
                        .unwrap_or(32);
                    Ok(IrExpr::DpiCall { name: name.clone(), args: ir_args?, return_width })
                } else {
                    Err(format!("function '{}' not found (not a DPI import)", name))
                }
            }
            _ => Err(format!("expression type not yet supported")),
        }
    }

    fn elaborate_expr_to_signal(&self, expr: &Expr, signal_map: &HashMap<String, SignalId>)
        -> Result<SignalId, String>
    {
        match expr {
            Expr::Ident(name) => {
                signal_map.get(name)
                    .ok_or_else(|| format!("signal '{}' not found", name))
                    .copied()
            }
            Expr::MethodCall { .. } => Err("method calls cannot resolve to a signal".to_string()),
            Expr::MemberAccess { .. } => Err("member access cannot resolve to a signal".to_string()),
            _ => Err("expected simple signal identifier".to_string())
        }
    }

    fn resolve_typedef_width(&self, dtype: &DataType, range: Option<&ExprRange>) -> usize {
        if let Some(er) = range {
            if let (Ok(msb), Ok(lsb)) = (const_eval_simple(&er.msb), const_eval_simple(&er.lsb)) {
                return if msb >= lsb { (msb - lsb + 1) as usize } else { (lsb - msb + 1) as usize };
            }
        }
        match dtype {
            DataType::UserDefined(name) => self.typedef_map.get(name).copied().unwrap_or(64),
            DataType::Signed(inner) => self.resolve_typedef_width(inner, None),
            _ => dtype.width(),
        }
    }
}

fn expand_all_generates(module: &mut Module, param_vals: &HashMap<String, i64>) -> Result<(), String> {
    let mut i = 0;
    while i < module.items.len() {
        if let ModuleItem::Generate(gen) = &module.items[i] {
            let expanded = expand_generate_block(gen, param_vals)?;
            for item in &expanded {
                if let ModuleItem::Decl(d) = item {
                    module.decls.push(d.clone());
                }
            }
            module.items.splice(i..=i, expanded);
        } else {
            i += 1;
        }
    }
    Ok(())
}

fn extract_generate_step(step: &Option<Stmt>, param_vals: &HashMap<String, i64>) -> i64 {
    let Some(Stmt::BlockingAssign { rhs, .. }) = step else { return 1 };
    match rhs {
        Expr::BinaryOp { op: BinaryOp::Add, lhs: _, rhs } => {
            const_eval_with_params(rhs, param_vals).unwrap_or(1)
        }
        Expr::BinaryOp { op: BinaryOp::Sub, lhs: _, rhs } => {
            -const_eval_with_params(rhs, param_vals).unwrap_or(1)
        }
        _ => 1
    }
}

fn expand_generate_block(gen: &GenerateBlock, param_vals: &HashMap<String, i64>) -> Result<Vec<ModuleItem>, String> {
    let mut result = Vec::new();
    for item in &gen.items {
        match item {
            GenerateItem::If { cond, true_items, false_items } => {
                let val = const_eval_with_params(cond, param_vals)
                    .map_err(|_| format!("non-constant condition in generate if"))?;
                let branch = if val != 0 { true_items } else { false_items };
                for item in branch {
                    result.push(item.clone());
                }
            }
            GenerateItem::For { var, init, cond, step, body_items } => {
                let start_val: i64 = match init {
                    Some(Stmt::BlockingAssign { rhs, .. }) => const_eval_with_params(rhs, param_vals)?,
                    _ => 0,
                };
                let limit: i64 = match cond {
                    Some(Expr::BinaryOp { op: BinaryOp::Lt, rhs, .. }) => {
                        const_eval_with_params(rhs, param_vals)?
                    }
                    Some(Expr::BinaryOp { op: BinaryOp::Le, rhs, .. }) => {
                        const_eval_with_params(rhs, param_vals)? + 1
                    }
                    Some(c) => {
                        if const_eval_with_params(c, param_vals)? != 0 { 1 } else { 0 }
                    }
                    None => 0,
                };
                let step_val = extract_generate_step(step, param_vals);
                if step_val > 0 {
                    let mut cur = start_val;
                    while cur < limit {
                        for mut item in body_items.clone() {
                            substitute_genvar_in_module_item(&mut item, var, cur);
                            result.push(item);
                        }
                        cur += step_val;
                    }
                } else if step_val < 0 {
                    let mut cur = start_val;
                    while cur > limit {
                        for mut item in body_items.clone() {
                            substitute_genvar_in_module_item(&mut item, var, cur);
                            result.push(item);
                        }
                        cur += step_val;
                    }
                }
            }
            GenerateItem::Case { case_type: _case_type, expr, items, default } => {
                let case_val = const_eval_with_params(expr, param_vals)
                    .map_err(|_| format!("non-constant expression in generate case"))?;
                let mut matched = false;
                for ci in items {
                    for label in &ci.labels {
                        let label_val = const_eval_with_params(label, param_vals)?;
                        if label_val == case_val {
                            for item in &ci.body {
                                result.push(item.clone());
                            }
                            matched = true;
                            break;
                        }
                    }
                    if matched { break; }
                }
                if !matched {
                    if let Some(default_items) = default {
                        for item in default_items {
                            result.push(item.clone());
                        }
                    }
                }
            }
            GenerateItem::Items(items) => {
                for item in items {
                    result.push(item.clone());
                }
            }
        }
    }
    Ok(result)
}

fn substitute_genvar_in_module_item(item: &mut ModuleItem, var_name: &str, value: i64) {
    match item {
        ModuleItem::Always(always) => {
            for stmt in &mut always.stmts {
                let old = std::mem::replace(stmt, Stmt::Null);
                *stmt = substitute_loop_var_in_stmt(&old, var_name, value);
            }
        }
        ModuleItem::Initial(initial) => {
            for stmt in &mut initial.stmts {
                let old = std::mem::replace(stmt, Stmt::Null);
                *stmt = substitute_loop_var_in_stmt(&old, var_name, value);
            }
        }
        ModuleItem::Final(final_block) => {
            for stmt in &mut final_block.stmts {
                let old = std::mem::replace(stmt, Stmt::Null);
                *stmt = substitute_loop_var_in_stmt(&old, var_name, value);
            }
        }
        ModuleItem::Assign(assign) => {
            let old_lhs = std::mem::replace(
                &mut assign.lhs, Expr::Value(crate::ast::expr::Value::Decimal(0))
            );
            let old_rhs = std::mem::replace(
                &mut assign.rhs, Expr::Value(crate::ast::expr::Value::Decimal(0))
            );
            assign.lhs = substitute_loop_var_in_expr(&old_lhs, var_name, value);
            assign.rhs = substitute_loop_var_in_expr(&old_rhs, var_name, value);
        }
        ModuleItem::Instance(inst) => {
            if let Some(range) = &mut inst.range {
                let old_msb = std::mem::replace(
                    &mut range.msb, Expr::Value(crate::ast::expr::Value::Decimal(0))
                );
                let old_lsb = std::mem::replace(
                    &mut range.lsb, Expr::Value(crate::ast::expr::Value::Decimal(0))
                );
                range.msb = substitute_loop_var_in_expr(&old_msb, var_name, value);
                range.lsb = substitute_loop_var_in_expr(&old_lsb, var_name, value);
            }
            for (_, expr) in &mut inst.param_assigns {
                let old = std::mem::replace(
                    expr, Expr::Value(crate::ast::expr::Value::Decimal(0))
                );
                *expr = substitute_loop_var_in_expr(&old, var_name, value);
            }
            for conn in &mut inst.port_conns {
                match conn {
                    PortConnection::Positional(expr) => {
                        let old = std::mem::replace(
                            expr, Expr::Value(crate::ast::expr::Value::Decimal(0))
                        );
                        *expr = substitute_loop_var_in_expr(&old, var_name, value);
                    }
                    PortConnection::Named { expr, .. } => {
                        let old = std::mem::replace(
                            expr, Expr::Value(crate::ast::expr::Value::Decimal(0))
                        );
                        *expr = substitute_loop_var_in_expr(&old, var_name, value);
                    }
                }
            }
        }
        ModuleItem::Decl(decl) => {
            for var in &mut decl.names {
                if let Some(er) = &var.expr_range {
                    let old_msb = er.msb.clone();
                    let old_lsb = er.lsb.clone();
                    let new_msb = substitute_loop_var_in_expr(&old_msb, var_name, value);
                    let new_lsb = substitute_loop_var_in_expr(&old_lsb, var_name, value);
                    if let (Ok(msb), Ok(lsb)) = (const_eval_simple(&new_msb), const_eval_simple(&new_lsb)) {
                        var.expr_range = None;
                        var.range = Some(Range { msb: msb as usize, lsb: lsb as usize });
                    }
                }
            }
        }
        ModuleItem::Gate(ref mut gate) => {
            for port in &mut gate.ports {
                let old = std::mem::replace(
                    port, Expr::Value(crate::ast::expr::Value::Decimal(0))
                );
                *port = substitute_loop_var_in_expr(&old, var_name, value);
            }
        }
        ModuleItem::Generate(gen) => {
            for gi in &mut gen.items {
                substitute_genvar_in_generate_item(gi, var_name, value);
            }
        }
        ModuleItem::Func(_) | ModuleItem::Typedef(_) | ModuleItem::Import { .. }
        | ModuleItem::Covergroup(_) | ModuleItem::DpiImport(_) | ModuleItem::Param(_) => {}
    }
}

fn substitute_genvar_in_generate_item(item: &mut GenerateItem, var_name: &str, value: i64) {
    match item {
        GenerateItem::If { cond, true_items, false_items } => {
            let old_cond = std::mem::replace(cond, Expr::Value(crate::ast::expr::Value::Decimal(0)));
            *cond = substitute_loop_var_in_expr(&old_cond, var_name, value);
            for item in true_items.iter_mut() {
                substitute_genvar_in_module_item(item, var_name, value);
            }
            for item in false_items.iter_mut() {
                substitute_genvar_in_module_item(item, var_name, value);
            }
        }
        GenerateItem::For { var: _, init, cond, step, body_items } => {
            if let Some(stmt) = init {
                let old = std::mem::replace(stmt, Stmt::Null);
                *stmt = substitute_loop_var_in_stmt(&old, var_name, value);
            }
            if let Some(expr) = cond {
                let old = std::mem::replace(expr, Expr::Value(crate::ast::expr::Value::Decimal(0)));
                *expr = substitute_loop_var_in_expr(&old, var_name, value);
            }
            if let Some(stmt) = step {
                let old = std::mem::replace(stmt, Stmt::Null);
                *stmt = substitute_loop_var_in_stmt(&old, var_name, value);
            }
            for item in body_items.iter_mut() {
                substitute_genvar_in_module_item(item, var_name, value);
            }
        }
        GenerateItem::Case { expr, items, default, .. } => {
            let old_expr = std::mem::replace(expr, Expr::Value(crate::ast::expr::Value::Decimal(0)));
            *expr = substitute_loop_var_in_expr(&old_expr, var_name, value);
            for ci in items.iter_mut() {
                for label in ci.labels.iter_mut() {
                    let old = std::mem::replace(label, Expr::Value(crate::ast::expr::Value::Decimal(0)));
                    *label = substitute_loop_var_in_expr(&old, var_name, value);
                }
                for item in ci.body.iter_mut() {
                    substitute_genvar_in_module_item(item, var_name, value);
                }
            }
            if let Some(default_items) = default {
                for item in default_items.iter_mut() {
                    substitute_genvar_in_module_item(item, var_name, value);
                }
            }
        }
        GenerateItem::Items(items) => {
            for item in items.iter_mut() {
                substitute_genvar_in_module_item(item, var_name, value);
            }
        }
    }
}

fn try_unroll_for_loop<'a, F>(init: Option<&'a Stmt>, cond: Option<&'a Expr>, step: Option<&'a Stmt>,
                           stmts: &[Stmt], elaborate_body: &F,
                           params: &HashMap<String, i64>)
    -> Result<Option<Vec<IrStmt>>, String>
    where F: Fn(&[Stmt], &str, i64) -> Result<Vec<IrStmt>, String>
{
    // Extract loop variable name and initial value from init statement
    let (var_name, init_val) = match init {
        Some(Stmt::BlockingAssign { lhs: Expr::Ident(name), rhs, .. }) => {
            (name.clone(), const_eval_with_params(rhs, params)?)
        }
        _ => return Ok(None),
    };

    // Extract step function from step statement
    let step_fn: Box<dyn Fn(i64) -> Result<i64, String>> = match step {
        Some(Stmt::BlockingAssign { lhs: Expr::Ident(n), rhs, .. }) if *n == var_name => {
            match rhs {
                Expr::BinaryOp { op: BinaryOp::Add, lhs, rhs } => {
                    if let Expr::Ident(n2) = lhs.as_ref() {
                        if n2 == &var_name {
                            let inc = const_eval_with_params(rhs, params)?;
                            Box::new(move |v| Ok(v + inc))
                        } else { return Ok(None) }
                    } else if let Expr::Ident(n2) = rhs.as_ref() {
                        if n2 == &var_name {
                            let inc = const_eval_with_params(lhs, params)?;
                            Box::new(move |v| Ok(v + inc))
                        } else { return Ok(None) }
                    } else { return Ok(None) }
                }
                _ => return Ok(None),
            }
        }
        _ => return Ok(None),
    };

    // Extract loop limit from condition: var < limit
    let limit = match cond {
        Some(Expr::BinaryOp { op: BinaryOp::Lt, lhs, rhs }) => {
            match lhs.as_ref() {
                Expr::Ident(n) if *n == var_name => const_eval_with_params(rhs, params)?,
                _ => return Ok(None),
            }
        }
        _ => return Ok(None),
    };

    // Unroll the loop
    let mut all_stmts = Vec::new();
    let mut ivar = init_val;
    while ivar < limit {
        let body = elaborate_body(stmts, &var_name, ivar)?;
        all_stmts.extend(body);
        ivar = step_fn(ivar)?;
    }

    Ok(Some(all_stmts))
}

fn substitute_loop_var_in_stmts(stmts: &[Stmt], var_name: &str, value: i64) -> Vec<Stmt> {
    stmts.iter().map(|s| substitute_loop_var_in_stmt(s, var_name, value)).collect()
}

fn substitute_loop_var_in_stmt(stmt: &Stmt, var_name: &str, value: i64) -> Stmt {
    match stmt {
        Stmt::Block { stmts } => Stmt::Block {
            stmts: substitute_loop_var_in_stmts(stmts, var_name, value),
        },
        Stmt::BlockingAssign { lhs, rhs, delay } => Stmt::BlockingAssign {
            lhs: substitute_loop_var_in_expr(lhs, var_name, value),
            rhs: substitute_loop_var_in_expr(rhs, var_name, value),
            delay: delay.clone(),
        },
        Stmt::NonBlockingAssign { lhs, rhs, delay } => Stmt::NonBlockingAssign {
            lhs: substitute_loop_var_in_expr(lhs, var_name, value),
            rhs: substitute_loop_var_in_expr(rhs, var_name, value),
            delay: delay.clone(),
        },
        Stmt::IfElse { cond, true_branch, false_branch } => Stmt::IfElse {
            cond: substitute_loop_var_in_expr(cond, var_name, value),
            true_branch: Box::new(substitute_loop_var_in_stmt(true_branch, var_name, value)),
            false_branch: false_branch.as_ref().map(|fb|
                Box::new(substitute_loop_var_in_stmt(fb, var_name, value))),
        },
        Stmt::Case { expr, items, default } => Stmt::Case {
            expr: substitute_loop_var_in_expr(expr, var_name, value),
            items: items.iter().map(|item| crate::ast::stmt::CaseItem {
                labels: item.labels.iter().map(|l| substitute_loop_var_in_expr(l, var_name, value)).collect(),
                stmt: Box::new(substitute_loop_var_in_stmt(&item.stmt, var_name, value)),
            }).collect(),
            default: default.as_ref().map(|d| Box::new(substitute_loop_var_in_stmt(d, var_name, value))),
        },
        Stmt::StmtAssign { lhs, rhs } => Stmt::StmtAssign {
            lhs: substitute_loop_var_in_expr(lhs, var_name, value),
            rhs: substitute_loop_var_in_expr(rhs, var_name, value),
        },
        Stmt::Delay { delay, stmt } => Stmt::Delay {
            delay: substitute_loop_var_in_expr(delay, var_name, value),
            stmt: Box::new(substitute_loop_var_in_stmt(stmt, var_name, value)),
        },
        Stmt::SysCall { name, args } => Stmt::SysCall {
            name: name.clone(),
            args: args.iter().map(|a| substitute_loop_var_in_expr(a, var_name, value)).collect(),
        },
        Stmt::Expr { expr } => Stmt::Expr {
            expr: substitute_loop_var_in_expr(expr, var_name, value),
        },
        Stmt::CaseX { expr, items, default } => Stmt::CaseX {
            expr: substitute_loop_var_in_expr(expr, var_name, value),
            items: items.iter().map(|item| crate::ast::stmt::CaseItem {
                labels: item.labels.iter().map(|l| substitute_loop_var_in_expr(l, var_name, value)).collect(),
                stmt: Box::new(substitute_loop_var_in_stmt(&item.stmt, var_name, value)),
            }).collect(),
            default: default.as_ref().map(|d| Box::new(substitute_loop_var_in_stmt(d, var_name, value))),
        },
        Stmt::CaseZ { expr, items, default } => Stmt::CaseZ {
            expr: substitute_loop_var_in_expr(expr, var_name, value),
            items: items.iter().map(|item| crate::ast::stmt::CaseItem {
                labels: item.labels.iter().map(|l| substitute_loop_var_in_expr(l, var_name, value)).collect(),
                stmt: Box::new(substitute_loop_var_in_stmt(&item.stmt, var_name, value)),
            }).collect(),
            default: default.as_ref().map(|d| Box::new(substitute_loop_var_in_stmt(d, var_name, value))),
        },
        Stmt::StmtCase { expr, items, default } => Stmt::StmtCase {
            expr: substitute_loop_var_in_expr(expr, var_name, value),
            items: items.iter().map(|item| crate::ast::stmt::CaseItem {
                labels: item.labels.iter().map(|l| substitute_loop_var_in_expr(l, var_name, value)).collect(),
                stmt: Box::new(substitute_loop_var_in_stmt(&item.stmt, var_name, value)),
            }).collect(),
            default: default.as_ref().map(|d| Box::new(substitute_loop_var_in_stmt(d, var_name, value))),
        },
        Stmt::LoopForever { stmts } => Stmt::LoopForever {
            stmts: substitute_loop_var_in_stmts(stmts, var_name, value),
        },
        Stmt::LoopWhile { cond, stmts } => Stmt::LoopWhile {
            cond: substitute_loop_var_in_expr(cond, var_name, value),
            stmts: substitute_loop_var_in_stmts(stmts, var_name, value),
        },
        Stmt::LoopFor { init, cond, step, stmts } => Stmt::LoopFor {
            init: init.as_ref().map(|s| Box::new(substitute_loop_var_in_stmt(s, var_name, value))),
            cond: cond.as_ref().map(|c| substitute_loop_var_in_expr(c, var_name, value)),
            step: step.as_ref().map(|s| Box::new(substitute_loop_var_in_stmt(s, var_name, value))),
            stmts: substitute_loop_var_in_stmts(stmts, var_name, value),
        },
        Stmt::Repeat { count, stmts } => Stmt::Repeat {
            count: substitute_loop_var_in_expr(count, var_name, value),
            stmts: substitute_loop_var_in_stmts(stmts, var_name, value),
        },
        Stmt::Wait { cond, stmt } => Stmt::Wait {
            cond: substitute_loop_var_in_expr(cond, var_name, value),
            stmt: stmt.as_ref().map(|s| Box::new(substitute_loop_var_in_stmt(s, var_name, value))),
        },
        Stmt::Disable { name } => Stmt::Disable { name: name.clone() },
        Stmt::Force { lhs, rhs } => Stmt::Force {
            lhs: substitute_loop_var_in_expr(lhs, var_name, value),
            rhs: substitute_loop_var_in_expr(rhs, var_name, value),
        },
        Stmt::Release { expr } => Stmt::Release {
            expr: substitute_loop_var_in_expr(expr, var_name, value),
        },
        Stmt::Deassign { expr } => Stmt::Deassign {
            expr: substitute_loop_var_in_expr(expr, var_name, value),
        },
        Stmt::Return(expr) => Stmt::Return(
            expr.as_ref().map(|e| Box::new(substitute_loop_var_in_expr(e, var_name, value))),
        ),
        Stmt::Null => Stmt::Null,
        Stmt::SysFinish => Stmt::SysFinish,
        Stmt::EventControl { events, stmt } => Stmt::EventControl {
            events: events.iter().map(|e| substitute_sensitivity_event(e, var_name, value)).collect(),
            stmt: stmt.as_ref().map(|s| Box::new(substitute_loop_var_in_stmt(s, var_name, value))),
        },
        Stmt::EventTrigger { name } => Stmt::EventTrigger { name: name.clone() },
        Stmt::ForeachLoop { array_var, index_vars, stmts } => Stmt::ForeachLoop {
            array_var: array_var.clone(),
            index_vars: index_vars.clone(),
            stmts: substitute_loop_var_in_stmts(stmts, var_name, value),
        },
        Stmt::NamedBlock { name, stmts, decls } => Stmt::NamedBlock {
            name: name.clone(),
            stmts: substitute_loop_var_in_stmts(stmts, var_name, value),
            decls: decls.clone(),
        },
        Stmt::Break => Stmt::Break,
        Stmt::Continue => Stmt::Continue,
        Stmt::DoWhile { cond, stmts } => Stmt::DoWhile {
            cond: substitute_loop_var_in_expr(cond, var_name, value),
            stmts: substitute_loop_var_in_stmts(stmts, var_name, value),
        },
        _ => stmt.clone(),
    }
}

fn substitute_sensitivity_event(event: &SensitivityEvent, var_name: &str, value: i64) -> SensitivityEvent {
    match event {
        SensitivityEvent::PosEdge(e) => SensitivityEvent::PosEdge(substitute_loop_var_in_expr(e, var_name, value)),
        SensitivityEvent::NegEdge(e) => SensitivityEvent::NegEdge(substitute_loop_var_in_expr(e, var_name, value)),
        SensitivityEvent::Level(e) => SensitivityEvent::Level(substitute_loop_var_in_expr(e, var_name, value)),
        SensitivityEvent::Wildcard => SensitivityEvent::Wildcard,
    }
}

fn substitute_loop_var_in_expr(expr: &Expr, var_name: &str, value: i64) -> Expr {
    match expr {
        Expr::Ident(name) if name == var_name => Expr::Value(crate::ast::expr::Value::Decimal(value)),
        Expr::Ident(_) => expr.clone(),
        Expr::Value(_) | Expr::String(_) | Expr::Null => expr.clone(),
        Expr::RangeSelect { expr: inner, msb, lsb } => Expr::RangeSelect {
            expr: Box::new(substitute_loop_var_in_expr(inner, var_name, value)),
            msb: Box::new(substitute_loop_var_in_expr(msb, var_name, value)),
            lsb: Box::new(substitute_loop_var_in_expr(lsb, var_name, value)),
        },
        Expr::BitSelect { expr: inner, index } => Expr::BitSelect {
            expr: Box::new(substitute_loop_var_in_expr(inner, var_name, value)),
            index: Box::new(substitute_loop_var_in_expr(index, var_name, value)),
        },
        Expr::PartSelect { expr: inner, base, width } => Expr::PartSelect {
            expr: Box::new(substitute_loop_var_in_expr(inner, var_name, value)),
            base: Box::new(substitute_loop_var_in_expr(base, var_name, value)),
            width: Box::new(substitute_loop_var_in_expr(width, var_name, value)),
        },
        Expr::Concat(exprs) => Expr::Concat(
            exprs.iter().map(|e| substitute_loop_var_in_expr(e, var_name, value)).collect()
        ),
        Expr::FuncCall { name, args } => Expr::FuncCall {
            name: name.clone(),
            args: args.iter().map(|a| substitute_loop_var_in_expr(a, var_name, value)).collect(),
        },
        Expr::Replicate { count, expr: inner } => Expr::Replicate {
            count: Box::new(substitute_loop_var_in_expr(count, var_name, value)),
            expr: Box::new(substitute_loop_var_in_expr(inner, var_name, value)),
        },
        Expr::UnaryOp { op, expr: inner } => Expr::UnaryOp {
            op: op.clone(),
            expr: Box::new(substitute_loop_var_in_expr(inner, var_name, value)),
        },
        Expr::BinaryOp { op, lhs, rhs } => Expr::BinaryOp {
            op: op.clone(),
            lhs: Box::new(substitute_loop_var_in_expr(lhs, var_name, value)),
            rhs: Box::new(substitute_loop_var_in_expr(rhs, var_name, value)),
        },
        Expr::TernaryOp { cond, true_expr, false_expr } => Expr::TernaryOp {
            cond: Box::new(substitute_loop_var_in_expr(cond, var_name, value)),
            true_expr: Box::new(substitute_loop_var_in_expr(true_expr, var_name, value)),
            false_expr: Box::new(substitute_loop_var_in_expr(false_expr, var_name, value)),
        },
        Expr::Paren(inner) => Expr::Paren(Box::new(substitute_loop_var_in_expr(inner, var_name, value))),
        Expr::MethodCall { obj, method, args } => Expr::MethodCall {
            obj: Box::new(substitute_loop_var_in_expr(obj, var_name, value)),
            method: method.clone(),
            args: args.iter().map(|a| substitute_loop_var_in_expr(a, var_name, value)).collect(),
        },
        Expr::MemberAccess { obj, field } => Expr::MemberAccess {
            obj: Box::new(substitute_loop_var_in_expr(obj, var_name, value)),
            field: field.clone(),
        },
        Expr::FillLit(val) => Expr::FillLit(*val),
        Expr::Inside { expr: inner, range_list } => Expr::Inside {
            expr: Box::new(substitute_loop_var_in_expr(inner, var_name, value)),
            range_list: range_list.iter().map(|e| substitute_loop_var_in_expr(e, var_name, value)).collect(),
        },
        Expr::StreamingConcat { op, slices } => Expr::StreamingConcat {
            op: op.clone(),
            slices: slices.iter().map(|e| substitute_loop_var_in_expr(e, var_name, value)).collect(),
        },
        Expr::Cast { dtype, expr: inner } => Expr::Cast {
            dtype: dtype.clone(),
            expr: Box::new(substitute_loop_var_in_expr(inner, var_name, value)),
        },
        Expr::ScopedIdent { package, item } => Expr::ScopedIdent {
            package: package.clone(),
            item: item.clone(),
        },
    }
}

fn infer_comb_sensitivity(body: &[IrStmt]) -> Vec<SignalId> {
    let mut sigs = Vec::new();
    collect_read_signals_stmts(body, &mut sigs);
    sigs.sort();
    sigs.dedup();
    sigs
}

fn collect_read_signals_stmts(stmts: &[IrStmt], out: &mut Vec<SignalId>) {
    for stmt in stmts {
        collect_read_signals_stmt(stmt, out);
    }
}

fn collect_read_signals_stmt(stmt: &IrStmt, out: &mut Vec<SignalId>) {
    match stmt {
        IrStmt::Block { stmts } => collect_read_signals_stmts(stmts, out),
        IrStmt::BlockingAssign { rhs, .. } | IrStmt::NonBlockingAssign { rhs, .. } => {
            collect_read_signals_expr(rhs, out);
        }
        IrStmt::If { cond, true_branch, false_branch } => {
            collect_read_signals_expr(cond, out);
            collect_read_signals_stmts(true_branch, out);
            collect_read_signals_stmts(false_branch, out);
        }
        IrStmt::Case { expr, items, default, .. } => {
            collect_read_signals_expr(expr, out);
            for item in items {
                for label in &item.labels {
                    collect_read_signals_expr(label, out);
                }
                collect_read_signals_stmts(&item.body, out);
            }
            collect_read_signals_stmts(default, out);
        }
        IrStmt::Delay { body, .. } => collect_read_signals_stmts(body, out),
        IrStmt::Wait { cond, body } => {
            collect_read_signals_expr(cond, out);
            collect_read_signals_stmts(body, out);
        }
        IrStmt::SysCall { args, .. } => {
            for arg in args {
                collect_read_signals_expr(arg, out);
            }
        }
        IrStmt::LoopWhile { cond, body } => {
            collect_read_signals_expr(cond, out);
            collect_read_signals_stmts(body, out);
        }
        IrStmt::LoopDoWhile { cond, body } => {
            collect_read_signals_expr(cond, out);
            collect_read_signals_stmts(body, out);
        }
        IrStmt::LoopFor { init, cond, step, body } => {
            if let Some(s) = init {
                collect_read_signals_stmt(s, out);
            }
            collect_read_signals_expr(cond, out);
            if let Some(s) = step {
                collect_read_signals_stmt(s, out);
            }
            collect_read_signals_stmts(body, out);
        }
        IrStmt::EventControl { sig_id, body, .. } => {
            out.push(*sig_id);
            collect_read_signals_stmts(body, out);
        }
        IrStmt::EventTrigger { sig_id } => {
            out.push(*sig_id);
        }
        IrStmt::MethodCallStmt { obj, args, .. } => {
            collect_read_signals_expr(obj, out);
            for arg in args {
                collect_read_signals_expr(arg, out);
            }
        }
        IrStmt::NamedBlock { stmts, .. } => {
            collect_read_signals_stmts(stmts, out);
        }
        IrStmt::Release { .. } | IrStmt::Deassign { .. } => {}
        IrStmt::Force { rhs, .. } => {
            collect_read_signals_expr(rhs, out);
        }
        IrStmt::Disable { .. } => {}
        _ => {}
    }
}

fn collect_read_signals_expr(expr: &IrExpr, out: &mut Vec<SignalId>) {
    match expr {
        IrExpr::Signal(id, _) | IrExpr::RangeSelect(id, ..) | IrExpr::BitSelect(id, _) | IrExpr::ArrayIndex { sig_id: id, .. } => {
            out.push(*id);
        }
        IrExpr::Const(_) | IrExpr::String(_) | IrExpr::FillLit(_) => {}
        IrExpr::Concat(exprs) => {
            for e in exprs {
                collect_read_signals_expr(e, out);
            }
        }
        IrExpr::Replicate(_, inner) => {
            collect_read_signals_expr(inner, out);
        }
        IrExpr::UnaryOp(_, inner) => collect_read_signals_expr(inner, out),
        IrExpr::BinaryOp(_, lhs, rhs) => {
            collect_read_signals_expr(lhs, out);
            collect_read_signals_expr(rhs, out);
        }
        IrExpr::Cond(c, t, f) => {
            collect_read_signals_expr(c, out);
            collect_read_signals_expr(t, out);
            collect_read_signals_expr(f, out);
        }
        IrExpr::Signed(inner) => collect_read_signals_expr(inner, out),
        IrExpr::NewCall { args, .. } => {
            for arg in args {
                collect_read_signals_expr(arg, out);
            }
        }
        IrExpr::This => {}
        IrExpr::SysFunc { args, .. } => {
            for arg in args {
                collect_read_signals_expr(arg, out);
            }
        }
        IrExpr::MethodCall { obj, args, .. } => {
            collect_read_signals_expr(obj, out);
            for arg in args {
                collect_read_signals_expr(arg, out);
            }
        }
        IrExpr::MemberAccess { obj, .. } => {
            collect_read_signals_expr(obj, out);
        }
        IrExpr::ExprRangeSelect(inner, _, _) => {
            collect_read_signals_expr(inner, out);
        }
        IrExpr::ExprBitSelect(inner, _) => {
            collect_read_signals_expr(inner, out);
        }
        IrExpr::ExprPartSelect(inner, base_expr, width_expr) => {
            collect_read_signals_expr(inner, out);
            collect_read_signals_expr(base_expr, out);
            collect_read_signals_expr(width_expr, out);
        }
        IrExpr::DpiCall { args, .. } => {
            for arg in args {
                collect_read_signals_expr(arg, out);
            }
        }
        IrExpr::HierRef(_) => {}
        IrExpr::Inside { expr, list } => {
            collect_read_signals_expr(expr, out);
            for item in list {
                collect_read_signals_expr(item, out);
            }
        }
        IrExpr::Cast { expr, .. } => {
            collect_read_signals_expr(expr, out);
        }
    }
}

fn resolve_expr_signal(expr: &Expr, signal_map: &HashMap<String, SignalId>) -> Option<SignalId> {
    match expr {
        Expr::Ident(name) => signal_map.get(name).copied(),
        Expr::MethodCall { .. } => None,
        Expr::MemberAccess { .. } => None,
        _ => None,
    }
}

fn compute_expr_width(expr: &Expr, signal_map: &HashMap<String, SignalId>,
                      signals: &[SignalInfo], param_vals: &HashMap<String, i64>) -> Result<usize, String> {
    match expr {
        Expr::Ident(name) => {
            if let Some(sig_id) = signal_map.get(name) {
                let info = &signals[*sig_id];
                Ok(info.width * if info.array_depth > 0 { info.array_depth } else { 1 })
            } else if let Some(&val) = param_vals.get(name) {
                let abs = val.unsigned_abs();
                Ok(if val == 0 { 1 } else { 64 - (abs.leading_zeros() as usize) }.max(1))
            } else {
                Err(format!("cannot determine width of '{}'", name))
            }
        }
        Expr::Value(v) => {
            match v {
                Value::Binary { width, .. } => Ok(width.unwrap_or(1)),
                Value::Hex { width, .. } => Ok(width.unwrap_or(1)),
                Value::Octal { width, .. } => Ok(width.unwrap_or(1)),
                Value::Decimal(_) => Ok(32),
                Value::Real(_) => Ok(64),
            }
        }
        Expr::FillLit(_) => Ok(1),
        Expr::FuncCall { name, .. } => {
            if let Some(width) = param_vals.get(name) {
                let abs = width.unsigned_abs();
                Ok(if *width == 0 { 1 } else { 64 - (abs.leading_zeros() as usize) }.max(1))
            } else {
                Err(format!("cannot determine width of function '{}'", name))
            }
        }
        Expr::Paren(inner) => compute_expr_width(inner, signal_map, signals, param_vals),
        Expr::UnaryOp { op, expr: inner } => {
            match op {
                UnaryOp::ReductionAnd | UnaryOp::ReductionNand | UnaryOp::ReductionOr
                | UnaryOp::ReductionNor | UnaryOp::ReductionXor | UnaryOp::ReductionXnor
                | UnaryOp::Not => Ok(1),
                _ => compute_expr_width(inner, signal_map, signals, param_vals),
            }
        }
        Expr::BinaryOp { lhs, rhs, .. } => {
            let lw = compute_expr_width(lhs, signal_map, signals, param_vals)?;
            let rw = compute_expr_width(rhs, signal_map, signals, param_vals)?;
            Ok(lw.max(rw))
        }
        Expr::Concat(items) => {
            let mut total = 0;
            for item in items {
                total += compute_expr_width(item, signal_map, signals, param_vals)?;
            }
            Ok(total)
        }
        Expr::Replicate { count, expr: inner } => {
            let c = const_eval_with_params(count, param_vals).unwrap_or(1) as usize;
            let w = compute_expr_width(inner, signal_map, signals, param_vals)?;
            Ok(c * w)
        }
        Expr::TernaryOp { true_expr, false_expr, .. } => {
            let tw = compute_expr_width(true_expr, signal_map, signals, param_vals)?;
            let fw = compute_expr_width(false_expr, signal_map, signals, param_vals)?;
            Ok(tw.max(fw))
        }
        Expr::RangeSelect { msb, lsb, .. } => {
            if let (Ok(m), Ok(l)) = (const_eval_with_params(msb, param_vals), const_eval_with_params(lsb, param_vals)) {
                Ok((m.abs_diff(l) + 1) as usize)
            } else {
                Err("dynamic range select width not computable at compile time".to_string())
            }
        }
        Expr::BitSelect { .. } => Ok(1),
        Expr::PartSelect { width, .. } => {
            Ok(const_eval_with_params(width, param_vals).unwrap_or(1) as usize)
        }
        Expr::MemberAccess { obj, field } => {
            // Check if obj resolves to a struct signal
            if let Expr::Ident(name) = obj.as_ref() {
                if let Some(&sig_id) = signal_map.get(name) {
                    if !signals[sig_id].struct_fields.is_empty() {
                        if let Some(f) = signals[sig_id].struct_fields.iter().find(|f| f.name == *field) {
                            return Ok(f.width);
                        }
                    }
                }
            }
            compute_expr_width(obj, signal_map, signals, param_vals)
        }
        Expr::Cast { dtype, .. } => {
            match parse_type_spec_str(dtype) {
                Some(dt) => match dt {
                    DataType::UserDefined(name) => {
                        param_vals.get(&name).map(|&v| v as usize).ok_or_else(|| format!("unknown type '{}'", name))
                    }
                    _ => Ok(dt.width()),
                },
                None => Err(format!("unknown type '{}' in cast", dtype)),
            }
        }
        Expr::MethodCall { .. } | Expr::StreamingConcat { .. } => {
            Err("width not computable for this expression type".to_string())
        }
        Expr::ScopedIdent { package, item } => {
            Err(format!("cannot determine width of '{}.{}' at compile time", package, item))
        }
        _ => Err("cannot determine width of expression".to_string()),
    }
}

fn collect_sensitivity(expr: &Expr, signal_map: &HashMap<String, SignalId>) -> Vec<SignalId> {
    match expr {
        Expr::Ident(name) => {
            signal_map.get(name).map(|&id| vec![id]).unwrap_or_default()
        }
        Expr::BinaryOp { lhs, rhs, .. } => {
            let mut v = collect_sensitivity(lhs, signal_map);
            v.extend(collect_sensitivity(rhs, signal_map));
            v
        }
        Expr::UnaryOp { expr: inner, .. } => collect_sensitivity(inner, signal_map),
        Expr::Concat(exprs) => {
            exprs.iter().flat_map(|e| collect_sensitivity(e, signal_map)).collect()
        }
        Expr::BitSelect { expr: inner, index } => {
            let mut v = collect_sensitivity(inner, signal_map);
            v.extend(collect_sensitivity(index, signal_map));
            v
        }
        Expr::RangeSelect { expr: inner, msb, lsb } => {
            let mut v = collect_sensitivity(inner, signal_map);
            v.extend(collect_sensitivity(msb, signal_map));
            v.extend(collect_sensitivity(lsb, signal_map));
            v
        }
        Expr::PartSelect { expr: inner, base, width } => {
            let mut v = collect_sensitivity(inner, signal_map);
            v.extend(collect_sensitivity(base, signal_map));
            v.extend(collect_sensitivity(width, signal_map));
            v
        }
        Expr::TernaryOp { cond, true_expr, false_expr } => {
            let mut v = collect_sensitivity(cond, signal_map);
            v.extend(collect_sensitivity(true_expr, signal_map));
            v.extend(collect_sensitivity(false_expr, signal_map));
            v
        }
        Expr::MethodCall { obj, .. } => collect_sensitivity(obj, signal_map),
        Expr::MemberAccess { obj, .. } => collect_sensitivity(obj, signal_map),
        _ => vec![],
    }
}

fn const_eval_params(expr: &Expr, params: &HashMap<String, i64>) -> Result<i64, String> {
    const_eval_with_params(expr, params)
}

/// Try to constant-fold an AST expression into an IrExpr::Const.
/// Returns Ok(Some(IrExpr::Const(...))) if the expression is fully constant,
/// Ok(None) if it cannot be folded, or Err on evaluation error.
fn try_fold_const(expr: &Expr, params: &HashMap<String, i64>) -> Result<Option<IrExpr>, String> {
    match const_eval_with_params(expr, params) {
        Ok(val) => {
            let abs = val.unsigned_abs();
            let min_width = if val >= 0 {
                if val == 0 { 1 } else { 64 - (abs.leading_zeros() as usize) }
            } else {
                64 - (abs.leading_zeros() as usize) + 1
            };
            let width = min_width.max(32);
            Ok(Some(IrExpr::Const(LogicVec::from_u64(val as u64, width))))
        }
        Err(_) => Ok(None),
    }
}

fn value_to_logicvec(val: &Value) -> LogicVec {
    match val {
        Value::Decimal(n) => {
            let abs = n.unsigned_abs();
            let min_width = if *n == 0 { 1 } else { 64 - (abs.leading_zeros() as usize) };
            let width = min_width.max(32);
            let mut lv = LogicVec::from_u64(abs, width);
            if *n < 0 {
                // Two's complement
                for b in lv.bits.iter_mut() {
                    *b = match b {
                        LogicVal::Zero => LogicVal::One,
                        LogicVal::One => LogicVal::Zero,
                        _ => LogicVal::X,
                    };
                }
                // Add 1
                let mut carry = true;
                for b in lv.bits.iter_mut() {
                    if carry {
                        match b {
                            LogicVal::Zero => { *b = LogicVal::One; carry = false; }
                            LogicVal::One => { *b = LogicVal::Zero; }
                            _ => {}
                        }
                    }
                }
            }
            lv
        }
        Value::Binary { bits, width, .. } => {
            let w = width.unwrap_or(bits.len());
            let mut vec = LogicVec::new(w);
            for (i, c) in bits.chars().rev().enumerate() {
                if i >= w { break; }
                vec.bits[i] = match c {
                    '0' => LogicVal::Zero,
                    '1' => LogicVal::One,
                    'x' | 'X' => LogicVal::X,
                    'z' | 'Z' => LogicVal::Z,
                    '_' => continue,
                    _ => LogicVal::X,
                };
            }
            vec
        }
        Value::Hex { bits, width, .. } => {
            let w = width.unwrap_or(bits.len() * 4);
            let mut vec = LogicVec::new(w);
            let digits: String = bits.chars().filter(|c| *c != '_').collect();
            for (i, c) in digits.chars().rev().enumerate() {
                let hex_val = c.to_digit(16).unwrap_or(0);
                for j in 0..4 {
                    let bit_idx = i * 4 + j;
                    if bit_idx >= w { break; }
                    vec.bits[bit_idx] = if (hex_val >> j) & 1 == 1 {
                        LogicVal::One
                    } else {
                        LogicVal::Zero
                    };
                }
            }
            vec
        }
        Value::Octal { bits, width, .. } => {
            let w = width.unwrap_or(bits.len() * 3);
            let mut vec = LogicVec::new(w);
            let digits: String = bits.chars().filter(|c| *c != '_').collect();
            for (i, c) in digits.chars().rev().enumerate() {
                let oct_val = c.to_digit(8).unwrap_or(0);
                for j in 0..3 {
                    let bit_idx = i * 3 + j;
                    if bit_idx >= w { break; }
                    vec.bits[bit_idx] = if (oct_val >> j) & 1 == 1 {
                        LogicVal::One
                    } else {
                        LogicVal::Zero
                    };
                }
            }
            vec
        }
        Value::Real(r) => LogicVec::from_u64(r.to_bits(), 64),
    }
}

fn map_unary_op(op: &UnaryOp) -> Result<UnaryIrOp, String> {
    match op {
        UnaryOp::Plus => Ok(UnaryIrOp::Plus),
        UnaryOp::Minus => Ok(UnaryIrOp::Minus),
        UnaryOp::Not => Ok(UnaryIrOp::Not),
        UnaryOp::BitNot => Ok(UnaryIrOp::BitNot),
        UnaryOp::ReductionAnd => Ok(UnaryIrOp::RedAnd),
        UnaryOp::ReductionNand => Ok(UnaryIrOp::RedNand),
        UnaryOp::ReductionOr => Ok(UnaryIrOp::RedOr),
        UnaryOp::ReductionNor => Ok(UnaryIrOp::RedNor),
        UnaryOp::ReductionXor => Ok(UnaryIrOp::RedXor),
        UnaryOp::ReductionXnor => Ok(UnaryIrOp::RedXnor),
    }
}

fn map_binary_op(op: &BinaryOp) -> Result<BinaryIrOp, String> {
    match op {
        BinaryOp::Add => Ok(BinaryIrOp::Add),
        BinaryOp::Sub => Ok(BinaryIrOp::Sub),
        BinaryOp::Mul => Ok(BinaryIrOp::Mul),
        BinaryOp::Div => Ok(BinaryIrOp::Div),
        BinaryOp::Mod => Ok(BinaryIrOp::Mod),
        BinaryOp::Power => Ok(BinaryIrOp::Power),
        BinaryOp::Eq => Ok(BinaryIrOp::Eq),
        BinaryOp::Neq => Ok(BinaryIrOp::Neq),
        BinaryOp::CaseEq => Ok(BinaryIrOp::CaseEq),
        BinaryOp::CaseNeq => Ok(BinaryIrOp::CaseNeq),
        BinaryOp::EqWild => Ok(BinaryIrOp::EqWild),
        BinaryOp::NeqWild => Ok(BinaryIrOp::NeqWild),
        BinaryOp::Lt => Ok(BinaryIrOp::Lt),
        BinaryOp::Le => Ok(BinaryIrOp::Le),
        BinaryOp::Gt => Ok(BinaryIrOp::Gt),
        BinaryOp::Ge => Ok(BinaryIrOp::Ge),
        BinaryOp::BitAnd => Ok(BinaryIrOp::BitAnd),
        BinaryOp::BitOr => Ok(BinaryIrOp::BitOr),
        BinaryOp::BitXor => Ok(BinaryIrOp::BitXor),
        BinaryOp::BitXnor => Ok(BinaryIrOp::BitXnor),
        BinaryOp::Shl => Ok(BinaryIrOp::Shl),
        BinaryOp::Shr => Ok(BinaryIrOp::Shr),
        BinaryOp::Sshl => Ok(BinaryIrOp::Sshl),
        BinaryOp::Sshr => Ok(BinaryIrOp::Sshr),
        BinaryOp::LogicalAnd => Ok(BinaryIrOp::LogicalAnd),
        BinaryOp::LogicalOr => Ok(BinaryIrOp::LogicalOr),
    }
}

fn build_gate_expr(gate_type: &GateType, inputs: &[IrExpr]) -> IrExpr {
    match gate_type {
        GateType::And => fold_binary(BinaryIrOp::BitAnd, inputs),
        GateType::Or => fold_binary(BinaryIrOp::BitOr, inputs),
        GateType::Nand => IrExpr::UnaryOp(UnaryIrOp::BitNot, Box::new(fold_binary(BinaryIrOp::BitAnd, inputs))),
        GateType::Nor => IrExpr::UnaryOp(UnaryIrOp::BitNot, Box::new(fold_binary(BinaryIrOp::BitOr, inputs))),
        GateType::Xor => fold_binary(BinaryIrOp::BitXor, inputs),
        GateType::Xnor => IrExpr::UnaryOp(UnaryIrOp::BitNot, Box::new(fold_binary(BinaryIrOp::BitXor, inputs))),
        GateType::Buf => inputs[0].clone(),
        GateType::Not => IrExpr::UnaryOp(UnaryIrOp::BitNot, Box::new(inputs[0].clone())),
    }
}

fn fold_binary(op: BinaryIrOp, exprs: &[IrExpr]) -> IrExpr {
    if exprs.is_empty() {
        return IrExpr::Const(LogicVec::from_u64(0, 1));
    }
    let mut result = exprs[0].clone();
    for e in &exprs[1..] {
        result = IrExpr::BinaryOp(op.clone(), Box::new(result), Box::new(e.clone()));
    }
    result
}

impl DataType {
    fn width(&self) -> usize {
        match self {
            DataType::Bit | DataType::Logic => 1,
            DataType::Byte => 8,
            DataType::Shortint => 16,
            DataType::Int | DataType::Integer => 32,
            DataType::Longint => 64,
            DataType::Time => 64,
            DataType::Real | DataType::Realtime => 64,
            DataType::String => 0,
            DataType::Signed(inner) => inner.width(),
            DataType::UserDefined(_) => 64,
            DataType::EnumType { base: _, members: _ } => 32,
            DataType::StructType { members } => members.iter().map(|m| m.range.as_ref().map(|r| r.width()).unwrap_or(1)).sum(),
            DataType::UnionType { members } => members.iter().map(|m| m.range.as_ref().map(|r| r.width()).unwrap_or(1)).max().unwrap_or(1),
            DataType::Void => 0,
        }
    }
}

impl DeclKind {
    fn default_width(&self) -> usize {
        match self {
            DeclKind::Wire | DeclKind::Reg | DeclKind::Logic | DeclKind::Wand | DeclKind::Wor
                | DeclKind::Tri | DeclKind::Tri0 | DeclKind::Tri1 | DeclKind::TriAnd | DeclKind::TriOr
                | DeclKind::Supply0 | DeclKind::Supply1 => 1,
            DeclKind::Int | DeclKind::Integer => 32,
        }
    }
}

pub(crate) fn parse_type_spec_str(s: &str) -> Option<DataType> {
    match s {
        "bit" => Some(DataType::Bit),
        "logic" => Some(DataType::Logic),
        "int" => Some(DataType::Int),
        "integer" => Some(DataType::Integer),
        "byte" => Some(DataType::Byte),
        "shortint" => Some(DataType::Shortint),
        "longint" => Some(DataType::Longint),
        "time" => Some(DataType::Time),
        "real" => Some(DataType::Real),
        "realtime" => Some(DataType::Realtime),
        "string" => Some(DataType::String),
        _ => {
            // Check for 'signed <type>' pattern
            if let Some(inner) = s.strip_prefix("signed ") {
                parse_type_spec_str(inner).map(|dt| DataType::Signed(Box::new(dt)))
            } else {
                None
            }
        }
    }
}

fn substitute_class_types(cd: ClassDecl, param_name: &str, replacement: &DataType) -> ClassDecl {
    let mut new_members = Vec::new();
    for member in cd.members {
        match member {
            ClassMember::Decl(mut decl) => {
                decl.dtype = substitute_data_type(decl.dtype, param_name, replacement);
                new_members.push(ClassMember::Decl(decl));
            }
            ClassMember::Function(mut fd) => {
                fd.return_type = fd.return_type.map(|dt| {
                    Box::new(substitute_data_type(*dt, param_name, replacement))
                });
                new_members.push(ClassMember::Function(fd));
            }
            ClassMember::Task(td) => {
                new_members.push(ClassMember::Task(td));
            }
            ClassMember::Constraint { name, body } => {
                let new_body = body.into_iter().map(|ci| {
                    match ci {
                        ConstraintItem::Expr(e) => {
                            ConstraintItem::Expr(substitute_expr_types(e, param_name, replacement))
                        }
                    }
                }).collect();
                new_members.push(ClassMember::Constraint { name, body: new_body });
            }
            other => new_members.push(other),
        }
    }
    ClassDecl { members: new_members, ..cd }
}

fn substitute_data_type(dt: DataType, param_name: &str, replacement: &DataType) -> DataType {
    match dt {
        DataType::UserDefined(ref name) if name == param_name => replacement.clone(),
        DataType::Signed(inner) => DataType::Signed(Box::new(substitute_data_type(*inner, param_name, replacement))),
        DataType::EnumType { base, members } => {
            DataType::EnumType {
                base: base.map(|b| Box::new(substitute_data_type(*b, param_name, replacement))),
                members,
            }
        }
        DataType::StructType { members } => {
            DataType::StructType {
                members: members.into_iter().map(|m| {
                    StructMember {
                        dtype: Box::new(substitute_data_type(*m.dtype, param_name, replacement)),
                        ..m
                    }
                }).collect(),
            }
        }
        DataType::UnionType { members } => {
            DataType::UnionType {
                members: members.into_iter().map(|m| {
                    StructMember {
                        dtype: Box::new(substitute_data_type(*m.dtype, param_name, replacement)),
                        ..m
                    }
                }).collect(),
            }
        }
        other => other,
    }
}

fn substitute_expr_types(e: Expr, param_name: &str, replacement: &DataType) -> Expr {
    match e {
        Expr::BinaryOp { lhs, op, rhs } => {
            Expr::BinaryOp {
                lhs: Box::new(substitute_expr_types(*lhs, param_name, replacement)),
                op,
                rhs: Box::new(substitute_expr_types(*rhs, param_name, replacement)),
            }
        }
        Expr::UnaryOp { op, expr } => {
            Expr::UnaryOp { op, expr: Box::new(substitute_expr_types(*expr, param_name, replacement)) }
        }
        Expr::Paren(inner) => {
            Expr::Paren(Box::new(substitute_expr_types(*inner, param_name, replacement)))
        }
        Expr::Concat(items) => {
            Expr::Concat(items.into_iter().map(|e| substitute_expr_types(e, param_name, replacement)).collect())
        }
        Expr::Replicate { count, expr } => {
            Expr::Replicate {
                count: Box::new(substitute_expr_types(*count, param_name, replacement)),
                expr: Box::new(substitute_expr_types(*expr, param_name, replacement)),
            }
        }
        Expr::TernaryOp { cond, true_expr, false_expr } => {
            Expr::TernaryOp {
                cond: Box::new(substitute_expr_types(*cond, param_name, replacement)),
                true_expr: Box::new(substitute_expr_types(*true_expr, param_name, replacement)),
                false_expr: Box::new(substitute_expr_types(*false_expr, param_name, replacement)),
            }
        }
        Expr::FuncCall { name, args } => {
            Expr::FuncCall {
                name,
                args: args.into_iter().map(|a| substitute_expr_types(a, param_name, replacement)).collect(),
            }
        }
        other => other,
    }
}
