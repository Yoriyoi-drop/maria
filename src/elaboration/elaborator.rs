use std::collections::{HashMap, HashSet};

use crate::ast::*;
use crate::ast::types::const_eval_simple;
use crate::ast::types::const_eval_with_params;
use crate::ir::*;
use super::util::*;
use crate::error::SimError;

const BUILTIN_UVM_CLASSES: &[&str] = &[
    "uvm_object", "uvm_component", "uvm_sequence_item", "uvm_sequence",
    "uvm_sequencer", "uvm_driver", "uvm_monitor", "uvm_scoreboard",
    "uvm_analysis_port", "uvm_analysis_imp", "uvm_test", "uvm_config_db", "uvm_report_object", "uvm_factory", "uvm_resource_db",];

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
                    PackageItem::Export { .. } => continue,
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
        // Third pass: resolve exports within packages (re-export items from other packages)
        let exports: Vec<(String, String, String)> = design.packages.iter()
            .flat_map(|pkg| {
                pkg.items.iter().filter_map(|item| {
                    if let PackageItem::Export { package, item: export_item } = item {
                        Some((pkg.name.clone(), package.clone(), export_item.clone()))
                    } else {
                        None
                    }
                })
            })
            .collect();
        for (pkg_name, source_pkg_name, export_item) in exports {
            let source_items = package_symbols.get(&source_pkg_name).cloned();
            if let Some(source_items) = source_items {
                if let Some(pkg_items) = package_symbols.get_mut(&pkg_name) {
                    let names: Vec<String> = if export_item == "*" {
                        source_items.keys().cloned().collect()
                    } else {
                        vec![export_item]
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

    pub fn elaborate(&mut self, top_module: Option<&str>) -> Result<IrDesign, SimError> {
        // Process bind declarations: add bound instances to target modules
        let binds = std::mem::take(&mut self.design.binds);
        for bind in &binds {
            if let Some(target_module) = self.design.modules.iter_mut().find(|m| m.name == bind.target) {
                target_module.items.push(ModuleItem::Instance(bind.instance.clone()));
            } else {
                eprintln!("warning: bind target '{}' not found", bind.target);
            }
        }

        // Pre-pass: import package functions/tasks into modules before inlining
        let pkg_symbols = &self.package_symbols;
        for module in &mut self.design.modules {
            let imports: Vec<(String, String)> = module.items.iter().filter_map(|item| {
                if let ModuleItem::Import { package, item: import_item } = item {
                    Some((package.clone(), import_item.clone()))
                } else {
                    None
                }
            }).collect();
            // Also include $unit-level imports
            let all_imports: Vec<(String, String)> = {
                let mut imps = imports;
                for (pkg, item) in &self.design.unit_imports {
                    if !imps.iter().any(|(p, i)| p == pkg && i == item) {
                        imps.push((pkg.clone(), item.clone()));
                    }
                }
                imps
            };
            for (package, import_item) in &all_imports {
                if let Some(pkg_items) = pkg_symbols.get(package) {
                    let names: Vec<&str> = if import_item == "*" {
                        pkg_items.keys().map(|s| s.as_str()).collect()
                    } else {
                        vec![import_item.as_str()]
                    };
                    for name in names {
                        if let Some(pkg_item) = pkg_items.get(name) {
                            match pkg_item {
                                PackageItem::Function(f) => {
                                    if !module.items.iter().any(|mi| matches!(mi, ModuleItem::Func(fd) if fd.name == f.name)) {
                                        module.items.push(ModuleItem::Func(f.clone()));
                                    }
                                }
                                PackageItem::Task(t) => {
                                    if !module.items.iter().any(|mi| matches!(mi, ModuleItem::Func(fd) if fd.name == t.name)) {
                                        module.items.push(ModuleItem::Func(FunctionDecl {
                                            name: t.name.clone(),
                                            range: None,
                                            return_type: None,
                                            ports: t.ports.clone(),
                                            decls: t.decls.clone(),
                                            stmts: t.stmts.clone(),
                                            virtual_flag: t.virtual_flag,
                                            is_static: t.is_static,
                                        }));
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }

        // Inject $unit function/task declarations into all modules
        for module in &mut self.design.modules {
            for func in &self.design.unit_funcs {
                if !module.items.iter().any(|mi| matches!(mi, ModuleItem::Func(fd) if fd.name == func.name)) {
                    module.items.push(ModuleItem::Func(func.clone()));
                }
            }
            for task in &self.design.unit_tasks {
                if !module.items.iter().any(|mi| matches!(mi, ModuleItem::Func(fd) if fd.name == task.name)) {
                    module.items.push(ModuleItem::Func(FunctionDecl {
                        name: task.name.clone(),
                        range: None,
                        return_type: None,
                        ports: task.ports.clone(),
                        decls: task.decls.clone(),
                        stmts: task.stmts.clone(),
                        virtual_flag: task.virtual_flag,
                        is_static: task.is_static,
                    }));
                }
            }
        }

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
                        

                        extra_packed_dims: vec![],is_dynamic: false,
                        is_queue: false,
                        is_associative: false,
                        assoc_key_type: None,
                        is_rand: false,
                        is_const: false,
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
                        is_associative: false,
                        is_signed: false,
                        is_const: false,
                        msb: if is_real { 63 } else { 0 },
                        lsb: 0,
                        struct_fields: vec![],
                        packed_dims: vec![], delay_rise: None, delay_fall: None, iface_type: None, iface_modport: None,
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
                        is_associative: false,
                        is_signed: is_signed_type(&decl.dtype),
                        is_const: false,
                        msb: if elem_width > 0 { elem_width - 1 } else { 0 },
                        lsb: 0,
                        struct_fields: vec![],
                        packed_dims: vec![], delay_rise: None, delay_fall: None, iface_type: None, iface_modport: None,
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
                    .ok_or_else(|| SimError::elaborate("no modules in design"))?
            }
        };

        let mut top = self.modules.remove(&top_name)
            .ok_or_else(|| SimError::elaborate(format!("top module '{}' not found", top_name)))?;

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

        let mut specify_items = Vec::new();
        for module in &self.design.modules {
            for item in &module.items {
                if let ModuleItem::Specify(sb) = item {
                    specify_items.extend(sb.items.clone());
                }
            }
        }

        // Collect recursive function declarations from module items for runtime evaluation
        let mut module_functions: HashMap<String, crate::ast::types::FunctionDecl> = HashMap::new();
        for module in &self.design.modules {
            for item in &module.items {
                if let ModuleItem::Func(f) = item {
                    module_functions.insert(f.name.clone(), f.clone());
                }
            }
        }

        Ok(IrDesign {
            top,
            modules: self.modules.clone(),
            classes,
            covergroups,
            dpi_imports,
            hier_signal_map,
            udp_defs: self.design.udp_defs.clone(),
            specify_items,
            timescale: self.design.timescale.clone(),
            module_functions,
        })
    }

    fn resolve_param_values(&self, module: &Module, instance_overrides: &HashMap<String, i64>) -> Result<HashMap<String, i64>, SimError> {
        resolve_param_values_fn(module, instance_overrides).map_err(|e| SimError::elaborate(e))
    }

    fn store_typedef_fields(&mut self, name: &str, dtype: &DataType) {
        let fields = Self::compute_struct_fields(dtype);
        if !fields.is_empty() {
            self.typedef_field_map.insert(name.to_string(), fields);
        }
    }

    fn resolve_type_width(&self, dtype: &DataType) -> Result<usize, SimError> {
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
                    .ok_or_else(|| SimError::elaborate(format!("unknown type '{}' is not defined in this scope", name)))
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

    fn elaborate_module(&mut self, module: &Module, known_modules: &[String]) -> Result<IrModule, SimError> {
        let param_vals = self.resolve_param_values(module, &HashMap::new())?;
        self.elaborate_module_with_params(module, known_modules, &param_vals)
    }

    fn elaborate_module_with_params(&mut self, module: &Module, known_modules: &[String],
                                    param_vals: &HashMap<String, i64>) -> Result<IrModule, SimError> {
        self.elaborate_module_with_params_and_type(module, known_modules, param_vals, &HashMap::new())
    }

    fn elaborate_module_with_params_and_type(&mut self, module: &Module, known_modules: &[String],
                                    param_vals: &HashMap<String, i64>,
                                    type_param_overrides: &HashMap<String, usize>) -> Result<IrModule, SimError> {
        let mut effective_params = param_vals.clone();

        // Process $unit parameters (top-level param declarations)
        for param in &self.design.unit_params {
            if !effective_params.contains_key(&param.name) {
                if let Some(expr) = &param.default {
                    if let Ok(val) = const_eval_with_params(expr, &effective_params) {
                        effective_params.insert(param.name.clone(), val);
                    }
                }
            }
        }

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
        // Pre-pass: process $unit typedefs (top-level typedefs outside any module)
        let unit_typedefs = self.design.unit_typedefs.clone();
        for td in &unit_typedefs {
            let width = self.resolve_typedef_width(&td.dtype, td.range.as_ref());
            self.typedef_map.insert(td.name.clone(), width);
            if matches!(&td.dtype, DataType::StructType { .. } | DataType::UnionType { .. }) {
                self.store_typedef_fields(&td.name, &td.dtype);
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
                        is_associative: false,
                    is_signed,
                    is_const: false,
                    msb,
                    lsb,
                    struct_fields: vec![],
                    packed_dims: vec![], delay_rise: None, delay_fall: None, iface_type: None, iface_modport: None,
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
                PortDirection::Ref => SignalKind::Inout,
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
                PortDirection::Ref => inouts.push(sid),
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
                        is_associative: false,
                        is_signed: false,
                        is_const: false,
                        msb: if is_real { 63 } else { 0 },
                        lsb: 0,
                        struct_fields: vec![],
                        packed_dims: vec![], delay_rise: None, delay_fall: None, iface_type: None, iface_modport: None,
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
                        is_associative: var.is_associative,
                        is_signed: is_signed_type(&decl.dtype),
                        is_const: false,
                    msb: 0,
                    lsb: 0,
                    struct_fields: vec![],
                    packed_dims: vec![], delay_rise: None, delay_fall: None, iface_type: None, iface_modport: None,
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
                        // Compute packed dimension widths for multi-dim packed arrays
                        if !var.extra_packed_dims.is_empty() {
                            let first_width = if let Some(er) = &var.expr_range {
                                resolve_expr_range(er, &effective_params).map(|r| r.width())
                            } else if let Some(r) = &var.range {
                                Ok(r.width())
                            } else {
                                Ok(1usize)
                            };
                            if let Ok(fw) = first_width {
                                let mut pd = vec![fw];
                                for (extra_er, _) in &var.extra_packed_dims {
                                    if let Ok(or) = resolve_expr_range(extra_er, &effective_params) {
                                        pd.push(or.width());
                                    }
                                }
                                sig.packed_dims = pd;
                            }
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
                        // Compute packed dimension widths for multi-dim packed arrays
                        if !var.extra_packed_dims.is_empty() {
                            if let Some(er) = &var.expr_range {
                                if let Ok(r) = resolve_expr_range(er, &effective_params) {
                                    let mut pd = vec![r.width()];
                                    for (extra_er, _) in &var.extra_packed_dims {
                                        if let Ok(or) = resolve_expr_range(extra_er, &effective_params) {
                                            pd.push(or.width());
                                        }
                                    }
                                    sig.packed_dims = pd;
                                }
                            }
                        }
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
                    // Check if this is a UDP instance
                    let udp_match = self.design.udp_defs.iter()
                        .find(|u| u.name == inst.module_name).cloned();

                    if let Some(udp) = udp_match {
                        // UDP instance: create combinational process with table lookup
                        let mut sig_ids = Vec::new();
                        for conn in &inst.port_conns {
                            let expr = match conn {
                                PortConnection::Positional(e) => e,
                                PortConnection::Named { expr, .. } => expr,
                            };
                            let sid = self.instance_port_expr_to_signal(
                                expr, &signal_map, &mut signals, &mut next_id,
                                &mut processes, &format!("{}.udp", inst.instance_name)
                            )?;
                            sig_ids.push(sid);
                        }
                        if sig_ids.len() < 2 {
                            return Err(SimError::elaborate(format!("UDP '{}' requires at least 2 ports (1 output + 1+ inputs)", udp.name)));
                        }
                        let out_id = sig_ids[0];
                        let in_ids: Vec<SignalId> = sig_ids[1..].to_vec();
                        let mut in_exprs: Vec<IrExpr> = in_ids.iter().map(|id| IrExpr::Signal(*id, 0)).collect();
                        // For sequential UDP, add output state as last arg (state feedback)
                        if udp.is_sequential {
                            in_exprs.push(IrExpr::Signal(out_id, 0));
                        }
                        let mut sensitivity = in_ids.clone();
                        if udp.is_sequential {
                            sensitivity.push(out_id);
                        }
                        let process = Process::Combinational {
                            name: format!("udp_{}_{}", udp.name, inst.instance_name),
                            sensitivity: sensitivity.clone(),
                            body: vec![IrStmt::BlockingAssign {
                                lhs: IrLValue::Signal(out_id, 0),
                                rhs: IrExpr::UdpLookup {
                                    udp_name: udp.name.clone(),
                                    args: in_exprs,
                                },
                                delay: None,
                            }],
                        };
                        processes.push(process);
                        // Handle initial output for sequential UDP
                        if let Some(ref init_sym) = udp.initial_output {
                            let init_val = match init_sym {
                                UdpSymbol::Zero => LogicVec::fill(LogicVal::Zero, 1),
                                UdpSymbol::One => LogicVec::fill(LogicVal::One, 1),
                                _ => LogicVec::fill(LogicVal::X, 1),
                            };
                            processes.push(Process::Initial {
                                name: format!("udp_init_{}_{}", udp.name, inst.instance_name),
                                body: vec![IrStmt::BlockingAssign {
                                    lhs: IrLValue::Signal(out_id, 0),
                                    rhs: IrExpr::Const(init_val),
                                    delay: None,
                                }],
                            });
                        }
                    } else {
                        // Regular module instance
                        let mut port_map = HashMap::new();
                        // Look up target module to get port order for positional connections
                        let target_module: Option<&Module> = self.design.modules.iter()
                            .find(|m| m.name == inst.module_name);
                        for (i, conn) in inst.port_conns.iter().enumerate() {
                            match conn {
                                PortConnection::Positional(expr) => {
                                    if let Some(tm) = target_module {
                                        if let Some(port) = tm.ports.get(i) {
                                            let sig_id = self.instance_port_expr_to_signal(
                                                expr, &signal_map, &mut signals, &mut next_id,
                                                &mut processes, &format!("{}.{}", inst.instance_name, port.name)
                                            )?;
                                            port_map.insert(port.name.clone(), sig_id);
                                        }
                                    }
                                }
                                PortConnection::Named { port, expr } => {
                                    let sig_id = self.instance_port_expr_to_signal(
                                        expr, &signal_map, &mut signals, &mut next_id,
                                        &mut processes, &format!("{}.{}", inst.instance_name, port)
                                    )?;
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
                }
                ModuleItem::Gate(gate) => {
                    let mut sig_ids = Vec::new();
                    for port in &gate.ports {
                        let sid = match port {
                            Expr::Ident(name) => signal_map.get(name).copied()
                                .ok_or_else(|| SimError::elaborate(format!("signal '{}' not found for gate", name)))?,
                            _ => return Err(SimError::elaborate(format!("gate port must be a simple signal (port expression: {:?})", port))),
                        };
                        sig_ids.push(sid);
                    }
                    if sig_ids.len() < 2 {
                        return Err(SimError::elaborate(format!("gate requires at least 2 ports (gate type: {:?}, got {} ports)", gate.gate_type, sig_ids.len())));
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

    fn elaborate_classes(&self) -> Result<HashMap<String, IrClassDef>, SimError> {
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
                        is_task: false,
                        virtual_flag: fd.virtual_flag,
                        is_static: fd.is_static,
                        ports: fd.ports.clone(),
                        decls: fd.decls.clone(),
                        stmts: fd.stmts.clone(),
                    }),
                    ClassMember::Task(td) => Some(IrClassMethod {
                        name: td.name.clone(),
                        is_task: true,
                        virtual_flag: td.virtual_flag,
                        is_static: td.is_static,
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
                              signals: &[SignalInfo]) -> Result<Vec<IrCovergroup>, SimError> {
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

    fn elaborate_dpi_imports(&self) -> Result<Vec<IrDpiImport>, SimError> {
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

    fn detect_multi_driver_signals(&self, top: &mut IrModule) -> Result<(), SimError> {
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

    fn flatten_instances(&mut self, top: &mut IrModule) -> Result<HashMap<String, SignalId>, SimError> {
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
                return Err(SimError::elaborate(format!("module or interface '{}' not found for instance '{}'",
                    inst.module_name, inst.instance_name)));
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
                    .ok_or_else(|| SimError::elaborate(format!("module '{}' not found", inst.module_name)))?
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
                        return Err(SimError::elaborate(format!(
                            "port width mismatch on instance '{}': port '{}' expects width {}, connected signal '{}' has width {}",
                            inst.instance_name, port_name, child_width,
                            top.signals[parent_sig].name, parent_width
                        )));
                    }
                    // Port type checking: inout must connect to tri
                    if child.signals[child_sig].kind == SignalKind::Inout
                        && top.signals[parent_sig].net_type != NetType::Tri
                    {
                        return Err(SimError::elaborate(format!(
                            "port type mismatch on instance '{}': inout port '{}' must connect to a tri signal, but '{}' has net type {:?}",
                            inst.instance_name, port_name,
                            top.signals[parent_sig].name,
                            top.signals[parent_sig].net_type
                        )));
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
                        is_associative: sig.is_associative,
                        is_signed: sig.is_signed,
                        is_const: sig.is_const,
                    msb: sig.msb,
                        lsb: sig.lsb,
                    struct_fields: sig.struct_fields.clone(),
                    packed_dims: sig.packed_dims.clone(),
                    delay_rise: sig.delay_rise,
                    delay_fall: sig.delay_fall,
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

    fn translate_process(&self, process: &Process, map_sig: &dyn Fn(SignalId) -> SignalId) -> Result<Process, SimError> {
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

    fn translate_stmts(&self, stmts: &[IrStmt], map_sig: &dyn Fn(SignalId) -> SignalId) -> Result<Vec<IrStmt>, SimError> {
        stmts.iter().map(|s| self.translate_stmt(s, map_sig)).collect()
    }

    fn translate_stmt(&self, stmt: &IrStmt, map_sig: &dyn Fn(SignalId) -> SignalId) -> Result<IrStmt, SimError> {
        match stmt {
            IrStmt::Block { stmts } => {
                let new = self.translate_stmts(stmts, map_sig)?;
                Ok(IrStmt::Block { stmts: new })
            }
            IrStmt::NamedBlock { name, stmts, decls } => {
                let new = self.translate_stmts(stmts, map_sig)?;
                Ok(IrStmt::NamedBlock { name: name.clone(), stmts: new, decls: decls.clone() })
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
                }).collect::<Result<Vec<_>, SimError>>()?;
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
            IrStmt::MethodCallStmt { obj, method, args, with_clause } => {
                Ok(IrStmt::MethodCallStmt {
                    obj: self.translate_expr(obj, map_sig),
                    method: method.clone(),
                    args: args.iter().map(|a| self.translate_expr(a, map_sig)).collect(),
                    with_clause: with_clause.as_ref().map(|wc| Box::new(self.translate_expr(wc, map_sig))),
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
                let new_proc = processes.iter().map(|p| self.translate_stmts(p, map_sig)).collect::<Result<Vec<_>, SimError>>()?;
                Ok(IrStmt::Fork { processes: new_proc, join_type: join_type.clone() })
            }
            IrStmt::Assert { cond, pass_stmt, fail_stmt, clock_event, disable_iff } => {
                let new_cond = self.translate_expr(cond, map_sig);
                let new_pass = self.translate_stmts(pass_stmt, map_sig)?;
                let new_fail = self.translate_stmts(fail_stmt, map_sig)?;
                let new_disable = disable_iff.as_ref().map(|e| Box::new(self.translate_expr(e, map_sig)));
                Ok(IrStmt::Assert { cond: new_cond, pass_stmt: new_pass, fail_stmt: new_fail, clock_event: clock_event.clone(), disable_iff: new_disable })
            }
            IrStmt::Assume { cond, pass_stmt, fail_stmt, clock_event, disable_iff } => {
                let new_cond = self.translate_expr(cond, map_sig);
                let new_pass = self.translate_stmts(pass_stmt, map_sig)?;
                let new_fail = self.translate_stmts(fail_stmt, map_sig)?;
                let new_disable = disable_iff.as_ref().map(|e| Box::new(self.translate_expr(e, map_sig)));
                Ok(IrStmt::Assume { cond: new_cond, pass_stmt: new_pass, fail_stmt: new_fail, clock_event: clock_event.clone(), disable_iff: new_disable })
            }
            IrStmt::Cover { cond, pass_stmt, clock_event, disable_iff } => {
                let new_cond = self.translate_expr(cond, map_sig);
                let new_pass = self.translate_stmts(pass_stmt, map_sig)?;
                let new_disable = disable_iff.as_ref().map(|e| Box::new(self.translate_expr(e, map_sig)));
                Ok(IrStmt::Cover { cond: new_cond, pass_stmt: new_pass, clock_event: clock_event.clone(), disable_iff: new_disable })
            }
            IrStmt::WaitOrder { events, failure_stmts } => {
                let new_events = events.iter().map(|id| map_sig(*id)).collect();
                let new_failure = self.translate_stmts(failure_stmts, map_sig)?;
                Ok(IrStmt::WaitOrder { events: new_events, failure_stmts: new_failure })
            }
            IrStmt::RandCase { items } => {
                let new_items: Result<Vec<(IrExpr, Vec<IrStmt>)>, SimError> = items.iter().map(|(weight_expr, body)| {
                    let new_weight = self.translate_expr(weight_expr, map_sig);
                    let new_body = self.translate_stmts(body, map_sig)?;
                    Ok((new_weight, new_body))
                }).collect();
                Ok(IrStmt::RandCase { items: new_items? })
            }
            IrStmt::RandSequence { productions } => {
                let new_prods: Result<Vec<(String, Vec<(IrExpr, Vec<IrStmt>)>)>, SimError> = productions.iter().map(|(name, items)| {
                    let new_items: Vec<(IrExpr, Vec<IrStmt>)> = items.iter().map(|(weight_expr, body)| {
                        let new_weight = self.translate_expr(weight_expr, map_sig);
                        let new_body = self.translate_stmts(body, map_sig).unwrap_or_default();
                        (new_weight, new_body)
                    }).collect();
                    Ok((name.clone(), new_items))
                }).collect();
                Ok(IrStmt::RandSequence { productions: new_prods? })
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
            IrExpr::MethodCall { obj, method, args, with_clause } => IrExpr::MethodCall {
                obj: Box::new(self.translate_expr(obj, map_sig)),
                method: method.clone(),
                args: args.iter().map(|a| self.translate_expr(a, map_sig)).collect(),
                with_clause: with_clause.as_ref().map(|wc| Box::new(self.translate_expr(wc, map_sig))),
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
            IrExpr::StreamingConcat { op, slice_size, slices } => IrExpr::StreamingConcat {
                op: op.clone(),
                slice_size: *slice_size,
                slices: slices.iter().map(|e| self.translate_expr(e, map_sig)).collect(),
            },
            IrExpr::Dist { expr, items } => IrExpr::Dist {
                expr: Box::new(self.translate_expr(expr, map_sig)),
                items: items.clone(),
            },
            IrExpr::UdpLookup { udp_name, args } => IrExpr::UdpLookup {
                udp_name: udp_name.clone(),
                args: args.iter().map(|a| self.translate_expr(a, map_sig)).collect(),
            },
            IrExpr::FuncCall { func_name, args } => IrExpr::FuncCall {
                func_name: func_name.clone(),
                args: args.iter().map(|a| self.translate_expr(a, map_sig)).collect(),
            },
        }
    }

    fn elaborate_always(&self, always: &AlwaysBlock, signal_map: &HashMap<String, SignalId>,
                         signals: &[SignalInfo])
        -> Result<Process, SimError>
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
        -> Result<(ClockEdge, Option<ResetInfo>), SimError>
    {
        let events = match sensitivity {
            Some(sl) => &sl.events,
            None => return Err(SimError::elaborate("always_ff requires sensitivity list")),
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

        clock_edge.ok_or_else(|| SimError::elaborate("always_ff must have at least one clock edge"))
            .map(|ce| (ce, reset))
    }

    fn elaborate_stmt_block(&self, stmts: &[Stmt],
                            signal_map: &HashMap<String, SignalId>,
                            _known_modules: &[String],
                            signals: &[SignalInfo])
        -> Result<Vec<IrStmt>, SimError>
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
        -> Result<IrStmt, SimError>
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
                // Try constant-fold the case expression
                if let Ok(case_val) = const_eval_with_params(expr, &self.param_vals) {
                    // Case expression is compile-time constant — find matching branch
                    let mut matched_body: Option<&Stmt> = None;
                    for item in items {
                        for label in &item.labels {
                            let label_val = const_eval_with_params(label, &self.param_vals);
                            if let Ok(lv) = label_val {
                                if lv == case_val {
                                    matched_body = Some(&item.stmt);
                                    break;
                                }
                            } else if let Expr::Value(v) = label {
                                let lv = match v {
                                    Value::Decimal(d) => *d,
                                    Value::Hex { bits, .. } => i64::from_str_radix(bits.trim_start_matches("0x").trim_start_matches("0X"), 16).unwrap_or(0),
                                    Value::Binary { bits, .. } => i64::from_str_radix(bits.trim_start_matches("0b").trim_start_matches("0B"), 2).unwrap_or(0),
                                    Value::Octal { bits, .. } => i64::from_str_radix(bits.trim_start_matches("0o").trim_start_matches("0O"), 8).unwrap_or(0),
                                    Value::Real(_) => 0,
                                };
                                if lv == case_val {
                                    matched_body = Some(&item.stmt);
                                    break;
                                }
                            }
                        }
                        if matched_body.is_some() { break; }
                    }
                    match matched_body {
                        Some(body) => self.elaborate_stmt(body, signal_map, known_modules, signals),
                        None => {
                            if let Some(def) = default {
                                self.elaborate_stmt(def, signal_map, known_modules, signals)
                            } else {
                                Ok(IrStmt::Block { stmts: vec![] })
                            }
                        }
                    }
                } else {
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
            }
            Stmt::StmtAssign { lhs, rhs } => {
                let ir_lhs = self.elaborate_lvalue(lhs, signal_map, signals)?;
                let ir_rhs = self.elaborate_expr(rhs, signal_map, signals)?;
                Ok(IrStmt::BlockingAssign { lhs: ir_lhs, rhs: ir_rhs, delay: None })
            }
            Stmt::Expr { expr } => {
                match expr {
                    Expr::MethodCall { obj, method, args, with_clause } => {
                        let ir_obj = self.elaborate_expr(obj, signal_map, signals)?;
                        let ir_args: Vec<IrExpr> = args.iter()
                            .map(|a| self.elaborate_expr(a, signal_map, signals))
                            .collect::<Result<_, _>>()?;
                        let ir_with = match with_clause {
                            Some(wc) => Some(Box::new(self.elaborate_expr(wc, signal_map, signals)?)),
                            None => None,
                        };
                        Ok(IrStmt::MethodCallStmt {
                            obj: ir_obj,
                            method: method.clone(),
                            args: ir_args,
                            with_clause: ir_with,
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
                            Err(SimError::elaborate(format!("cannot resolve signal in @(...)")))
                        }
                    }
                    SensitivityEvent::Level(expr) => {
                        if let Some(sig_id) = resolve_expr_signal(expr, signal_map) {
                            Ok(IrStmt::EventControl { sig_id, edge: None, body })
                        } else {
                            Err(SimError::elaborate(format!("cannot resolve signal in @(...)")))
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
            Stmt::NamedBlock { name, stmts, decls } => {
                let body = self.elaborate_stmt_block(stmts, signal_map, known_modules, signals)?;
                Ok(IrStmt::NamedBlock { name: name.clone(), stmts: body, decls: decls.clone() })
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
                            .map_err(|e| e.to_string())
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
                    .ok_or_else(|| SimError::elaborate(format!("array '{}' not found for foreach", array_var)))?;
                let sig_info = signals.get(*sig_id)
                    .ok_or_else(|| SimError::elaborate(format!("signal info not found for '{}'", array_var)))?;
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
                        return Err(SimError::elaborate(format!("'{}' is not an array, cannot use foreach", array_var)));
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
            Stmt::Assert { cond, pass_stmt, fail_stmt, clock_event, disable_iff } => {
                let ir_cond = self.elaborate_expr(cond, signal_map, signals)?;
                let pass = match pass_stmt {
                    Some(s) => vec![self.elaborate_stmt(s, signal_map, known_modules, signals)?],
                    None => vec![],
                };
                let fail = match fail_stmt {
                    Some(s) => vec![self.elaborate_stmt(s, signal_map, known_modules, signals)?],
                    None => vec![],
                };
                let ir_disable = match disable_iff {
                    Some(e) => Some(Box::new(self.elaborate_expr(&*e, signal_map, signals)?)),
                    None => None,
                };
                Ok(IrStmt::Assert { cond: ir_cond, pass_stmt: pass, fail_stmt: fail, clock_event: clock_event.clone(), disable_iff: ir_disable })
            }
            Stmt::Assume { cond, pass_stmt, fail_stmt, clock_event, disable_iff } => {
                let ir_cond = self.elaborate_expr(cond, signal_map, signals)?;
                let pass = match pass_stmt {
                    Some(s) => vec![self.elaborate_stmt(s, signal_map, known_modules, signals)?],
                    None => vec![],
                };
                let fail = match fail_stmt {
                    Some(s) => vec![self.elaborate_stmt(s, signal_map, known_modules, signals)?],
                    None => vec![],
                };
                let ir_disable = match disable_iff {
                    Some(e) => Some(Box::new(self.elaborate_expr(&*e, signal_map, signals)?)),
                    None => None,
                };
                Ok(IrStmt::Assume { cond: ir_cond, pass_stmt: pass, fail_stmt: fail, clock_event: clock_event.clone(), disable_iff: ir_disable })
            }
            Stmt::Cover { cond, pass_stmt, clock_event, disable_iff } => {
                let ir_cond = self.elaborate_expr(cond, signal_map, signals)?;
                let pass = match pass_stmt {
                    Some(s) => vec![self.elaborate_stmt(s, signal_map, known_modules, signals)?],
                    None => vec![],
                };
                let ir_disable = match disable_iff {
                    Some(e) => Some(Box::new(self.elaborate_expr(&*e, signal_map, signals)?)),
                    None => None,
                };
                Ok(IrStmt::Cover { cond: ir_cond, pass_stmt: pass, clock_event: clock_event.clone(), disable_iff: ir_disable })
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
                        return Err(SimError::elaborate(format!("wait_order: signal '{}' not found", name)));
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
            Stmt::RandCase { items } => {
                let new_items: Result<Vec<(IrExpr, Vec<IrStmt>)>, SimError> = items.iter().map(|rc| {
                    let weight_expr = IrExpr::Const(LogicVec::from_u64(rc.weight as u64, 32));
                    let body = self.elaborate_stmt_block(&[*rc.stmt.clone()], signal_map, known_modules, signals)?;
                    Ok((weight_expr, body))
                }).collect();
                Ok(IrStmt::RandCase { items: new_items? })
            }
            Stmt::RandSequence { productions } => {
                let mut ir_productions = Vec::new();
                for prod in productions {
                    let mut ir_items = Vec::new();
                    for item in &prod.items {
                        let weight_expr = if let Some(w) = item.weight {
                            IrExpr::Const(LogicVec::from_u64(w, 32))
                        } else {
                            IrExpr::Const(LogicVec::from_u64(1, 32))
                        };
                        let body = self.elaborate_stmt_block(&[(*item.value).clone()], signal_map, known_modules, signals)?;
                        ir_items.push((weight_expr, body));
                    }
                    ir_productions.push((prod.name.clone(), ir_items));
                }
                Ok(IrStmt::RandSequence { productions: ir_productions })
            }
        }
    }

    fn elaborate_lvalue(&self, expr: &Expr, signal_map: &HashMap<String, SignalId>, signals: &[SignalInfo]) -> Result<IrLValue, SimError> {
        match expr {
            Expr::Ident(name) => {
                let sig_id = signal_map.get(name)
                    .ok_or_else(|| SimError::elaborate(format!("signal '{}' not found", name)))?;
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
                    _ => Err(SimError::elaborate("nested range select not supported")),
                }
            }
            Expr::BitSelect { expr: inner, index: bs_index } => {
                let inner_lv = self.elaborate_lvalue(inner, signal_map, signals)?;
                match inner_lv {
                    IrLValue::Signal(sid, _) => {
                        let sig = &signals[sid];
                        // Check for multi-dim packed array: packed_dims.len() > 1
                        if sig.packed_dims.len() > 1 {
                            let outer_elem_width = sig.width / sig.packed_dims[0];
                            if let Ok(idx) = const_eval_params(bs_index, &self.param_vals) {
                                let idx = idx as usize;
                                let lsb = idx * outer_elem_width;
                                let msb = lsb + outer_elem_width - 1;
                                Ok(IrLValue::RangeSelect(sid, msb, lsb))
                            } else {
                                let index_expr = self.elaborate_expr(bs_index, signal_map, signals)?;
                                Ok(IrLValue::ArrayIndex { sig_id: sid, index: Box::new(index_expr), elem_width: outer_elem_width })
                            }
                        } else if sig.array_depth > 1 || sig.is_dynamic || sig.is_queue {
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
                            Err(SimError::elaborate("dynamic bit-select on array element not supported"))
                        }
                    }
                    _ => Err(SimError::elaborate("nested bit select not supported")),
                }
            }
            Expr::PartSelect { expr: inner, base, width } => {
                let inner_lv = self.elaborate_lvalue(inner, signal_map, signals)?;
                let base_r = const_eval_params(base, &self.param_vals);
                let width_r = const_eval_params(width, &self.param_vals);
                let (base_c, width_c) = match (base_r, width_r) {
                    (Ok(b), Ok(w)) => (b as usize, w as usize),
                    _ => return Err(SimError::elaborate("dynamic part-select not supported")),
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
                    _ => Err(SimError::elaborate("nested part-select in lvalue not supported")),
                }
            }
            Expr::Concat(exprs) => {
                let parts: Result<Vec<IrLValue>, SimError> = exprs.iter()
                    .map(|e| self.elaborate_lvalue(e, signal_map, signals))
                    .collect();
                Ok(IrLValue::Concat(parts?))
            }
            Expr::MethodCall { .. } => Err(SimError::elaborate("method calls cannot be used as lvalues")),
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
                            return Err(SimError::elaborate(format!("field '{}' not found in struct type", field)));
                        }
                        Err(SimError::elaborate(format!("member access on signal '{:?}' that has no struct fields (cannot use as lvalue)", obj)))
                    }
                    _ => Err(SimError::elaborate("member access cannot be used as lvalues")),
                }
            }
            _ => Err(SimError::elaborate(format!("invalid lvalue expression: {:?}", expr))),
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

    fn elaborate_expr(&self, expr: &Expr, signal_map: &HashMap<String, SignalId>, signals: &[SignalInfo]) -> Result<IrExpr, SimError> {
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
                    .ok_or_else(|| SimError::elaborate(format!("signal '{}' not found", name)))?;
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
                                return Err(SimError::elaborate(format!("package param '{}.{}' has no default", package, item)));
                            }
                            _ => return Err(SimError::elaborate(format!("'{}' is not a constant in package '{}'", item, package))),
                        }
                    }
                }
                return Err(SimError::elaborate(format!("'{}' not found in package '{}'", item, package)));
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
                    Err(SimError::elaborate("dynamic range select not supported"))
                }
            }
            Expr::BitSelect { expr: inner, index } => {
                let inner_expr = self.elaborate_expr(inner, signal_map, signals)?;
                if let IrExpr::Signal(sid, _) = &inner_expr {
                    let sig = &signals[*sid];
                    // Check for multi-dim packed array: packed_dims.len() > 1
                    if sig.packed_dims.len() > 1 {
                        let outer_elem_width = sig.width / sig.packed_dims[0];
                        if let Ok(idx) = const_eval_params(index, &self.param_vals) {
                            let idx = idx as usize;
                            let lsb = idx * outer_elem_width;
                            let msb = lsb + outer_elem_width - 1;
                            Ok(IrExpr::RangeSelect(*sid, msb, lsb))
                        } else {
                            let index_expr = self.elaborate_expr(index, signal_map, signals)?;
                            let base_expr = IrExpr::BinaryOp(BinaryIrOp::Mul,
                                Box::new(index_expr),
                                Box::new(IrExpr::Const(LogicVec::from_u64(outer_elem_width as u64, 32))));
                            Ok(IrExpr::ExprPartSelect(
                                Box::new(IrExpr::Signal(*sid, sig.width)),
                                Box::new(base_expr),
                                Box::new(IrExpr::Const(LogicVec::from_u64(outer_elem_width as u64, 32)))))
                        }
                    } else if sig.array_depth > 1 || sig.is_dynamic || sig.is_queue {
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
                    Err(SimError::elaborate("dynamic bit-select on non-signal not supported"))
                }
            }
            Expr::Concat(exprs) => {
                if let Some(folded) = try_fold_const(expr, &self.param_vals)? {
                    return Ok(folded);
                }
                let parts: Result<Vec<IrExpr>, SimError> = exprs.iter()
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
                            return Err(SimError::elaborate("$signed requires exactly one argument"));
                        }
                        let inner = self.elaborate_expr(&args[0], signal_map, signals)?;
                        Ok(IrExpr::Signed(Box::new(inner)))
                    }
                    "$unsigned" => {
                        if args.len() != 1 {
                            return Err(SimError::elaborate("$unsigned requires exactly one argument"));
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
                            Err(SimError::elaborate("$clog2 requires one argument"))
                        }
                    }
                    "$bits" => {
                        if let Some(arg) = args.first() {
                            let width = resolve_expr_signal(arg, signal_map)
                                .map(|sig_id| {
                                    let info = &signals[sig_id];
                                    info.width * if info.array_depth > 0 { info.array_depth } else { 1 }
                                })
                                .or_else(|| compute_expr_width(arg, signal_map, signals, &self.param_vals, &self.package_symbols).ok())
                                .ok_or_else(|| SimError::elaborate("$bits argument must resolve to a signal or computable expression"))?;
                            Ok(IrExpr::Const(LogicVec::from_u64(width as u64, 32)))
                        } else {
                            Err(SimError::elaborate("$bits requires one argument"))
                        }
                    }
                    "$high" => {
                        if let Some(arg) = args.first() {
                            let sig_id = resolve_expr_signal(arg, signal_map)
                                .ok_or_else(|| SimError::elaborate("$high argument must resolve to a signal"))?;
                            let info = &signals[sig_id];
                            let high = info.msb.max(info.lsb);
                            Ok(IrExpr::Const(LogicVec::from_u64(high as u64, 32)))
                        } else {
                            Err(SimError::elaborate("$high requires one argument"))
                        }
                    }
                    "$low" => {
                        if let Some(arg) = args.first() {
                            let sig_id = resolve_expr_signal(arg, signal_map)
                                .ok_or_else(|| SimError::elaborate("$low argument must resolve to a signal"))?;
                            let info = &signals[sig_id];
                            let low = info.msb.min(info.lsb);
                            Ok(IrExpr::Const(LogicVec::from_u64(low as u64, 32)))
                        } else {
                            Err(SimError::elaborate("$low requires one argument"))
                        }
                    }
                    "$left" => {
                        if let Some(arg) = args.first() {
                            let sig_id = resolve_expr_signal(arg, signal_map)
                                .ok_or_else(|| SimError::elaborate("$left argument must resolve to a signal"))?;
                            let info = &signals[sig_id];
                            Ok(IrExpr::Const(LogicVec::from_u64(info.msb as u64, 32)))
                        } else {
                            Err(SimError::elaborate("$left requires one argument"))
                        }
                    }
                    "$right" => {
                        if let Some(arg) = args.first() {
                            let sig_id = resolve_expr_signal(arg, signal_map)
                                .ok_or_else(|| SimError::elaborate("$right argument must resolve to a signal"))?;
                            let info = &signals[sig_id];
                            Ok(IrExpr::Const(LogicVec::from_u64(info.lsb as u64, 32)))
                        } else {
                            Err(SimError::elaborate("$right requires one argument"))
                        }
                    }
                    "$size" => {
                        if let Some(arg) = args.first() {
                            let sig_id = resolve_expr_signal(arg, signal_map)
                                .ok_or_else(|| SimError::elaborate("$size argument must resolve to a signal"))?;
                            let info = &signals[sig_id];
                            Ok(IrExpr::Const(LogicVec::from_u64(info.width as u64, 32)))
                        } else {
                            Err(SimError::elaborate("$size requires one argument"))
                        }
                    }
                    "$countones" => {
                        if let Some(arg) = args.first() {
                            let ir_arg = self.elaborate_expr(arg, signal_map, signals)?;
                            if let Ok(val) = const_eval_params(arg, &self.param_vals) {
                                let count = (0..64).filter(|i| (val >> i) & 1 == 1).count() as u64;
                                Ok(IrExpr::Const(LogicVec::from_u64(count, 32)))
                            } else {
                                Ok(IrExpr::SysFunc { name: "$countones".to_string(), args: vec![ir_arg] })
                            }
                        } else {
                            Err(SimError::elaborate("$countones requires one argument"))
                        }
                    }
                    "$onehot" => {
                        if let Some(arg) = args.first() {
                            let ir_arg = self.elaborate_expr(arg, signal_map, signals)?;
                            if let Ok(val) = const_eval_params(arg, &self.param_vals) {
                                let ones = (0..64).filter(|i| (val >> i) & 1 == 1).count();
                                Ok(IrExpr::Const(LogicVec::from_u64(if ones == 1 { 1 } else { 0 }, 1)))
                            } else {
                                Ok(IrExpr::SysFunc { name: "$onehot".to_string(), args: vec![ir_arg] })
                            }
                        } else {
                            Err(SimError::elaborate("$onehot requires one argument"))
                        }
                    }
                    "$isunknown" => {
                        if let Some(arg) = args.first() {
                            let ir_arg = self.elaborate_expr(arg, signal_map, signals)?;
                            if let Ok(val) = const_eval_params(arg, &self.param_vals) {
                                let has_xz = val as u8 >= 0xFE;
                                Ok(IrExpr::Const(LogicVec::from_u64(if has_xz { 1 } else { 0 }, 1)))
                            } else {
                                Ok(IrExpr::SysFunc { name: "$isunknown".to_string(), args: vec![ir_arg] })
                            }
                        } else {
                            Err(SimError::elaborate("$isunknown requires one argument"))
                        }
                    }
                    _ => {
                        let ir_args: Result<Vec<IrExpr>, SimError> = args.iter()
                            .map(|a| self.elaborate_expr(a, signal_map, signals))
                            .collect();
                        Ok(IrExpr::SysFunc { name: name.to_string(), args: ir_args? })
                    }
                }
            }
            Expr::FuncCall { name, args } if name == "new" => {
                let ir_args: Result<Vec<IrExpr>, SimError> = args.iter()
                    .map(|a| self.elaborate_expr(a, signal_map, signals))
                    .collect();
                Ok(IrExpr::NewCall { class_name: String::new(), args: ir_args? })
            }
            Expr::String(s) => Ok(IrExpr::String(s.clone())),
            Expr::MethodCall { obj, method, args, with_clause } => {
                let ir_obj = self.elaborate_expr(obj, signal_map, signals)?;
                let ir_args: Result<Vec<IrExpr>, SimError> = args.iter()
                    .map(|a| self.elaborate_expr(a, signal_map, signals))
                    .collect();
                let ir_with = match with_clause {
                    Some(wc) => Some(Box::new(self.elaborate_expr(wc, signal_map, signals)?)),
                    None => None,
                };
                Ok(IrExpr::MethodCall {
                    obj: Box::new(ir_obj),
                    method: method.clone(),
                    args: ir_args?,
                    with_clause: ir_with,
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
                        return Err(SimError::elaborate(format!("field '{}' not found in struct type (width {})", field, sig_info.width)));
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
            Expr::StreamingConcat { op, slice_size, slices } => {
                let mut ir_slices = Vec::new();
                for sl in slices {
                    ir_slices.push(self.elaborate_expr(sl, signal_map, signals)?);
                }
                let ir_slice_size = if let Some(ss) = slice_size {
                    match const_eval_params(ss, &self.param_vals) {
                        Ok(v) if v > 0 => Some(v as usize),
                        Ok(_) => return Err(SimError::elaborate("streaming slice_size must be > 0")),
                        Err(_) => return Err(SimError::elaborate("slice_size must be a constant expression")),
                    }
                } else {
                    None
                };
                Ok(IrExpr::StreamingConcat {
                    op: op.clone(),
                    slice_size: ir_slice_size,
                    slices: ir_slices,
                })
            }
            Expr::Dist {
                expr: inner,
                items,
            } => {
                let inner_ir = self.elaborate_expr(inner, signal_map, signals)?;
                let ir_items = items.iter().map(|di| {
                    match di {
                        crate::ast::DistItem::Value(e, crate::ast::DistWeight::Item(w)) => {
                            let ev = self.elaborate_expr(e, signal_map, signals).unwrap_or(IrExpr::Const(LogicVec::from_u64(0, 32)));
                            let lo = if let IrExpr::Const(ref lv) = ev { Some(lv.to_u64() as i64) } else { None };
                            crate::ir::IrDistItem { range_lo: lo, range_hi: lo, weight_type: crate::ir::DistWeightType::Item, weight: *w as i64 }
                        }
                        crate::ast::DistItem::Value(e, crate::ast::DistWeight::Range(w)) => {
                            let ev = self.elaborate_expr(e, signal_map, signals).unwrap_or(IrExpr::Const(LogicVec::from_u64(0, 32)));
                            let lo = if let IrExpr::Const(ref lv) = ev { Some(lv.to_u64() as i64) } else { None };
                            crate::ir::IrDistItem { range_lo: lo, range_hi: lo, weight_type: crate::ir::DistWeightType::Range, weight: *w as i64 }
                        }
                        crate::ast::DistItem::Range(lo, hi, crate::ast::DistWeight::Item(w)) => {
                            let lo_v = const_eval_with_params(lo, &self.param_vals).ok();
                            let hi_v = const_eval_with_params(hi, &self.param_vals).ok();
                            crate::ir::IrDistItem { range_lo: lo_v, range_hi: hi_v, weight_type: crate::ir::DistWeightType::Item, weight: *w as i64 }
                        }
                        crate::ast::DistItem::Range(lo, hi, crate::ast::DistWeight::Range(w)) => {
                            let lo_v = const_eval_with_params(lo, &self.param_vals).ok();
                            let hi_v = const_eval_with_params(hi, &self.param_vals).ok();
                            crate::ir::IrDistItem { range_lo: lo_v, range_hi: hi_v, weight_type: crate::ir::DistWeightType::Range, weight: *w as i64 }
                        }
                    }
                }).collect::<Vec<_>>();
                Ok(IrExpr::Dist {
                    expr: Box::new(inner_ir),
                    items: ir_items,
                })
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
                let ir_args: Result<Vec<IrExpr>, SimError> = args.iter()
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
                let ir_args: Result<Vec<IrExpr>, SimError> = args.iter()
                    .map(|a| self.elaborate_expr(a, signal_map, signals))
                    .collect();
                Ok(IrExpr::NewCall { class_name, args: ir_args? })
            }
            Expr::FuncCall { name, args } if name == "uvm_factory::set_type_override_by_type" => {
                let ir_args: Result<Vec<IrExpr>, SimError> = args.iter()
                    .map(|a| self.elaborate_expr(a, signal_map, signals))
                    .collect();
                Ok(IrExpr::SysFunc { name: name.clone(), args: ir_args? })
            }
            Expr::FuncCall { name, args } if name == "uvm_config_db::set" || name == "uvm_config_db::get"
                || name == "uvm_resource_db::set" || name == "uvm_resource_db::get" => {
                let ir_args: Result<Vec<IrExpr>, SimError> = args.iter()
                    .map(|a| self.elaborate_expr(a, signal_map, signals))
                    .collect();
                // Use SysFunc variant for engine dispatch
                Ok(IrExpr::SysFunc { name: name.clone(), args: ir_args? })
            }
            Expr::FuncCall { name, args } if name != "new" && name.contains("::") => {
                self.elaborate_package_func_call(name, args, signal_map, signals)
            }
            Expr::FuncCall { name, args } if name != "new" => {
                let is_dpi = self.design.modules.iter().flat_map(|m| m.items.iter())
                    .any(|item| matches!(item, ModuleItem::DpiImport(d) if d.name == *name));
                if is_dpi {
                    let ir_args: Result<Vec<IrExpr>, SimError> = args.iter()
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
                    // Check if this is a module-level function (recursive, not inlined)
                    let func_exists = self.design.modules.iter().any(|m|
                        m.items.iter().any(|mi| matches!(mi, ModuleItem::Func(fd) if fd.name == *name))
                    );
                    if func_exists {
                        let ir_args: Result<Vec<IrExpr>, SimError> = args.iter()
                            .map(|a| self.elaborate_expr(a, signal_map, signals))
                            .collect();
                        return Ok(IrExpr::FuncCall { func_name: name.clone(), args: ir_args? });
                    }
                    Err(SimError::elaborate(format!("function '{}' not found (not a DPI import)", name)))
                }
            }
            _ => Err(SimError::elaborate(format!("expression type not yet supported"))),
        }
    }

    fn elaborate_package_func_call(
        &self,
        name: &str,
        args: &[Expr],
        signal_map: &HashMap<String, SignalId>,
        signals: &[SignalInfo],
    ) -> Result<IrExpr, SimError> {
        let (pkg_name, func_name) = name.split_once("::")
            .ok_or_else(|| SimError::elaborate(format!("invalid function name '{}'", name)))?;

        let func = self.package_symbols.get(pkg_name)
            .and_then(|items| items.get(func_name))
            .and_then(|item| if let PackageItem::Function(f) = item { Some(f) } else { None })
            .ok_or_else(|| SimError::elaborate(format!("function '{}' not found in package '{}'", func_name, pkg_name)))?;

        // Find return expression
        let ret_expr = func.stmts.iter().find_map(|s| {
            if let Stmt::Return(Some(e)) = s { Some(e.clone()) } else { None }
        }).ok_or_else(|| SimError::elaborate(format!("function '{}' has no return expression", name)))?;

        // Substitute formal parameters with actual arguments
        let mut result = *ret_expr;

        // First: resolve package-scoped identifiers (e.g. MuBi4True → constant value)
        let pkg_symbols = self.package_symbols.get(pkg_name);
        if let Some(items) = pkg_symbols {
            // Collect all enum member names and their values from typedefs
            let mut enum_member_values: HashMap<String, Expr> = HashMap::new();
            for item in items.values() {
                if let PackageItem::Typedef(td) = item {
                    if let DataType::EnumType { members, .. } = &td.dtype {
                        for (member_name, member_expr) in members {
                            if let Some(expr) = member_expr {
                                enum_member_values.insert(member_name.clone(), expr.clone());
                            }
                        }
                    }
                }
            }
            for (item_name, item) in items {
                if let PackageItem::Param(p) = item {
                    if let Some(expr) = &p.default {
                        result = Self::substitute_ident_in_expr(
                            result, item_name, expr.clone()
                        );
                    }
                }
            }
            // Substitute enum member names with their constant values
            for (member_name, member_value) in &enum_member_values {
                result = Self::substitute_ident_in_expr(
                    result, member_name, member_value.clone()
                );
            }
        }

        // Then: substitute formal parameters with actual arguments
        for (i, param) in func.ports.iter().enumerate() {
            if let Some(arg) = args.get(i) {
                result = Self::substitute_ident_in_expr(result, &param.name, arg.clone());
            }
        }

        self.elaborate_expr(&result, signal_map, signals)
    }

    fn substitute_ident_in_expr(expr: Expr, target: &str, replacement: Expr) -> Expr {
        match expr {
            Expr::Ident(ref name) if name == target => replacement,
            Expr::Ident(_) => expr,
            Expr::Value(_) | Expr::String(_) | Expr::Null | Expr::FillLit(_) => expr,
            Expr::BinaryOp { op, lhs, rhs } => Expr::BinaryOp {
                op,
                lhs: Box::new(Self::substitute_ident_in_expr(*lhs, target, replacement.clone())),
                rhs: Box::new(Self::substitute_ident_in_expr(*rhs, target, replacement.clone())),
            },
            Expr::UnaryOp { op, expr: inner } => Expr::UnaryOp {
                op,
                expr: Box::new(Self::substitute_ident_in_expr(*inner, target, replacement.clone())),
            },
            Expr::Paren(inner) => Expr::Paren(Box::new(Self::substitute_ident_in_expr(*inner, target, replacement.clone()))),
            Expr::Concat(exprs) => Expr::Concat(
                exprs.into_iter().map(|e| Self::substitute_ident_in_expr(e, target, replacement.clone())).collect()
            ),
            Expr::Replicate { count, expr: inner } => Expr::Replicate {
                count: Box::new(Self::substitute_ident_in_expr(*count, target, replacement.clone())),
                expr: Box::new(Self::substitute_ident_in_expr(*inner, target, replacement.clone())),
            },
            Expr::RangeSelect { expr: inner, msb, lsb } => Expr::RangeSelect {
                expr: Box::new(Self::substitute_ident_in_expr(*inner, target, replacement.clone())),
                msb: Box::new(Self::substitute_ident_in_expr(*msb, target, replacement.clone())),
                lsb: Box::new(Self::substitute_ident_in_expr(*lsb, target, replacement.clone())),
            },
            Expr::BitSelect { expr: inner, index } => Expr::BitSelect {
                expr: Box::new(Self::substitute_ident_in_expr(*inner, target, replacement.clone())),
                index: Box::new(Self::substitute_ident_in_expr(*index, target, replacement.clone())),
            },
            Expr::PartSelect { expr: inner, base, width } => Expr::PartSelect {
                expr: Box::new(Self::substitute_ident_in_expr(*inner, target, replacement.clone())),
                base: Box::new(Self::substitute_ident_in_expr(*base, target, replacement.clone())),
                width: Box::new(Self::substitute_ident_in_expr(*width, target, replacement.clone())),
            },
            Expr::ScopedIdent { package, item } => {
                if package == target {
                    match &replacement {
                        Expr::Ident(name) => Expr::ScopedIdent { package: name.clone(), item },
                        _ => Expr::ScopedIdent { package, item },
                    }
                } else {
                    Expr::ScopedIdent { package, item }
                }
            }
            Expr::Cast { dtype, expr: inner } => Expr::Cast {
                expr: Box::new(Self::substitute_ident_in_expr(*inner, target, replacement.clone())),
                dtype,
            },
            Expr::MemberAccess { obj, field } => Expr::MemberAccess {
                obj: Box::new(Self::substitute_ident_in_expr(*obj, target, replacement.clone())),
                field,
            },
            Expr::TernaryOp { cond, true_expr, false_expr } => Expr::TernaryOp {
                cond: Box::new(Self::substitute_ident_in_expr(*cond, target, replacement.clone())),
                true_expr: Box::new(Self::substitute_ident_in_expr(*true_expr, target, replacement.clone())),
                false_expr: Box::new(Self::substitute_ident_in_expr(*false_expr, target, replacement.clone())),
            },
            Expr::FuncCall { name: n, args: a } => Expr::FuncCall {
                name: n,
                args: a.into_iter().map(|e| Self::substitute_ident_in_expr(e, target, replacement.clone())).collect(),
            },
            Expr::MethodCall { obj, method, args, with_clause } => Expr::MethodCall {
                obj: Box::new(Self::substitute_ident_in_expr(*obj, target, replacement.clone())),
                method,
                args: args.into_iter().map(|e| Self::substitute_ident_in_expr(e, target, replacement.clone())).collect(),
                with_clause,
            },
            Expr::Inside { expr: inner, range_list } => Expr::Inside {
                expr: Box::new(Self::substitute_ident_in_expr(*inner, target, replacement.clone())),
                range_list: range_list.into_iter().map(|e| Self::substitute_ident_in_expr(e, target, replacement.clone())).collect(),
            },
            Expr::StreamingConcat { op, slice_size, slices } => Expr::StreamingConcat {
                op,
                slice_size: slice_size.map(|ss| Box::new(Self::substitute_ident_in_expr(*ss, target, replacement.clone()))),
                slices: slices.into_iter().map(|e| Self::substitute_ident_in_expr(e, target, replacement.clone())).collect(),
            },
            Expr::Dist { expr: inner, items } => Expr::Dist {
                expr: Box::new(Self::substitute_ident_in_expr(*inner, target, replacement.clone())),
                items,
            },
        }
    }

    fn elaborate_expr_to_signal(&self, expr: &Expr, signal_map: &HashMap<String, SignalId>)
        -> Result<SignalId, SimError>
    {
        match expr {
            Expr::Ident(name) => {
                signal_map.get(name)
                    .ok_or_else(|| SimError::elaborate(format!("signal '{}' not found", name)))
                    .copied()
            }
            Expr::MethodCall { .. } => Err(SimError::elaborate("method calls cannot resolve to a signal")),
            Expr::MemberAccess { .. } => Err(SimError::elaborate("member access cannot resolve to a signal")),
            _ => Err(SimError::elaborate("expected simple signal identifier"))
        }
    }

    /// Create a signal from a port connection expression.
    /// For simple identifiers, resolves directly.
    /// For compound expressions (e.g. ~clk_i), creates an implicit wire + continuous assign.
    fn instance_port_expr_to_signal(
        &self,
        expr: &Expr,
        signal_map: &HashMap<String, SignalId>,
        signals: &mut Vec<SignalInfo>,
        next_id: &mut SignalId,
        processes: &mut Vec<Process>,
        hint_name: &str,
    ) -> Result<SignalId, SimError> {
        // Try simple signal resolution first
        if let Ok(sid) = self.elaborate_expr_to_signal(expr, signal_map) {
            return Ok(sid);
        }
        // For compound expressions, create an implicit wire
        let ir_expr = self.elaborate_expr(expr, signal_map, signals)?;
        let width_val = compute_expr_width(expr, signal_map, signals, &self.param_vals, &self.package_symbols)?;
        let width = if width_val > 0 { width_val } else { 1 };
        // Create a unique implicit signal name
        let sig_name = format!("__port_{}", hint_name.replace('.', "_"));
        let sid = *next_id;
        *next_id += 1;
        signals.push(SignalInfo {
            name: sig_name.clone(),
            width,
            kind: SignalKind::Wire,
            net_type: NetType::Wire,
            multi_driver: false,
            init_val: crate::ir::LogicVec::fill(crate::ir::LogicVal::Z, width),
            array_depth: 1,
            elem_width: width,
            array_dims: vec![],
            class_name: None,
            is_string: false,
            is_mailbox: false,
            is_semaphore: false,
            is_real: false,
            is_2state: false,
            is_dynamic: false,
            is_queue: false,
                        is_associative: false,
            is_signed: false,
            is_const: false,
            msb: width - 1,
            lsb: 0,
            struct_fields: vec![],
            packed_dims: vec![], delay_rise: None, delay_fall: None, iface_type: None, iface_modport: None,
        });
        // Add a continuous assignment process
        let sensitivity = collect_sensitivity(expr, signal_map);
        processes.push(Process::Combinational {
            name: format!("port_assign_{}", hint_name.replace('.', "_")),
            sensitivity,
            body: vec![IrStmt::BlockingAssign {
                lhs: IrLValue::Signal(sid, 0),
                rhs: ir_expr,
                delay: None,
            }],
        });
        Ok(sid)
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



















/// Try to constant-fold an AST expression into an IrExpr::Const.
/// Returns Ok(Some(IrExpr::Const(...))) if the expression is fully constant,
/// Ok(None) if it cannot be folded, or Err on evaluation error.






impl DataType {
    pub(super) fn width(&self) -> usize {
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



