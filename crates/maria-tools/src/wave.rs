//! `mwave` — Wave Utility.
//!
//! Bukan viewer. Subcommand:
//! - `merge`   — gabungkan beberapa VCD jadi satu (offset waktu kumulatif)
//! - `export`  — VCD → CSV/TXT
//! - `filter`  — pertahankan subset sinyal

use std::collections::HashMap;
use std::path::PathBuf;

use maria_core::error::SimError;

/// Satu sinyal VCD.
#[derive(Debug, Clone)]
struct VcdSignal {
    id: String,
    name: String,
    width: usize,
    scope: Vec<String>,
}

/// Hasil parse satu file VCD.
#[derive(Debug, Clone)]
struct VcdData {
    timescale: String,
    signals: Vec<VcdSignal>,
    /// (time, signal id, nilai mentah)
    changes: Vec<(u64, String, String)>,
    max_time: u64,
}

/// Opsi mwave.
pub enum WaveArgs {
    Merge {
        inputs: Vec<String>,
        output: Option<String>,
    },
    Export {
        input: String,
        format: String,
        output: Option<String>,
    },
    Filter {
        input: String,
        signals: Vec<String>,
        output: Option<String>,
    },
}

/// Jalankan mwave.
pub fn run(args: &WaveArgs) -> Result<(), SimError> {
    match args {
        WaveArgs::Merge { inputs, output } => merge(inputs, output.as_deref()),
        WaveArgs::Export { input, format, output } => export(input, format, output.as_deref()),
        WaveArgs::Filter { input, signals, output } => filter(input, signals, output.as_deref()),
    }
}

fn err(msg: impl Into<String>) -> SimError {
    SimError::with_diag(maria_core::diagnostics::DiagCode::WaveformError, msg)
}

/// ── Parser VCD ──

fn parse_vcd(path: &str) -> Result<VcdData, SimError> {
    let src = std::fs::read_to_string(path)
        .map_err(|e| err(format!("{}: {}", path, e)))?;
    let lines: Vec<&str> = src.lines().map(|l| l.trim()).collect();

    let mut signals: Vec<VcdSignal> = Vec::new();
    let mut changes: Vec<(u64, String, String)> = Vec::new();
    let mut timescale = String::new();
    let mut scope: Vec<String> = Vec::new();
    let mut max_time = 0u64;
    let mut in_header = true;
    let mut cur_time = 0u64;

    let mut i = 0usize;
    while i < lines.len() {
        let line = lines[i];
        if in_header {
            if let Some(rest) = line.strip_prefix("$scope") {
                // scope module top $end
                let parts: Vec<&str> = rest.split_whitespace().collect();
                if parts.len() >= 2 {
                    scope.push(parts[1].to_string());
                }
            } else if let Some(rest) = line.strip_prefix("$upscope") {
                scope.pop();
            } else if let Some(rest) = line.strip_prefix("$var") {
                let parts: Vec<&str> = rest.split_whitespace().collect();
                // $var wire 1 ! clk $end
                if parts.len() >= 4 {
                    let width = parts[1].parse::<usize>().unwrap_or(1);
                    let id = parts[2].to_string();
                    let name = parts[3].to_string();
                    signals.push(VcdSignal {
                        id,
                        name,
                        width,
                        scope: scope.clone(),
                    });
                }
            } else if let Some(rest) = line.strip_prefix("$timescale") {
                timescale = rest.split_whitespace().next().unwrap_or("1ns").to_string();
            } else if line.starts_with("$enddefinitions") {
                in_header = false;
            }
            i += 1;
            continue;
        }

        // Body: #time atau value line
        if let Some(t) = line.strip_prefix('#') {
            cur_time = t.parse::<u64>().unwrap_or(0);
            if cur_time > max_time {
                max_time = cur_time;
            }
            i += 1;
            continue;
        }

        if let Some(rest) = line.strip_prefix('b') {
            // b1010 <id>
            let mut parts = rest.splitn(2, char::is_whitespace);
            let val = parts.next().unwrap_or("").to_string();
            let id = parts.next().unwrap_or("").trim().to_string();
            if !id.is_empty() {
                changes.push((cur_time, id, val));
            }
        } else if let Some(rest) = line.strip_prefix('B') {
            // B<hex> <id>
            let mut parts = rest.splitn(2, char::is_whitespace);
            let val = parts.next().unwrap_or("").to_string();
            let id = parts.next().unwrap_or("").trim().to_string();
            if !id.is_empty() {
                // Konversi hex → biner agar konsisten
                let bin = hex_to_bin(&val);
                changes.push((cur_time, id, bin));
            }
        } else if line.len() >= 2 {
            // scalar: 0<id>, 1<id>, x<id>, z<id>
            let first = line.as_bytes()[0] as char;
            if matches!(first, '0' | '1' | 'x' | 'z') {
                let id = line[1..].trim().to_string();
                if !id.is_empty() {
                    changes.push((cur_time, id, first.to_string()));
                }
            }
        }
        i += 1;
    }

    if signals.is_empty() {
        return Err(err(format!("{}: bukan file VCD valid (tidak ada $var)", path)));
    }

    Ok(VcdData {
        timescale,
        signals,
        changes,
        max_time,
    })
}

