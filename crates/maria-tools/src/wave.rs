//! `mwave` — Wave Utility.
//!
//! Bukan viewer. Subcommand:
//! - `merge`   — gabungkan beberapa VCD jadi satu (offset waktu kumulatif)
//! - `export`  — VCD → CSV/TXT
//! - `filter`  — pertahankan subset sinyal
//! - `compare` — bandingkan 2 VCD (perbedaan nilai per signal, WAV-09)
//! - `search`  — cari sinyal by pola wildcard (WAV-10)
//! - `tree`    — index hierarki scope + sinyal (WAV-08)
//! - `stats`   — statistik per sinyal: toggle, transitions, aktivitas (WAV-17)
//! - `decode`  — protokol-aware decode transaksi bus dari VCD: apb /
//!               axi4lite / ahb (WAV-16)

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
pub struct VcdData {
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
    Compare {
        a: String,
        b: String,
    },
    Search {
        input: String,
        patterns: Vec<String>,
    },
    Tree {
        input: String,
    },
    Stats {
        input: String,
    },
    Get {
        /// VCD input
        input: String,
        /// Sinyal yang di-query (koma/space terpisah, dukung * dan ?)
        signals: Vec<String>,
        /// Nilai pada waktu T (random access — perubahan terakhir ≤ T)
        at: Option<u64>,
        /// Rentang waktu t1:t2 (semua perubahan dalam [t1, t2])
        range: Option<(u64, u64)>,
    },
    Decode {
        /// VCD input
        input: String,
        /// Protokol bus: apb (default) | axi4lite | ahb
        proto: String,
    },
}

/// Jalankan mwave.
pub fn run(args: &WaveArgs) -> Result<(), SimError> {
    match args {
        WaveArgs::Merge { inputs, output } => merge(inputs, output.as_deref()),
        WaveArgs::Export {
            input,
            format,
            output,
        } => export(input, format, output.as_deref()),
        WaveArgs::Filter {
            input,
            signals,
            output,
        } => filter(input, signals, output.as_deref()),
        WaveArgs::Compare { a, b } => compare(a, b),
        WaveArgs::Search { input, patterns } => search(input, patterns),
        WaveArgs::Tree { input } => tree(input),
        WaveArgs::Stats { input } => stats(input),
        WaveArgs::Get {
            input,
            signals,
            at,
            range,
        } => get(input, signals, *at, *range),
        WaveArgs::Decode { input, proto } => decode(input, proto),
    }
}

fn err(msg: impl Into<String>) -> SimError {
    SimError::with_diag(maria_core::diagnostics::DiagCode::WaveformError, msg)
}

/// ── Parser VCD ──

