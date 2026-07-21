use std::borrow::Cow;
use std::collections::HashMap;
use std::fs;

use crate::ir::{IrDesign, LogicVal, LogicVec};

use wavefst::{
    FstWriter, GeomEntry, Header, ScopeType, SignalValue, TimeCompression, VarDir, VarType,
};

pub struct FstWaveWriter {
    writer: Option<FstWriter<fs::File>>,
    var_handles: HashMap<String, u32>,
    last_values: HashMap<String, String>,
    current_time: u64,
    pub enabled: bool,
}

impl FstWaveWriter {
    /// Parse signal name into (scope_path, bare_name)
    /// Example: "top.u_sub.count" → (["top", "u_sub"], "count")
    /// Example: "clk" → ([], "clk")
    fn parse_scope(name: &str) -> (Vec<String>, String) {
        let parts: Vec<&str> = name.rsplitn(2, '.').collect();
        if parts.len() == 2 {
            let bare_name = parts[0].to_string();
            let scope_parts: Vec<String> = parts[1].split('.').map(|s| s.to_string()).collect();
            (scope_parts, bare_name)
        } else {
            (vec![], name.to_string())
        }
    }

    /// Build a full-qualified key from scope + bare_name for var_handles hashmap
    fn signal_key(scope: &[String], bare_name: &str) -> String {
        if scope.is_empty() {
            bare_name.to_string()
        } else {
            format!("{}.{}", scope.join("."), bare_name)
        }
    }

    pub fn new(path: &str, design: &IrDesign) -> Result<Self, String> {
        let file = fs::File::create(path)
            .map_err(|e| format!("cannot create FST file '{}': {}", path, e))?;

        let mut writer = FstWriter::builder(file)
            .time_compression(TimeCompression::Zlib)
            .build()
            .map_err(|e| format!("FST writer build failed: {}", e))?;

        // Write header
        let mut header = Header::default();
        header.version = "Maria RTL Simulator".to_string();
        header.timescale_exponent = -9; // 1ns
        header.end_time = 0;
        writer
            .write_header(header)
            .map_err(|e| format!("FST header write failed: {}", e))?;

        // ── Build scope hierarchy ──
        // Step 1: Group signals by scope
        let mut scope_map: HashMap<Vec<String>, Vec<(String, usize, usize)>> = HashMap::new();
        for sig in &design.top.signals {
            let (scope_parts, bare_name) = Self::parse_scope(&sig.name);
            scope_map
                .entry(scope_parts)
                .or_default()
                .push((bare_name, sig.width, sig.array_depth));
        }

        // Step 2: Sort scopes for consistent ordering
        let mut sorted_scopes: Vec<Vec<String>> = scope_map.keys().cloned().collect();
        sorted_scopes.sort();

        // Step 3: Open top-level module scope
        writer
            .begin_scope(ScopeType::VcdModule, &design.top.name, None)
            .map_err(|e| format!("FST scope begin failed: {}", e))?;

        let mut var_handles = HashMap::new();

        // Step 4: Process each scope path, opening/closing sub-scopes as needed
        let mut active_stack: Vec<String> = Vec::new();

        for scope_path in &sorted_scopes {
            // Close scopes that are no longer needed
            let mut common_prefix = 0usize;
            for (i, p) in scope_path.iter().enumerate() {
                if i < active_stack.len() && active_stack[i] == *p {
                    common_prefix = i + 1;
                } else {
                    break;
                }
            }
            // Close excess scopes
            for _ in common_prefix..active_stack.len() {
                writer
                    .end_scope()
                    .map_err(|e| format!("FST scope end failed: {}", e))?;
            }
            // Open new scopes
            for p in &scope_path[common_prefix..] {
                writer
                    .begin_scope(ScopeType::VcdModule, p, None)
                    .map_err(|e| format!("FST begin scope '{}' failed: {}", p, e))?;
            }
            active_stack = scope_path.clone();

            // Add variables in this scope
            let sigs = scope_map.get(scope_path).unwrap();
            for (bare_name, width, array_depth) in sigs {
                if *width == 0 { continue; }  // skip dynamic arrays
                let key = Self::signal_key(scope_path, bare_name);

                if *array_depth > 1 {
                    for elem in 0..*array_depth {
                        let elem_name = format!("{}[{}]", bare_name, elem);
                        let elem_key = Self::signal_key(scope_path, &elem_name);
                        let elem_width = *width / array_depth;
                        let handle = writer
                            .add_variable(
                                VarType::VcdWire,
                                VarDir::Implicit,
                                &elem_name,
                                GeomEntry::Fixed(elem_width as u32),
                            )
                            .map_err(|e| format!("FST add_variable '{}' failed: {}", elem_name, e))?;
                        var_handles.insert(elem_key, handle);
                    }
                } else {
                    let handle = writer
                        .add_variable(
                            VarType::VcdWire,
                            VarDir::Implicit,
                            bare_name,
                            GeomEntry::Fixed(*width as u32),
                        )
                        .map_err(|e| format!("FST add_variable '{}' failed: {}", bare_name, e))?;
                    var_handles.insert(key, handle);
                }
            }
        }

        // Close all remaining scopes
        for _ in 0..active_stack.len() {
            writer
                .end_scope()
                .map_err(|e| format!("FST scope end failed: {}", e))?;
        }
        // Close top-level scope
        writer
            .end_scope()
            .map_err(|e| format!("FST scope end failed: {}", e))?;

        Ok(FstWaveWriter {
            writer: Some(writer),
            var_handles,
            last_values: HashMap::new(),
            current_time: 0,
            enabled: true,
        })
    }

