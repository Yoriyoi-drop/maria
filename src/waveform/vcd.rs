use std::collections::HashMap;
use std::fs;
use std::io::Write;

use crate::ir::{IrDesign, LogicVal};

pub struct VcdWriter {
    file: fs::File,
    last_values: HashMap<String, String>,
    code_by_key: HashMap<(Vec<String>, String), String>,
    pub enabled: bool,
    pub max_dump_size: Option<u64>,
    total_written: u64,
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
            max_dump_size: None,
            total_written: 0,
        };

        writer.write_header(design)?;
        Ok(writer)
    }

    pub fn reopen(&mut self, path: &str, design: &IrDesign, state: &[crate::ir::LogicVec]) -> Result<(), String> {
        self.close_inner()?;
        let file = fs::File::create(path)
            .map_err(|e| format!("cannot create VCD file '{}': {}", path, e))?;
        self.file = file;
        self.last_values.clear();
        self.code_by_key.clear();
        self.total_written = 0;
        self.enabled = true;
        self.write_header(design)?;
        self.dump_all(design, state)
    }

    fn write_raw(&mut self, buf: &[u8]) -> Result<(), String> {
        if let Some(limit) = self.max_dump_size {
            if self.total_written + buf.len() as u64 > limit {
                self.enabled = false;
                return Ok(());
            }
        }
        self.file.write_all(buf).map_err(|e| format!("VCD write error: {}", e))?;
        self.total_written += buf.len() as u64;
        Ok(())
    }

    fn write_vals(&mut self, sig_val: &crate::ir::LogicVec, code: &str, is_one_bit: bool) -> Result<(), String> {
        let val_str = vec_to_vcd(sig_val);
        if self.last_values.get(code) != Some(&val_str) {
            let line = if is_one_bit {
                format!("{}{}\n", val_str, code)
            } else {
                format!("b{} {}\n", val_str, code)
            };
            self.write_raw(line.as_bytes())?;
            self.last_values.insert(code.to_string(), val_str);
        }
        Ok(())
    }

    fn write_vals_force(&mut self, sig_val: &crate::ir::LogicVec, code: &str, is_one_bit: bool) -> Result<(), String> {
        let val_str = vec_to_vcd(sig_val);
        let line = if is_one_bit {
            format!("{}{}\n", val_str, code)
        } else {
            format!("b{} {}\n", val_str, code)
        };
        self.write_raw(line.as_bytes())?;
        self.last_values.insert(code.to_string(), val_str);
        Ok(())
    }

    fn elem_val<'a>(&self, sig_val: &'a crate::ir::LogicVec, elem: usize, elem_width: usize) -> crate::ir::LogicVec {
        let start = elem * elem_width;
        let mut bits = Vec::with_capacity(elem_width);
        for j in start..start + elem_width {
            bits.push(sig_val.bits.get(j).copied().unwrap_or(LogicVal::X));
        }
        crate::ir::LogicVec { width: elem_width, bits }
    }

    fn code_for_signal(&self, sig_scope: &[String], sig_bare: &str, elem: Option<usize>) -> Option<String> {
        if let Some(e) = elem {
            let elem_name = format!("{}[{}]", sig_bare, e);
            self.code_by_key.get(&(sig_scope.to_vec(), elem_name)).cloned()
        } else {
            self.code_by_key.get(&(sig_scope.to_vec(), sig_bare.to_string())).cloned()
        }
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
            let _ = self.write_raw(b"$upscope $end\n");
        }

        let keep = current.len() - close_count;
        for p in &target[keep..] {
            writeln!(self.file, "$scope module {} $end", p).unwrap();
        }

        for (bare_name, width, array_depth) in sigs {
            if *width == 0 { continue; }  // skip dynamic/queue arrays before allocation
            if *array_depth > 1 {
                for elem in 0..*array_depth {
                    let code = format!("s{:x}", entry_idx);
                    *entry_idx += 1;
                    let elem_name = format!("{}[{}]", bare_name, elem);
                    let width_disp = if *width == 1 { "1".to_string() } else { width.to_string() };
                    let range = if *width == 1 { String::new() } else { format!(" [{}:0]", width - 1) };
                    writeln!(self.file, "$var wire {} {} {} {} $end", width_disp, code, elem_name, range).unwrap();
                    self.code_by_key.insert((target.to_vec(), elem_name), code);
                }
            } else {
                let code = format!("s{:x}", entry_idx);
                *entry_idx += 1;
                let width_disp = if *width == 1 { "1".to_string() } else { width.to_string() };
                let range = if *width == 1 { String::new() } else { format!(" [{}:0]", width - 1) };
                writeln!(self.file, "$var wire {} {} {} {} $end", width_disp, code, bare_name, range).unwrap();
                self.code_by_key.insert((target.to_vec(), bare_name.clone()), code);
            }
        }
        Ok(())
    }

    fn write_header(&mut self, design: &IrDesign) -> Result<(), String> {
        self.write_raw(b"$version Maria RTL Simulator v0.1.0 $end\n")?;
        let ts = if let Some((ref unit, _)) = design.timescale {
            format!("$timescale {} $end\n", unit)
        } else {
            "$timescale 1ns $end\n".to_string()
        };
        self.write_raw(ts.as_bytes())?;

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
        self.write_raw(b"$enddefinitions $end\n")?;
        self.write_raw(b"$dumpvars\n")?;

        for sig in &design.top.signals {
            let (sig_scope, sig_bare) = Self::parse_scope(&sig.name);
            if sig.array_depth > 1 {
                for elem in 0..sig.array_depth {
                    if let Some(code) = self.code_for_signal(&sig_scope, &sig_bare, Some(elem)) {
                        let e_val = self.elem_val(&sig.init_val, elem, sig.elem_width);
                        self.write_vals_force(&e_val, &code, sig.elem_width == 1)?;
                    }
                }
            } else {
                if let Some(code) = self.code_for_signal(&sig_scope, &sig_bare, None) {
                    self.write_vals_force(&sig.init_val, &code, sig.width == 1)?;
                }
            }
        }

        self.write_raw(b"$end\n")
    }

    pub fn write_time_header(&mut self, time: u64) -> Result<(), String> {
        if !self.enabled { return Ok(()); }
        self.write_raw(format!("#{}\n", time).as_bytes())
    }

    pub fn dump_state(&mut self, design: &IrDesign, state: &[crate::ir::LogicVec]) -> Result<(), String> {
        if !self.enabled { return Ok(()); }
        for (sig_val, sig) in state.iter().zip(design.top.signals.iter()) {
            let (sig_scope, sig_bare) = Self::parse_scope(&sig.name);
            if sig.array_depth > 1 {
                for elem in 0..sig.array_depth {
                    if let Some(code) = self.code_for_signal(&sig_scope, &sig_bare, Some(elem)) {
                        let e_val = self.elem_val(sig_val, elem, sig.elem_width);
                        self.write_vals(&e_val, &code, sig.elem_width == 1)?;
                    }
                }
            } else {
                if let Some(code) = self.code_for_signal(&sig_scope, &sig_bare, None) {
                    self.write_vals(sig_val, &code, sig.width == 1)?;
                }
            }
        }
        Ok(())
    }

    pub fn dump_all(&mut self, design: &IrDesign, state: &[crate::ir::LogicVec]) -> Result<(), String> {
        if !self.enabled { return Ok(()); }
        for (sig_val, sig) in state.iter().zip(design.top.signals.iter()) {
            let (sig_scope, sig_bare) = Self::parse_scope(&sig.name);
            if sig.array_depth > 1 {
                for elem in 0..sig.array_depth {
                    if let Some(code) = self.code_for_signal(&sig_scope, &sig_bare, Some(elem)) {
                        let e_val = self.elem_val(sig_val, elem, sig.elem_width);
                        self.write_vals_force(&e_val, &code, sig.elem_width == 1)?;
                    }
                }
            } else {
                if let Some(code) = self.code_for_signal(&sig_scope, &sig_bare, None) {
                    self.write_vals_force(sig_val, &code, sig.width == 1)?;
                }
            }
        }
        Ok(())
    }

    fn close_inner(&mut self) -> Result<(), String> {
        let _ = self.file.flush();
        Ok(())
    }

    pub fn close(&mut self) -> Result<(), String> {
        self.close_inner()
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
