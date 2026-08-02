use std::collections::HashMap;

use super::util::*;
use crate::ast::types::const_eval_with_params;
use crate::ast::*;
use crate::diagnostics::diagnostic::{DiagCode, DiagLevel, Diagnostic, DiagSink, RuntimeContext, SourceSnippet};
use crate::error::SimError;
use crate::intern::Symbol;
pub mod ext;
pub mod flatten;

/// Format a prefix + integer counter into a Symbol without heap allocation.
/// Uses a stack buffer and writes the decimal number directly.
fn format_sym(prefix: &[u8], n: usize) -> Symbol {
    let mut buf = [0u8; 32];
    let plen = prefix.len().min(buf.len() - 1);
    buf[..plen].copy_from_slice(&prefix[..plen]);
    let mut i = n;
    let mut end = buf.len();
    loop {
        end -= 1;
        buf[end] = b'0' + (i % 10) as u8;
        i /= 10;
        if i == 0 {
            break;
        }
    }
    // Shift digits to right after prefix
    let dlen = buf.len() - end;
    let total = plen + dlen;
    buf.copy_within(end..end + dlen, plen);
    Symbol::intern(unsafe { std::str::from_utf8_unchecked(&buf[..total]) })
}
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
    /// Konstanta package ter-evaluasi (kualifikasi `pkg::name`): skalar & array.
    pub pkg_const_scalars: HashMap<Symbol, i64>,
    pub pkg_const_arrays: HashMap<Symbol, Vec<i64>>,
    /// Context package global (qualified `pkg::name` + enum members) yang
    /// dihitung SEKALI per compile. Tidak bergantung pada module sehingga bisa
    /// dipakai bersama (clone) oleh semua module — menghindari rescan semua
    /// package + fixed-point 64 iterasi untuk tiap module (bottleneck di
    /// desain besar seperti OpenTitan).
    pub pkg_param_ctx: HashMap<Symbol, i64>,
    /// Context hasil `import pkg::*` / `import pkg::item` level $unit
    /// (unit_imports). Juga konstan antar-module sehingga dihitung sekali.
    pub unit_import_ctx: HashMap<Symbol, i64>,
    /// Nilai plain param per package (dengan base = context global package).
    /// Dipakai untuk `import pkg::item`/`pkg::*` milik module — menghindari
    /// evaluasi ulang default param package untuk tiap module.
    pub pkg_plain_params: HashMap<Symbol, HashMap<Symbol, i64>>,
    pub specialized_classes: std::cell::RefCell<Vec<ClassDecl>>,
    pub diag_sink: DiagSink,
    pub source_lines: Vec<String>,
    pub source_file: String,
    pub current_module: Option<Symbol>,
    // ── Incremental elaboration cache ──
    /// Global cache: signature → cached IrModule (session-wide)
    pub module_cache: HashMap<u64, IrModule>,
    /// Stats for profiling
    pub cache_hits: usize,
    pub cache_misses: usize,
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
                    PackageItem::Class(c) => c.name,
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
                            pkg_items.entry(name).or_insert(source_item.clone());
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
                            pkg_items.entry(name).or_insert(source_item.clone());
                        }
                    }
                }
            }
        }

        // Evaluate package constants once (scalars + arrays)
        // eval_package_constants menerima referensi — tidak perlu clone map.
        let (pkg_const_scalars, pkg_const_arrays) =
            crate::ast::const_eval_ext::eval_package_constants(&package_symbols);

        if std::env::var("DBG_ELAB").is_ok() {
            let gp = Symbol::intern("gpio_env_pkg");
            let dv = Symbol::intern("dv_utils_pkg");
            eprintln!("[DBG-ELAB] with_source: design.packages={} package_symbols={} gpio_env_pkg={} dv_utils_pkg={}", design.packages.len(), package_symbols.len(), package_symbols.contains_key(&gp), package_symbols.contains_key(&dv));
        }

        Elaborator {
            design,
            modules: HashMap::new(),
            param_vals: HashMap::new(),
            typedef_map: HashMap::new(),
            typedef_field_map: HashMap::new(),
            package_symbols,
            pkg_const_scalars,
            pkg_const_arrays,
            pkg_param_ctx: HashMap::new(),
            unit_import_ctx: HashMap::new(),
            pkg_plain_params: HashMap::new(),
            specialized_classes: std::cell::RefCell::new(Vec::new()),
            diag_sink: DiagSink::new(),
            source_lines,
            source_file,
            current_module: None,

            module_cache: HashMap::new(),
            cache_hits: 0,
            cache_misses: 0,
        }
    }

    pub fn elaborate(&mut self, top_module: Option<&str>) -> Result<IrDesign, SimError> {
        let elab_t0 = std::time::Instant::now();
        if std::env::var("DBG_ELAB").is_ok() {
            eprintln!("[DBG-ELAB] elaborate() start (n_modules={})", self.design.modules.len());
        }
        // Build global package param context ONCE — dipakai bersama oleh semua
        // module. Sebelumnya dihitung ulang per-modul (rescan semua package +
        // fixed-point 64 iterasi) yang menjadi bottleneck di desain besar.
        self.build_pkg_param_ctx();
        if std::env::var("DBG_ELAB").is_ok() {
            let n_arr_elems: usize = self.pkg_const_arrays.values().map(|v| v.len()).sum();
            eprintln!("[DBG-ELAB] global package param ctx built in {:?} ({} entries; scalars={} arrays={} array_elems={})", elab_t0.elapsed(), self.pkg_param_ctx.len(), self.pkg_const_scalars.len(), self.pkg_const_arrays.len(), n_arr_elems);
        }
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
                self.elab_warn(DiagCode::ModuleNotFound, format!("bind target '{}' not found", bind.target));
            }
        }

        // Pre-pass: import package functions/tasks into modules + inject $unit declarations
        let pkg_symbols = &self.package_symbols;
        let unit_funcs = &self.design.unit_funcs;
        let unit_tasks = &self.design.unit_tasks;
        let unit_imports = &self.design.unit_imports;
        for module in &mut self.design.modules {
            // Collect module-level imports
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
            // Merge with $unit-level imports
            let all_imports: Vec<(Symbol, Symbol)> = {
                let mut imps = imports;
                for (pkg, item) in unit_imports {
                    if !imps.iter().any(|(p, i)| p == pkg && i == item) {
                        imps.push((*pkg, *item));
                    }
                }
                imps
            };
            // Process package imports
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
                                            name: t.name,
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
        // Resolve type parameter widths from module's param declarations and overrides
        // moved to elaborate_module_with_params_and_type
                    }
                }
            }
            // Inject $unit function/task declarations
            for func in unit_funcs {
                if !module
                    .items
                    .iter()
                    .any(|mi| matches!(mi, ModuleItem::Func(fd) if fd.name == func.name))
                {
                    module.items.push(ModuleItem::Func(func.clone()));
                }
            }
            for task in unit_tasks {
                if !module
                    .items
                    .iter()
                    .any(|mi| matches!(mi, ModuleItem::Func(fd) if fd.name == task.name))
                {
                    module.items.push(ModuleItem::Func(FunctionDecl {
                        name: task.name,
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

        if std::env::var("DBG_ELAB").is_ok() {
            eprintln!("[DBG-ELAB] bind+import prepass done in {:?}", elab_t0.elapsed());
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
                                msb: Expr::Value(crate::ast::expr::Value::Decimal(
                                    (width - 1) as i64,
                                )),
                                lsb: Expr::Value(crate::ast::expr::Value::Decimal(0)),
                            })
                        } else {
                            None
                        },
                        array_range: None,
                        array_size_expr: None,

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

        if std::env::var("DBG_ELAB").is_ok() {
            eprintln!("[DBG-ELAB] inline done in {:?}", elab_t0.elapsed());
        }
        // Expand generates in all modules (with resolved params)
        for i in 0..self.design.modules.len() {
            let mod_t0 = std::time::Instant::now();
            let ctx = self.collect_package_param_ctx(&self.design.modules[i]);
            let ctx_ms = mod_t0.elapsed().as_millis();
            let param_vals =
                resolve_param_values_with_ctx(&self.design.modules[i], &HashMap::new(), &ctx)?;
            let resolve_ms = mod_t0.elapsed().as_millis();
            let module_name = self.design.modules[i].name;
            self.current_module = Some(module_name);
            if std::env::var("DBG_ELAB").is_ok() {
                eprintln!("[DBG-ELAB] expanding generates in module '{}' ({}/{}) ctx={}ms resolve={}ms", module_name.as_str(), i + 1, self.design.modules.len(), ctx_ms, resolve_ms);
            }
            // Process generate expansion in isolated block to release mutable borrow before elab_diag_at
            let gen_result = {
                let module = &mut self.design.modules[i];
                expand_all_generates(module, &param_vals, &self.diag_sink)
            };
            if let Err(e) = gen_result {
                return Err(self.elab_diag_at(DiagCode::ModuleNotFound, format!("generate expansion failed in '{}': {}", module_name, e.msg), e.line, e.col));
            }
        }

        if std::env::var("DBG_ELAB").is_ok() {
            eprintln!("[DBG-ELAB] generate expansion done in {:?}", elab_t0.elapsed());
        }
        // Dead module detection: find unreachable modules via reachability from top
        let top_sym = top_module.map(Symbol::intern);
        {
            use std::collections::{HashSet, VecDeque};
            let module_map: HashMap<Symbol, &Module> =
                self.design.modules.iter().map(|m| (m.name, m)).collect();
            let all_names: HashSet<Symbol> = module_map.keys().copied().collect();
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
                if let Some(module) = module_map.get(&name) {
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
                    self.elab_warn(DiagCode::UnusedSignal, format!("module '{}' is unreachable (not instantiated from top)", m.name));
                }
            }
        }

        if std::env::var("DBG_ELAB").is_ok() {
            eprintln!("[DBG-ELAB] dead-module detection done in {:?}", elab_t0.elapsed());
        }
        // ── Incremental elaboration pass ──
        // 1. Compute structural checksums for all modules
        // 2. Compute topological order (children before parents)
        // 3. Compute dependency-aware signatures
        // 4. Check cache before each elaborate_module()

        let module_names: Vec<Symbol> =
            self.design.modules.iter().map(|m| m.name).collect();

        // Phase A: Compute structural checksums from design.modules directly (no clone needed)
        let mut struct_sigs: HashMap<Symbol, u64> = HashMap::new();
        for module in &self.design.modules {
            let sig = self.compute_module_checksum(module);
            struct_sigs.insert(module.name, sig);
        }

        // Phase B: Compute topological order from design.modules directly
        let topo_order = self.compute_topo_order(&self.design.modules, &module_names);

        // Phase C: Clone modules for elaboration (avoids borrow conflict with &mut self)
        let modules_snapshot: Vec<Module> = self.design.modules.clone();
        let snapshot_map: HashMap<Symbol, &Module> =
            modules_snapshot.iter().map(|m| (m.name, m)).collect();

        let mut dep_sigs: HashMap<Symbol, u64> = HashMap::new();

        if std::env::var("DBG_ELAB").is_ok() {
            eprintln!("[DBG-ELAB] checksums+topo done in {:?} (topo_len={})", elab_t0.elapsed(), topo_order.len());
        }
        for &mod_name in &topo_order {
            let module = snapshot_map.get(&mod_name)
                .ok_or_else(|| self.elab_diag(DiagCode::ModuleNotFound,
                    format!("module '{}' not found in snapshot", mod_name)))?;

            let structural = struct_sigs.get(&mod_name).copied().unwrap_or(0);

            // Combine with dependency (child) signatures
            let mut dep_aware = structural;
            for item in &module.items {
                if let ModuleItem::Instance(inst) = item {
                    if let Some(child_sig) = dep_sigs.get(&inst.module_name) {
                        dep_aware = crate::cache::checksum::combine_checksum(dep_aware, *child_sig);
                    }
                }
            }
            dep_sigs.insert(mod_name, dep_aware);

            // Check cache: if dependency-aware signature matches, skip elaboration
            if let Some(cached_ir) = self.module_cache.get(&dep_aware) {
                self.modules.insert(mod_name, cached_ir.clone());
                self.cache_hits += 1;
            } else {
                match self.elaborate_module(module, &module_names) {
                    Ok(ir) => {
                        self.module_cache.insert(dep_aware, ir.clone());
                        self.modules.insert(mod_name, ir);
                        self.cache_misses += 1;
                    }
                    Err(e) => {
                        // Module yang gagal elaborasi dilewati dengan warning
                        // (mis. package/type yang dirujuk tidak tersedia), bukan
                        // mematikan seluruh elaborasi.
                        self.elab_warn(
                            DiagCode::ModuleNotFound,
                            format!("module '{}' skipped due to elaboration error: {}", mod_name, e),
                        );
                    }
                }
            }
        }

        if std::env::var("DBG_ELAB").is_ok() {
            eprintln!("[DBG-ELAB] module elaboration loop done in {:?} (hits={} misses={})", elab_t0.elapsed(), self.cache_hits, self.cache_misses);
        }
        // Elaborate interfaces as modules (ports + decls + processes), so
        // interface initial/always/assign blocks actually run inside the
        // flattened hierarchy. Falls back to a signal-only module if the
        // interface body references unsupported constructs.
        let interfaces_snapshot: Vec<Interface> = self.design.interfaces.clone();
        for iface in &interfaces_snapshot {
            let synthetic = Module {
                name: iface.name,
                ports: iface.ports.clone(),
                params: iface.params.clone(),
                decls: iface.decls.clone(),
                items: iface.items.clone(),
            };
            match self.elaborate_module(&synthetic, &module_names) {
                Ok(ir) => {
                    self.modules.insert(iface.name, ir);
                }
                Err(e) => {
                    self.elab_warn(
                        DiagCode::ModuleNotFound,
                        format!(
                            "interface '{}' skipped due to elaboration error: {}",
                            iface.name, e
                        ),
                    );
                    self.modules.insert(
                        iface.name,
                        IrModule {
                            name: iface.name,
                            signals: Vec::new(),
                            inputs: vec![],
                            outputs: vec![],
                            inouts: vec![],
                            processes: vec![],
                            sub_instances: vec![],
                        },
                    );
                }
            }
        }

        if std::env::var("DBG_ELAB").is_ok() {
            eprintln!("[DBG-ELAB] interfaces done in {:?}", elab_t0.elapsed());
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

        let mut top = match self.modules.remove(&top_name) {
            Some(m) => m,
            None => {
                // Top otomatis (tanpa --top) gagal elaborasi. Fallback ke modul
                // pertama yang BERHASIL elaborasi agar simulasi tetap berjalan;
                // module yang gagal dilewati dengan warning. Untuk top eksplisit
                // tetap error agar tidak menyesatkan pengguna.
                let fallback = self
                    .design
                    .modules
                    .iter()
                    .filter_map(|m| self.modules.get(&m.name).cloned())
                    .next();
                match fallback {
                    Some(fb) if top_module.is_none() => {
                        self.elab_warn(
                            DiagCode::ModuleNotFound,
                            format!(
                                "top module '{}' not elaborated (skipped due to elaboration error); falling back to first elaboratable module '{}'",
                                top_name, fb.name
                            ),
                        );
                        fb
                    }
                    _ => {
                        return Err(self.elab_diag(
                            DiagCode::ModuleNotFound,
                            format!(
                                "top module '{}' not found: no module could be elaborated (all skipped due to elaboration errors)",
                                top_name
                            ),
                        ))
                    }
                }
            }
        };

        if std::env::var("DBG_ELAB").is_ok() {
            eprintln!("[DBG-ELAB] top found in {:?}", elab_t0.elapsed());
        }
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
                    module_functions.insert(f.name, f.clone());
                }
            }
        }

        Ok(IrDesign {
            top,
            modules: std::mem::take(&mut self.modules),
            classes,
            covergroups,
            dpi_imports,
            hier_signal_map,
            udp_defs: self.design.udp_defs.clone(),
            specify_items,
            timescale: self.design.timescale.clone(),
            module_functions,
            source_lines: if self.source_lines.is_empty() {
                None
            } else {
                Some(self.source_lines.clone())
            },
            source_file: if self.source_file.is_empty() {
                None
            } else {
                Some(self.source_file.clone())
            },
            pkg_scoped_consts: std::mem::take(&mut self.pkg_param_ctx),
            coverage_exclusions: Vec::new(),
        })
    }

    // ── Module signature computation (for incremental caching) ──

    /// Compute a structural checksum for a module AST.
    /// Uses Debug formatting of the entire module to capture ALL content:
    /// ports, params, decls, always/initial/assign bodies, function bodies, etc.
    /// Dependency instance names are also included for topological signature combining.
    fn compute_module_checksum(&self, module: &Module) -> u64 {
        use crate::cache::checksum::{combine_checksum, compute_str_checksum, compute_checksum};

        // Hash structural fields instead of Debug-formatting the entire AST
        let mut h = compute_str_checksum(module.name.as_str());
        h = combine_checksum(h, compute_checksum(&(module.ports.len() as u64).to_le_bytes()));
        for port in &module.ports {
            h = combine_checksum(h, compute_str_checksum(port.name.as_str()));
            h = combine_checksum(h, compute_checksum(&[(port.direction.clone() as u8)]));
        }
        for param in &module.params {
            h = combine_checksum(h, compute_str_checksum(param.name.as_str()));
        }
        for item in &module.items {
            match item {
                ModuleItem::Instance(inst) => {
                    h = combine_checksum(h, compute_str_checksum(inst.module_name.as_str()));
                    h = combine_checksum(h, compute_checksum(&(inst.port_conns.len() as u64).to_le_bytes()));
                }
                ModuleItem::Param(p) => {
                    h = combine_checksum(h, compute_str_checksum(p.name.as_str()));
                }
                ModuleItem::Decl(d) => {
                    for v in &d.names {
                        h = combine_checksum(h, compute_str_checksum(v.name.as_str()));
                    }
                }
                ModuleItem::Func(f) => {
                    h = combine_checksum(h, compute_str_checksum(f.name.as_str()));
                }
                ModuleItem::Generate(g) => {
                    h = combine_checksum(h, compute_checksum(&(g.items.len() as u64).to_le_bytes()));
                }
                ModuleItem::Import { package, item } => {
                    h = combine_checksum(h, compute_str_checksum(package.as_str()));
                    h = combine_checksum(h, compute_str_checksum(item.as_str()));
                }
                _ => {}
            }
        }
        h
    }

    /// Compute topological order of modules so children (instantiated) come before parents.
    /// Uses Kahn's algorithm.
    fn compute_topo_order(&self, modules: &[Module], module_names: &[Symbol]) -> Vec<Symbol> {
        use std::collections::{HashMap, HashSet, VecDeque};

        let name_set: HashSet<Symbol> = module_names.iter().copied().collect();
        let mut in_degree: HashMap<Symbol, usize> = HashMap::new();
        let mut dependents: HashMap<Symbol, Vec<Symbol>> = HashMap::new(); // child → [parents]

        for module in modules {
            in_degree.entry(module.name).or_insert(0);
            for item in &module.items {
                if let ModuleItem::Instance(inst) = item {
                    if name_set.contains(&inst.module_name) {
                        // parent depends on child (child must be elaborated first)
                        dependents.entry(inst.module_name).or_default().push(module.name);
                        *in_degree.entry(module.name).or_insert(0) += 1;
                    }
                }
            }
        }

        // Kahn: start with modules that have no dependencies (in_degree == 0)
        let mut queue: VecDeque<Symbol> = VecDeque::new();
        for (&name, &deg) in &in_degree {
            if deg == 0 {
                queue.push_back(name);
            }
        }

        let mut order = Vec::new();
        let mut order_set: HashSet<Symbol> = HashSet::new();
        while let Some(name) = queue.pop_front() {
            order.push(name);
            order_set.insert(name);
            if let Some(children) = dependents.get(&name) {
                for &parent in children {
                    if let Some(deg) = in_degree.get_mut(&parent) {
                        *deg -= 1;
                        if *deg == 0 {
                            queue.push_back(parent);
                        }
                    }
                }
            }
        }

        // Handle cycles: add remaining modules not in order
        for module in modules {
            if !order_set.contains(&module.name) {
                order.push(module.name);
                order_set.insert(module.name);
            }
        }

        order
    }

    fn resolve_param_values(
        &self,
        module: &Module,
        instance_overrides: &HashMap<Symbol, i64>,
    ) -> Result<HashMap<Symbol, i64>, SimError> {
        let ctx = self.collect_package_param_ctx(module);
        resolve_param_values_with_ctx(module, instance_overrides, &ctx)
            .map_err(|e| self.elab_diag(DiagCode::ParamMismatch, e))
    }

    /// Hitung context package global SEKALI: qualified `pkg::name` untuk semua
    /// parameter package + enum member (plain & qualified) dari semua package,
    /// plus konstanta package ter-evaluasi. Hasil disimpan di `self.pkg_param_ctx`
    /// dan di-clone oleh tiap module (lihat `collect_package_param_ctx`).
    fn build_pkg_param_ctx(&mut self) {
        let mut ctx: HashMap<Symbol, i64> = HashMap::new();
        // Enum member constants dari package (plain + qualified, sequential)
        let pkg_enums: Vec<(Symbol, Vec<(Symbol, Option<Expr>)>)> = self
            .package_symbols
            .iter()
            .filter_map(|(pkg_name, items)| {
                let enums: Vec<(Symbol, Option<Expr>)> = items
                    .values()
                    .filter_map(|item| {
                        if let PackageItem::Typedef(td) = item {
                            if let DataType::EnumType { members, .. } = &td.dtype {
                                Some(members.clone())
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    })
                    .flatten()
                    .collect();
                if enums.is_empty() {
                    None
                } else {
                    Some((*pkg_name, enums))
                }
            })
            .collect();
        for _ in 0..64 {
            let mut changed = false;
            // Qualified names untuk SEMUA package (agar scoped reference resolve)
            for (pkg_name, items) in &self.package_symbols {
                for (name, item) in items {
                    let PackageItem::Param(p) = item else { continue };
                    let qualified = Symbol::intern(&format!("{}::{}", pkg_name, name));
                    if ctx.contains_key(&qualified) {
                        continue;
                    }
                    if let Some(expr) = &p.default {
                        if let Ok(val) = const_eval_with_params(expr, &ctx) {
                            ctx.insert(qualified, val);
                            changed = true;
                        }
                    }
                }
            }
            // Enum member constants di package (plain + qualified, sequential)
            for (pkg_name, members) in &pkg_enums {
                let mut last = 0i64;
                for (member_name, member_expr) in members {
                    let val = match member_expr {
                        Some(expr) => match const_eval_with_params(expr, &ctx) {
                            Ok(v) => v,
                            Err(_) => last,
                        },
                        None => last,
                    };
                    if !ctx.contains_key(member_name) {
                        ctx.insert(*member_name, val);
                        changed = true;
                    }
                    let qualified = Symbol::intern(&format!("{}::{}", pkg_name, member_name));
                    if let std::collections::hash_map::Entry::Vacant(e) = ctx.entry(qualified) {
                        e.insert(val);
                        changed = true;
                    }
                    last = val + 1;
                }
            }
            if !changed {
                break;
            }
        }
        // Merge konstanta package yang sudah dievaluasi penuh (skalar + array).
        crate::ast::const_eval_ext::flatten_consts_into_ctx(
            &self.pkg_const_scalars,
            &self.pkg_const_arrays,
            &mut ctx,
        );
        self.pkg_param_ctx = ctx;

        // ── Context $unit imports (unit_imports) — sekali, untuk semua module ──
        // Resolusi plain param names + flatten konstanta untuk import set global.
        let unit_imports: Vec<(Symbol, Symbol)> = self.design.unit_imports.clone();
        let mut unit_ctx: HashMap<Symbol, i64> = self.pkg_param_ctx.clone();
        for _ in 0..64 {
            let mut changed = false;
            for (package, import_item) in &unit_imports {
                let Some(pkg_items) = self.package_symbols.get(package) else {
                    continue;
                };
                let names: Vec<Symbol> = if import_item.as_str() == "*" {
                    pkg_items.keys().copied().collect()
                } else {
                    vec![*import_item]
                };
                for name in names {
                    if let Some(PackageItem::Param(p)) = pkg_items.get(&name) {
                        if unit_ctx.contains_key(&p.name) {
                            continue;
                        }
                        if let Some(expr) = &p.default {
                            if let Ok(val) = const_eval_with_params(expr, &unit_ctx) {
                                unit_ctx.insert(p.name, val);
                                changed = true;
                            }
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }
        for (package, import_item) in &unit_imports {
            crate::ast::const_eval_ext::flatten_imported_consts_into_ctx(
                package.as_str(),
                import_item.as_str(),
                &self.pkg_const_scalars,
                &self.pkg_const_arrays,
                &mut unit_ctx,
            );
        }
        // Hanya simpan DELTA vs pkg_param_ctx — sebagian besar entri sudah sama
        // (qualified names). Collect per-modul tinggal extend delta kecil,
        // menghindari ~33k insert duplikat untuk tiap module.
        let mut delta: HashMap<Symbol, i64> = HashMap::new();
        for (k, v) in &unit_ctx {
            if self.pkg_param_ctx.get(k) != Some(v) {
                delta.insert(*k, *v);
            }
        }
        self.unit_import_ctx = delta;

        // ── Plain param per package — sekali, untuk import milik module ──
        // Base: context global yang sudah lengkap (qualified + unit imports).
        let base: HashMap<Symbol, i64> = self.unit_import_ctx.clone();
        let mut plain_map: HashMap<Symbol, HashMap<Symbol, i64>> = HashMap::new();
        for (pkg_name, items) in &self.package_symbols {
            let mut pctx = base.clone();
            for _ in 0..64 {
                let mut changed = false;
                for (name, item) in items {
                    let PackageItem::Param(p) = item else { continue };
                    if pctx.contains_key(&p.name) {
                        continue;
                    }
                    if let Some(expr) = &p.default {
                        if let Ok(val) = const_eval_with_params(expr, &pctx) {
                            pctx.insert(p.name, val);
                            changed = true;
                        }
                    }
                }
                if !changed {
                    break;
                }
            }
            let mut plain: HashMap<Symbol, i64> = HashMap::new();
            for (name, item) in items {
                let PackageItem::Param(p) = item else { continue };
                if let Some(&v) = pctx.get(&p.name) {
                    plain.insert(*name, v);
                }
            }
            plain_map.insert(*pkg_name, plain);
        }
        self.pkg_plain_params = plain_map;
        if std::env::var("DBG_ELAB").is_ok() {
            let gp = Symbol::intern("gpio_env_pkg");
            let gn = Symbol::intern("NUM_GPIOS");
            let plain_n = self.pkg_plain_params.get(&gp).map(|m| m.len()).unwrap_or(0);
            let has_ng = self.pkg_plain_params.get(&gp).map(|m| m.contains_key(&gn)).unwrap_or(false);
            let pkg_has = self.package_symbols.contains_key(&gp);
            eprintln!("[DBG-ELAB]   pkg_plain_params[gpio_env_pkg] len={} has_NUM_GPIOS={} pkg_present={}", plain_n, has_ng, pkg_has);
        }
    }

    /// Kumpulkan nilai parameter package yang terlihat oleh module via
    /// `import pkg::*`/`import pkg::item` (baik di header module maupun $unit).
    /// Dipakai sebagai base context saat evaluasi konstanta (generate limit,
    /// parameter default, dll.) sehingga package parameter bisa di-resolve.
    /// Package param yang di-referensikan secara scoped (`pkg::name`) juga
    /// didaftarkan agar `const_eval_with_params` bisa me-resolve.
    fn collect_package_param_ctx(&self, module: &Module) -> HashMap<Symbol, i64> {
        let dbg = std::env::var("DBG_ELAB").is_ok();
        let t0 = std::time::Instant::now();
        let mut ctx: HashMap<Symbol, i64> = self.pkg_param_ctx.clone();
        let t1 = std::time::Instant::now();
        if dbg {
            eprintln!("[DBG-ELAB]   collect clone pkg ctx: {:?} (pkg_ctx={})", t1.duration_since(t0), self.pkg_param_ctx.len());
        }
        // Context $unit imports sudah di-precompute (konstan antar-module).
        ctx.extend(self.unit_import_ctx.iter().map(|(k, v)| (*k, *v)));
        let t2 = std::time::Instant::now();
        if dbg {
            eprintln!("[DBG-ELAB]   collect extend unit ctx: {:?} (unit_ctx={})", t2.duration_since(t1), self.unit_import_ctx.len());
        }
        // Import set milik module itu sendiri (di luar $unit).
        let module_imports: Vec<(Symbol, Symbol)> = module
            .items
            .iter()
            .filter_map(|item| {
                if let ModuleItem::Import { package, item } = item {
                    Some((*package, *item))
                } else {
                    None
                }
            })
            .collect();
        // Enum member constants dari typedef di body module (e.g. LastEdnEntry)
        let module_enums: Vec<Vec<(Symbol, Option<Expr>)>> = module
            .items
            .iter()
            .filter_map(|item| {
                if let ModuleItem::Typedef(td) = item {
                    if let DataType::EnumType { members, .. } = &td.dtype {
                        Some(members.clone())
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect();
        // Fixed-point: modul-local enum member & plain import names bisa
        // mereferensikan package param (dan satu sama lain). Plain param
        // package sudah di-precompute (pkg_plain_params); hanya enum modul yang
        // perlu fixed-point.
        let tf = std::time::Instant::now();
        for _ in 0..64 {
            let mut changed = false;
            // Enum member constants dari body module (sequential values)
            for members in &module_enums {
                let mut last = 0i64;
                for (member_name, member_expr) in members {
                    let val = match member_expr {
                        Some(expr) => match const_eval_with_params(expr, &ctx) {
                            Ok(v) => v,
                            Err(_) => last,
                        },
                        None => last,
                    };
                    if !ctx.contains_key(member_name) {
                        ctx.insert(*member_name, val);
                        changed = true;
                    }
                    last = val + 1;
                }
            }
            // Plain names untuk package yang di-import module
            for (package, import_item) in &module_imports {
                let Some(plain) = self.pkg_plain_params.get(package) else {
                    continue;
                };
                if import_item.as_str() == "*" {
                    for (&name, &val) in plain {
                        ctx.entry(name).or_insert(val);
                    }
                } else if let Some(&val) = plain.get(import_item) {
                    ctx.entry(*import_item).or_insert(val);
                }
            }
            if !changed {
                break;
            }
        }
        if dbg && tf.elapsed().as_millis() > 20 {
            eprintln!("[DBG-ELAB]   collect fixed-point: {:?} (module_imports={}, enums={})", tf.elapsed(), module_imports.len(), module_enums.len());
        }
        // Merge konstanta package yang sudah dievaluasi penuh — hanya untuk
        // import set milik module ($unit sudah ada di unit_import_ctx).
        let tl = std::time::Instant::now();
        for (package, import_item) in &module_imports {
            crate::ast::const_eval_ext::flatten_imported_consts_into_ctx(
                package.as_str(),
                import_item.as_str(),
                &self.pkg_const_scalars,
                &self.pkg_const_arrays,
                &mut ctx,
            );
        }
        if dbg && module.name.as_str() == "rv_core_ibex_peri" {
            let gn = Symbol::intern("NumRegions");
            let ga = Symbol::intern("NumAlerts");
            eprintln!("[DBG-ELAB]   peri imports={:?}", module_imports.iter().map(|(p, i)| format!("{}::{}", p.as_str(), i.as_str())).collect::<Vec<_>>());
            eprintln!("[DBG-ELAB]   peri has_pkg_reg={} NumRegions_in_ctx={} NumAlerts_in_ctx={} ctx_len={}", self.package_symbols.contains_key(&Symbol::intern("rv_core_ibex_reg_pkg")), ctx.contains_key(&gn), ctx.contains_key(&ga), ctx.len());
            eprintln!("[DBG-ELAB]   pkg_plain_params has reg_pkg={}", self.pkg_plain_params.contains_key(&Symbol::intern("rv_core_ibex_reg_pkg")));
        }
        if dbg && module.name.as_str() == "tb" {
            let gn = Symbol::intern("NUM_GPIOS");
            eprintln!("[DBG-ELAB]   tb imports={:?} NUM_GPIOS_in_ctx={}", module_imports.iter().map(|(p, i)| format!("{}::{}", p.as_str(), i.as_str())).collect::<Vec<_>>(), ctx.contains_key(&gn));
        }
        if dbg && tl.elapsed().as_millis() > 20 {
            eprintln!("[DBG-ELAB]   collect flatten-module: {:?}", tl.elapsed());
        }
        if dbg && t0.elapsed().as_millis() > 50 {
            eprintln!("[DBG-ELAB]   collect total: {:?}", t0.elapsed());
        }
        ctx
    }

    /// Buat structured diagnostic untuk elaboration error dengan error code tepat.
    fn elab_diag(&self, code: DiagCode, message: impl Into<String>) -> SimError {
        self.elab_diag_at(code, message, 0, 0)
    }

    /// Extract source file name from `line directives in source_lines for a given line.
    /// The source_lines contain `` `line 1 "filename.sv" `` directives from preprocessing.
    /// Resolve nama file + baris relatif-file untuk posisi di merged source.
    fn resolve_source_location(&self, line: usize) -> (String, usize) {
        crate::diagnostics::resolve_source_location(&self.source_lines, &self.source_file, line)
    }

    /// Buat error diagnostic dengan posisi source.
    fn elab_diag_at(&self, code: DiagCode, message: impl Into<String>, line: usize, col: usize) -> SimError {
        let msg: String = message.into();
        let mut diag = Diagnostic::new(DiagLevel::Error, code, msg)
            .with_code_context();
        if line > 0 && line <= self.source_lines.len() {
            let source_line = &self.source_lines[line - 1];
            let (file, display_line) = self.resolve_source_location(line);
            let snippet = SourceSnippet::new(file, display_line, col, source_line);
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
        self.elab_warn_at(code, message, 0, 0)
    }

    /// Emit warning dengan posisi source.
    fn elab_warn_at(&self, code: DiagCode, message: impl Into<String>, line: usize, col: usize) {
        let msg: String = message.into();
        let mut diag = Diagnostic::new(DiagLevel::Warning, code, msg)
            .with_code_context();
        if line > 0 && line <= self.source_lines.len() {
            let source_line = &self.source_lines[line - 1];
            let (file, display_line) = self.resolve_source_location(line);
            let snippet = SourceSnippet::new(file, display_line, col, source_line);
            diag = diag.with_source_snippet(snippet);
        }
        self.diag_sink.push(diag);
    }

    /// Flush diagnostics from DiagSink and return them.
    pub fn flush_diagnostics(&self) -> Vec<Diagnostic> {
        self.diag_sink.diagnostics()
    }

    /// Prime the module cache from an existing cache (session-level persistence).
    pub fn set_cache(&mut self, cache: HashMap<u64, IrModule>) {
        self.module_cache = cache;
    }

    /// Take the module cache out for session-level storage after elaboration.
    pub fn take_cache(&mut self) -> HashMap<u64, IrModule> {
        std::mem::take(&mut self.module_cache)
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
                for pkg_items in self.package_symbols.values() {
                    if let Some(PackageItem::Typedef(td)) = pkg_items.get(name.as_str()) {
                        let width = self.resolve_typedef_width(&td.dtype, td.range.as_ref());
                        if width > 0 {
                            return Ok(width);
                        }
                    }
                }
                // Scoped type name `pkg::type` — cari di package yang tepat.
                // Nama disimpan sebagai "pkg::type", sedangkan key package
                // symbols hanya berisi nama tipe tanpa prefix.
                if let Some((pkg, type_name)) = name.as_str().split_once("::") {
                    if let Some(pkg_items) = self.package_symbols.get(pkg) {
                        if let Some(PackageItem::Typedef(td)) = pkg_items.get(type_name) {
                            let width = self.resolve_typedef_width(&td.dtype, td.range.as_ref());
                            if width > 0 {
                                return Ok(width);
                            }
                        }
                    }
                }
                // Check in-module typedefs stored in typedef_map
                if let Some(&width) = self.typedef_map.get(name) {
                    return Ok(width);
                }
                // Type not found — warn and return default width of 32 (common SV default)
                self.elab_warn(DiagCode::UninitializedRegister, format!("unknown type '{}' is not defined in this scope (using default width 32)", name));
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
                .map(|m| Self::struct_field_from_member(m, 0))
                .collect(),
            DataType::StructType { members } => {
                let mut fields = Vec::new();
                let mut offset = 0usize;
                let members_rev: Vec<_> = members.iter().rev().collect();
                for m in &members_rev {
                    fields.push(Self::struct_field_from_member(m, offset));
                    offset += m.range.as_ref().map(|r| r.width()).unwrap_or(1);
                }
                fields.reverse();
                fields
            }
            _ => vec![],
        }
    }

    /// Bangun satu `StructFieldInfo` dari member struct/union. Untuk field
    /// bertipe typedef (`UserDefined`) simpan `type_name` agar chain bisa
    /// resolve lewat typedef_field_map; untuk anonymous struct/union inline
    /// simpan `sub_fields` (dari compute_struct_fields) agar `a.b.c` tetap
    /// bisa di-resolve berjenjang tanpa nama tipe.
    fn struct_field_from_member(m: &StructMember, offset: usize) -> StructFieldInfo {
        let w = m.range.as_ref().map(|r| r.width()).unwrap_or(1);
        match m.dtype.as_ref() {
            DataType::UserDefined(t) => StructFieldInfo {
                name: m.name,
                offset,
                width: w,
                type_name: Some(*t),
                sub_fields: vec![],
            },
            DataType::StructType { .. } | DataType::UnionType { .. } => StructFieldInfo {
                name: m.name,
                offset,
                width: w,
                // Anonymous struct/union inline — tidak ada nama tipe untuk
                // lookup typedef_field_map; simpan fields langsung.
                type_name: None,
                sub_fields: Self::compute_struct_fields(m.dtype.as_ref()),
            },
            _ => StructFieldInfo {
                name: m.name,
                offset,
                width: w,
                type_name: None,
                sub_fields: vec![],
            },
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
        let module_idx: HashMap<Symbol, usize> =
            self.design.modules.iter().enumerate().map(|(i, m)| (m.name, i)).collect();

        // Process $unit parameters (top-level param declarations)
        for param in &self.design.unit_params {
            if !effective_params.contains_key(&param.name) {
                if let Some(expr) = &param.default {
                    if let Ok(val) = const_eval_with_params(expr, &effective_params) {
                        effective_params.insert(param.name, val);
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
                                        effective_params.insert(p.name, val);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Process package imports: add package params + typedefs, collect in-module typedefs
        for item in &module.items {
            match item {
                ModuleItem::Import {
                    package,
                    item: import_item,
                } => {
                    if let Some(pkg_items) = self.package_symbols.get(package) {
                        let names: Vec<&str> = if import_item.as_str() == "*" {
                            pkg_items.keys().map(|s| s.as_str()).collect()
                        } else {
                            vec![import_item.as_str()]
                        };
                        let mut struct_imports: Vec<(Symbol, DataType)> = Vec::new();
                        for name in names {
                            if let Some(pkg_item) = pkg_items.get(name) {
                                match pkg_item {
                                    PackageItem::Param(p) => {
                                        if !effective_params.contains_key(&p.name) {
                                            if let Some(expr) = &p.default {
                                                if let Ok(val) = const_eval_with_params(expr, &effective_params) {
                                                    effective_params.insert(p.name, val);
                                                }
                                            }
                                        }
                                    }
                                    PackageItem::Typedef(td) => {
                                        if !self.typedef_map.contains_key(&td.name) {
                                            let width = self.resolve_typedef_width(&td.dtype, td.range.as_ref());
                                            self.typedef_map.insert(td.name, width);
                                        }
                                        if matches!(&td.dtype, DataType::StructType { .. } | DataType::UnionType { .. }) {
                                            struct_imports.push((td.name, td.dtype.clone()));
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                        for (name, dtype) in struct_imports {
                            self.store_typedef_fields(name, &dtype);
                        }
                    }
                }
                ModuleItem::Typedef(td) => {
                    let width = self.resolve_typedef_width(&td.dtype, td.range.as_ref());
                    self.typedef_map.insert(td.name, width);
                    if matches!(&td.dtype, DataType::StructType { .. } | DataType::UnionType { .. }) {
                        self.store_typedef_fields(td.name, &td.dtype);
                    }
                }
                _ => {}
            }
        }
        // Merge konstanta package yang sudah dievaluasi (skalar + array) ke effective_params.
        crate::ast::const_eval_ext::flatten_consts_into_ctx(
            &self.pkg_const_scalars,
            &self.pkg_const_arrays,
            &mut effective_params,
        );
        let mut unit_import_sets = self.design.unit_imports.clone();
        for item in &module.items {
            if let ModuleItem::Import { package, item: import_item } = item {
                unit_import_sets.push((*package, *import_item));
            }
        }
        for (package, import_item) in &unit_import_sets {
            crate::ast::const_eval_ext::flatten_imported_consts_into_ctx(
                package.as_str(),
                import_item.as_str(),
                &self.pkg_const_scalars,
                &self.pkg_const_arrays,
                &mut effective_params,
            );
        }
        self.param_vals = effective_params.clone();
        // Pre-pass: process $unit typedefs (top-level typedefs outside any module)
        let unit_typedefs = self.design.unit_typedefs.clone();
        for td in &unit_typedefs {
            let width = self.resolve_typedef_width(&td.dtype, td.range.as_ref());
            self.typedef_map.insert(td.name, width);
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
                            self.typedef_map.insert(td.name, width);
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
                                Some((td.name, td.dtype.clone()))
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
                                    Some((td.name, td.dtype.clone()))
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
                self.typedef_field_map.entry(*name).or_insert(fields);
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
                type_param_widths.insert(param.name, width);
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
            let (p_msb, p_lsb) = if !port.extra_packed_dims.is_empty() {
                // Packed multi-dimensi: signal flat mewakili SELURUH array → [width-1:0].
                (width.saturating_sub(1), 0)
            } else if let Some(r) = &port.range {
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
            // Unpacked array port: lipat dimensi array ke lebar total.
            let (array_depth, total_width, port_msb, port_lsb) =
                if let Some(ar) = &port.array_range {
                    let depth = if ar.msb >= ar.lsb {
                        ar.msb - ar.lsb + 1
                    } else {
                        ar.lsb - ar.msb + 1
                    };
                    (depth, width * depth, width * depth - 1, 0)
                } else {
                    (1, width, p_msb, p_lsb)
                };
            let sid = get_or_create_signal(
                port.name,
                total_width,
                kind.clone(),
                net_type,
                &mut signals,
                &mut signal_map,
                &mut next_id,
                array_depth,
                width,
                port_msb,
                port_lsb,
                false,
                false,
            );
            match port.direction {
                PortDirection::Input => inputs.push(sid),
                PortDirection::Output => outputs.push(sid),
                PortDirection::Inout => inouts.push(sid),
                PortDirection::Ref => inouts.push(sid),
            }
            // Struct-typed port: isi struct_fields agar member access
            // (`hw2reg.val.d = ...`) bisa di-resolve sebagai lvalue. dtype_name
            // bisa berformat `pkg::type` (scoped) atau nama typedef biasa.
            if let Some(tn) = &port.dtype_name {
                let lookup = tn.as_str();
                let resolved: Option<Vec<StructFieldInfo>> = if let Some((_, t)) =
                    lookup.split_once("::")
                {
                    self.typedef_field_map.get(t).cloned()
                } else {
                    self.typedef_field_map.get(tn).cloned()
                };
                if let Some(fields) = resolved {
                    if !fields.is_empty() {
                        let sig = &mut signals[sid];
                        sig.struct_fields = fields;
                    }
                }
            }
            // Set packed_dims untuk port packed multi-dimensi (`[a:b][c:d] name`)
            // agar akses elemen `name[k]` benar (via RangeSelect).
            if !port.extra_packed_dims.is_empty() {
                let mut pd = Vec::with_capacity(port.extra_packed_dims.len() + 1);
                let base_w = if let Some(r) = &port.range {
                    r.width()
                } else if let Some(er) = &port.expr_range {
                    resolve_expr_range(er, &effective_params)
                        .map(|r| r.width())
                        .unwrap_or(1)
                } else {
                    1
                };
                pd.push(base_w);
                for er in &port.extra_packed_dims {
                    if let Ok(r) = resolve_expr_range(er, &effective_params) {
                        pd.push(r.width());
                    }
                }
                let sig = &mut signals[sid];
                sig.packed_dims = pd;
            }
        }

        // ── Generate expansion (SEBELUM pemrosesan deklarasi) ──
        // Expand generate blocks in module items — dilakukan lebih awal agar
        // deklarasi yang dihasilkan branch generate (mis. `logic wptr_err;`
        // saat Secure=1) ikut terdaftar sebagai sinyal. Tanpa ini, sinyal
        // seperti itu hilang → error "signal not found" (E2001).
        // Collect body-level params (localparam, parameter) into effective_params
        for item in &module.items {
            if let ModuleItem::Param(p) = item {
                if !effective_params.contains_key(&p.name) {
                    if let Some(expr) = &p.default {
                        if let Ok(val) = const_eval_with_params(expr, &effective_params) {
                            effective_params.insert(p.name, val);
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
                        let expanded = expand_generate_block(gen, &effective_params, &self.diag_sink)
                            .map_err(|e| self.elab_diag_at(DiagCode::InvalidSyntax, format!("generate block expansion failed: {}", e.msg), e.line, e.col))?;
                        // Collect params from expanded generate items too
                        for ei in &expanded {
                            if let ModuleItem::Param(p) = ei {
                                if !effective_params.contains_key(&p.name) {
                                    if let Some(expr) = &p.default {
                                        if let Ok(val) =
                                            const_eval_with_params(expr, &effective_params)
                                        {
                                            effective_params.insert(p.name, val);
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

        // Gabungkan deklarasi module-level dengan deklarasi hasil generate.
        // Parser menaruh deklarasi normal di module.decls DAN module.items,
        // jadi dedup by nama (get_or_create_signal juga idempoten).
        let mut seen_names: std::collections::HashSet<Symbol> = std::collections::HashSet::new();
        let mut all_decls: Vec<Decl> = Vec::new();
        for decl in &module.decls {
            let mut new_vars = Vec::new();
            for var in &decl.names {
                if seen_names.insert(var.name) {
                    new_vars.push(var.clone());
                }
            }
            if !new_vars.is_empty() {
                all_decls.push(Decl {
                    dtype: decl.dtype.clone(),
                    kind: decl.kind.clone(),
                    names: new_vars,
                });
            }
        }
        for item in &expanded_items {
            if let ModuleItem::Decl(d) = item {
                let mut new_vars = Vec::new();
                for var in &d.names {
                    if seen_names.insert(var.name) {
                        new_vars.push(var.clone());
                    }
                }
                if !new_vars.is_empty() {
                    all_decls.push(Decl {
                        dtype: d.dtype.clone(),
                        kind: d.kind.clone(),
                        names: new_vars,
                    });
                }
            }
        }

        // Process declarations with parameter-aware width resolution
        for decl in &all_decls {
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
                    signal_map.insert(var.name, sid);
                    signals.push(SignalInfo {
                        name: var.name,
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
                    signal_map.insert(var.name, sid);
                    signals.push(SignalInfo {
                        name: var.name,
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
                // Unpacked array dimension: prioritas `array_range` (sudah
                // di-resolve saat parse); fallback ke `array_size_expr` yang
                // bisa memuat parameter (`logic [N-1:0] arr [Width]`).
                let resolved_arr = var.array_range.clone().or_else(|| {
                    var.array_size_expr.as_ref().and_then(|sz| {
                        const_eval_params(sz, &effective_params)
                            .ok()
                            .filter(|n| *n > 0)
                            .map(|n| Range {
                                msb: (n - 1) as usize,
                                lsb: 0,
                            })
                    })
                });
                if let Some(ar) = &resolved_arr {
                    let depth = if ar.msb >= ar.lsb {
                        ar.msb - ar.lsb + 1
                    } else {
                        ar.lsb - ar.msb + 1
                    };
                    let total_width = elem_width * depth;
                    let sid = get_or_create_signal(
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
                    let sig = &mut signals[sid];
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
                } else {
                    let sid = get_or_create_signal(
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
                    let sig = &mut signals[sid];
                    if let Some(class) = &class_name {
                        sig.class_name = Some(Symbol::intern(class));
                        if class == "__mailbox" {
                            sig.is_mailbox = true;
                        }
                        if class == "__semaphore" {
                            sig.is_semaphore = true;
                        }
                    }
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
                // Compute struct/union field offsets for member access
                if let Some(&sid) = signal_map.get(&var.name) {
                    let sig = &mut signals[sid];
                    match &decl.dtype {
                        DataType::StructType { members } | DataType::UnionType { members } => {
                            match &decl.dtype {
                                DataType::UnionType { members } => {
                                    for m in members {
                                        sig.struct_fields
                                            .push(Self::struct_field_from_member(m, 0));
                                    }
                                }
                                _ => {
                                    let mut offset = 0usize;
                                    let members_rev: Vec<_> = members.iter().rev().collect();
                                    for m in &members_rev {
                                        sig.struct_fields
                                            .push(Self::struct_field_from_member(m, offset));
                                        offset += m.range.as_ref().map(|r| r.width()).unwrap_or(1);
                                    }
                                    sig.struct_fields.reverse();
                                }
                            }
                        }
                        DataType::UserDefined(name) => {
                            if let Some(fields) = self.typedef_field_map.get(name) {
                                if !fields.is_empty() {
                                    sig.struct_fields = fields.clone();
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        // Process module items
        let mut proc_counter = 0usize;
        for item in &expanded_items {
            match item {
                ModuleItem::Always(always) => {
                    let process = self.elaborate_always(always, &signal_map, &signals)?;
                    processes.push(process);
                }
                ModuleItem::Initial(initial) => {
                    let body = self.elaborate_stmt_block(
                        &initial.stmts,
                        &signal_map,
                        known_modules,
                        &signals,
                    )?;
                    let name = format_sym(b"initial_", proc_counter);
                    proc_counter += 1;
                    processes.push(Process::Initial { name, body });
                }
                ModuleItem::Final(final_block) => {
                    let body = self.elaborate_stmt_block(
                        &final_block.stmts,
                        &signal_map,
                        known_modules,
                        &signals,
                    )?;
                    let name = format_sym(b"final_", proc_counter);
                    proc_counter += 1;
                    processes.push(Process::Final { name, body });
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
                    let sensitivity = collect_sensitivity(&assign.rhs, &signal_map)
                        .into_iter()
                        .map(SignalSensitivity::whole)
                        .collect();
                    processes.push(Process::Combinational {
                        name: format_sym(b"assign_", proc_counter),
                        sensitivity,
                        body: stmts,
                    });
                    proc_counter += 1;
                }
                ModuleItem::Typedef(td) => {
                    // Already collected in pre-pass; register for UserDefined resolution
                    let width = self.typedef_map.get(&td.name).copied().unwrap_or_else(|| {
                        self.resolve_typedef_width(&td.dtype, td.range.as_ref())
                    });
                    self.typedef_map.insert(td.name, width);
                    // Store struct/union field info for member access
                    match &td.dtype {
                        DataType::StructType { members } | DataType::UnionType { members } => {
                            let mut fields = Vec::new();
                            match &td.dtype {
                                DataType::UnionType { members } => {
                                    for m in members {
                                        fields.push(Self::struct_field_from_member(m, 0));
                                    }
                                }
                                _ => {
                                    let mut offset = 0usize;
                                    let members_rev: Vec<_> = members.iter().rev().collect();
                                    for m in &members_rev {
                                        fields.push(Self::struct_field_from_member(m, offset));
                                        offset += m.range.as_ref().map(|r| r.width()).unwrap_or(1);
                                    }
                                    fields.reverse();
                                }
                            }
                            self.typedef_field_map.insert(td.name, fields);
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
                        let mut sensitivity: Vec<SignalSensitivity> =
                            in_ids.iter().map(|&id| SignalSensitivity::whole(id)).collect();
                        if udp.is_sequential {
                            sensitivity.push(SignalSensitivity::whole(out_id));
                        }
                        let process = Process::Combinational {
                            name: Symbol::intern(&format!("udp_{}_{}", udp.name, inst.instance_name)),
                            sensitivity: sensitivity.clone(),
                            body: vec![IrStmt::BlockingAssign {
                                lhs: IrLValue::Signal(out_id, 0),
                                rhs: IrExpr::UdpLookup {
                                    udp_name: udp.name,
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
                        let target_module: Option<&Module> = module_idx.get(&inst.module_name)
                            .and_then(|&i| self.design.modules.get(i));
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
                                            port_map.insert(port.name, sig_id);
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
                                    port_map.insert(*port, sig_id);
                                }
                            }
                        }
                        // Resolve parameter overrides to integer values
                        let mut param_map = HashMap::new();
                        for (pname, pexpr) in &inst.param_assigns {
                            let val = const_eval_with_params(pexpr, &effective_params).unwrap_or(0);
                            param_map.insert(*pname, val);
                        }
                        let mut type_param_map: HashMap<Symbol, usize> = HashMap::new();
                        for (pname, dt) in &inst.type_param_assigns {
                            type_param_map.insert(*pname, dt.width());
                        }

                        if let Some(range) = &inst.range {
                            let msb = const_eval_with_params(&range.msb, &effective_params)?;
                            let lsb = const_eval_with_params(&range.lsb, &effective_params)?;
                            let (start, end) = if msb >= lsb { (lsb, msb) } else { (msb, lsb) };
                            let pm = std::sync::Arc::new(port_map);
                            let pam = std::sync::Arc::new(param_map);
                            let tpam = std::sync::Arc::new(type_param_map);
                            for idx in start..=end {
                                let inst_name = format!("{}[{}]", inst.instance_name, idx);
                                sub_instances.push(IrInstance {
                                    module_name: inst.module_name,
                                    instance_name: Symbol::intern(&inst_name),
                                    port_map: pm.clone(),
                                    param_map: pam.clone(),
                                    type_param_map: tpam.clone(),
                                    line: inst.line,
                                    col: inst.col,
                                });
                            }
                        } else {
                            sub_instances.push(IrInstance {
                                module_name: inst.module_name,
                                instance_name: inst.instance_name,
                                port_map: std::sync::Arc::new(port_map),
                                param_map: std::sync::Arc::new(param_map),
                                type_param_map: std::sync::Arc::new(type_param_map),
                                line: inst.line,
                                col: inst.col,
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
                    signal_map.insert(*vif_name, sid);
                    signals.push(SignalInfo {
                        name: *vif_name,
                        width: 64,
                        kind: SignalKind::Reg,
                        net_type: NetType::Wire,
                        multi_driver: false,
                        init_val: LogicVec::new(64),
                        array_depth: 1,
                        elem_width: 64,
                        array_dims: vec![],
                        class_name: Some(*iface_type),
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
                        iface_type: Some(*iface_type),
                        iface_modport: *modport,
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
                        let gate_sens: Vec<SignalSensitivity> = in_ids
                            .iter()
                            .map(|&id| SignalSensitivity::whole(id))
                            .collect();
                        let process = Process::Combinational {
                            name: Symbol::intern(&format!("gate_{}", out_id)),
                            sensitivity: gate_sens,
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
        for decl in &all_decls {
            for var in &decl.names {
                if let Some(init_expr) = &var.expr {
                    let lhs = self.elaborate_lvalue(
                        &Expr::Ident { name: var.name, line: 0, col: 0 },
                        &signal_map,
                        &signals,
                    )?;
                    let rhs = self.elaborate_expr(init_expr, &signal_map, &signals)?;
                    if decl.kind.is_net() {
                        let sensitivity = collect_sensitivity(init_expr, &signal_map)
                            .into_iter()
                            .map(SignalSensitivity::whole)
                            .collect();
                        processes.push(Process::Combinational {
                            name: format_sym(b"decl_assign_", proc_counter),
                            sensitivity,
                            body: vec![IrStmt::BlockingAssign {
                                lhs,
                                rhs,
                                delay: None,
                            }],
                        });
                        proc_counter += 1;
                    } else {
                        processes.push(Process::Initial {
                            name: format_sym(b"decl_init_", proc_counter),
                            body: vec![IrStmt::BlockingAssign {
                                lhs,
                                rhs,
                                delay: None,
                            }],
                        });
                        proc_counter += 1;
                    }
                }
            }
        }

        Ok(IrModule {
            name: module.name,
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
                            name: dv.name,
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
                        name: fd.name,
                        is_task: false,
                        virtual_flag: fd.virtual_flag,
                        is_static: fd.is_static,
                        ports: fd.ports.clone(),
                        decls: fd.decls.clone(),
                        stmts: fd.stmts.clone(),
                    }),
                    ClassMember::Task(td) => Some(IrClassMethod {
                        name: td.name,
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
                            if seen.insert(f.name) {
                                merged.push(f.clone());
                            }
                        }
                    }
                }
                for f in &fields {
                    if seen.insert(f.name) {
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
                cd.name,
                IrClassDef {
                    name: cd.name,
                    extends: cd.extends,
                    type_params: cd
                        .type_params
                        .iter()
                        .map(|tp| IrTypeParam {
                            name: tp.name,
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