fn parse_vcd(path: &str) -> Result<VcdData, SimError> {
    let src = std::fs::read_to_string(path).map_err(|e| err(format!("{}: {}", path, e)))?;
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
            } else if let Some(_rest) = line.strip_prefix("$upscope") {
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
        return Err(err(format!(
            "{}: bukan file VCD valid (tidak ada $var)",
            path
        )));
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
        let id = id_map
            .get(&sig.id)
            .cloned()
            .unwrap_or_else(|| sig.id.clone());
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

    std::fs::write(out_path, out).map_err(|e| err(format!("{}: {}", out_path, e)))?;
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
        other => Err(err(format!(
            "format tidak dikenal: '{}' (pilih csv|txt)",
            other
        ))),
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

    std::fs::write(out_path, out).map_err(|e| err(format!("{}: {}", out_path, e)))?;
    println!(
        "  exported {} → {} ({}, {} baris)",
        input_name(data),
        out_path,
        "csv",
        times.len()
    );
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
    std::fs::write(out_path, out).map_err(|e| err(format!("{}: {}", out_path, e)))?;
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
    data.signals
        .first()
        .map(|_| "vcd".to_string())
        .unwrap_or_default()
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
            data.signals
                .iter()
                .map(|s| full_name(s))
                .collect::<Vec<_>>()
                .join(", ")
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

    let out_path = output.map(|s| s.to_string()).unwrap_or_else(|| {
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

/// ── compare ──
///
/// Bandingkan dua VCD (WAV-09): untuk setiap signal yang ada di KEDUA file,
/// bangun timeline nilai (time → value) dan laporkan:
/// - jumlah waktu sampel di mana nilai berbeda (first mismatch + count),
/// - signal yang hanya ada di salah satu file (missing),
/// - range waktu yang dibandingkan (union dari kedua max_time).
///
/// Pendekatan deterministik: sample per detik/unit timescale gabungan.
fn compare(a: &str, b: &str) -> Result<(), SimError> {
    let da = parse_vcd(a)?;
    let db = parse_vcd(b)?;
    let r = compare_data(&da, &db);

    // ── Laporan ──
    println!("  compare {} vs {}", a, b);
    println!("    timescale: {} vs {}", da.timescale, db.timescale);
    println!(
        "    signals:   {} vs {} ({} sinyal umum, {} hanya di A, {} hanya di B)",
        da.signals.len(),
        db.signals.len(),
        r.common,
        r.only_a.len(),
        r.only_b.len()
    );
    println!("    max_time:  {} vs {}", r.max_time_a, r.max_time_b);
    println!("    total mismatch: {}", r.mismatches.len());
    for (name, t, fa, fb, cnt) in &r.mismatches {
        let fa_s = if fa.len() == 1 {
            fa.clone()
        } else {
            format!("b{} ", fa)
        };
        let fb_s = if fb.len() == 1 {
            fb.clone()
        } else {
            format!("b{} ", fb)
        };
        println!(
            "      {}  first t={}  a={} b={}  ({} sample berbeda)",
            name,
            t,
            fa_s.trim_end(),
            fb_s.trim_end(),
            cnt
        );
    }
    for n in &r.only_a {
        println!("      [hanya di A] {}", n);
    }
    for n in &r.only_b {
        println!("      [hanya di B] {}", n);
    }
    Ok(())
}

/// Hasil perbandingan dua VCD (dipisah dari pencetakan agar bisa diuji).
struct CompareResult {
    common: usize,
    only_a: Vec<String>,
    only_b: Vec<String>,
    mismatches: Vec<(String, u64, String, String, usize)>,
    max_time_a: u64,
    max_time_b: u64,
}

/// Logika perbandingan dua VCD: timeline per signal, sample per unit waktu.
fn compare_data(da: &VcdData, db: &VcdData) -> CompareResult {
    // Indeks signal per nama penuh (scope.name).
    let idx_a: HashMap<String, &VcdSignal> = da.signals.iter().map(|s| (full_name(s), s)).collect();
    let idx_b: HashMap<String, &VcdSignal> = db.signals.iter().map(|s| (full_name(s), s)).collect();

    // Timeline per signal: (time, value) terurut dari changes.
    let mut tl_a: HashMap<String, Vec<(u64, String)>> = HashMap::new();
    for (t, id, val) in &da.changes {
        if let Some(sig) = da.signals.iter().find(|s| &s.id == id) {
            tl_a.entry(full_name(sig))
                .or_default()
                .push((*t, val.clone()));
        }
    }
    let mut tl_b: HashMap<String, Vec<(u64, String)>> = HashMap::new();
    for (t, id, val) in &db.changes {
        if let Some(sig) = db.signals.iter().find(|s| &s.id == id) {
            tl_b.entry(full_name(sig))
                .or_default()
                .push((*t, val.clone()));
        }
    }

    // Nama gabungan (union) — urutan stabil: A dulu, lalu B yang tidak di A.
    let mut names: Vec<String> = idx_a.keys().cloned().collect();
    names.sort();
    for n in idx_b.keys() {
        if !idx_a.contains_key(n) {
            names.push(n.clone());
        }
    }

    let end = da.max_time.max(db.max_time);

    let mut only_a: Vec<String> = Vec::new();
    let mut only_b: Vec<String> = Vec::new();
    let mut mismatches: Vec<(String, u64, String, String, usize)> = Vec::new();

    for name in &names {
        let in_a = idx_a.contains_key(name);
        let in_b = idx_b.contains_key(name);
        match (in_a, in_b) {
            (true, false) => only_a.push(name.clone()),
            (false, true) => only_b.push(name.clone()),
            (false, false) => {} // union names — tidak mungkin
            (true, true) => {
                let ta = tl_a.get(name).cloned().unwrap_or_default();
                let tb = tl_b.get(name).cloned().unwrap_or_default();
                // Nilai pada tiap saat: value yang berlaku (last change ≤ t).
                let val_at = |tl: &[(u64, String)], t: u64| -> Option<String> {
                    tl.iter()
                        .rev()
                        .find(|(tt, _)| *tt <= t)
                        .map(|(_, v)| v.clone())
                };
                let mut first: Option<(u64, String, String)> = None;
                let mut count = 0usize;
                let mut t = 0u64;
                while t <= end {
                    let va = val_at(&ta, t);
                    let vb = val_at(&tb, t);
                    // Kedua punya nilai & berbeda → mismatch.
                    if va.is_some() && vb.is_some() && va != vb {
                        count += 1;
                        if first.is_none() {
                            first = Some((t, va.unwrap(), vb.unwrap()));
                        }
                    }
                    t += 1;
                }
                if count > 0 {
                    let (ft, fa, fb) = first.unwrap();
                    mismatches.push((name.clone(), ft, fa, fb, count));
                }
            }
        }
    }

    CompareResult {
        common: names.len() - only_a.len() - only_b.len(),
        only_a,
        only_b,
        mismatches,
        max_time_a: da.max_time,
        max_time_b: db.max_time,
    }
}

/// ── search ──
///
/// Cari sinyal VCD yang cocok dengan pola wildcard (WAV-10). Pola bisa
/// memakai `*` (any run) dan `?` (satu karakter). Cocok terhadap nama
/// polos (`cnt`) maupun nama penuh (`top.cnt`, hierarki). Output: daftar
/// nama + lebar + scope, urutan kemunculan di file.
fn search(input: &str, patterns: &[String]) -> Result<(), SimError> {
    let data = parse_vcd(input)?;
    let pats: Vec<&str> = patterns
        .iter()
        .flat_map(|p| p.split([',', ' ', ';']))
        .filter(|p| !p.is_empty())
        .collect();
    if pats.is_empty() {
        return Err(err("search membutuhkan minimal 1 pola (dukung * dan ?)"));
    }

    let mut found: Vec<(&VcdSignal, String)> = Vec::new();
    for sig in &data.signals {
        let full = full_name(sig);
        let hit = pats
            .iter()
            .any(|p| wildcard_match(p, &sig.name) || wildcard_match(p, &full));
        if hit {
            found.push((sig, full));
        }
    }

    if found.is_empty() {
        println!("  tidak ada sinyal yang cocok dengan: {}", pats.join(", "));
        return Ok(());
    }
    println!("  {} sinyal cocok dengan: {}", found.len(), pats.join(", "));
    for (sig, full) in &found {
        println!(
            "    {}  width={}  scope={}",
            full,
            sig.width,
            if sig.scope.is_empty() {
                "<top>".to_string()
            } else {
                sig.scope.join(".")
            }
        );
    }
    Ok(())
}

/// ── tree ──
///
/// Index hierarki scope + sinyal dari VCD (WAV-08): kelompokkan signal per
/// scope (dari $scope/$upscope), cetak sebagai pohon dengan indentasi dan
/// lebar tiap signal. Berguna untuk menemukan path lengkap signal sebelum
/// `filter`/`search`.
fn tree(input: &str) -> Result<(), SimError> {
    let data = parse_vcd(input)?;
    // scope → daftar signal (urut kemunculan)
    let mut scopes: std::collections::BTreeMap<Vec<String>, Vec<&VcdSignal>> =
        std::collections::BTreeMap::new();
    for sig in &data.signals {
        scopes.entry(sig.scope.clone()).or_default().push(sig);
    }

    println!(
        "  waveform tree: {} ({} signal, {} scope)",
        input,
        data.signals.len(),
        scopes.len()
    );
    for (scope, sigs) in &scopes {
        // Indentasi scope: 4 + 2 per kedalaman (pohon hierarki).
        let indent = "    ".repeat(scope.len().saturating_add(1));
        let name = if scope.is_empty() {
            "<top>".to_string()
        } else {
            scope.join(".")
        };
        println!("  {} {}", indent, name);
        let sig_indent = "    ".repeat(scope.len().saturating_add(2));
        for sig in sigs {
            println!(
                "  {}{:<8} bits={:<5} {}",
                sig_indent,
                format!("[{}:0]", sig.width.saturating_sub(1)),
                sig.width,
                sig.name
            );
        }
    }
    Ok(())
}

/// ── get ──
///
/// Random access query nilai sinyal dari VCD (WAV-07): diberikan pola sinyal
/// (wildcard `*`/`?`) dan mode query, kembalikan nilai sinyal tanpa harus
/// memindai seluruh dump secara manual. Tiga mode:
///   - `--at T`      : sample di waktu T (perubahan terakhir ≤ T; "x" bila
///                     belum ada perubahan) — random access murni.
///   - `--range a:b` : semua perubahan dalam [a, b].
///   - (tanpa flag)  : timeline penuh per sinyal.
///
/// Hasil terstruktur di `get_data` (bisa diuji); `get` hanya mencetak.

/// Satu entri hasil query: sinyal + daftar (waktu, nilai mentah bits).
#[derive(Debug, Clone)]
struct GetEntry {
    name: String,
    width: usize,
    values: Vec<(u64, String)>,
}

fn get(input: &str, patterns: &[String], at: Option<u64>, range: Option<(u64, u64)>) -> Result<(), SimError> {
    let data = parse_vcd(input)?;
    let entries = get_data(&data, patterns, at, range)?;

    println!(
        "  get: {} ({} sinyal cocok, timescale={})",
        input,
        entries.len(),
        data.timescale
    );
    for e in &entries {
        match (at, range) {
            (Some(t), _) => {
                let v = e.values.first().map(|(_, v)| v.as_str()).unwrap_or("x");
                println!("  {} [{}:0] @ {} = {}", e.name, e.width - 1, t, format_bits(v));
            }
            (None, Some((lo, hi))) => {
                println!("  {} [{}:0] [{}:{}]", e.name, e.width - 1, lo, hi);
                for (t, v) in &e.values {
                    println!("    {:>10}  {}", t, format_bits(v));
                }
            }
            (None, None) => {
                println!(
                    "  {} [{}:0] ({} perubahan)",
                    e.name,
                    e.width - 1,
                    e.values.len()
                );
                for (t, v) in &e.values {
                    println!("    {:>10}  {}", t, format_bits(v));
                }
            }
        }
    }
    Ok(())
}

/// Core query (murni, bisa diuji): resolve pola → entri nilai.
/// Mode `at`: satu sample per sinyal. Mode range/full: perubahan dalam window.
fn get_data(
    data: &VcdData,
    patterns: &[String],
    at: Option<u64>,
    range: Option<(u64, u64)>,
) -> Result<Vec<GetEntry>, SimError> {
    let pats: Vec<&str> = patterns
        .iter()
        .flat_map(|p| p.split([',', ' ', ';']))
        .filter(|p| !p.is_empty())
        .collect();
    if pats.is_empty() {
        return Err(err("get membutuhkan minimal 1 pola sinyal (dukung * dan ?)"));
    }

    let mut entries: Vec<GetEntry> = Vec::new();
    for sig in &data.signals {
        let full = full_name(sig);
        let hit = pats
            .iter()
            .any(|p| wildcard_match(p, &sig.name) || wildcard_match(p, &full));
        if !hit {
            continue;
        }
        let tl = timeline_of(data, sig);
        let values = match at {
            Some(t) => vec![(t, sample_at(&tl, t))],
            None => match range {
                Some((lo, hi)) => tl.into_iter().filter(|(t, _)| *t >= lo && *t <= hi).collect(),
                None => tl,
            },
        };
        entries.push(GetEntry {
            name: full,
            width: sig.width,
            values,
        });
    }

    if entries.is_empty() {
        return Err(err(format!(
            "tidak ada sinyal yang cocok dengan: {}",
            pats.join(", ")
        )));
    }
    Ok(entries)
}

/// Format nilai untuk tampilan: bits mentah + desimal bila biner murni.
fn format_bits(v: &str) -> String {
    if !v.is_empty() && v.chars().all(|c| c == '0' || c == '1') {
        format!("{} ({})", v, bits_val(v))
    } else {
        v.to_string()
    }
}

/// ── stats ──
///
/// Statistik aktivitas per sinyal (WAV-17): dari timeline VCD hitung per
/// sinyal — jumlah transisi (toggle count), jumlah perubahan nilai yang
/// tercatat, first/last change, dan aktivitas (persentase waktu di mana
/// sinyal punya nilai berbeda dari nilai awal). Berguna untuk menilai
/// signal yang "stuck" (0 aktivitas) atau toggling berlebihan.
struct SignalStats {
    name: String,
    width: usize,
    toggles: usize,
    changes: usize,
    first: u64,
    last: u64,
    activity: u64,
    duration: u64,
}

fn stats(input: &str) -> Result<(), SimError> {
    let data = parse_vcd(input)?;
    let r = stats_data(&data);

    // ── Laporan ──
    println!(
        "  stats: {} ({} sinyal, max_time={}, timescale={})",
        input,
        r.len(),
        data.max_time,
        data.timescale
    );
    println!(
        "  {:<22} {:<5} {:<7} {:<7} {:<10} {:<7} {}",
        "signal", "width", "toggle", "change", "first", "last", "activity%"
    );
    let mut total_toggle = 0usize;
    let mut stuck = 0usize;
    for s in &r {
        total_toggle += s.toggles;
        if s.toggles == 0 {
            stuck += 1;
        }
        let act = if s.duration > 0 {
            s.activity * 100 / s.duration
        } else {
            0
        };
        println!(
            "  {:<22} {:<5} {:<7} {:<7} {:<10} {:<7} {}%",
            s.name, s.width, s.toggles, s.changes, s.first, s.last, act
        );
    }
    println!(
        "  total toggle: {} ({} sinyal stuck / 0 aktivitas)",
        total_toggle, stuck
    );
    Ok(())
}

/// Logika statistik per sinyal (dipisah dari pencetakan agar bisa diuji).
/// Toggle = transisi nilai antar perubahan berurutan (bitwise berbeda),
/// activity = jumlah unit waktu di mana nilai != nilai commit pertama.
fn stats_data(data: &VcdData) -> Vec<SignalStats> {
    let mut out: Vec<SignalStats> = Vec::new();
    let end = data.max_time;

    for sig in &data.signals {
        let full = full_name(sig);
        let mut tl: Vec<(u64, String)> = data
            .changes
            .iter()
            .filter(|(_, id, _)| id == &sig.id)
            .map(|(t, _, v)| (*t, v.clone()))
            .collect();
        tl.sort_by_key(|(t, _)| *t);

        if tl.is_empty() {
            out.push(SignalStats {
                name: full,
                width: sig.width,
                toggles: 0,
                changes: 0,
                first: 0,
                last: 0,
                activity: 0,
                duration: end,
            });
            continue;
        }

        let first_t = tl[0].0;
        let last_t = tl[tl.len() - 1].0;
        let mut toggles = 0usize;
        let mut prev = tl[0].1.clone();
        for (_, v) in tl.iter().skip(1) {
            if v != &prev {
                toggles += 1;
            }
            prev = v.clone();
        }
        // Aktivitas: waktu di mana nilai berlaku != nilai commit pertama.
        let baseline = tl[0].1.clone();
        let mut activity = 0u64;
        for w in tl.windows(2) {
            let (t0, v0) = &w[0];
            let (t1, _) = &w[1];
            if *v0 != baseline {
                activity += t1 - t0;
            }
        }
        if let Some((t_last, v_last)) = tl.last() {
            if *v_last != baseline {
                activity += end.saturating_sub(*t_last);
            }
        }
        out.push(SignalStats {
            name: full,
            width: sig.width,
            toggles,
            changes: tl.len(),
            first: first_t,
            last: last_t,
            activity,
            duration: end,
        });
    }
    out
}

/// Wildcard match sederhana: `*` = any run (termasuk kosong), `?` = satu
/// karakter, sisanya literal. Case-sensitive (nama signal SV case-sensitive).
fn wildcard_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    // DP: dp[i][j] = pattern[..i] cocok text[..j].
    let mut dp = vec![vec![false; t.len() + 1]; p.len() + 1];
    dp[0][0] = true;
    for i in 1..=p.len() {
        if p[i - 1] == '*' {
            dp[i][0] = dp[i - 1][0];
        }
    }
    for i in 1..=p.len() {
        for j in 1..=t.len() {
            dp[i][j] = match p[i - 1] {
                '*' => dp[i - 1][j] || dp[i][j - 1],
                '?' => dp[i - 1][j - 1],
                c => dp[i - 1][j - 1] && c == t[j - 1],
            };
        }
    }
    dp[p.len()][t.len()]
}

#[allow(dead_code)]
fn _pathbuf(s: &str) -> PathBuf {
    PathBuf::from(s)
}

/// ── Protocol-aware decode (WAV-16) ──
///
/// Dekode transaksi bus dari timeline VCD. Mendukung:
/// - `apb`      — AMBA 3/4 APB (psel/penable/pwrite/paddr/pwdata/prdata/pready)
/// - `axi4lite` — AXI4-Lite (aw/w/b channel + ar/r channel, handshake valid/ready)
/// - `ahb`      — AHB-Lite single transfer (htrans NONSEQ/SEQ + hready pipelining)

/// Satu transaksi ter-dekode dari waveform.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedTx {
    /// nomor urut transaksi (1-based)
    pub index: usize,
    /// "WRITE" | "READ"
    pub kind: &'static str,
    pub start: u64,
    pub end: u64,
    pub addr: u64,
    /// wdata (write) atau rdata (read)
    pub data: u64,
    /// response AXI4-Lite (bresp/rresp), None utk APB/AHB
    pub resp: Option<u64>,
}

/// Cari sinyal by nama komponen terakhir (case-sensitive dulu, lalu case-insensitive).
fn find_signal<'a>(data: &'a VcdData, name: &str) -> Option<&'a VcdSignal> {
    let last = |s: &VcdSignal| full_name(s).rsplit('.').next().unwrap_or("").to_string();
    data.signals.iter().find(|s| last(s) == name).or_else(|| {
        data.signals
            .iter()
            .find(|s| last(s).eq_ignore_ascii_case(name))
    })
}

