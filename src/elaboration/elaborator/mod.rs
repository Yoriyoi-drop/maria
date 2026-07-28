use std::collections::HashMap;

use super::util::*;
use crate::ast::types::const_eval_with_params;
use crate::ast::*;
use crate::diagnostics::diagnostic::{DiagCode, DiagLevel, Diagnostic, DiagSink, RuntimeContext, SourceSnippet};
use crate::error::SimError;
use crate::intern::Symbol;
pub mod ext;
pub mod flatten;
pub mod always;
pub mod stmt;
pub mod expr;
use crate::ir::*;

const BUILTIN_UVM_CLASSES: &[&str] = &[
    "uvm_object",
    "uvm_component",
    "uvm_sequence_item",
    "uvm_sequence",
    "uvm_sequencer",
    "uvm_driver",
    "uvm_monitor",
    "uvm_scoreboard",
    "uvm_analysis_port",
    "uvm_analysis_imp",
    "uvm_test",
    "uvm_config_db",
    "uvm_report_object",
    "uvm_factory",
    "uvm_resource_db",
];

pub struct Elaborator {
    pub design: Design,
    pub modules: HashMap<Symbol, IrModule>,
    pub param_vals: HashMap<Symbol, i64>,
    pub typedef_map: HashMap<Symbol, usize>,
    pub typedef_field_map: HashMap<Symbol, Vec<StructFieldInfo>>,
    pub package_symbols: HashMap<Symbol, HashMap<Symbol, PackageItem>>,
    pub specialized_classes: std::cell::RefCell<Vec<ClassDecl>>,
    pub diag_sink: DiagSink,
    pub source_lines: Vec<String>,
    pub source_file: String,
    pub current_module: Option<Symbol>,
}

impl Elaborator {
    pub fn new(design: Design) -> Self {
        Self::with_source(design, Vec::new(), String::new())
    }

