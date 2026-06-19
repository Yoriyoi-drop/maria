use std::collections::HashMap;
use std::fs;
use std::io::Write;

use crate::ir::{IrDesign, LogicVal};

pub struct VcdWriter {
    file: fs::File,
    last_values: HashMap<String, String>,
    code_by_key: HashMap<(Vec<String>, String), String>,
    pub enabled: bool,
}

impl VcdWriter {
    pub fn new(path: &str, design: &IrDesign) -> Result<Self, String> {
        let file = fs::File::create(path)
            .map_err(|e| format!("cannot create VCD file '{}': {}", path, e))?;

        let mut writer = VcdWriter {
            file,
            last_values: HashMap::new(),
            code_by_key: HashMap::new(),
            enabled: true,
        };

        writer.write_header(design)?;
        Ok(writer)
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

    fn write_scopes(&mut self, current: &[String], target: &[String], sigs: &[(String, usize, usize)], entry_idx: &mut usize) -> Result<(), String> {
        let mut close_count = 0usize;
        for (i, p) in current.iter().enumerate() {
            if i >= target.len() || target[i] != *p {
                close_count = current.len() - i;
                break;
            }
        }
        for _ in 0..close_count {
            writeln!(self.file, "$upscope $end").unwrap();
        }

        let keep = current.len() - close_count;
        for p in &target[keep..] {
            writeln!(self.file, "$scope module {} $end", p).unwrap();
        }

        for (bare_name, width, array_depth) in sigs {
            if *array_depth > 1 {
                for elem in 0..*array_depth {
                    let code = format!("s{:x}", entry_idx);
                    *entry_idx += 1;
                    let elem_name = format!("{}[{}]", bare_name, elem);
                    let var_type = "wire";
                    let width_disp = if *width == 1 { format!("1") } else { width.to_string() };
                    let range = if *width == 1 { String::new() } else { format!(" [{}:0]", width - 1) };
                    writeln!(self.file, "$var {} {} {} {} {} $end", var_type, width_disp, code, elem_name, range).unwrap();
                    let key = (target.to_vec(), elem_name.clone());
                    self.code_by_key.insert(key, code.clone());
                }
            } else {
                let code = format!("s{:x}", entry_idx);
                *entry_idx += 1;
                let var_type = "wire";
                let width_disp = if *width == 1 { format!("1") } else { width.to_string() };
                let range = if *width == 1 { String::new() } else { format!(" [{}:0]", width - 1) };
                writeln!(self.file, "$var {} {} {} {} {} $end", var_type, width_disp, code, bare_name, range).unwrap();
                let key = (target.to_vec(), bare_name.clone());
                self.code_by_key.insert(key, code.clone());
            }
        }
        Ok(())
    }

    fn write_header(&mut self, design: &IrDesign) -> Result<(), String> {
        writeln!(self.file, "$version Maria RTL Simulator v0.1.0 $end").unwrap();
        writeln!(self.file, "$timescale 1ns $end").unwrap();

        let mut scope_map: HashMap<Vec<String>, Vec<(String, usize, usize)>> = HashMap::new();
        for sig in &design.top.signals {
            let (scope_parts, bare_name) = Self::parse_scope(&sig.name);
            scope_map.entry(scope_parts)
                .or_default()
                .push((bare_name, sig.width, sig.array_depth));
        }

        let mut sorted_scopes: Vec<Vec<String>> = scope_map.keys().cloned().collect();
        sorted_scopes.sort();

        writeln!(self.file, "$scope module {} $end", design.top.name).unwrap();

        let mut current_scope: Vec<String> = Vec::new();
        let mut entry_idx = 0usize;

        for scope_path in &sorted_scopes {
            let sigs = scope_map.get(scope_path).unwrap();
            self.write_scopes(&current_scope, scope_path, sigs, &mut entry_idx)?;
            current_scope = scope_path.clone();
        }

        for _ in 0..current_scope.len() {
            writeln!(self.file, "$upscope $end").unwrap();
        }
        writeln!(self.file, "$upscope $end").unwrap();
        writeln!(self.file, "$enddefinitions $end").unwrap();
        writeln!(self.file, "$dumpvars").unwrap();

        for sig in &design.top.signals {
            let (sig_scope, sig_bare) = Self::parse_scope(&sig.name);
            if sig.array_depth > 1 {
                for elem in 0..sig.array_depth {
                    let elem_name = format!("{}[{}]", sig_bare, elem);
                    let key = (sig_scope.clone(), elem_name);
                    if let Some(code) = self.code_by_key.get(&key) {
                        let start = elem * sig.elem_width;
                        let mut bits = Vec::with_capacity(sig.elem_width);
                        for j in start..start + sig.elem_width {
                            bits.push(sig.init_val.bits.get(j).copied().unwrap_or(LogicVal::X));
                        }
                        let elem_val = crate::ir::LogicVec { width: sig.elem_width, bits };
                        let val_str = vec_to_vcd(&elem_val);
                        writeln!(self.file, "b{} {}", val_str, code).unwrap();
                        self.last_values.insert(code.clone(), val_str);
                    }
                }
            } else {
                let key = (sig_scope.clone(), sig_bare.clone());
                if let Some(code) = self.code_by_key.get(&key) {
                    let val_str = vec_to_vcd(&sig.init_val);
                    let width = sig.width;
                    if width == 1 {
                        writeln!(self.file, "{}{}", val_str, code).unwrap();
                    } else {
                        writeln!(self.file, "b{} {}", val_str, code).unwrap();
                    }
                    self.last_values.insert(code.clone(), val_str);
                }
            }
        }

        writeln!(self.file, "$end").unwrap();
        Ok(())
    }

    pub fn write_time_header(&mut self, time: u64) -> Result<(), String> {
        if !self.enabled { return Ok(()); }
        writeln!(self.file, "#{}", time).unwrap();
        Ok(())
    }

    pub fn dump_state(&mut self, design: &IrDesign, state: &[crate::ir::LogicVec]) -> Result<(), String> {
        if !self.enabled { return Ok(()); }
        for (sig_val, sig) in state.iter().zip(design.top.signals.iter()) {
            let (sig_scope, sig_bare) = Self::parse_scope(&sig.name);
            if sig.array_depth > 1 {
                for elem in 0..sig.array_depth {
                    let elem_name = format!("{}[{}]", sig_bare, elem);
                    let key = (sig_scope.clone(), elem_name);
                    if let Some(code) = self.code_by_key.get(&key) {
                        let start = elem * sig.elem_width;
                        let mut bits = Vec::with_capacity(sig.elem_width);
                        for j in start..start + sig.elem_width {
                            bits.push(sig_val.bits.get(j).copied().unwrap_or(LogicVal::X));
                        }
                        let elem_val = crate::ir::LogicVec { width: sig.elem_width, bits };
                        let val_str = vec_to_vcd(&elem_val);
                        if self.last_values.get(code) != Some(&val_str) {
                            if sig.elem_width == 1 {
                                writeln!(self.file, "{}{}", val_str, code).unwrap();
                            } else {
                                writeln!(self.file, "b{} {}", val_str, code).unwrap();
                            }
                            self.last_values.insert(code.clone(), val_str);
                        }
                    }
                }
            } else {
                let key = (sig_scope.clone(), sig_bare.clone());
                if let Some(code) = self.code_by_key.get(&key) {
                    let val_str = vec_to_vcd(sig_val);
                    if self.last_values.get(code) != Some(&val_str) {
                        if sig.width == 1 {
                            writeln!(self.file, "{}{}", val_str, code).unwrap();
                        } else {
                            writeln!(self.file, "b{} {}", val_str, code).unwrap();
                        }
                        self.last_values.insert(code.clone(), val_str);
                    }
                }
            }
        }
        Ok(())
    }

    pub fn close(&mut self) -> Result<(), String> {
        Ok(())
    }
}

fn vec_to_vcd(val: &crate::ir::LogicVec) -> String {
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