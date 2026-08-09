use super::super::SimulationEngine;
use crate::diagnostics::DiagCode;
use crate::error::SimError;
use crate::ir::*;
use crate::simulator::util::string_to_logicvec;
use crate::Symbol;

impl SimulationEngine {
    pub(crate) fn find_phase_class_name(&self) -> Option<String> {
        let phase_methods = [
            "build_phase",
            "connect_phase",
            "end_of_elaboration_phase",
            "start_of_simulation_phase",
            "run_phase",
            "extract_phase",
            "check_phase",
            "report_phase",
            "final_phase",
        ];
        let mut best: Option<(String, usize)> = None;
        for (name, cls) in &self.design.classes {
            if !self.is_uvm_test_hierarchy(name.as_str()) {
                continue;
            }
            let count = phase_methods
                .iter()
                .filter(|pm| cls.methods.iter().any(|m| &m.name == *pm))
                .count();
            if count > 0 && best.as_ref().is_none_or(|b| count > b.1) {
                best = Some((name.to_string(), count));
            }
        }
        // fallback: any class with phase methods
        if best.is_none() {
            for (name, cls) in &self.design.classes {
                let count = phase_methods
                    .iter()
                    .filter(|pm| cls.methods.iter().any(|m| &m.name == *pm))
                    .count();
                if count > 0 && best.as_ref().is_none_or(|b| count > b.1) {
                    best = Some((name.to_string(), count));
                }
            }
        }
        best.map(|(name, _)| name)
    }