/// Timeline (time, nilai) untuk satu sinyal, sorted by time.
fn timeline_of(data: &VcdData, sig: &VcdSignal) -> Vec<(u64, String)> {
    let mut tl: Vec<(u64, String)> = data
        .changes
        .iter()
        .filter(|(_, id, _)| id == &sig.id)
        .map(|(t, _, v)| (*t, v.clone()))
        .collect();
    tl.sort_by_key(|(t, _)| *t);
    tl
}

/// Nilai sinyal pada waktu t (perubahan terakhir ≤ t; "x" bila belum ada).
fn sample_at(tl: &[(u64, String)], t: u64) -> String {
    let mut v = String::from("x");
    for (tt, val) in tl {
        if *tt <= t {
            v = val.clone();
        } else {
            break;
        }
    }
    v
}

/// Konversi string bit ("0101") → u64 (bit x/z dianggap 0).
fn bits_val(bits: &str) -> u64 {
    let mut v = 0u64;
    for c in bits.chars() {
        v = (v << 1) | u64::from(c == '1');
    }
    v
}

/// Union sorted-unik dari waktu perubahan beberapa timeline.
fn union_times(tls: &[&[(u64, String)]]) -> Vec<u64> {
    let mut ts: Vec<u64> = tls
        .iter()
        .flat_map(|tl| tl.iter().map(|(t, _)| *t))
        .collect();
    ts.sort_unstable();
    ts.dedup();
    ts
}