    pub fn with_source(design: Design, source_lines: Vec<String>, source_file: String) -> Self {
        let mut package_symbols: HashMap<Symbol, HashMap<Symbol, PackageItem>> = HashMap::new();
        // First pass: collect directly declared items
        for pkg in &design.packages {
            let mut items = HashMap::new();
            for item in &pkg.items {
                let name = match item {
                    PackageItem::Param(p) => p.name,
                    PackageItem::Typedef(t) => t.name,
                    PackageItem::Function(f) => f.name,
                    PackageItem::Task(t) => t.name,
                    PackageItem::Decl(d) => {
                        d.names.first().map(|v| v.name).unwrap_or(Symbol::EMPTY)
                    }
                    PackageItem::Import { .. } => continue,
                    PackageItem::Export { .. } => continue,
                };
                items.insert(name, item.clone());
            }
            package_symbols.insert(pkg.name, items);
        }
        // Second pass: resolve imports within packages
        let imports: Vec<(Symbol, Symbol, Symbol)> = design
            .packages
            .iter()
            .flat_map(|pkg| {
                pkg.items.iter().filter_map(|item| {
                    if let PackageItem::Import {
                        package,
                        item: import_item,
                    } = item
                    {
                        Some((pkg.name, *package, *import_item))
                    } else {
                        None
                    }
                })
            })
            .collect();
        for (pkg_name, source_pkg_name, import_item) in &imports {
            let source_items = package_symbols.get(source_pkg_name).cloned();
            if let Some(source_items) = source_items {
                if let Some(pkg_items) = package_symbols.get_mut(pkg_name) {
                    let names: Vec<Symbol> = if import_item == &Symbol::intern("*") {
                        source_items.keys().copied().collect()
                    } else {
                        vec![*import_item]
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
        let exports: Vec<(Symbol, Symbol, Symbol)> = design
            .packages
            .iter()
            .flat_map(|pkg| {
                pkg.items.iter().filter_map(|item| {
                    if let PackageItem::Export {
                        package,
                        item: export_item,
                    } = item
                    {
                        Some((pkg.name, *package, *export_item))
                    } else {
                        None
                    }
                })
            })
            .collect();
        for (pkg_name, source_pkg_name, export_item) in &exports {
            let source_items = package_symbols.get(source_pkg_name).cloned();
            if let Some(source_items) = source_items {
                if let Some(pkg_items) = package_symbols.get_mut(pkg_name) {
                    let names: Vec<Symbol> = if export_item == &Symbol::intern("*") {
                        source_items.keys().copied().collect()
                    } else {
                        vec![*export_item]
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
            diag_sink: DiagSink::new(),
            source_lines,
            source_file,
            current_module: None,
        }
    }

    pub fn elaborate(&mut self, top_module: Option<&str>) -> Result<IrDesign, SimError> {
        // Process bind declarations: add bound instances to target modules
        let binds = std::mem::take(&mut self.design.binds);
        for bind in &binds {
            if let Some(target_module) = self
                .design
                .modules
                .iter_mut()
                .find(|m| m.name == bind.target)
            {
                target_module
                    .items
                    .push(ModuleItem::Instance(bind.instance.clone()));
            } else {
                eprintln!("warning: bind target '{}' not found", bind.target);
            }
        }

        // Pre-pass: import package functions/tasks into modules before inlining
        let pkg_symbols = &self.package_symbols;
        for module in &mut self.design.modules {
            let imports: Vec<(Symbol, Symbol)> = module
                .items
                .iter()
                .filter_map(|item| {
                    if let ModuleItem::Import {
                        package,
                        item: import_item,
                    } = item
                    {
                        Some((*package, *import_item))
                    } else {
                        None
                    }
                })
                .collect();
            // Also include $unit-level imports
            let all_imports: Vec<(Symbol, Symbol)> = {
                let mut imps = imports;
                for (pkg, item) in &self.design.unit_imports {
                    if !imps.iter().any(|(p, i)| p == pkg && i == item) {
                        imps.push((*pkg, *item));
                    }
                }
                imps
            };
            for (package, import_item) in &all_imports {
                if let Some(pkg_items) = pkg_symbols.get(package) {
                    let names: Vec<&str> = if *import_item == Symbol::intern("*") {
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
                if !module
                    .items
                    .iter()
                    .any(|mi| matches!(mi, ModuleItem::Func(fd) if fd.name == func.name))
                {
                    module.items.push(ModuleItem::Func(func.clone()));
                }
            }
            for task in &self.design.unit_tasks {
                if !module
                    .items
                    .iter()
                    .any(|mi| matches!(mi, ModuleItem::Func(fd) if fd.name == task.name))
                {
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
            for (name_str, width) in temps {
                let name = Symbol::intern(&name_str);
                module.decls.push(Decl {
                    dtype: DataType::Logic,
                    kind: DeclKind::Reg,
                    names: vec![DeclVar {
                        name,
                        range: None,
                        expr_range: if width > 1 {
                            Some(ExprRange {
                                msb: Expr::Value(crate::ast::expr::Value::Decimal(
                                    (width - 1) as i64,
                                )),
                                lsb: Expr::Value(crate::ast::expr::Value::Decimal(0)),
                            })
                        } else {
                            None
                        },
                        array_range: None,

                        extra_packed_dims: vec![],
                        is_dynamic: false,
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

        // Dead module detection: find unreachable modules via reachability from top
        let top_sym = top_module.map(|s| Symbol::intern(s));
        {
            use std::collections::{HashSet, VecDeque};
            let all_names: HashSet<Symbol> =
                self.design.modules.iter().map(|m| m.name).collect();
            let mut reachable: HashSet<Symbol> = HashSet::new();
            let mut queue: VecDeque<Symbol> = VecDeque::new();
            if let Some(ref top) = top_sym {
                if all_names.contains(top) {
                    queue.push_back(*top);
                    reachable.insert(*top);
                }
            } else if let Some(first) = self.design.modules.first() {
                queue.push_back(first.name);
                reachable.insert(first.name);
            }
            while let Some(name) = queue.pop_front() {
                if let Some(module) = self.design.modules.iter().find(|m| m.name == name) {
                    for item in &module.items {
                        if let ModuleItem::Instance(inst) = item {
                            if all_names.contains(&inst.module_name)
                                && !reachable.contains(&inst.module_name)
                            {
                                reachable.insert(inst.module_name);
                                queue.push_back(inst.module_name);
                            }
                        }
                    }
                }
            }
            for m in &self.design.modules {
                if !reachable.contains(&m.name)
                    && top_sym.as_ref().map(|t| *t != m.name).unwrap_or(true)
                {
                    eprintln!(
                        "warning: module '{}' is unreachable (not instantiated from top)",
                        m.name
                    );
                }
            }
        }

        // First pass: elaborate all modules
        let module_names: Vec<Symbol> =
            self.design.modules.iter().map(|m| m.name).collect();

        let modules_snapshot: Vec<Module> = self.design.modules.clone();
        for module in &modules_snapshot {
            let ir = self.elaborate_module(module, &module_names)?;
            self.modules.insert(module.name.clone(), ir);
        }

        // Elaborate interfaces as signal-only modules
        for iface in &self.design.interfaces {
            let mut signals = Vec::new();
            let mut signal_map: HashMap<Symbol, SignalId> = HashMap::new();
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
                            init_val: if is_real {
                                LogicVec::new(64)
                            } else {
                                LogicVec::fill(LogicVal::Z, 0)
                            },
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
                            packed_dims: vec![],
                            delay_rise: None,
                            delay_fall: None,
                            iface_type: None,
                            iface_modport: None,
                        });
                        continue;
                    }
                    let width = self.resolve_type_width(&decl.dtype)?;
                    let elem_width = width
                        .max(var.resolved_width(&HashMap::new()).unwrap_or(width))
                        .max(decl.kind.default_width());
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
                        packed_dims: vec![],
                        delay_rise: None,
                        delay_fall: None,
                        iface_type: None,
                        iface_modport: None,
                    });
                }
            }
            self.modules.insert(
                iface.name.clone(),
                IrModule {
                    name: iface.name.clone(),
                    signals,
                    inputs: vec![],
                    outputs: vec![],
                    inouts: vec![],
                    processes: vec![],
                    sub_instances: vec![],
                },
            );
        }

        // Find top module
        let top_name = match top_module {
            Some(name) => Symbol::intern(name),
            None => self
                .design
                .modules
                .first()
                .map(|m| m.name)
                .ok_or_else(|| self.elab_diag(DiagCode::ModuleNotFound, "no modules in design"))?,
        };

        let mut top = self
            .modules
            .remove(&top_name)
            .ok_or_else(|| self.elab_diag(DiagCode::ModuleNotFound, format!("top module '{}' not found", top_name)))?;

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
        if !classes.contains_key(&Symbol::intern("__uvm_object")) {
            for (_, cls) in classes.iter_mut() {
                let extends_str = cls.extends.map(|s| s.as_str());
                match extends_str {
                    Some("uvm_object") => cls.extends = Some(Symbol::intern("__uvm_object")),
                    Some("uvm_component") => cls.extends = Some(Symbol::intern("__uvm_component")),
                    Some("uvm_sequence_item") => {
                        cls.extends = Some(Symbol::intern("__uvm_sequence_item"))
                    }
                    Some("uvm_sequence") => cls.extends = Some(Symbol::intern("__uvm_sequence")),
                    Some("uvm_sequencer") => cls.extends = Some(Symbol::intern("__uvm_sequencer")),
                    Some("uvm_driver") => cls.extends = Some(Symbol::intern("__uvm_driver")),
                    Some("uvm_monitor") => cls.extends = Some(Symbol::intern("__uvm_monitor")),
                    Some("uvm_scoreboard") => cls.extends = Some(Symbol::intern("__uvm_scoreboard")),
                    Some("uvm_analysis_port") => {
                        cls.extends = Some(Symbol::intern("__uvm_analysis_port"))
                    }
                    Some("uvm_analysis_imp") => {
                        cls.extends = Some(Symbol::intern("__uvm_analysis_imp"))
                    }
                    Some("uvm_test") => cls.extends = Some(Symbol::intern("__uvm_test")),
                    Some("uvm_config_db") => cls.extends = Some(Symbol::intern("__uvm_config_db")),
                    Some("uvm_report_object") => {
                        cls.extends = Some(Symbol::intern("__uvm_report_object"))
                    }
                    Some("uvm_factory") => cls.extends = Some(Symbol::intern("__uvm_factory")),
                    Some("uvm_resource_db") => cls.extends = Some(Symbol::intern("__uvm_resource_db")),
                    _ => {}
                }
            }
            classes.insert(
                Symbol::intern("__uvm_object"),
                IrClassDef {
                    name: Symbol::intern("__uvm_object"),
                    extends: None,
                    type_params: vec![],
                    fields: vec![],
                    methods: vec![],
                    constraints: vec![],
                    rand_fields: vec![],
                },
            );
            classes.insert(
                Symbol::intern("__uvm_report_object"),
                IrClassDef {
                    name: Symbol::intern("__uvm_report_object"),
                    extends: Some(Symbol::intern("__uvm_object")),
                    type_params: vec![],
                    fields: vec![],
                    methods: vec![],
                    constraints: vec![],
                    rand_fields: vec![],
                },
            );
            classes.insert(
                Symbol::intern("__uvm_component"),
                IrClassDef {
                    name: Symbol::intern("__uvm_component"),
                    extends: Some(Symbol::intern("__uvm_report_object")),
                    type_params: vec![],
                    fields: vec![],
                    methods: vec![],
                    constraints: vec![],
                    rand_fields: vec![],
                },
            );
            classes.insert(
                Symbol::intern("__uvm_sequence_item"),
                IrClassDef {
                    name: Symbol::intern("__uvm_sequence_item"),
                    extends: Some(Symbol::intern("__uvm_object")),
                    type_params: vec![],
                    fields: vec![],
                    methods: vec![],
                    constraints: vec![],
                    rand_fields: vec![],
                },
            );
            classes.insert(
                Symbol::intern("__uvm_sequence"),
                IrClassDef {
                    name: Symbol::intern("__uvm_sequence"),
                    extends: Some(Symbol::intern("__uvm_sequence_item")),
                    type_params: vec![],
                    fields: vec![],
                    methods: vec![],
                    constraints: vec![],
                    rand_fields: vec![],
                },
            );
            classes.insert(
                Symbol::intern("__uvm_sequencer"),
                IrClassDef {
                    name: Symbol::intern("__uvm_sequencer"),
                    extends: Some(Symbol::intern("__uvm_component")),
                    type_params: vec![],
                    fields: vec![],
                    methods: vec![],
                    constraints: vec![],
                    rand_fields: vec![],
                },
            );
            classes.insert(
                Symbol::intern("__uvm_driver"),
                IrClassDef {
                    name: Symbol::intern("__uvm_driver"),
                    extends: Some(Symbol::intern("__uvm_component")),
                    type_params: vec![],
                    fields: vec![],
                    methods: vec![],
                    constraints: vec![],
                    rand_fields: vec![],
                },
            );
            classes.insert(
                Symbol::intern("__uvm_monitor"),
                IrClassDef {
                    name: Symbol::intern("__uvm_monitor"),
                    extends: Some(Symbol::intern("__uvm_component")),
                    type_params: vec![],
                    fields: vec![],
                    methods: vec![],
                    constraints: vec![],
                    rand_fields: vec![],
                },
            );
            classes.insert(
                Symbol::intern("__uvm_scoreboard"),
                IrClassDef {
                    name: Symbol::intern("__uvm_scoreboard"),
                    extends: Some(Symbol::intern("__uvm_component")),
                    type_params: vec![],
                    fields: vec![],
                    methods: vec![],
                    constraints: vec![],
                    rand_fields: vec![],
                },
            );
            classes.insert(
                Symbol::intern("__uvm_analysis_port"),
                IrClassDef {
                    name: Symbol::intern("__uvm_analysis_port"),
                    extends: Some(Symbol::intern("__uvm_object")),
                    type_params: vec![],
                    fields: vec![],
                    methods: vec![],
                    constraints: vec![],
                    rand_fields: vec![],
                },
            );
            classes.insert(
                Symbol::intern("__uvm_analysis_imp"),
                IrClassDef {
                    name: Symbol::intern("__uvm_analysis_imp"),
                    extends: Some(Symbol::intern("__uvm_object")),
                    type_params: vec![],
                    fields: vec![],
                    methods: vec![],
                    constraints: vec![],
                    rand_fields: vec![],
                },
            );
            classes.insert(
                Symbol::intern("__uvm_test"),
                IrClassDef {
                    name: Symbol::intern("__uvm_test"),
                    extends: Some(Symbol::intern("__uvm_component")),
                    type_params: vec![],
                    fields: vec![],
                    methods: vec![],
                    constraints: vec![],
                    rand_fields: vec![],
                },
            );
            classes.insert(
                Symbol::intern("__uvm_config_db"),
                IrClassDef {
                    name: Symbol::intern("__uvm_config_db"),
                    extends: Some(Symbol::intern("__uvm_object")),
                    type_params: vec![],
                    fields: vec![],
                    methods: vec![],
                    constraints: vec![],
                    rand_fields: vec![],
                },
            );
            classes.insert(
                Symbol::intern("__uvm_resource_db"),
                IrClassDef {
                    name: Symbol::intern("__uvm_resource_db"),
                    extends: Some(Symbol::intern("__uvm_object")),
                    type_params: vec![],
                    fields: vec![],
                    methods: vec![],
                    constraints: vec![],
                    rand_fields: vec![],
                },
            );
            classes.insert(
                Symbol::intern("__uvm_factory"),
                IrClassDef {
                    name: Symbol::intern("__uvm_factory"),
                    extends: Some(Symbol::intern("__uvm_object")),
                    type_params: vec![],
                    fields: vec![],
                    methods: vec![],
                    constraints: vec![],
                    rand_fields: vec![],
                },
            );
        }

        self.detect_multi_driver_signals(&mut top)?;

        let top_signal_map: HashMap<Symbol, SignalId> = top
            .signals
            .iter()
            .enumerate()
            .map(|(i, s)| (s.name, i))
            .collect();
        let covergroups = self.elaborate_covergroups(top_name.as_str(), &top_signal_map, &top.signals)?;
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
        let mut module_functions: HashMap<Symbol, crate::ast::types::FunctionDecl> = HashMap::new();
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

    fn resolve_param_values(
        &self,
        module: &Module,
        instance_overrides: &HashMap<Symbol, i64>,
    ) -> Result<HashMap<Symbol, i64>, SimError> {
        resolve_param_values_fn(module, instance_overrides).map_err(|e| self.elab_diag(DiagCode::ParamMismatch, e))
    }

    /// Buat structured diagnostic untuk elaboration error dengan error code tepat.
    fn elab_diag(&self, code: DiagCode, message: impl Into<String>) -> SimError {
        self.elab_diag_at(code, message, 0, 0)
    }

    /// Buat error diagnostic dengan posisi source.
    fn elab_diag_at(&self, code: DiagCode, message: impl Into<String>, line: usize, col: usize) -> SimError {
        let msg: String = message.into();
        let mut diag = Diagnostic::new(DiagLevel::Error, code, msg)
            .with_code_context();
        if line > 0 && line <= self.source_lines.len() {
            let source_line = &self.source_lines[line - 1];
            let snippet = SourceSnippet::new(&self.source_file, line, col, source_line);
            diag = diag.with_source_snippet(snippet);
        }
        if let Some(ref mod_name) = self.current_module {
            let ctx = RuntimeContext::new()
                .with_module(mod_name.as_str());
            diag = diag.with_runtime_context(ctx);
        }
        SimError::from_elab_diagnostic(diag)
    }

    /// Emit warning diagnostic ke DiagSink (elaboration-time warnings).
    fn elab_warn(&self, code: DiagCode, message: impl Into<String>) {
        let msg: String = message.into();
        let diag = Diagnostic::new(DiagLevel::Warning, code, msg)
            .with_code_context();
        self.diag_sink.push(diag);
    }

    /// Flush diagnostics from DiagSink and return them.
    pub fn flush_diagnostics(&self) -> Vec<Diagnostic> {
        self.diag_sink.diagnostics()
    }

    fn store_typedef_fields(&mut self, name: Symbol, dtype: &DataType) {
        let fields = Self::compute_struct_fields(dtype);
        if !fields.is_empty() {
            self.typedef_field_map.insert(name, fields);
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
                if self.design.modules.iter().any(|m| {
                    m.items
                        .iter()
                        .any(|item| matches!(item, ModuleItem::Covergroup(cg) if cg.name == *name))
                }) {
                    return Ok(64);
                }
                // Check package symbols for typedefs
                for (_, pkg_items) in &self.package_symbols {
                    if let Some(PackageItem::Typedef(td)) = pkg_items.get(name.as_str()) {
                        let width = self.resolve_typedef_width(&td.dtype, td.range.as_ref());
                        if width > 0 {
                            return Ok(width);
                        }
                    }
                }
                // Check in-module typedefs stored in typedef_map
                if let Some(&width) = self.typedef_map.get(name) {
                    return Ok(width);
                }
                // Type not found — warn and return default width of 32 (common SV default)
                eprintln!("  ** WARNING: unknown type '{}' is not defined in this scope (using default width 32)", name);
                Ok(32)
            }
            DataType::Signed(inner) => self.resolve_type_width(inner),
            _ => Ok(dtype.width()),
        }
    }

    fn compute_struct_fields(dtype: &DataType) -> Vec<StructFieldInfo> {
        match dtype {
            DataType::UnionType { members } => members
                .iter()
                .map(|m| {
                    let w = m.range.as_ref().map(|r| r.width()).unwrap_or(1);
                    StructFieldInfo {
                        name: m.name.clone(),
                        offset: 0,
                        width: w,
                    }
                })
                .collect(),
            DataType::StructType { members } => {
                let mut fields = Vec::new();
                let mut offset = 0usize;
                let members_rev: Vec<_> = members.iter().rev().collect();
                for m in &members_rev {
                    let w = m.range.as_ref().map(|r| r.width()).unwrap_or(1);
                    fields.push(StructFieldInfo {
                        name: m.name.clone(),
                        offset,
                        width: w,
                    });
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

    fn elaborate_module(
        &mut self,
        module: &Module,
        known_modules: &[Symbol],
    ) -> Result<IrModule, SimError> {
        self.current_module = Some(module.name);
        let param_vals = self.resolve_param_values(module, &HashMap::new())?;
        self.elaborate_module_with_params(module, known_modules, &param_vals)
    }

    fn elaborate_module_with_params(
        &mut self,
        module: &Module,
        known_modules: &[Symbol],
        param_vals: &HashMap<Symbol, i64>,
    ) -> Result<IrModule, SimError> {
        self.elaborate_module_with_params_and_type(
            module,
            known_modules,
            param_vals,
            &HashMap::new(),
        )
    }

    fn elaborate_module_with_params_and_type(
        &mut self,
        module: &Module,
        known_modules: &[Symbol],
        param_vals: &HashMap<Symbol, i64>,
        type_param_overrides: &HashMap<Symbol, usize>,
    ) -> Result<IrModule, SimError> {
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
                let names: Vec<&str> = if import_item.as_str() == "*" {
                    pkg_items.keys().map(|s| s.as_str()).collect()
                } else {
                    vec![import_item.as_str()]
                };
                for name in names {
                    if let Some(pkg_item) = pkg_items.get(name) {
                        if let PackageItem::Param(p) = pkg_item {
                            if !effective_params.contains_key(&p.name) {
                                if let Some(expr) = &p.default {
                                    if let Ok(val) = const_eval_with_params(expr, &effective_params)
                                    {
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
            if let ModuleItem::Import {
                package,
                item: import_item,
            } = item
            {
                if let Some(pkg_items) = self.package_symbols.get(package) {
                    let names: Vec<&str> = if import_item.as_str() == "*" {
                        pkg_items.keys().map(|s| s.as_str()).collect()
                    } else {
                        vec![import_item.as_str()]
                    };
                    for name in names {
                        if let Some(pkg_item) = pkg_items.get(name) {
                            if let PackageItem::Param(p) = pkg_item {
                                if !effective_params.contains_key(&p.name) {
                                    if let Some(expr) = &p.default {
                                        if let Ok(val) =
                                            const_eval_with_params(expr, &effective_params)
                                        {
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
                    let names: Vec<&str> = if import_item.as_str() == "*" {
                        pkg_items.keys().map(|s| s.as_str()).collect()
                    } else {
                        vec![import_item.as_str()]
                    };
                    let mut struct_imports: Vec<(Symbol, DataType)> = Vec::new();
                    for name in names {
                        if let Some(pkg_item) = pkg_items.get(name) {
                            if let PackageItem::Typedef(td) = pkg_item {
                                if !self.typedef_map.contains_key(&td.name) {
                                    let width =
                                        self.resolve_typedef_width(&td.dtype, td.range.as_ref());
                                    self.typedef_map.insert(td.name, width);
                                }
                                if matches!(
                                    &td.dtype,
                                    DataType::StructType { .. } | DataType::UnionType { .. }
                                ) {
                                    struct_imports.push((td.name, td.dtype.clone()));
                                }
                            }
                        }
                    }
                    for (name, dtype) in struct_imports {
                        self.store_typedef_fields(name, &dtype);
                    }
                }
            }
        }
        // Resolve type parameter widths from module's param declarations and overrides
        let mut type_param_widths: HashMap<Symbol, usize> = HashMap::new();
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
                if matches!(
                    &td.dtype,
                    DataType::StructType { .. } | DataType::UnionType { .. }
                ) {
                    self.store_typedef_fields(td.name, &td.dtype);
                }
            }
        }
        // Pre-pass: process $unit typedefs (top-level typedefs outside any module)
        let unit_typedefs = self.design.unit_typedefs.clone();
        for td in &unit_typedefs {
            let width = self.resolve_typedef_width(&td.dtype, td.range.as_ref());
            self.typedef_map.insert(td.name.clone(), width);
            if matches!(
                &td.dtype,
                DataType::StructType { .. } | DataType::UnionType { .. }
            ) {
                self.store_typedef_fields(td.name, &td.dtype);
            }
        }
        // Pre-pass: process $unit imports for typedefs
        for (package, import_item) in &self.design.unit_imports {
            if let Some(pkg_items) = self.package_symbols.get(package) {
                let names: Vec<&str> = if import_item.as_str() == "*" {
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
        let typedef_imports: Vec<(Symbol, DataType)> = unit_imports
            .iter()
            .filter_map(|(package, import_item)| {
                self.package_symbols.get(package).and_then(|pkg_items| {                    let names: Vec<Symbol> = if import_item.as_str() == "*" {
                            pkg_items.keys().copied().collect()
                        } else {
                            vec![*import_item]
                        };
                        names.iter().find_map(|name| {
                            if let Some(PackageItem::Typedef(td)) = pkg_items.get(name) {
                            if matches!(
                                &td.dtype,
                                DataType::StructType { .. } | DataType::UnionType { .. }
                            ) {
                                Some((td.name.clone(), td.dtype.clone()))
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    })
                })
            })
            .collect();
        for (name, dtype) in &typedef_imports {
            self.store_typedef_fields(*name, dtype);
        }
        // Pre-pass: process package imports for typedefs before declaration processing
        // Pre-pass: process package imports for struct/union typedef fields
        let import_typedefs: Vec<(Symbol, DataType)> = module
            .items
            .iter()
            .filter_map(|item| {
                if let ModuleItem::Import {
                    package,
                    item: import_item,
                } = item
                {
                    self.package_symbols.get(package).and_then(|pkg_items| {
                        let names: Vec<Symbol> = if import_item.as_str() == "*" {
                            pkg_items.keys().copied().collect()
                        } else {
                            vec![*import_item]
                        };
                        names.iter().find_map(|name| {
                            let name_sym = *name;

                            if let Some(PackageItem::Typedef(td)) = pkg_items.get(&name_sym) {
                                if matches!(
                                    &td.dtype,
                                    DataType::StructType { .. } | DataType::UnionType { .. }
                                ) {
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
            })
            .collect();
        for (name, dtype) in &import_typedefs {
            let fields = Self::compute_struct_fields(dtype);
            if !fields.is_empty() {
                self.typedef_field_map.entry(name.clone()).or_insert(fields);
            }
        }

        let mut signals = Vec::new();
        let mut signal_map: HashMap<Symbol, SignalId> = HashMap::new();
        let mut inputs = Vec::new();
        let mut outputs = Vec::new();
        let mut inouts = Vec::new();
        let mut processes = Vec::new();
        let mut sub_instances = Vec::new();
        let mut next_id = 0usize;

        // Helper to get or create signal
        let get_or_create_signal = |name: Symbol,
                                    width: usize,
                                    kind: SignalKind,
                                    net_type: NetType,
                                    signals: &mut Vec<SignalInfo>,
                                    signal_map: &mut HashMap<Symbol, SignalId>,
                                    id: &mut SignalId,
                                    array_depth: usize,
                                    elem_width: usize,
                                    msb: usize,
                                    lsb: usize,
                                    is_2state: bool,
                                    is_signed: bool|
         -> SignalId {
            if let Some(&sid) = signal_map.get(&name) {
                sid
            } else {
                let sid = *id;
                *id += 1;
                signal_map.insert(name, sid);
                let init_val = match kind {
                    SignalKind::Wire | SignalKind::Inout => LogicVec::fill(LogicVal::Z, width),
                    _ => LogicVec::new(width),
                };
                signals.push(SignalInfo {
                    name,
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
                    packed_dims: vec![],
                    delay_rise: None,
                    delay_fall: None,
                    iface_type: None,
                    iface_modport: None,
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
            let sid = get_or_create_signal(
                port.name,
                width,
                kind.clone(),
                net_type,
                &mut signals,
                &mut signal_map,
                &mut next_id,
                1,
                width,
                p_msb,
                p_lsb,
                false,
                false,
            );
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
                DataType::UserDefined(cn) if cn.as_str() == "process" => Some("__process".to_string()),
                DataType::UserDefined(cn) => Some(cn.as_str().to_string()),
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
                        packed_dims: vec![],
                        delay_rise: None,
                        delay_fall: None,
                        iface_type: None,
                        iface_modport: None,
                    });
                    continue;
                }
                if var.is_dynamic || var.is_queue {
                    let dtype_width = self.resolve_type_width(&decl.dtype)?;
                    let elem_width = dtype_width
                        .max(var.resolved_width(&effective_params)?)
                        .max(decl.kind.default_width());
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
                        packed_dims: vec![],
                        delay_rise: None,
                        delay_fall: None,
                        iface_type: None,
                        iface_modport: None,
                    });
                }
                let dtype_width = self.resolve_type_width(&decl.dtype)?;
                let elem_width = dtype_width
                    .max(var.resolved_width(&effective_params)?)
                    .max(decl.kind.default_width());
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
                    DeclKind::Reg | DeclKind::Logic | DeclKind::Int | DeclKind::Integer => {
                        (SignalKind::Reg, NetType::Wire)
                    }
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
                    let depth = if ar.msb >= ar.lsb {
                        ar.msb - ar.lsb + 1
                    } else {
                        ar.lsb - ar.msb + 1
                    };
                    let total_width = elem_width * depth;
                    let _sid = get_or_create_signal(
                        var.name,
                        total_width,
                        kind.clone(),
                        net_type,
                        &mut signals,
                        &mut signal_map,
                        &mut next_id,
                        depth,
                        elem_width,
                        total_width - 1,
                        0,
                        decl_is_2state,
                        is_signed_type(&decl.dtype),
                    );
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
                            sig.class_name = Some(Symbol::intern(class));
                            if class == "__mailbox" {
                                sig.is_mailbox = true;
                            }
                            if class == "__semaphore" {
                                sig.is_semaphore = true;
                            }
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
                                    if let Ok(or) = resolve_expr_range(extra_er, &effective_params)
                                    {
                                        pd.push(or.width());
                                    }
                                }
                                sig.packed_dims = pd;
                            }
                        }
                    }
                } else {
                    let _sid = get_or_create_signal(
                        var.name,
                        elem_width,
                        kind,
                        net_type,
                        &mut signals,
                        &mut signal_map,
                        &mut next_id,
                        1,
                        elem_width,
                        d_msb,
                        d_lsb,
                        decl_is_2state,
                        is_signed_type(&decl.dtype),
                    );
                    if let Some(class) = &class_name {
                        if let Some(sig) = signals.iter_mut().find(|s| s.name == var.name) {
                            sig.class_name = Some(Symbol::intern(class));
                            if class == "__mailbox" {
                                sig.is_mailbox = true;
                            }
                            if class == "__semaphore" {
                                sig.is_semaphore = true;
                            }
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
                                        if let Ok(or) =
                                            resolve_expr_range(extra_er, &effective_params)
                                        {
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
                                        if let Ok(val) =
                                            const_eval_with_params(expr, &effective_params)
                                        {
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
                    let body = self.elaborate_stmt_block(
                        &initial.stmts,
                        &signal_map,
                        &known_modules,
                        &signals,
                    )?;
                    processes.push(Process::Initial {
                        name: Symbol::intern(&format!("initial_{}", processes.len())),
                        body,
                    });
                }
                ModuleItem::Final(final_block) => {
                    let body = self.elaborate_stmt_block(
                        &final_block.stmts,
                        &signal_map,
                        &known_modules,
                        &signals,
                    )?;
                    processes.push(Process::Final {
                        name: Symbol::intern(&format!("final_{}", processes.len())),
                        body,
                    });
                }
                ModuleItem::Assign(assign) => {
                    // Convert to a combinational process
                    let lhs = self.elaborate_lvalue(&assign.lhs, &signal_map, &signals)?;
                    let rhs = self.elaborate_expr(&assign.rhs, &signal_map, &signals)?;
                    let stmts = vec![IrStmt::BlockingAssign {
                        lhs,
                        rhs,
                        delay: None,
                    }];
                    let sensitivity = collect_sensitivity(&assign.rhs, &signal_map);
                    processes.push(Process::Combinational {
                        name: Symbol::intern(&format!("assign_{}", processes.len())),
                        sensitivity,
                        body: stmts,
                    });
                }
                ModuleItem::Typedef(td) => {
                    // Already collected in pre-pass; register for UserDefined resolution
                    let width = self.typedef_map.get(&td.name).copied().unwrap_or_else(|| {
                        self.resolve_typedef_width(&td.dtype, td.range.as_ref())
                    });
                    self.typedef_map.insert(td.name.clone(), width);
                    // Store struct/union field info for member access
                    match &td.dtype {
                        DataType::StructType { members } | DataType::UnionType { members } => {
                            let mut fields = Vec::new();
                            match &td.dtype {
                                DataType::UnionType { members } => {
                                    for m in members {
                                        let w = m.range.as_ref().map(|r| r.width()).unwrap_or(1);
                                        fields.push(StructFieldInfo {
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
                                        fields.push(StructFieldInfo {
                                            name: m.name.clone(),
                                            offset,
                                            width: w,
                                        });
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
                    let udp_match = self
                        .design
                        .udp_defs
                        .iter()
                        .find(|u| u.name == inst.module_name)
                        .cloned();

                    if let Some(udp) = udp_match {
                        // UDP instance: create combinational process with table lookup
                        let mut sig_ids = Vec::new();
                        for conn in &inst.port_conns {
                            let expr = match conn {
                                PortConnection::Positional(e) => e,
                                PortConnection::Named { expr, .. } => expr,
                            };
                            let sid = self.instance_port_expr_to_signal(
                                expr,
                                &signal_map,
                                &mut signals,
                                &mut next_id,
                                &mut processes,
                                &format!("{}.udp", inst.instance_name),
                            )?;
                            sig_ids.push(sid);
                        }
                        if sig_ids.len() < 2 {
                            return Err(self.elab_diag(DiagCode::ParamMismatch, format!(
                                "UDP '{}' requires at least 2 ports (1 output + 1+ inputs)",
                                udp.name
                            )));
                        }
                        let out_id = sig_ids[0];
                        let in_ids: Vec<SignalId> = sig_ids[1..].to_vec();
                        let mut in_exprs: Vec<IrExpr> =
                            in_ids.iter().map(|id| IrExpr::Signal(*id, 0)).collect();
                        // For sequential UDP, add output state as last arg (state feedback)
                        if udp.is_sequential {
                            in_exprs.push(IrExpr::Signal(out_id, 0));
                        }
                        let mut sensitivity = in_ids.clone();
                        if udp.is_sequential {
                            sensitivity.push(out_id);
                        }
                        let process = Process::Combinational {
                            name: Symbol::intern(&format!("udp_{}_{}", udp.name, inst.instance_name)),
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
                                name: Symbol::intern(&format!("udp_init_{}_{}", udp.name, inst.instance_name)),
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
                        let target_module: Option<&Module> = self
                            .design
                            .modules
                            .iter()
                            .find(|m| m.name == inst.module_name);
                        for (i, conn) in inst.port_conns.iter().enumerate() {
                            match conn {
                                PortConnection::Positional(expr) => {
                                    if let Some(tm) = target_module {
                                        if let Some(port) = tm.ports.get(i) {
                                            let sig_id = self.instance_port_expr_to_signal(
                                                expr,
                                                &signal_map,
                                                &mut signals,
                                                &mut next_id,
                                                &mut processes,
                                                &format!("{}.{}", inst.instance_name, port.name),
                                            )?;
                                            port_map.insert(port.name.clone(), sig_id);
                                        }
                                    }
                                }
                                PortConnection::Named { port, expr } => {
                                    let sig_id = self.instance_port_expr_to_signal(
                                        expr,
                                        &signal_map,
                                        &mut signals,
                                        &mut next_id,
                                        &mut processes,
                                        &format!("{}.{}", inst.instance_name, port),
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
                        let mut type_param_map: HashMap<Symbol, usize> = HashMap::new();
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
                                    instance_name: Symbol::intern(&inst_name),
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
                ModuleItem::VirtualInterface {
                    iface_type,
                    modport,
                    vif_name,
                } => {
                    // Create a signal for the virtual interface variable
                    let sid = next_id;
                    next_id += 1;
                    signal_map.insert(vif_name.clone(), sid);
                    signals.push(SignalInfo {
                        name: vif_name.clone(),
                        width: 64,
                        kind: SignalKind::Reg,
                        net_type: NetType::Wire,
                        multi_driver: false,
                        init_val: LogicVec::new(64),
                        array_depth: 1,
                        elem_width: 64,
                        array_dims: vec![],
                        class_name: Some(iface_type.clone()),
                        is_string: false,
                        is_mailbox: false,
                        is_semaphore: false,
                        is_real: false,
                        is_2state: true,
                        is_dynamic: false,
                        is_queue: false,
                        is_associative: false,
                        is_signed: false,
                        is_const: false,
                        msb: 63,
                        lsb: 0,
                        struct_fields: vec![],
                        packed_dims: vec![],
                        delay_rise: None,
                        delay_fall: None,
                        iface_type: Some(iface_type.clone()),
                        iface_modport: modport.clone(),
                    });
                }
                ModuleItem::Gate(gate) => {
                    let mut sig_ids = Vec::new();
                    for port in &gate.ports {
                        let sid = match port {
                            Expr::Ident { name, .. } => {
                                signal_map.get(name).copied().ok_or_else(|| {
                                    self.elab_diag(DiagCode::ModuleNotFound, format!(
                                        "signal '{}' not found for gate",
                                        name
                                    ))
                                })?
                            }
                            _ => {
                                return Err(self.elab_diag(DiagCode::InstanceNotFound, format!(
                                    "gate port must be a simple signal (port expression: {:?})",
                                    port
                                )))
                            }
                        };
                        sig_ids.push(sid);
                    }
                    if sig_ids.len() < 2 {
                        return Err(self.elab_diag(DiagCode::ParamMismatch, format!(
                            "gate requires at least 2 ports (gate type: {:?}, got {} ports)",
                            gate.gate_type,
                            sig_ids.len()
                        )));
                    }
                    let (out_ids, in_ids) = match gate.gate_type {
                        GateType::And
                        | GateType::Or
                        | GateType::Nand
                        | GateType::Nor
                        | GateType::Xor
                        | GateType::Xnor => (vec![sig_ids[0]], sig_ids[1..].to_vec()),
                        GateType::Buf | GateType::Not => {
                            let in_id = sig_ids[sig_ids.len() - 1];
                            let outs = sig_ids[..sig_ids.len() - 1].to_vec();
                            (outs, vec![in_id])
                        }
                    };
                    let in_exprs: Vec<IrExpr> =
                        in_ids.iter().map(|id| IrExpr::Signal(*id, 0)).collect();
                    let gate_expr = build_gate_expr(&gate.gate_type, &in_exprs);
                    for &out_id in &out_ids {
                        let process = Process::Combinational {
                            name: Symbol::intern(&format!("gate_{}", out_id)),
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
                        &Expr::Ident { name: var.name.clone(), line: 0, col: 0 },
                        &signal_map,
                        &signals,
                    )?;
                    let rhs = self.elaborate_expr(init_expr, &signal_map, &signals)?;
                    if decl.kind.is_net() {
                        let sensitivity = collect_sensitivity(init_expr, &signal_map);
                        processes.push(Process::Combinational {
                            name: Symbol::intern(&format!("decl_assign_{}", processes.len())),
                            sensitivity,
                            body: vec![IrStmt::BlockingAssign {
                                lhs,
                                rhs,
                                delay: None,
                            }],
                        });
                    } else {
                        processes.push(Process::Initial {
                            name: Symbol::intern(&format!("decl_init_{}", processes.len())),
                            body: vec![IrStmt::BlockingAssign {
                                lhs,
                                rhs,
                                delay: None,
                            }],
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

    fn elaborate_classes(&self) -> Result<HashMap<Symbol, IrClassDef>, SimError> {
        let mut classes = HashMap::new();
        for cd in &self.design.classes {
            let mut fields = Vec::new();
            for member in &cd.members {
                if let ClassMember::Decl(decl) = member {
                    for dv in &decl.names {
                        let decl_width =
                            self.resolve_class_field_width(&decl.dtype, &cd.type_params);
                        let var_width = dv.resolved_width(&HashMap::new()).unwrap_or(1);
                        let elem_width = decl_width.max(var_width).max(1);
                        let (array_depth, actual_elem_width) = if let Some(ar) = &dv.array_range {
                            let depth = if ar.msb >= ar.lsb {
                                ar.msb - ar.lsb + 1
                            } else {
                                ar.lsb - ar.msb + 1
                            };
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
            let methods = cd
                .members
                .iter()
                .filter_map(|m| match m {
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
                })
                .collect();
            let constraints: Vec<(Symbol, Vec<crate::ast::types::ConstraintItem>)> = cd
                .members
                .iter()
                .filter_map(|m| {
                    if let ClassMember::Constraint { name, body } = m {
                        Some((*name, body.clone()))
                    } else {
                        None
                    }
                })
                .collect();
            let rand_fields: Vec<Symbol> = cd
                .members
                .iter()
                .flat_map(|m| {
                    if let ClassMember::Decl(decl) = m {
                        decl.names
                            .iter()
                            .filter(|dv| dv.is_rand)
                            .map(|dv| dv.name)
                            .collect::<Vec<_>>()
                    } else {
                        vec![]
                    }
                })
                .collect();
            // Merge parent class fields (recursively) — parent fields come before child fields
            let all_fields = if let Some(ref parent_name) = cd.extends {
        let parent_key = parent_name
            .split("::")
            .last()
            .unwrap_or_else(|| parent_name.as_str());
        let mut merged = Vec::new();
        let mut seen = std::collections::HashSet::new();
        if let Some(parent_cd) = classes.get(parent_key) {
                    let mut ancestors: Vec<&IrClassDef> = vec![parent_cd];
                    loop {
                        let current = ancestors.last().unwrap();
                        if let Some(ref gp) = current.extends {            let gp_key = gp.split("::").last().unwrap_or_else(|| gp.as_str());
            if let Some(gp_cd) = classes.get(gp_key) {
                                ancestors.push(gp_cd);
                            } else {
                                break;
                            }
                        } else {
                            break;
                        }
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

            classes.insert(
                cd.name.clone(),
                IrClassDef {
                    name: cd.name.clone(),
                    extends: cd.extends.clone(),
                    type_params: cd
                        .type_params
                        .iter()
                        .map(|tp| IrTypeParam {
                            name: tp.name.clone(),
                            default_type: tp.default_type.clone(),
                        })
                        .collect(),
                    fields: all_fields,
                    methods,
                    constraints,
                    rand_fields,
                },
            );
        }
        Ok(classes)
    }

}
