//! Report utilisasi (SYNTHESIS.md §5.7) — gaya Vivado, output teks.

use std::fmt::Write;

use crate::netlist::Netlist;
use crate::subset::{SynCheck, SynIssue, SynSeverity};
use maria_core::intern::Symbol;

/// Kapasitas device default fpga-x7 (bisa di-override tool).
#[derive(Debug, Clone, Copy)]
pub struct DeviceCapacity {
    pub lut6: u64,
    pub ff: u64,
    pub carry4: u64,
    pub bram36: u64,
    pub dsp48: u64,
    pub io: u64,
    pub bufg: u64,
}

impl DeviceCapacity {
    pub fn fpga_x7() -> Self {
        DeviceCapacity {
            lut6: 12_000,
            ff: 24_000,
            carry4: 3_000,
            bram36: 32,
            dsp48: 96,
            io: 200,
            bufg: 32,
        }
    }
    pub fn generic() -> Self {
        DeviceCapacity {
            lut6: 0,
            ff: 0,
            carry4: 0,
            bram36: 0,
            dsp48: 0,
            io: 0,
            bufg: 0,
        }
    }
}

fn pct(used: u64, avail: u64) -> String {
    if avail == 0 {
        "-".into()
    } else {
        format!("{:.1} %", used as f64 * 100.0 / avail as f64)
    }
}

fn row(out: &mut String, name: &str, used: u64, avail: u64, note: &str) {
    let _ = writeln!(
        out,
        "│ {:<8} │ {:>8} │ {:>7} │ {:>9} │ {:<8} │",
        name,
        used,
        avail,
        pct(used, avail),
        note
    );
}

/// Report utilisasi lengkap (text).
pub fn render_util_report(nl: &Netlist, cap: &DeviceCapacity) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "── Utilization Report (device: {}) ──", nl.device.name());
    let _ = writeln!(
        s,
        "┌──────────┬──────────┬─────────┬───────────┬──────────┐"
    );
    let _ = writeln!(
        s,
        "│ Resource │  Used    │ Avail   │  Util %   │ Note     │"
    );
    let _ = writeln!(
        s,
        "├──────────┼──────────┼─────────┼───────────┼──────────┤"
    );
    row(
        &mut s,
        "LUT6",
        nl.stats.lut_count as u64,
        cap.lut6,
        "estimate",
    );
    row(&mut s, "FF", nl.stats.ff_count as u64, cap.ff, "");
    row(
        &mut s,
        "CARRY4",
        nl.stats.carry4_count as u64,
        cap.carry4,
        "",
    );
    row(&mut s, "BRAM36", nl.stats.bram_count as u64, cap.bram36, "");
    row(&mut s, "ROM36", nl.stats.rom_count as u64, cap.bram36, "");
    row(&mut s, "DSP48", nl.stats.dsp_count as u64, cap.dsp48, "est");
    row(&mut s, "IO", nl.stats.io_count as u64, cap.io, "");
    row(&mut s, "BUFG", nl.stats.bufg_count as u64, cap.bufg, "");
    let _ = writeln!(
        s,
        "└──────────┴──────────┴─────────┴───────────┴──────────┘"
    );
    let _ = writeln!(
        s,
        "Top: {} — {} proses, {} node logika (est), {} FSM hint, {} mem bits (est)",
        nl.name.as_str(),
        nl.stats.process_count,
        nl.stats.logic_nodes,
        nl.stats.fsm_count,
        nl.stats.mem_bits
    );
    let _ = writeln!(
        s,
        "Catatan: LUT/DSP/mem adalah ESTIMASI fase S1 — technology mapping nyata di S2/S3."
    );
    s
}