    fn logicvec_to_fst(val: &LogicVec) -> String {
        let mut s = String::with_capacity(val.width);
        for bit in val.bits.iter().rev() {
            match bit {
                LogicVal::Zero => s.push('0'),
                LogicVal::One => s.push('1'),
                LogicVal::X => s.push('x'),
                LogicVal::Z => s.push('z'),
            }
        }
        s
    }

    fn elem_val(sig_val: &LogicVec, elem: usize, elem_width: usize) -> LogicVec {
        let start = elem * elem_width;
        let mut bits = Vec::with_capacity(elem_width);
        for j in start..start + elem_width {
            bits.push(sig_val.bits.get(j).copied().unwrap_or(LogicVal::X));
        }
        LogicVec { width: elem_width, bits }
    }

    pub fn write_time_header(&mut self, time: u64) -> Result<(), String> {
        if !self.enabled { return Ok(()); }
        self.current_time = time;
        Ok(())
    }

    pub fn dump_state(&mut self, design: &IrDesign, state: &[LogicVec]) -> Result<(), String> {
        if !self.enabled { return Ok(()); }

        // Collect changes first to avoid borrow conflicts
        let mut changes: Vec<(u32, String)> = Vec::new();

        for (sig_val, sig) in state.iter().zip(design.top.signals.iter()) {
            let (scope, sig_bare) = Self::parse_scope(&sig.name);
            let key = Self::signal_key(&scope, &sig_bare);

            if sig.array_depth > 1 {
                for elem in 0..sig.array_depth {
                    let elem_name = format!("{}[{}]", sig_bare, elem);
                    let elem_key = Self::signal_key(&scope, &elem_name);
                    if let Some(&handle) = self.var_handles.get(&elem_key) {
                        let e_val = Self::elem_val(sig_val, elem, sig.elem_width);
                        let val_str = Self::logicvec_to_fst(&e_val);
                        if self.last_values.get(&elem_key) != Some(&val_str) {
                            changes.push((handle, val_str.clone()));
                            self.last_values.insert(elem_key, val_str);
                        }
                    }
                }
            } else {
                if let Some(&handle) = self.var_handles.get(&key) {
                    let val_str = Self::logicvec_to_fst(sig_val);
                    if self.last_values.get(&key) != Some(&val_str) {
                        changes.push((handle, val_str.clone()));
                        self.last_values.insert(key, val_str);
                    }
                }
            }
        }

        // Apply changes
        if let Some(writer) = self.writer.as_mut() {
            for (handle, val_str) in &changes {
                let fst_val = Self::str_to_signal_value(val_str);
                writer
                    .emit_change(self.current_time, *handle, fst_val)
                    .map_err(|e| format!("FST emit_change failed: {}", e))?;
            }
        }

        Ok(())
    }

    pub fn dump_all(&mut self, design: &IrDesign, state: &[LogicVec]) -> Result<(), String> {
        if !self.enabled { return Ok(()); }

        // Collect changes first to avoid borrow conflicts
        let mut changes: Vec<(u32, String)> = Vec::new();

        for (sig_val, sig) in state.iter().zip(design.top.signals.iter()) {
            let (scope, sig_bare) = Self::parse_scope(&sig.name);
            let key = Self::signal_key(&scope, &sig_bare);

            if sig.array_depth > 1 {
                for elem in 0..sig.array_depth {
                    let elem_name = format!("{}[{}]", sig_bare, elem);
                    let elem_key = Self::signal_key(&scope, &elem_name);
                    if let Some(&handle) = self.var_handles.get(&elem_key) {
                        let e_val = Self::elem_val(sig_val, elem, sig.elem_width);
                        let val_str = Self::logicvec_to_fst(&e_val);
                        changes.push((handle, val_str.clone()));
                        self.last_values.insert(elem_key, val_str);
                    }
                }
            } else {
                if let Some(&handle) = self.var_handles.get(&key) {
                    let val_str = Self::logicvec_to_fst(sig_val);
                    changes.push((handle, val_str.clone()));
                    self.last_values.insert(key, val_str);
                }
            }
        }

        // Apply changes
        if let Some(writer) = self.writer.as_mut() {
            for (handle, val_str) in &changes {
                let fst_val = Self::str_to_signal_value(val_str);
                writer
                    .emit_change(self.current_time, *handle, fst_val)
                    .map_err(|e| format!("FST emit_change failed: {}", e))?;
            }
        }

        Ok(())
    }

    fn str_to_signal_value(s: &str) -> SignalValue<'_> {
        if s.len() == 1 {
            SignalValue::Bit(s.chars().next().unwrap())
        } else {
            SignalValue::Vector(Cow::Borrowed(s))
        }
    }

    pub fn close(&mut self) -> Result<(), String> {
        if let Some(writer) = self.writer.take() {
            writer
                .finish()
                .map_err(|e| format!("FST finish failed: {}", e))?;
        }
        Ok(())
    }
}
