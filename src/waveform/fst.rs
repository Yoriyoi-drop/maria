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

        // Create hierarchy
        writer
            .begin_scope(ScopeType::VcdModule, &design.top.name, None)
            .map_err(|e| format!("FST scope begin failed: {}", e))?;

        let mut var_handles = HashMap::new();

        for sig in &design.top.signals {
            let (_, sig_bare) = Self::parse_scope(&sig.name);

            if sig.array_depth > 1 {
                for elem in 0..sig.array_depth {
                    let elem_name = format!("{}[{}]", sig_bare, elem);
                    let handle = writer
                        .add_variable(
                            VarType::VcdWire,
                            VarDir::Implicit,
                            &elem_name,
                            GeomEntry::Fixed(sig.elem_width as u32),
                        )
                        .map_err(|e| format!("FST add_variable failed: {}", e))?;
                    var_handles.insert(elem_name, handle);
                }
            } else {
                let handle = writer
                    .add_variable(
                        VarType::VcdWire,
                        VarDir::Implicit,
                        &sig_bare,
                        GeomEntry::Fixed(sig.width as u32),
                    )
                    .map_err(|e| format!("FST add_variable failed: {}", e))?;
                var_handles.insert(sig_bare, handle);
            }
        }

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
            let (_, sig_bare) = Self::parse_scope(&sig.name);

            if sig.array_depth > 1 {
                for elem in 0..sig.array_depth {
                    let elem_name = format!("{}[{}]", sig_bare, elem);
                    if let Some(&handle) = self.var_handles.get(&elem_name) {
                        let e_val = Self::elem_val(sig_val, elem, sig.elem_width);
                        let val_str = Self::logicvec_to_fst(&e_val);
                        if self.last_values.get(&elem_name) != Some(&val_str) {
                            changes.push((handle, val_str));
                        }
                    }
                }
            } else {
                if let Some(&handle) = self.var_handles.get(&sig_bare) {
                    let val_str = Self::logicvec_to_fst(sig_val);
                    if self.last_values.get(&sig_bare) != Some(&val_str) {
                        changes.push((handle, val_str));
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

        // Update last_values
        for (handle, val_str) in changes {
            for (name, &h) in &self.var_handles {
                if h == handle {
                    self.last_values.insert(name.clone(), val_str);
                    break;
                }
            }
        }

        Ok(())
    }

    pub fn dump_all(&mut self, design: &IrDesign, state: &[LogicVec]) -> Result<(), String> {
        if !self.enabled { return Ok(()); }

        // Collect changes first to avoid borrow conflicts
        let mut changes: Vec<(u32, String)> = Vec::new();

        for (sig_val, sig) in state.iter().zip(design.top.signals.iter()) {
            let (_, sig_bare) = Self::parse_scope(&sig.name);

            if sig.array_depth > 1 {
                for elem in 0..sig.array_depth {
                    let elem_name = format!("{}[{}]", sig_bare, elem);
                    if let Some(&handle) = self.var_handles.get(&elem_name) {
                        let e_val = Self::elem_val(sig_val, elem, sig.elem_width);
                        let val_str = Self::logicvec_to_fst(&e_val);
                        changes.push((handle, val_str));
                    }
                }
            } else {
                if let Some(&handle) = self.var_handles.get(&sig_bare) {
                    let val_str = Self::logicvec_to_fst(sig_val);
                    changes.push((handle, val_str));
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

        // Update last_values
        for (handle, val_str) in changes {
            for (name, &h) in &self.var_handles {
                if h == handle {
                    self.last_values.insert(name.clone(), val_str);
                    break;
                }
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