/// Decode APB: SETUP (psel=1, penable=0) → ACCESS (psel=1, penable=1),
/// selesai saat pready=1. pready tidak ada → diasumsikan selalu 1.
pub fn apb_decode(data: &VcdData) -> Result<Vec<DecodedTx>, SimError> {
    for n in ["psel", "penable", "pwrite", "paddr"] {
        if find_signal(data, n).is_none() {
            return Err(err(format!("decode apb: sinyal '{}' tidak ditemukan", n)));
        }
    }
    let tl = |n: &str| {
        find_signal(data, n)
            .map(|s| timeline_of(data, s))
            .unwrap_or_default()
    };
    let psel = tl("psel");
    let penable = tl("penable");
    let pwrite = tl("pwrite");
    let paddr = tl("paddr");
    let pwdata = tl("pwdata");
    let prdata = tl("prdata");
    // pready absent → selalu ready (AMBA: tanpa wait state).
    let pready = find_signal(data, "pready")
        .map(|s| timeline_of(data, s))
        .unwrap_or_else(|| vec![(0, "1".to_string())]);

    let times = union_times(&[&psel, &penable, &pready]);
    let mut out: Vec<DecodedTx> = Vec::new();
    // transaksi berjalan: (write, start, addr, wdata)
    let mut cur: Option<(bool, u64, u64, u64)> = None;

    for &t in &times {
        let sel = sample_at(&psel, t) == "1";
        let en = sample_at(&penable, t) == "1";
        let rdy = sample_at(&pready, t) == "1";

        if sel && !en {
            // fase SETUP — mulai transaksi baru
            cur = Some((
                sample_at(&pwrite, t) == "1",
                t,
                bits_val(&sample_at(&paddr, t)),
                bits_val(&sample_at(&pwdata, t)),
            ));
        } else if sel && en && rdy {
            // ACCESS dengan pready=1 — transaksi selesai
            if let Some((write, start, addr, wdata)) = cur.take() {
                let data_v = if write {
                    wdata
                } else {
                    bits_val(&sample_at(&prdata, t))
                };
                out.push(DecodedTx {
                    index: out.len() + 1,
                    kind: if write { "WRITE" } else { "READ" },
                    start,
                    end: t,
                    addr,
                    data: data_v,
                    resp: None,
                });
            }
        }
    }
    Ok(out)
}

/// Handshake valid/ready pada waktu t.
fn handshake(vld: &[(u64, String)], rdy: &[(u64, String)], t: u64) -> bool {
    sample_at(vld, t) == "1" && sample_at(rdy, t) == "1"
}

