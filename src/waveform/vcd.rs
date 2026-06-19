use std::collections::HashMap;
use std::fs;
use std::io::Write;

use crate::ir::{IrDesign, LogicVal};

struct VcdSignalEntry {
    code: String,
    name: String,
    width: usize,
}

pub struct VcdWriter {
    file: fs::File,
    entries: Vec<VcdSignalEntry>,
    last_values: HashMap<String, String>,
    pub enabled: bool,
}

impl VcdWriter {
    pub fn new(path: &str, design: &IrDesign) -> Result<Self, String> {
        let file = fs::File::create(path)
            .map_err(|e| format!("cannot create VCD file '{}': {}", path, e))?;

        let mut writer = VcdWriter {
            file,
            entries: Vec::new(),
            last_values: HashMap::new(),
            enabled: true,
        };

        writer.write_header(design)?;
        Ok(writer)
    }

    fn write_header(&mut self, design: &IrDesign) -> Result<(), String> {
        writeln!(self.file, "$version Maria RTL Simulator v0.1.0 $end").unwrap();
        writeln!(self.file, "$timescale 1ns $end").unwrap();

        writeln!(self.file, "$scope module {}", design.top.name).unwrap();

        let mut entry_idx = 0usize;
        for (_i, sig) in design.top.signals.iter().enumerate() {
            if sig.array_depth > 1 {
                // Dump each array element as a separate VCD signal
                for elem in 0..sig.array_depth {
                    let code = format!("s{:x}", entry_idx);
                    entry_idx += 1;
                    let elem_name = format!("{}[{}]", sig.name, elem);
                    let var_type = "wire";
                    if sig.elem_width == 1 {
                        writeln!(self.file, "$var {} 1 {} {} $end", var_type, code, elem_name).unwrap();
                    } else {
                        writeln!(self.file, "$var {} {} {} {} [{}:0] $end",
                            var_type, sig.elem_width, code, elem_name, sig.elem_width - 1).unwrap();
                    }
                    self.entries.push(VcdSignalEntry {
                        code: code.clone(),
                        name: elem_name,
                        width: sig.elem_width,
                    });
                }
            } else {
                let code = format!("s{:x}", entry_idx);
                entry_idx += 1;
                let var_type = "wire";
                if sig.width == 1 {
                    writeln!(self.file, "$var {} 1 {} {} $end", var_type, code, sig.name).unwrap();
                } else {
                    writeln!(self.file, "$var {} {} {} {} [{}:0] $end",
                        var_type, sig.width, code, sig.name, sig.width - 1).unwrap();
                }
                self.entries.push(VcdSignalEntry {
                    code: code.clone(),
                    name: sig.name.clone(),
                    width: sig.width,
                });
            }
        }
        writeln!(self.file, "$upscope $end").unwrap();
        writeln!(self.file, "$enddefinitions $end").unwrap();
        writeln!(self.file, "$dumpvars").unwrap();

        // Dump initial values
        for (_i, sig) in design.top.signals.iter().enumerate() {
            let sig_entries: Vec<&VcdSignalEntry> = self.entries.iter().filter(|e| {
                if sig.array_depth > 1 {
                    e.name.starts_with(&sig.name)
                } else {
                    e.name == sig.name
                }
            }).collect();

            for (e_idx, entry) in sig_entries.iter().enumerate() {
                let elem_val = if sig.array_depth > 1 {
                    let start = e_idx * sig.elem_width;
                    let mut bits = Vec::with_capacity(sig.elem_width);
                    for j in start..start + sig.elem_width {
                        bits.push(sig.init_val.bits.get(j).copied().unwrap_or(LogicVal::X));
                    }
                    crate::ir::LogicVec { width: sig.elem_width, bits }
                } else {
                    sig.init_val.clone()
                };
                let val_str = vec_to_vcd(&elem_val);
                if entry.width == 1 {
                    writeln!(self.file, "{}{}", val_str, entry.code).unwrap();
                } else {
                    writeln!(self.file, "b{} {}", val_str, entry.code).unwrap();
                }
                self.last_values.insert(entry.code.clone(), val_str);
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
        let mut entry_idx = 0usize;
        for (i, sig_val) in state.iter().enumerate() {
            let sig = &design.top.signals[i];
            if sig.array_depth > 1 {
                for elem in 0..sig.array_depth {
                    if entry_idx < self.entries.len() {
                        let entry = &self.entries[entry_idx];
                        let start = elem * sig.elem_width;
                        let mut bits = Vec::with_capacity(sig.elem_width);
                        for j in start..start + sig.elem_width {
                            bits.push(sig_val.bits.get(j).copied().unwrap_or(LogicVal::X));
                        }
                        let elem_val = crate::ir::LogicVec { width: sig.elem_width, bits };
                        let val_str = vec_to_vcd(&elem_val);
                        if self.last_values.get(&entry.code) != Some(&val_str) {
                            if entry.width == 1 {
                                writeln!(self.file, "{}{}", val_str, entry.code).unwrap();
                            } else {
                                writeln!(self.file, "b{} {}", val_str, entry.code).unwrap();
                            }
                            self.last_values.insert(entry.code.clone(), val_str);
                        }
                        entry_idx += 1;
                    }
                }
            } else {
                if entry_idx < self.entries.len() {
                    let entry = &self.entries[entry_idx];
                    let val_str = vec_to_vcd(sig_val);
                    if self.last_values.get(&entry.code) != Some(&val_str) {
                        if entry.width == 1 {
                            writeln!(self.file, "{}{}", val_str, entry.code).unwrap();
                        } else {
                            writeln!(self.file, "b{} {}", val_str, entry.code).unwrap();
                        }
                        self.last_values.insert(entry.code.clone(), val_str);
                    }
                    entry_idx += 1;
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