fn hex_to_bin(hex: &str) -> String {
    let mut out = String::new();
    for c in hex.chars() {
        let v = match c.to_digit(16) {
            Some(v) => format!("{:04b}", v),
            None => {
                // x/z → biarkan sebagai 'x'/'z'
                if c == 'x' || c == 'X' {
                    "xxxx".to_string()
                } else if c == 'z' || c == 'Z' {
                    "zzzz".to_string()
                } else {
                    String::new()
                }
            }
        };
        out.push_str(&v);
    }
    out
}

/// Tulis VCD (header + changes) dengan fungsi remap id.
fn write_vcd(
    out_path: &str,
    timescale: &str,
    signals: &[VcdSignal],
    changes: &[(u64, String, String)],
    id_map: &HashMap<String, String>,
) -> Result<(), SimError> {
    let mut out = String::new();
    out.push_str("$date\n  mwave\n$end\n");
    out.push_str("$version\n  maria mwave\n$end\n");
    if !timescale.is_empty() {
        out.push_str(&format!("$timescale {} $end\n", timescale));
    }
    // Kelompokkan signal per scope (urutkan stabil sesuai input)
    let mut prev_scope: Option<Vec<String>> = None;
    for sig in signals {
        if prev_scope.as_ref() != Some(&sig.scope) {
            // Tutup scope sebelumnya
            if prev_scope.is_some() {
                out.push_str("$upscope $end\n");
            }
            for s in &sig.scope {
                out.push_str(&format!("$scope module {} $end\n", s));
            }
            prev_scope = Some(sig.scope.clone());
        }
        let id = id_map.get(&sig.id).cloned().unwrap_or_else(|| sig.id.clone());
        let range = if sig.width > 1 {
            format!(" [{}:0]", sig.width - 1)
        } else {
            String::new()
        };
        out.push_str(&format!(
            "$var wire {} {} {}{} $end\n",
            sig.width, id, sig.name, range
        ));
    }
    if prev_scope.is_some() {
        out.push_str("$upscope $end\n");
    }
    out.push_str("$enddefinitions $end\n");

    // Body: changes diurutkan per waktu
    let mut cur_t = None;
    for (t, id, val) in changes {
        let mapped = id_map.get(id).cloned().unwrap_or_else(|| id.clone());
        if cur_t != Some(*t) {
            out.push_str(&format!("#{}\n", t));
            cur_t = Some(*t);
        }
        if val.len() == 1 && matches!(val.as_bytes()[0] as char, '0' | '1' | 'x' | 'z') {
            out.push_str(&format!("{}{}\n", val, mapped));
        } else {
            out.push_str(&format!("b{} {}\n", val, mapped));
        }
    }

    std::fs::write(out_path, out)
        .map_err(|e| err(format!("{}: {}", out_path, e)))?;
    Ok(())
}

/// ── merge ──

fn merge(inputs: &[String], output: Option<&str>) -> Result<(), SimError> {
    if inputs.len() < 2 {
        return Err(err("merge membutuhkan minimal 2 VCD input"));
    }
    let mut all_signals: Vec<VcdSignal> = Vec::new();
    let mut all_changes: Vec<(u64, String, String)> = Vec::new();
    let mut id_map: HashMap<String, String> = HashMap::new();
    let mut offset = 0u64;
    let mut timescale = String::new();

    for (idx, path) in inputs.iter().enumerate() {
        let data = parse_vcd(path)?;
        let prefix = format!("{}", char::from_u32(0x41 + idx as u32).unwrap_or('A'));
        for sig in &data.signals {
            let new_id = format!("{}{}", prefix, sig.id);
            id_map.insert(sig.id.clone(), new_id.clone());
            all_signals.push(VcdSignal {
                id: new_id,
                name: sig.name.clone(),
                width: sig.width,
                scope: sig.scope.clone(),
            });
        }
        for (t, id, val) in &data.changes {
            all_changes.push((offset + t, id.clone(), val.clone()));
        }
        offset += data.max_time;
        if timescale.is_empty() {
            timescale = data.timescale.clone();
        }
    }

    all_changes.sort_by_key(|(t, _, _)| *t);
    let out_path = output
        .map(|s| s.to_string())
        .unwrap_or_else(|| "merged.vcd".to_string());
    write_vcd(&out_path, &timescale, &all_signals, &all_changes, &id_map)?;
    println!(
        "  merged {} file → {} ({} signals, {} perubahan, max time {})",
        inputs.len(),
        out_path,
        all_signals.len(),
        all_changes.len(),
        offset
    );
    Ok(())
}

/// ── export ──

fn export(input: &str, format: &str, output: Option<&str>) -> Result<(), SimError> {
    let data = parse_vcd(input)?;
    let out_path = output
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("{}.{}", input.replace(".vcd", ""), format));

    match format {
        "csv" => export_csv(&data, &out_path),
        "txt" => export_txt(&data, &out_path),
        other => Err(err(format!("format tidak dikenal: '{}' (pilih csv|txt)", other))),
    }
}