/// Decode AXI4-Lite: write (aw/w/b) + read (ar/r) channel handshake valid/ready.
pub fn axi4lite_decode(data: &VcdData) -> Result<Vec<DecodedTx>, SimError> {
    let tl = |n: &str| {
        find_signal(data, n)
            .map(|s| timeline_of(data, s))
            .unwrap_or_default()
    };
    let has_write = find_signal(data, "awvalid").is_some();
    let has_read = find_signal(data, "arvalid").is_some();
    if !has_write && !has_read {
        return Err(err(
            "decode axi4lite: tidak ada channel write (awvalid) / read (arvalid)",
        ));
    }

    let mut txs: Vec<DecodedTx> = Vec::new();

    if has_write {
        let awv = tl("awvalid");
        let awr = tl("awready");
        let wv = tl("wvalid");
        let wr = tl("wready");
        let bv = tl("bvalid");
        let br = tl("bready");
        let awaddr = tl("awaddr");
        let wdata = tl("wdata");
        let bresp = tl("bresp");

        let mut addr: Option<(u64, u64)> = None; // (start, addr)
        let mut wdat = 0u64;
        let mut have_wdata = false;
        let mut times = union_times(&[&awv, &awr, &wv, &wr, &bv, &br]);
        times.sort_unstable();

        for &t in &times {
            if handshake(&awv, &awr, t) {
                addr = Some((t, bits_val(&sample_at(&awaddr, t))));
                have_wdata = false;
            }
            if handshake(&wv, &wr, t) && !have_wdata {
                wdat = bits_val(&sample_at(&wdata, t));
                have_wdata = true;
            }
            if handshake(&bv, &br, t) {
                if let Some((start, a)) = addr.take() {
                    txs.push(DecodedTx {
                        index: 0,
                        kind: "WRITE",
                        start,
                        end: t,
                        addr: a,
                        data: wdat,
                        resp: Some(bits_val(&sample_at(&bresp, t))),
                    });
                }
            }
        }
    }

    if has_read {
        let arv = tl("arvalid");
        let arr = tl("arready");
        let rv = tl("rvalid");
        let rr = tl("rready");
        let araddr = tl("araddr");
        let rdata = tl("rdata");
        let rresp = tl("rresp");

        let mut addr: Option<(u64, u64)> = None;
        let mut times = union_times(&[&arv, &arr, &rv, &rr]);
        times.sort_unstable();

        for &t in &times {
            if handshake(&arv, &arr, t) {
                addr = Some((t, bits_val(&sample_at(&araddr, t))));
            }
            if handshake(&rv, &rr, t) {
                if let Some((start, a)) = addr.take() {
                    txs.push(DecodedTx {
                        index: 0,
                        kind: "READ",
                        start,
                        end: t,
                        addr: a,
                        data: bits_val(&sample_at(&rdata, t)),
                        resp: Some(bits_val(&sample_at(&rresp, t))),
                    });
                }
            }
        }
    }

    txs.sort_by_key(|tx| tx.end);
    for (i, tx) in txs.iter_mut().enumerate() {
        tx.index = i + 1;
    }
    Ok(txs)
}

/// Decode AHB-Lite single transfer: alamat phase saat htrans=NONSEQ("10")/
/// SEQ("11") dengan hready=1; data phase selesai pada hready=1 berikutnya.
pub fn ahb_decode(data: &VcdData) -> Result<Vec<DecodedTx>, SimError> {
    for n in ["htrans", "haddr", "hwrite", "hready"] {
        if find_signal(data, n).is_none() {
            return Err(err(format!("decode ahb: sinyal '{}' tidak ditemukan", n)));
        }
    }
    let tl = |n: &str| {
        find_signal(data, n)
            .map(|s| timeline_of(data, s))
            .unwrap_or_default()
    };
    let htrans = tl("htrans");
    let haddr = tl("haddr");
    let hwrite = tl("hwrite");
    let hready = tl("hready");
    let hwdata = tl("hwdata");
    let hrdata = tl("hrdata");

    let is_addr_phase = |tr: &str| tr == "10" || tr == "11"; // NONSEQ | SEQ

    let times = union_times(&[&htrans, &hready]);
    let mut out: Vec<DecodedTx> = Vec::new();
    // transaksi menunggu data phase: (write, start, addr)
    let mut cur: Option<(bool, u64, u64)> = None;

    for &t in &times {
        let tr = sample_at(&htrans, t);
        let rdy = sample_at(&hready, t) == "1";

        match cur.take() {
            None => {
                if is_addr_phase(&tr) && rdy {
                    cur = Some((
                        sample_at(&hwrite, t) == "1",
                        t,
                        bits_val(&sample_at(&haddr, t)),
                    ));
                }
            }
            Some((write, start, addr)) => {
                if rdy {
                    let data_v = if write {
                        bits_val(&sample_at(&hwdata, t))
                    } else {
                        bits_val(&sample_at(&hrdata, t))
                    };
                    out.push(DecodedTx {
                        index: out.len() + 1,
                        kind: if write { "WRITE" } else { "READ" },
                        start,
                        end: t,
                        addr,
                        data: data_v,
                        resp: None,
                    });
                } else {
                    cur = Some((write, start, addr));
                }
                // Pipelining: address phase transfer baru bisa dimulai pada
                // cycle yang sama dengan data phase sebelumnya selesai.
                if rdy && is_addr_phase(&tr) {
                    cur = Some((
                        sample_at(&hwrite, t) == "1",
                        t,
                        bits_val(&sample_at(&haddr, t)),
                    ));
                }
            }
        }
    }
    Ok(out)
}