    pub(crate) fn is_uvm_test_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_test" {
                return true;
            }
            match self.design.classes.get::<str>(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    pub(crate) fn execute_phases(&mut self) -> Result<(), SimError> {
        // F18: guard — bila run_test() sudah menjalankan fase (jalur user
        // eksplisit), auto-detect tidak perlu/ tidak boleh jalan lagi.
        if self.uvm_phases_started {
            return Ok(());
        }
        let class_name = match self.find_phase_class_name() {
            Some(c) => c,
            None => return Ok(()),
        };
        self.run_uvm_test(&class_name)
    }

    /// F19: apakah source mengandung panggilan `run_test(...)` EKSPLISIT
    /// (statement di initial/program block ATAU body method class)?
    ///
    /// Masalah yang diperbaiki: `execute_phases()` (auto-detect) dipanggil di
    /// `run()` SEBELUM event loop, jadi ia SELALU menang duluan — class phase
    /// dipilih asal dari iterasi HashMap (bukan nama test dari initial block),
    /// lalu guard `uvm_phases_started` memblokir `initial run_test("my_test")`
    /// yang seharusnya menjadi penentu. Akibatnya test build_phase (yang berisi
    /// `uvm_config_db::set`) tidak pernah dieksekusi. Deteksi ini membuat
    /// auto-detect SKIP total bila user sudah menulis run_test eksplisit.
    pub(crate) fn design_has_explicit_run_test(&self) -> bool {
        // 1) Statement `run_test(...)` di process IR (initial/program/always).
        for p in &self.design.top.processes {
            let body = match p {
                Process::Combinational { body, .. }
                | Process::CombReactive { body, .. }
                | Process::Sequential { body, .. }
                | Process::Initial { body, .. }
                | Process::Final { body, .. }
                | Process::AlwaysWithDelay { body, .. } => body,
            };
            if Self::ir_has_run_test(body) {
                return true;
            }
        }
        // 2) `run_test(...)` di body method class (AST — method UVM seperti
        //    run_phase sering memanggil run_test untuk test berlapis).
        for cls in self.design.classes.values() {
            for m in &cls.methods {
                if Self::ast_has_run_test(&m.stmts) {
                    return true;
                }
            }
        }
        false
    }

    fn ir_has_run_test(stmts: &[IrStmt]) -> bool {
        for s in stmts {
            match s {
                IrStmt::SysCall { name, .. } if name.as_str() == "run_test" => return true,
                IrStmt::Block { stmts } | IrStmt::NamedBlock { stmts, .. } => {
                    if Self::ir_has_run_test(stmts) {
                        return true;
                    }
                }
                IrStmt::If {
                    true_branch,
                    false_branch,
                    ..
                } => {
                    if Self::ir_has_run_test(true_branch)
                        || Self::ir_has_run_test(false_branch)
                    {
                        return true;
                    }
                }
                IrStmt::Case { items, default, .. } => {
                    if Self::ir_has_run_test(default) {
                        return true;
                    }
                    for it in items {
                        if Self::ir_has_run_test(&it.body) {
                            return true;
                        }
                    }
                }
                IrStmt::LoopFor {
                    init,
                    step,
                    body,
                    ..
                } => {
                    if let Some(i) = init {
                        if Self::ir_has_run_test(std::slice::from_ref(i)) {
                            return true;
                        }
                    }
                    if let Some(s) = step {
                        if Self::ir_has_run_test(std::slice::from_ref(s)) {
                            return true;
                        }
                    }
                    if Self::ir_has_run_test(body) {
                        return true;
                    }
                }
                IrStmt::RandCase { items } => {
                    for (_, body) in items {
                        if Self::ir_has_run_test(body) {
                            return true;
                        }
                    }
                }
                IrStmt::RandSequence { productions } => {
                    for (_, items) in productions {
                        for (_, body) in items {
                            if Self::ir_has_run_test(body) {
                                return true;
                            }
                        }
                    }
                }
                IrStmt::LoopWhile { body, .. }
                | IrStmt::LoopDoWhile { body, .. }
                | IrStmt::Repeat { body, .. }
                | IrStmt::Foreach { body, .. }
                | IrStmt::Delay { body, .. } => {
                    if Self::ir_has_run_test(body) {
                        return true;
                    }
                }
                IrStmt::Fork { processes, .. } => {
                    for p in processes {
                        if Self::ir_has_run_test(p) {
                            return true;
                        }
                    }
                }
                IrStmt::Assert {
                    pass_stmt,
                    fail_stmt,
                    ..
                }
                | IrStmt::Assume {
                    pass_stmt,
                    fail_stmt,
                    ..
                } => {
                    if Self::ir_has_run_test(pass_stmt)
                        || Self::ir_has_run_test(fail_stmt)
                    {
                        return true;
                    }
                }
                IrStmt::Cover { pass_stmt, .. } => {
                    if Self::ir_has_run_test(pass_stmt) {
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
    }

    fn ast_has_run_test(stmts: &[crate::ast::Stmt]) -> bool {
        for s in stmts {
            match s {
                crate::ast::Stmt::Expr { expr } => {
                    if let crate::ast::Expr::FuncCall { name, .. } = expr {
                        if name.as_str() == "run_test" {
                            return true;
                        }
                    }
                }
                crate::ast::Stmt::SysCall { name, .. } if name.as_str() == "run_test" => {
                    return true;
                }
                crate::ast::Stmt::Block { stmts } | crate::ast::Stmt::NamedBlock { stmts, .. } => {
                    if Self::ast_has_run_test(stmts) {
                        return true;
                    }
                }
                crate::ast::Stmt::IfElse {
                    true_branch,
                    false_branch,
                    ..
                } => {
                    if Self::ast_has_run_test(std::slice::from_ref(true_branch))
                        || false_branch
                            .as_deref()
                            .is_some_and(|b| Self::ast_has_run_test(std::slice::from_ref(b)))
                    {
                        return true;
                    }
                }
                crate::ast::Stmt::LoopForever { stmts, .. }
                | crate::ast::Stmt::LoopWhile { stmts, .. }
                | crate::ast::Stmt::DoWhile { stmts, .. }
                | crate::ast::Stmt::Repeat { stmts, .. }
                | crate::ast::Stmt::ForeachLoop { stmts, .. } => {
                    if Self::ast_has_run_test(stmts) {
                        return true;
                    }
                }
                crate::ast::Stmt::LoopFor {
                    init,
                    step,
                    stmts,
                    ..
                } => {
                    if let Some(i) = init {
                        if Self::ast_has_run_test(std::slice::from_ref(i)) {
                            return true;
                        }
                    }
                    if let Some(s) = step {
                        if Self::ast_has_run_test(std::slice::from_ref(s)) {
                            return true;
                        }
                    }
                    if Self::ast_has_run_test(stmts) {
                        return true;
                    }
                }
                crate::ast::Stmt::RandCase { items } => {
                    for it in items {
                        if Self::ast_has_run_test(std::slice::from_ref(&it.stmt)) {
                            return true;
                        }
                    }
                }
                crate::ast::Stmt::RandSequence { productions } => {
                    for p in productions {
                        for it in &p.items {
                            if Self::ast_has_run_test(std::slice::from_ref(&it.value)) {
                                return true;
                            }
                        }
                    }
                }
                crate::ast::Stmt::Case { items, default, .. }
                | crate::ast::Stmt::CaseX { items, default, .. }
                | crate::ast::Stmt::CaseZ { items, default, .. }
                | crate::ast::Stmt::StmtCase { items, default, .. } => {
                    if let Some(d) = default {
                        if Self::ast_has_run_test(std::slice::from_ref(d)) {
                            return true;
                        }
                    }
                    for it in items {
                        if Self::ast_has_run_test(std::slice::from_ref(&it.stmt)) {
                            return true;
                        }
                    }
                }
                crate::ast::Stmt::Delay { stmt, .. } => {
                    if Self::ast_has_run_test(std::slice::from_ref(stmt)) {
                        return true;
                    }
                }
                crate::ast::Stmt::Wait { stmt, .. } => {
                    if let Some(st) = stmt {
                        if Self::ast_has_run_test(std::slice::from_ref(st)) {
                            return true;
                        }
                    }
                }
                _ => {}
            }
        }
        false
    }

    /// F18: entry `run_test(name)` — buat objek test, panggil constructor
    /// `new("uvm_test_top", null)`, lalu jalankan seluruh fase UVM.
    /// Guard `uvm_phases_started`: execute_phases (auto-detect, dipanggil di
    /// run() SEBELUM event loop) sudah menjalankan fase, sedangkan run_test
    /// dari initial block berjalan SETELAH-nya — tanpa guard objek test dibuat
    /// dua kali. Kedua jalur bertemu di objek yang sama.
    pub(crate) fn run_uvm_test(&mut self, test_name: &str) -> Result<(), SimError> {
        if self.uvm_phases_started {
            return Ok(());
        }
        let class_name = if !test_name.is_empty() {
            test_name.to_string()
        } else {
            // run_test() tanpa argumen: fallback auto-detect
            match self.find_phase_class_name() {
                Some(c) => c,
                None => {
                    self.emit_warning(
                        DiagCode::DpiError,
                        "run_test() tanpa nama test dan tidak ada phase class ditemukan",
                    );
                    return Ok(());
                }
            }
        };
        if !self.design.classes.contains_key::<str>(class_name.as_str()) {
            self.emit_warning(
                DiagCode::DpiError,
                format!("run_test: class '{}' tidak ditemukan", class_name),
            );
            return Ok(());
        }
        let obj_id = self.state.alloc_object(Symbol::intern(class_name.as_str()));
        // Panggil constructor bila class mendefinisikan `new` (jalur user
        // eksplisit). Objek tanpa new dibiarkan kosong (perilaku legacy).
        if self
            .find_method_in_hierarchy(class_name.as_str(), "new")
            .is_ok()
        {
            self.current_this = Some(obj_id);
            let r = self.execute_method(
                obj_id,
                "new",
                &[string_to_logicvec("uvm_test_top"), LogicVec::from_u64(0, 64)],
            );
            r?;
            self.current_this = None;
        }
        self.root_test_obj_id = Some(obj_id);
        self.uvm_phases_started = true;
        self.run_phase_tree(obj_id)
    }

    /// F18: jalankan seluruh fase UVM pada tree komponen mulai dari root test.
    /// Fase sebelum run (build/connect/end_of_elaboration/start_of_simulation)
    /// sinkron root→children. run_phase adalah task — suspend di delay dan
    /// kontinuasinya dijadwalkan ke event loop (async). Fase akhir
    /// (extract/check/report/final) dijalankan saat semua objection turun
    /// (drop_objection → execute_report_phases).
    fn run_phase_tree(&mut self, obj_id: ObjId) -> Result<(), SimError> {
        let class_name = self
            .state
            .get_object(obj_id)
            .map(|o| o.class_name.to_string())
            .unwrap_or_default();
        if std::env::var("DBG_UVM").is_ok() {
            eprintln!("[DBG-PT] class='{}'", class_name);
        }
        for phase in [
            "build_phase",
            "connect_phase",
            "end_of_elaboration_phase",
            "start_of_simulation_phase",
        ] {
            let has = self.find_method_in_hierarchy(&class_name, phase).is_ok();
            if has {
                self.current_this = Some(obj_id);
                self.execute_method(obj_id, phase, &[])?;
                self.current_this = None;
            }
            // F18: children SELALU di-propagate — child component boleh punya
            // phase walau root tidak (mis. env punya connect_phase, test tidak).
            self.call_phase_on_children(obj_id, phase)?;
        }
        // run_phase: root lalu anak — task boleh suspend (delay/forever),
        // kontinuasi dijadwalkan event loop setelah execute_phases/run_test
        // selesai.
        if self.find_method_in_hierarchy(&class_name, "run_phase").is_ok() {
            self.current_this = Some(obj_id);
            self.execute_method(obj_id, "run_phase", &[])?;
            self.current_this = None;
        }
        // F21: run_phase child SELALU di-propagate walau root tidak punya
        // run_phase (pola umum: run_phase didefinisikan di env/agent, bukan
        // di test). Sebelumnya pemanggilan ini ada DI DALAM if di atas →
        // env run_phase (berisi uvm_event/uvm_barrier sinkronisasi) tidak
        // pernah dieksekusi bila test tanpa run_phase.
        self.call_phase_on_children(obj_id, "run_phase")?;
        Ok(())
    }

    /// F18: fase akhir — extract/check/report/final, sinkron root→children.
    /// Dipanggil dari drop_objection saat objection_count turun ke 0
    /// (end-of-test), SEBELUM `running = false`.
    pub(crate) fn execute_report_phases(&mut self) -> Result<(), SimError> {
        let Some(obj_id) = self.root_test_obj_id else {
            return Ok(());
        };
        let class_name = self
            .state
            .get_object(obj_id)
            .map(|o| o.class_name.to_string())
            .unwrap_or_default();
        for phase in ["extract_phase", "check_phase", "report_phase", "final_phase"] {
            // F22: children phase SELALU di-propagate walau root TIDAK punya
            // phase tsb (sama seperti run_phase) — pola nyata: env punya
            // check_phase/report_phase tapi test tidak → check/report env
            // tetap harus dijalankan. Sebelumnya `call_phase_on_children`
            // berada DI DALAM if root punya phase → child terlewat diam-diam.
            if self.find_method_in_hierarchy(&class_name, phase).is_ok() {
                self.current_this = Some(obj_id);
                self.execute_method(obj_id, phase, &[])?;
                self.current_this = None;
            }
            self.call_phase_on_children(obj_id, phase)?;
        }
        Ok(())
    }

    pub(crate) fn call_phase_on_children(&mut self, obj_id: ObjId, phase: &str) -> Result<(), SimError> {
        if let Some(cdata) = self.uvm_component_data.get(&obj_id) {
            let children = cdata.children.clone();
            for child_id in children {
                if let Some(obj) = self.state.get_object(child_id) {
                    let child_class = &obj.class_name;
                    if self.find_method_in_hierarchy(child_class.as_str(), phase).is_ok() {
                        self.current_this = Some(child_id);
                        self.execute_method(child_id, phase, &[])?;
                        self.current_this = None;
                    }
                }
                self.call_phase_on_children(child_id, phase)?;
            }
        }
        Ok(())
    }

    pub(crate) fn is_uvm_object_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_object" {
                return true;
            }
            match self.design.classes.get::<str>(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    pub(crate) fn is_uvm_component_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_component" {
                return true;
            }
            match self.design.classes.get::<str>(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    pub(crate) fn is_uvm_report_object_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_report_object" {
                return true;
            }
            match self.design.classes.get::<str>(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    pub(crate) fn is_uvm_sequence_item_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_sequence_item" {
                return true;
            }
            match self.design.classes.get::<str>(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    pub(crate) fn is_uvm_sequence_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_sequence" || current == "uvm_sequence" {
                return true;
            }
            match self.design.classes.get::<str>(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    pub(crate) fn is_uvm_sequencer_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_sequencer" || current == "uvm_sequencer" {
                return true;
            }
            match self.design.classes.get::<str>(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    // F21: uvm_event / uvm_barrier — cek extends chain sampai ke
    // `__uvm_event` / `__uvm_barrier` (class user `extends uvm_event` juga
    // terdeteksi — objek dibuat dengan class_name asli dari tipe field,
    // bukan nama builtin `__uvm_*`).
    pub(crate) fn is_uvm_event_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_event" || current == "uvm_event" {
                return true;
            }
            match self.design.classes.get::<str>(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    pub(crate) fn is_uvm_barrier_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_barrier" || current == "uvm_barrier" {
                return true;
            }
            match self.design.classes.get::<str>(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    pub(crate) fn is_uvm_monitor_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_monitor" {
                return true;
            }
            match self.design.classes.get::<str>(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    pub(crate) fn is_uvm_analysis_port_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_analysis_port" {
                return true;
            }
            match self.design.classes.get::<str>(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    pub(crate) fn is_uvm_analysis_imp_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_analysis_imp" {
                return true;
            }
            match self.design.classes.get::<str>(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    // F22: `class my_sub extends uvm_subscriber` — telusur extends chain ke
    // `__uvm_subscriber` (dispatch new builtin untuk auto-buat analysis_imp).
    pub(crate) fn is_uvm_subscriber_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_subscriber" || current == "uvm_subscriber" {
                return true;
            }
            match self.design.classes.get::<str>(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    // F23: uvm_tlm_fifo / export analysis internal fifo — telusur extends
    // chain ke `__uvm_tlm_fifo` / `__uvm_fifo_export`.
    pub(crate) fn is_uvm_tlm_fifo_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_tlm_fifo" || current == "uvm_tlm_fifo" {
                return true;
            }
            match self.design.classes.get::<str>(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    pub(crate) fn is_uvm_fifo_export_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_fifo_export" {
                return true;
            }
            match self.design.classes.get::<str>(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    pub(crate) fn is_uvm_callback_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_callback" {
                return true;
            }
            match self.design.classes.get::<str>(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    pub(crate) fn is_uvm_callbacks_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_callbacks" {
                return true;
            }
            match self.design.classes.get::<str>(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    pub(crate) fn is_uvm_driver_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_driver" || current == "uvm_driver" {
                return true;
            }
            match self.design.classes.get::<str>(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    /// F24 review: helper `new`-dispatch — builtin UVM (`methods: vec![]`)
    /// atau class dgn `new` override. Dipakai 3 jalur alokasi object
    /// (eval/ast.rs ×2 + eval/expr.rs jalur IR) agar data builtin selalu
    /// dibuat. Satu sumber kebenaran — F26+ cukup update di sini.
    pub(crate) fn uvm_needs_new_dispatch(&self, class: &str) -> bool {
        self.is_uvm_event_hierarchy(class)
            || self.is_uvm_barrier_hierarchy(class)
            || self.is_uvm_subscriber_hierarchy(class)
            || self.is_uvm_tlm_fifo_hierarchy(class)
            || self.is_uvm_fifo_export_hierarchy(class)
            || self.is_uvm_seq_item_port_hierarchy(class)
            || self.is_uvm_driver_hierarchy(class)
            || self.is_uvm_sequencer_hierarchy(class)
            || self.is_uvm_sequence_hierarchy(class)
            || self.find_method_in_hierarchy(class, "new").is_ok()
    }

    /// F24: apakah class_name subclass `uvm_seq_item_port` (port driver↔sequencer).
    pub(crate) fn is_uvm_seq_item_port_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_seq_item_port" || current == "uvm_seq_item_port" {
                return true;
            }
            match self.design.classes.get::<str>(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    pub(crate) fn is_uvm_reg_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_reg" {
                return true;
            }
            match self.design.classes.get::<str>(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    pub(crate) fn is_uvm_reg_field_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_reg_field" {
                return true;
            }
            match self.design.classes.get::<str>(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    pub(crate) fn is_uvm_reg_block_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_reg_block" {
                return true;
            }
            match self.design.classes.get::<str>(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

    pub(crate) fn is_uvm_reg_map_hierarchy(&self, class_name: &str) -> bool {
        let mut current = class_name;
        loop {
            if current == "__uvm_reg_map" {
                return true;
            }
            match self.design.classes.get::<str>(current) {
                Some(c) => match &c.extends {
                    Some(parent) => current = parent.as_str(),
                    None => return false,
                },
                None => return false,
            }
        }
    }

}