/// Bangun peta sinyal: id → (name, list perubahan (time, value)).
fn signal_timelines(data: &VcdData) -> HashMap<String, Vec<(u64, String)>> {
    let mut map: HashMap<String, Vec<(u64, String)>> = HashMap::new();
    for sig in &data.signals {
        map.entry(sig.id.clone()).or_default();
    }
    for (t, id, val) in &data.changes {
        if let Some(v) = map.get_mut(id) {
            v.push((*t, val.clone()));
        }
    }
    for v in map.values_mut() {
        v.sort_by_key(|(t, _)| *t);
    }
    map
}

fn export_csv(data: &VcdData, out_path: &str) -> Result<(), SimError> {
    let timelines = signal_timelines(data);
    let names: Vec<&VcdSignal> = data.signals.iter().collect();

    let mut header = String::from("time");
    for s in &names {
        header.push(',');
        header.push_str(&full_name(s));
    }

    // Semua momen waktu: 0 + semua waktu perubahan
    let mut times: Vec<u64> = vec![0];
    times.extend(data.changes.iter().map(|(t, _, _)| *t));
    times.sort_unstable();
    times.dedup();

    let mut out = String::new();
    out.push_str(&header);
    out.push('\n');
    for t in &times {
        let mut row = format!("{}", t);
        for s in &names {
            let last = timelines
                .get(&s.id)
                .and_then(|v| v.iter().rev().find(|(tt, _)| tt <= t))
                .map(|(_, v)| v.clone());
            row.push(',');
            match last {
                Some(v) => row.push_str(&v),
                None => row.push('z'),
            }
        }
        out.push_str(&row);
        out.push('\n');
    }

    std::fs::write(out_path, out)
        .map_err(|e| err(format!("{}: {}", out_path, e)))?;
    println!("  exported {} → {} ({}, {} baris)", input_name(data), out_path, "csv", times.len());
    Ok(())
}

fn export_txt(data: &VcdData, out_path: &str) -> Result<(), SimError> {
    let timelines = signal_timelines(data);
    let mut out = String::new();
    for s in &data.signals {
        out.push_str(&format!("{} [{}]:\n", full_name(s), s.width));
        let ts = timelines.get(&s.id).cloned().unwrap_or_default();
        for (t, v) in ts {
            out.push_str(&format!("    #{} = {}\n", t, v));
        }
    }
    std::fs::write(out_path, out)
        .map_err(|e| err(format!("{}: {}", out_path, e)))?;
    println!("  exported {} → {}", input_name(data), out_path);
    Ok(())
}

fn full_name(s: &VcdSignal) -> String {
    let mut name = s.scope.join(".");
    if !name.is_empty() {
        name.push('.');
    }
    name.push_str(&s.name);
    name
}

fn input_name(data: &VcdData) -> String {
    data.signals.first().map(|_| "vcd".to_string()).unwrap_or_default()
}

/// ── filter ──

fn filter(input: &str, keep: &[String], output: Option<&str>) -> Result<(), SimError> {
    let data = parse_vcd(input)?;
    let keep_set: std::collections::HashSet<String> = keep
        .iter()
        .flat_map(|s| s.split([',', ' ', ';']))
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();

    let keep_ids: std::collections::HashSet<String> = data
        .signals
        .iter()
        .filter(|s| keep_set.contains(&s.name) || keep_set.contains(&full_name(s)))
        .map(|s| s.id.clone())
        .collect();

    let signals: Vec<VcdSignal> = data
        .signals
        .iter()
        .filter(|s| keep_ids.contains(&s.id))
        .cloned()
        .collect();
    let changes: Vec<(u64, String, String)> = data
        .changes
        .iter()
        .filter(|(_, id, _)| keep_ids.contains(id))
        .cloned()
        .collect();

    if signals.is_empty() {
        return Err(err(format!(
            "tidak ada sinyal yang cocok — sinyal yang ada: {}",
            data.signals.iter().map(|s| full_name(s)).collect::<Vec<_>>().join(", ")
        )));
    }

    // Remap id berurutan agar ringkas
    let mut id_map: HashMap<String, String> = HashMap::new();
    let mut counter = 33u8;
    for (i, sig) in signals.iter().enumerate() {
        let new_id = format!("{}", char::from_u32(counter as u32).unwrap_or('!'));
        id_map.insert(sig.id.clone(), new_id);
        counter = (counter + 1).max(127);
        let _ = i;
    }

    let out_path = output
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            let base = input.rsplit('/').next().unwrap_or(input);
            format!("filtered_{}", base)
        });
    write_vcd(&out_path, &data.timescale, &signals, &changes, &id_map)?;
    println!(
        "  filtered {} → {} ({} sinyal dipertahankan)",
        input,
        out_path,
        signals.len()
    );
    Ok(())
}

#[allow(dead_code)]
fn _pathbuf(s: &str) -> PathBuf {
    PathBuf::from(s)
}