/// Subcommand `decode` — dekode transaksi protokol bus dari VCD (WAV-16).
fn decode(input: &str, proto: &str) -> Result<(), SimError> {
    let data = parse_vcd(input)?;
    let p = proto.to_ascii_lowercase().replace('_', "");
    let txs = match p.as_str() {
        "apb" => apb_decode(&data)?,
        "axi4lite" | "axi-lite" | "axilite" => axi4lite_decode(&data)?,
        "ahb" | "ahblite" | "ahb-lite" => ahb_decode(&data)?,
        other => {
            return Err(err(format!(
                "protokol tidak dikenal: '{}' (pilihan: apb | axi4lite | ahb)",
                other
            )))
        }
    };

    println!(
        "  decode: {} (proto={}, {} sinyal, max_time={})",
        input,
        proto,
        data.signals.len(),
        data.max_time
    );
    for tx in &txs {
        let dir = if tx.kind == "WRITE" { "wdata" } else { "rdata" };
        match tx.resp {
            Some(r) => println!(
                "  #{:<3} {:<5} [{:>5}..{:>5}] addr=0x{:x} {}=0x{:x} resp=0x{:x}",
                tx.index, tx.kind, tx.start, tx.end, tx.addr, dir, tx.data, r
            ),
            None => println!(
                "  #{:<3} {:<5} [{:>5}..{:>5}] addr=0x{:x} {}=0x{:x}",
                tx.index, tx.kind, tx.start, tx.end, tx.addr, dir, tx.data
            ),
        }
    }
    let nw = txs.iter().filter(|t| t.kind == "WRITE").count();
    let nr = txs.len() - nw;
    println!(
        "  total: {} transaksi ({} write, {} read)",
        txs.len(),
        nw,
        nr
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vcd(extra: bool, cnt_seq: &[&str]) -> String {
        let mut s = String::new();
        s.push_str("$timescale 1ns $end\n");
        s.push_str("$scope module top $end\n");
        s.push_str("$var wire 1 ! clk $end\n");
        s.push_str("$var wire 4 \" cnt $end\n");
        if extra {
            s.push_str("$var wire 2 # extra $end\n");
        }
        s.push_str("$upscope $end\n");
        s.push_str("$enddefinitions $end\n");
        s.push_str("#0\n0!\nb0000 \"\n");
        for (i, v) in cnt_seq.iter().enumerate() {
            let t = (i + 1) * 5;
            s.push_str(&format!("#{}\n", t));
            if i % 2 == 0 {
                s.push_str("1!\n");
            } else {
                s.push_str("0!\n");
            }
            s.push_str(&format!("b{} \"\n", v));
        }
        s
    }

    #[test]
    fn test_compare_identical_vcds() {
        std::fs::write("/tmp/__cmp_ident_a.vcd", vcd(false, &["0010", "0011"])).unwrap();
        std::fs::write("/tmp/__cmp_ident_b.vcd", vcd(false, &["0010", "0011"])).unwrap();
        let a = parse_vcd("/tmp/__cmp_ident_a.vcd").unwrap();
        let b = parse_vcd("/tmp/__cmp_ident_b.vcd").unwrap();
        let r = compare_data(&a, &b);
        assert_eq!(r.common, 2, "2 sinyal umum (clk + cnt)");
        assert!(r.only_a.is_empty(), "tidak ada sinyal hanya di A");
        assert!(r.only_b.is_empty(), "tidak ada sinyal hanya di B");
        assert!(r.mismatches.is_empty(), "VCD identik → 0 mismatch");
        let _ = std::fs::remove_file("/tmp/__cmp_ident_a.vcd");
        let _ = std::fs::remove_file("/tmp/__cmp_ident_b.vcd");
        let _ = a;
    }

    #[test]
    fn test_compare_detects_mismatch_and_only_b() {
        std::fs::write("/tmp/__cmp_diff_a.vcd", vcd(false, &["0010", "0011"])).unwrap();
        std::fs::write("/tmp/__cmp_diff_b.vcd", vcd(true, &["0101", "0101"])).unwrap();
        let a = parse_vcd("/tmp/__cmp_diff_a.vcd").unwrap();
        let b = parse_vcd("/tmp/__cmp_diff_b.vcd").unwrap();
        let r = compare_data(&a, &b);
        // cnt berbeda mulai t=5: a=b0010 vs b=b0101.
        let cnt = r
            .mismatches
            .iter()
            .find(|(name, _, _, _, _)| name == "top.cnt")
            .expect("cnt harus mismatch");
        assert_eq!(cnt.1, 5, "first mismatch di t=5");
        assert_eq!(cnt.2, "0010", "nilai A");
        assert_eq!(cnt.3, "0101", "nilai B");
        assert!(cnt.4 >= 1, "ada sample berbeda");
        // extra hanya di B.
        assert_eq!(r.only_b, vec!["top.extra"], "extra hanya ada di B");
        assert!(r.only_a.is_empty());
        let _ = std::fs::remove_file("/tmp/__cmp_diff_a.vcd");
        let _ = std::fs::remove_file("/tmp/__cmp_diff_b.vcd");
    }

    #[test]
    fn test_tree_indexes_scopes() {
        // VCD dengan 2 scope: top.clk/cnt + top.sub.extra.
        let src = "$timescale 1ns $end\n\
                   $scope module top $end\n\
                   $var wire 1 ! clk $end\n\
                   $scope module sub $end\n\
                   $var wire 2 # extra $end\n\
                   $upscope $end\n\
                   $upscope $end\n\
                   $enddefinitions $end\n\
                   #0\n0!\nb00 #\n";
        std::fs::write("/tmp/__tree.vcd", src).unwrap();
        let data = parse_vcd("/tmp/__tree.vcd").unwrap();
        // 2 scope: [top] dan [top, sub]; signal terkelompok benar.
        let mut scopes: std::collections::BTreeMap<Vec<String>, usize> =
            std::collections::BTreeMap::new();
        for sig in &data.signals {
            *scopes.entry(sig.scope.clone()).or_insert(0) += 1;
        }
        assert_eq!(scopes.len(), 2, "2 scope: {}", data.signals.len());
        assert_eq!(
            scopes.get(&vec!["top".to_string()]),
            Some(&1),
            "top punya 1 signal"
        );
        assert_eq!(
            scopes.get(&vec!["top".to_string(), "sub".to_string()]),
            Some(&1),
            "top.sub punya 1 signal"
        );
        let _ = std::fs::remove_file("/tmp/__tree.vcd");
    }

    #[test]
    fn test_wildcard_match() {
        assert!(wildcard_match("cnt", "cnt"));
        assert!(wildcard_match("c*", "cnt"));
        assert!(wildcard_match("*nt", "cnt"));
        assert!(wildcard_match("*", "anything"));
        assert!(wildcard_match("c?t", "cnt"));
        assert!(wildcard_match("top.*", "top.cnt"));
        assert!(!wildcard_match("c?t", "count"));
        assert!(!wildcard_match("top.*", "other.cnt"));
        assert!(!wildcard_match("cnt", "clk"));
    }

    #[test]
    fn test_get_at_samples_value() {
        std::fs::write("/tmp/__get_at.vcd", vcd(false, &["0010", "0011", "0100"])).unwrap();
        let data = parse_vcd("/tmp/__get_at.vcd").unwrap();
        // cnt berubah di t=0(0000), 5(0010), 10(0011), 15(0100).
        let r = get_data(&data, &["cnt".to_string()], Some(7), None).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].name, "top.cnt");
        assert_eq!(r[0].width, 4);
        assert_eq!(
            r[0].values,
            vec![(7, "0010".to_string())],
            "@7 = perubahan terakhir ≤ 7 (t=5)"
        );
        // @0 sebelum perubahan pertama cnt tetap b0000 (perubahan #0 ada).
        let r0 = get_data(&data, &["cnt".to_string()], Some(0), None).unwrap();
        assert_eq!(r0[0].values, vec![(0, "0000".to_string())]);
        // Sinyal tanpa perubahan sebelum t → "x".
        let rx = get_data(&data, &["extra".to_string()], Some(3), None);
        assert!(rx.is_err(), "extra tidak ada di VCD ini → error no-match");
        let _ = std::fs::remove_file("/tmp/__get_at.vcd");
    }

    #[test]
    fn test_get_range_filters_changes() {
        std::fs::write(
            "/tmp/__get_range.vcd",
            vcd(false, &["0010", "0011", "0100", "0101"]),
        )
        .unwrap();
        let data = parse_vcd("/tmp/__get_range.vcd").unwrap();
        // cnt changes: 0, 5, 10, 15, 20 → range [5:15] = 3 perubahan.
        let r =
            get_data(&data, &["top.cnt".to_string()], None, Some((5, 15))).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(
            r[0].values,
            vec![
                (5, "0010".to_string()),
                (10, "0011".to_string()),
                (15, "0100".to_string())
            ],
            "range [5:15] menyaring perubahan di luar window"
        );
        // Range kosong → 0 nilai (sinyal cocok tapi tak ada perubahan).
        let re =
            get_data(&data, &["cnt".to_string()], None, Some((100, 200))).unwrap();
        assert!(re[0].values.is_empty());
        let _ = std::fs::remove_file("/tmp/__get_range.vcd");
    }

    #[test]
    fn test_get_wildcard_multiple_and_full_timeline() {
        std::fs::write(
            "/tmp/__get_multi.vcd",
            vcd(false, &["0010", "0011"]),
        )
        .unwrap();
        let data = parse_vcd("/tmp/__get_multi.vcd").unwrap();
        // Pola "c*" cocok clk + cnt; timeline penuh (tanpa at/range).
        let r = get_data(&data, &["c*".to_string()], None, None).unwrap();
        assert_eq!(r.len(), 2, "c* → clk + cnt");
        let clk = r.iter().find(|e| e.name == "top.clk").unwrap();
        assert_eq!(
            clk.values,
            vec![(0, "0".to_string()), (5, "1".to_string()), (10, "0".to_string())],
            "timeline penuh clk"
        );
        // Nama polos (tanpa scope) juga match.
        let r2 = get_data(&data, &["clk".to_string()], None, None).unwrap();
        assert_eq!(r2.len(), 1);
        assert_eq!(r2[0].name, "top.clk");
        // Tidak ada yang cocok → error.
        assert!(get_data(&data, &["nosuch".to_string()], None, None).is_err());
        let _ = std::fs::remove_file("/tmp/__get_multi.vcd");
    }

    #[test]
    fn test_search_finds_signals_by_pattern() {
        std::fs::write("/tmp/__srch.vcd", vcd(true, &["0010"])).unwrap();
        let data = parse_vcd("/tmp/__srch.vcd").unwrap();
        // Pola `c*` cocok clk + cnt (bukan extra); `*extra*` cocok extra.
        let names: Vec<String> = data
            .signals
            .iter()
            .filter(|s| wildcard_match("c*", &s.name) || wildcard_match("*extra*", &s.name))
            .map(full_name)
            .collect();
        assert!(names.contains(&"top.clk".to_string()));
        assert!(names.contains(&"top.cnt".to_string()));
        assert!(names.contains(&"top.extra".to_string()));
        assert_eq!(names.len(), 3);
        // Pola sempit `cnt` hanya 1.
        let cnt_only: Vec<String> = data
            .signals
            .iter()
            .filter(|s| wildcard_match("cnt", &s.name))
            .map(full_name)
            .collect();
        assert_eq!(cnt_only, vec!["top.cnt".to_string()]);
        let _ = std::fs::remove_file("/tmp/__srch.vcd");
    }

    #[test]
    fn test_stats_counts_toggles_and_activity() {
        // clk: 0@0, 1@5, 0@10 → 2 toggle, first=5, last=10.
        // cnt: 0010@0, 0011@5 (1 toggle), 0101@10 (1 toggle) → 2 toggle.
        std::fs::write("/tmp/__st.vcd", vcd(false, &["0010", "0011"])).unwrap();
        let data = parse_vcd("/tmp/__st.vcd").unwrap();
        let r = stats_data(&data);
        assert_eq!(r.len(), 2, "2 sinyal di-stats");
        let clk = r.iter().find(|s| s.name == "top.clk").expect("clk");
        assert_eq!(clk.width, 1);
        assert_eq!(clk.toggles, 2, "clk 0→1→0 = 2 toggle");
        assert_eq!(clk.changes, 3, "3 perubahan (0,5,10)");
        assert_eq!(clk.first, 0, "perubahan pertama di inisialisasi t=0");
        assert_eq!(clk.last, 10);
        // Aktivitas clk: nilai != baseline 0 selama [5,10) = 5 unit + sampai
        // akhir (max_time=10) nilai 0 = 0 → activity 5.
        assert_eq!(clk.activity, 5, "clk aktif 5 unit waktu");
        assert!(clk.duration >= 10);
        let cnt = r.iter().find(|s| s.name == "top.cnt").expect("cnt");
        assert_eq!(cnt.toggles, 2, "cnt 2 transisi nilai");
        assert_eq!(cnt.first, 0, "cnt inisialisasi di t=0");
        assert_eq!(cnt.last, 10);
        let _ = std::fs::remove_file("/tmp/__st.vcd");
    }

    #[test]
    fn test_stats_stuck_signal_zero_toggle() {
        // Signal tanpa perubahan → toggles=0 (stuck) — aktivitas 0.
        let src = "$timescale 1ns $end\n\
                   $scope module top $end\n\
                   $var wire 1 ! clk $end\n\
                   $var wire 4 \" cnt $end\n\
                   $upscope $end\n\
                   $enddefinitions $end\n\
                   #0\n0!\nb0000 \"\n\
                   #10\n1!\n";
        std::fs::write("/tmp/__stuck.vcd", src).unwrap();
        let data = parse_vcd("/tmp/__stuck.vcd").unwrap();
        let r = stats_data(&data);
        let cnt = r.iter().find(|s| s.name == "top.cnt").expect("cnt");
        assert_eq!(cnt.toggles, 0, "cnt tidak pernah berubah → stuck");
        assert_eq!(cnt.changes, 1, "hanya inisialisasi");
        assert_eq!(cnt.activity, 0, "aktivitas 0");
        let _ = std::fs::remove_file("/tmp/__stuck.vcd");
    }

    /// Helper: bangun file VCD dari definisi $var + body mentah (WAV-16).
    /// vars: (id, width, name) — semua di scope `top`.
    fn vcd_proto(vars: &[(&str, usize, &str)], body: &str) -> String {
        let mut s = String::from("$timescale 1ns $end\n$scope module top $end\n");
        for (id, w, name) in vars {
            s.push_str(&format!("$var wire {} {} {}  $end\n", w, id, name));
        }
        s.push_str("$upscope $end\n$enddefinitions $end\n");
        s.push_str(body);
        s
    }

    #[test]
    fn test_apb_decode_write_read_and_wait_state() {
        // TX1 WRITE 0x10<=0xAB (pready langsung 1).
        // TX2 READ 0x40 dengan WAIT STATE: penable t=60, pready=0 t=60,
        // pready=1 t=70 → transaksi selesai di t=70, rdata=0x51.
        let src = vcd_proto(
            &[
                ("!", 1, "psel"),
                ("\"", 1, "penable"),
                ("#", 1, "pwrite"),
                ("$", 1, "pready"),
                ("%", 8, "paddr"),
                ("&", 8, "pwdata"),
                ("'", 8, "prdata"),
            ],
            "#0\n0!\n0\"\n0#\n1$\nb00000000 %\nb00000000 &\nb00000000 '\n\
             #10\n1!\n1#\nb00010000 %\nb10101011 &\n\
             #20\n1\"\n\
             #30\n0!\n0\"\n0#\n\
             #50\n1!\n0#\nb01000000 %\nb01010001 '\n\
             #60\n1\"\n0$\n\
             #70\n1$\n\
             #80\n0!\n0\"\n",
        );
        std::fs::write("/tmp/__dec_apb.vcd", src).unwrap();
        let data = parse_vcd("/tmp/__dec_apb.vcd").unwrap();
        let txs = apb_decode(&data).expect("apb decode ok");
        assert_eq!(txs.len(), 2, "2 transaksi APB");

        let w = &txs[0];
        assert_eq!(w.kind, "WRITE");
        assert_eq!(w.start, 10);
        assert_eq!(w.end, 20);
        assert_eq!(w.addr, 0x10);
        assert_eq!(w.data, 0xAB);
        assert_eq!(w.resp, None);

        let r = &txs[1];
        assert_eq!(r.kind, "READ");
        assert_eq!(r.start, 50);
        assert_eq!(r.end, 70, "wait state: selesai saat pready naik t=70");
        assert_eq!(r.addr, 0x40);
        assert_eq!(r.data, 0x51);
        let _ = std::fs::remove_file("/tmp/__dec_apb.vcd");
    }

    #[test]
    fn test_axi4lite_decode_write_and_read_channels() {
        // Write: aw handshake t=10, w handshake t=10, b handshake t=20.
        // Read : ar handshake t=40, r handshake t=50 (rresp=2 → 0b10).
        let src = vcd_proto(
            &[
                ("a", 1, "awvalid"),
                ("b", 1, "awready"),
                ("c", 8, "awaddr"),
                ("d", 1, "wvalid"),
                ("e", 1, "wready"),
                ("f", 8, "wdata"),
                ("g", 1, "bvalid"),
                ("h", 1, "bready"),
                ("i", 2, "bresp"),
                ("j", 1, "arvalid"),
                ("k", 1, "arready"),
                ("l", 8, "araddr"),
                ("m", 1, "rvalid"),
                ("n", 1, "rready"),
                ("o", 8, "rdata"),
                ("p", 2, "rresp"),
            ],
            "#0\n0a\n1b\n0d\n1e\n0g\n1h\nb00 i\n0j\n1k\n1n\nb00 p\n\
             #10\n1a\nb00110000 c\n1d\nb11011110 f\n\
             #20\n0a\n0d\n1g\nb01 i\n\
             #30\n0g\n\
             #40\n1j\nb01010000 l\n\
             #50\n0j\n1m\nb01110111 o\nb10 p\n\
             #60\n0m\n",
        );
        std::fs::write("/tmp/__dec_axi.vcd", src).unwrap();
        let data = parse_vcd("/tmp/__dec_axi.vcd").unwrap();
        let mut txs = axi4lite_decode(&data).expect("axi decode ok");
        assert_eq!(txs.len(), 2, "1 write + 1 read");
        txs.sort_by_key(|t| t.start);

        let w = &txs[0];
        assert_eq!(w.kind, "WRITE");
        assert_eq!(w.start, 10);
        assert_eq!(w.end, 20);
        assert_eq!(w.addr, 0x30);
        assert_eq!(w.data, 0xDE);
        assert_eq!(w.resp, Some(0b01), "bresp OKAY=01 tercatat");

        let r = &txs[1];
        assert_eq!(r.kind, "READ");
        assert_eq!(r.start, 40);
        assert_eq!(r.end, 50);
        assert_eq!(r.addr, 0x50);
        assert_eq!(r.data, 0x77);
        assert_eq!(r.resp, Some(0b10));
        let _ = std::fs::remove_file("/tmp/__dec_axi.vcd");
    }

    #[test]
    fn test_ahb_decode_single_transfers() {
        // WRITE: addr phase NONSEQ t=10, data phase selesai t=20 (hwdata).
        // READ : addr phase NONSEQ t=40, data phase selesai t=50 (hrdata).
        let src = vcd_proto(
            &[
                ("a", 2, "htrans"),
                ("b", 1, "hwrite"),
                ("c", 1, "hready"),
                ("d", 8, "haddr"),
                ("e", 8, "hwdata"),
                ("f", 8, "hrdata"),
            ],
            "#0\nb00 a\n0b\n1c\nb00000000 d\ne\nf\n\
             #10\nb10 a\n1b\nb01100000 d\n\
             #20\nb00 a\nb10011010 e\n\
             #40\nb10 a\n0b\nb01100100 d\nb00111110 f\n\
             #50\nb00 a\n",
        );
        std::fs::write("/tmp/__dec_ahb.vcd", src).unwrap();
        let data = parse_vcd("/tmp/__dec_ahb.vcd").unwrap();
        let txs = ahb_decode(&data).expect("ahb decode ok");
        assert_eq!(txs.len(), 2, "2 transfer AHB");

        let w = &txs[0];
        assert_eq!(w.kind, "WRITE");
        assert_eq!(w.start, 10);
        assert_eq!(w.end, 20);
        assert_eq!(w.addr, 0x60);
        assert_eq!(w.data, 0x9A);

        let r = &txs[1];
        assert_eq!(r.kind, "READ");
        assert_eq!(r.start, 40);
        assert_eq!(r.end, 50);
        assert_eq!(r.addr, 0x64);
        assert_eq!(r.data, 0x3E);
        let _ = std::fs::remove_file("/tmp/__dec_ahb.vcd");
    }

    #[test]
    fn test_decode_missing_signal_errors() {
        // VCD tanpa psel → apb_decode error jelas.
        let src = vcd_proto(
            &[("\"", 1, "penable"), ("#", 1, "pwrite"), ("$", 8, "paddr")],
            "#0\n0\"\n0#\n",
        );
        std::fs::write("/tmp/__dec_miss.vcd", src).unwrap();
        let data = parse_vcd("/tmp/__dec_miss.vcd").unwrap();
        let e = apb_decode(&data).expect_err("psel hilang harus error");
        assert!(
            format!("{}", e).contains("psel"),
            "pesan menyebut sinyal yang hilang"
        );
        let _ = std::fs::remove_file("/tmp/__dec_miss.vcd");
    }

    #[test]
    fn test_sample_at_and_bits_val() {
        let tl = vec![(0u64, "0".to_string()), (10, "1".to_string())];
        assert_eq!(sample_at(&tl, 5), "0", "sebelum perubahan kedua");
        assert_eq!(sample_at(&tl, 10), "1");
        assert_eq!(sample_at(&tl, 99), "1", "nilai bertahan");
        assert_eq!(sample_at(&[], 0), "x", "timeline kosong → x");
        assert_eq!(bits_val("1010"), 0xA);
        assert_eq!(bits_val("00000000"), 0);
        assert_eq!(bits_val("xxxx"), 0, "x dianggap 0");
    }
}