/// Report sintesizability (SYN check).
pub fn render_syn_report(check: &SynCheck) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "── Synthesizability Check ──");
    let _ = writeln!(
        s,
        "Skor keseluruhan: {:.1} / 100  ({} error, {} warning)",
        check.overall_score(),
        check.error_count(),
        check.warning_count()
    );
    // Sortir DETERMINISTIK (ditemukan maria-fuzz target synth: SYN-9 latatan
    // latch dari proses berbeda keluar dalam urutan acak antar 2 run identik —
    // urutan proses IR hasil merge parallel tidak stabil). Sortir di sini
    // menjamin output tool stabil walau urutan IR bervariasi.
    let mut per_module: Vec<&(Symbol, usize, usize, f64)> = check.per_module.iter().collect();
    per_module.sort_by_key(|x| x.0.as_str().to_string());
    for (name, err, warn, score) in &per_module {
        let _ = writeln!(
            s,
            "  {:<24} error={:<3} warning={:<3} score={:.1}",
            name.as_str(),
            err,
            warn,
            score
        );
    }
    let mut issues: Vec<&SynIssue> = check.issues.iter().collect();
    issues.sort_by(|a, b| {
        a.module
            .as_str()
            .cmp(b.module.as_str())
            .then_with(|| a.code.cmp(b.code))
            .then_with(|| a.severity.name().cmp(b.severity.name()))
            .then_with(|| a.message.cmp(&b.message))
    });
    for issue in &issues {
        let _ = writeln!(
            s,
            "  [{}] {:>7} {} — {}",
            issue.code,
            issue.severity.name(),
            issue.module.as_str(),
            issue.message
        );
    }
    if check.issues.is_empty() {
        let _ = writeln!(s, "  (tidak ada issue — design 100% sintesizable)");
    }
    s
}

/// Render issue SYN ke baris ringkas (untuk exit/error).
pub fn first_error(check: &SynCheck) -> Option<String> {
    check
        .issues
        .iter()
        .find(|i| i.severity == SynSeverity::Error)
        .map(|i| format!("[{}] {}: {}", i.code, i.module.as_str(), i.message))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::netlist::DeviceKind;
    use maria_core::intern::Symbol;

    #[test]
    fn util_report_has_rows() {
        let mut nl = Netlist::new(Symbol::intern("top"), DeviceKind::FpgaX7);
        nl.stats.ff_count = 8;
        nl.stats.lut_count = 12;
        nl.stats.io_count = 5;
        let s = render_util_report(&nl, &DeviceCapacity::fpga_x7());
        assert!(s.contains("Utilization Report"));
        assert!(s.contains("│ FF"));
        assert!(s.contains("8"));
        assert!(s.contains("0.0 %"));
        assert!(s.contains("top"));
    }

    #[test]
    fn pct_formats() {
        assert_eq!(pct(0, 0), "-");
        assert_eq!(pct(100, 200), "50.0 %");
    }

    /// Regresi maria-fuzz (target synth): output SYN report harus DETERMINISTIK
    /// walau urutan issues input acak (urutan proses IR merge parallel tak
    /// stabil) — render_syn_report sortir issues + per_module.
    #[test]
    fn syn_report_render_deterministic() {
        use crate::subset::{SynCheck, SynIssue, SynSeverity};
        let mk = |code: &'static str, module: &str, msg: &str| SynIssue {
            code,
            severity: SynSeverity::Warning,
            module: Symbol::intern(module),
            message: msg.to_string(),
        };
        let issues_a = vec![
            mk("SYN-9", "top", "potensi LATCH pada 'sig#6'"),
            mk("SYN-9", "top", "potensi LATCH pada 'sig#3'"),
            mk("SYN-9", "zmod", "potensi LATCH pada 'sig#7'"),
            mk("SYN-7", "top", "multi-driver"),
        ];
        let issues_b = vec![
            mk("SYN-7", "top", "multi-driver"),
            mk("SYN-9", "zmod", "potensi LATCH pada 'sig#7'"),
            mk("SYN-9", "top", "potensi LATCH pada 'sig#3'"),
            mk("SYN-9", "top", "potensi LATCH pada 'sig#6'"),
        ];
        let per_a = vec![
            (Symbol::intern("zmod"), 0usize, 1usize, 98.0f64),
            (Symbol::intern("top"), 0usize, 1usize, 98.0f64),
        ];
        let per_b = vec![
            (Symbol::intern("top"), 0usize, 1usize, 98.0f64),
            (Symbol::intern("zmod"), 0usize, 1usize, 98.0f64),
        ];
        let a = render_syn_report(&SynCheck {
            issues: issues_a,
            per_module: per_a,
        });
        let b = render_syn_report(&SynCheck {
            issues: issues_b,
            per_module: per_b,
        });
        assert_eq!(a, b, "render SYN report harus identik walau order input beda");
        // Urutan: module asc → code asc → severity → message.
        let pos3 = a.find("sig#3").unwrap();
        let pos6 = a.find("sig#6").unwrap();
        let pos7 = a.find("sig#7").unwrap();
        assert!(
            pos3 < pos6 && pos6 < pos7,
            "issues ter-sortir modul+kode: sig#3 < sig#6 < sig#7"
        );
    }
}
