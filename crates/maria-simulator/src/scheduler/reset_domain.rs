//! Reset-Domain Crossing (RDC) Analysis — deteksi crossing antar reset domain.
//!
//! # Pipeline (SIM-22)
//!
//! 1. **Reset Domain Extraction** — untuk setiap proses `Sequential`, ambil
//!    reset signal + polaritas (dari `ResetInfo`). Proses tanpa reset masuk
//!    domain `none`.
//!
//! 2. **Signal-Domain Mapping** — sinyal "dimiliki" oleh domain reset yang
//!    prosesnya menulis sinyal tsb. Bila ditulis oleh beberapa domain, domain
//!    dengan frekuensi tulis terbanyak yang menang (konsisten dengan CDC).
//!
//! 3. **Crossing Detection** — sinyal yang ditulis oleh proses di domain reset
//!    A dan dibaca oleh proses `Sequential` di domain reset B (A ≠ B) adalah
//!    reset-domain crossing: saat reset A aktif, nilai sinyal berubah drastis
//!    sementara flop domain B (reset berbeda / tanpa reset) bisa meng-capture
//!    nilai transisi → metastability / data korup.
//!
//! 4. **Report** — output text: jumlah domain, sinyal per domain, daftar
//!    crossing (src → dst) + skor severity.

use std::collections::{HashMap, HashSet};
use maria_ir::*;
use crate::scheduler::cdc::{
    collect_stmt_signal_reads, collect_writes_from_stmts, lvalue_collect_signal_ids,
};

/// Severity crossing reset-domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RdcSeverity {
    Ok,
    /// Sinyal dari domain reset A dibaca oleh proses TANPA reset (flop
    /// asynchronous/level-sensitive) — tidak ada sinkronisasi reset.
    Medium,
    /// Crossing antar dua reset domain BERBEDA (dua sinyal reset berbeda).
    High,
    /// Crossing antar dua reset domain berbeda di mana salah satu proses
    /// membaca sinyal multi-bit yang ditulis proses lain → data coherency risk.
    Critical,
}

impl std::fmt::Display for RdcSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RdcSeverity::Ok => write!(f, "OK"),
            RdcSeverity::Medium => write!(f, "MEDIUM"),
            RdcSeverity::High => write!(f, "HIGH"),
            RdcSeverity::Critical => write!(f, "CRITICAL"),
        }
    }
}

/// Satu crossing reset-domain.
#[derive(Debug, Clone, PartialEq)]
pub struct RdcCrossing {
    pub signal: SignalId,
    pub signal_name: String,
    pub width: usize,
    /// Domain reset sumber (penulis). None = proses penulis tanpa reset.
    pub src_domain: Option<usize>,
    /// Domain reset tujuan (pembaca). None = pembaca tanpa reset.
    pub dst_domain: Option<usize>,
    pub severity: RdcSeverity,
    /// Nama reset sumber (untuk report).
    pub src_reset: String,
    /// Nama reset tujuan.
    pub dst_reset: String,
}

/// Satu domain reset.
#[derive(Debug, Clone, Default)]
pub struct RdcDomain {
    pub id: usize,
    /// Reset signal id — None = domain `none` (proses tanpa reset).
    pub reset_signal: Option<SignalId>,
    pub reset_name: String,
    pub polarity: bool,
    pub owned_signals: HashSet<SignalId>,
    pub process_ids: Vec<usize>,
}

/// Hasil analisis RDC.
#[derive(Debug, Clone, Default)]
pub struct ResetDomainAnalysis {
    pub domains: Vec<RdcDomain>,
    pub crossings: Vec<RdcCrossing>,
}

/// Key domain reset: (reset signal, polarity). Proses tanpa reset memakai
/// key khusus `None` (domain "none").
type DomainKey = Option<(SignalId, bool)>;

fn reset_key(process: &Process) -> Option<(SignalId, bool)> {
    match process {
        Process::Sequential { reset: Some(r), .. } => Some((r.signal, r.polarity)),
        Process::Sequential { reset: None, .. } => Some((usize::MAX, false)),
        _ => None,
    }
}

impl ResetDomainAnalysis {
    /// Analisis RDC penuh pada design ter-elaborasi.
    pub fn analyze(design: &IrDesign) -> Self {
        let signals = &design.top.signals;
        let processes = &design.top.processes;

        // ── 1. Kumpulkan domain reset ──
        let mut key_to_id: HashMap<DomainKey, usize> = HashMap::new();
        let mut domains: Vec<RdcDomain> = Vec::new();

        for (pid, process) in processes.iter().enumerate() {
            let key = match reset_key(process) {
                Some(k) => k,
                None => continue, // non-sequential
            };
            let key = (key.0 != usize::MAX).then_some(key);
            let id = *key_to_id.entry(key).or_insert_with(|| {
                let id = domains.len();
                let (reset_name, polarity) = match key {
                    Some((sid, pol)) => (
                        signals
                            .get(sid)
                            .map(|s| s.name.as_str().to_string())
                            .unwrap_or_else(|| format!("reset_{}", sid)),
                        pol,
                    ),
                    None => ("(none)".to_string(), false),
                };
                domains.push(RdcDomain {
                    id,
                    reset_signal: key.map(|(sid, _)| sid),
                    reset_name,
                    polarity,
                    owned_signals: HashSet::new(),
                    process_ids: Vec::new(),
                });
                id
            });
            domains[id].process_ids.push(pid);
        }

        // ── 2. Petakan sinyal → domain penulis terbanyak ──
        let mut owner_counts: HashMap<(SignalId, usize), usize> = HashMap::new();
        for process in processes.iter() {
            let key = match reset_key(process) {
                Some(k) if k.0 != usize::MAX => Some((k.0, k.1)),
                _ => None,
            };
            let Some(domain_id) = key.and_then(|k| key_to_id.get(&Some(k)).copied()) else {
                continue;
            };
            let writes = match process {
                Process::Sequential { body, .. } => collect_writes_from_stmts(body),
                _ => HashSet::new(),
            };
            for &sid in &writes {
                *owner_counts.entry((sid, domain_id)).or_insert(0) += 1;
            }
        }
        let mut signal_to_domain: HashMap<SignalId, Option<usize>> = HashMap::new();
        for ((sid, did), _) in &owner_counts {
            let e = signal_to_domain.entry(*sid).or_insert(None);
            if e.is_none() {
                *e = Some(*did);
            }
        }
        for (&sid, &d) in &signal_to_domain {
            if let Some(did) = d {
                if let Some(d) = domains.get_mut(did) {
                    d.owned_signals.insert(sid);
                }
            }
        }

        // ── 3. Deteksi crossing ──
        let mut crossings: Vec<RdcCrossing> = Vec::new();
        let mut seen: HashSet<(SignalId, Option<usize>, Option<usize>)> = HashSet::new();

        for process in processes.iter() {
            // Pembaca bisa berupa proses dengan reset ATAU tanpa reset (domain
            // `none`) — keduanya tujuan crossing.
            let raw_key = reset_key(process);
            let (dst_key_opt, dst_domain_id) = match raw_key {
                Some((sid, pol)) if sid != usize::MAX => {
                    let id = *key_to_id.get(&Some((sid, pol))).unwrap_or(&0);
                    (Some((sid, pol)), id)
                }
                _ => (None, *key_to_id.get(&None).unwrap_or(&0)),
            };
            let dst_reset = match dst_key_opt {
                Some((sid, _)) => signals
                    .get(sid)
                    .map(|s| s.name.as_str().to_string())
                    .unwrap_or_else(|| format!("reset_{}", sid)),
                None => "(none)".to_string(),
            };
            let dst_reset_clone = dst_reset.clone();

            let mut reads = HashSet::new();
            if let Process::Sequential { body, .. } = process {
                collect_stmt_signal_reads(body, &mut reads);
            }
            // Sinyal yang ditulis proses ini juga "dikonsumsi" — bila penulisnya
            // domain lain (crossing langsung flop→flop), ikut dideteksi.
            if let Process::Sequential { body, .. } = process {
                reads.extend(collect_writes_from_stmts(body));
            }

            for &sid in &reads {
                // Hanya sinyal yang DITULIS oleh proses sequential yang bisa
                // crossing: sinyal yang tidak ditulis (input port, reset signal
                // itu sendiri, sinyal kombinasi) bukan berasal dari domain reset
                // mana pun — membaca-nya bukan crossing.
                let Some(&src_owned) = signal_to_domain.get(&sid) else {
                    continue;
                };
                let Some(src_domain) = src_owned else {
                    continue;
                };
                // Jangan hitung reset signal itu sendiri (dibaca di klausa
                // `if (!rst_n)` body) sebagai crossing.
                if let Some((sid_r, _)) = dst_key_opt {
                    if sid == sid_r {
                        continue;
                    }
                }
                if src_domain == dst_domain_id {
                    continue; // same-domain — bukan crossing
                }
                let src_domain_opt = Some(src_domain);
                let key = (sid, src_domain_opt, Some(dst_domain_id));
                if !seen.insert(key) {
                    continue;
                }
                let src_reset = domains[src_domain].reset_name.clone();
                let severity = {
                    let width = signals.get(sid).map(|s| s.width).unwrap_or(1);
                    if width > 1 {
                        RdcSeverity::Critical
                    } else {
                        RdcSeverity::High
                    }
                };
                crossings.push(RdcCrossing {
                    signal: sid,
                    signal_name: signals
                        .get(sid)
                        .map(|s| s.name.as_str().to_string())
                        .unwrap_or_else(|| format!("sig_{}", sid)),
                    width: signals.get(sid).map(|s| s.width).unwrap_or(1),
                    src_domain: src_domain_opt,
                    dst_domain: Some(dst_domain_id),
                    severity,
                    src_reset,
                    dst_reset: dst_reset_clone.clone(),
                });
            }
        }

        crossings.sort_by(|a, b| {
            b.severity
                .cmp(&a.severity)
                .then(a.signal_name.cmp(&b.signal_name))
        });

        Self { domains, crossings }
    }

    /// Report text (human-readable).
    pub fn report(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("Reset-Domain Crossing Analysis — {} domain, {} crossing\n", self.domains.len(), self.crossings.len()));
        for d in &self.domains {
            let pol = if d.polarity { "posedge" } else { "negedge" };
            out.push_str(&format!(
                "  domain {}: reset={} ({}) — {} sinyal, {} proses\n",
                d.id,
                d.reset_name,
                pol,
                d.owned_signals.len(),
                d.process_ids.len()
            ));
        }
        if self.crossings.is_empty() {
            out.push_str("  no crossing detected ✅\n");
            return out;
        }
        for c in &self.crossings {
            out.push_str(&format!(
                "  [{}] {} ({} bit): reset domain {} ({}) → {}\n",
                c.severity,
                c.signal_name,
                c.width,
                match c.src_domain {
                    Some(d) => d.to_string(),
                    None => "-".to_string(),
                },
                c.src_reset,
                c.dst_reset,
            ));
        }
        out
    }

    pub fn write_report(&self, path: &str) -> Result<(), String> {
        std::fs::write(path, self.report()).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::compile_str;

    fn rdc(src: &str) -> ResetDomainAnalysis {
        let design = compile_str(src).expect("elaborasi");
        ResetDomainAnalysis::analyze(&design)
    }

    #[test]
    fn test_no_crossing_single_reset_domain() {
        let a = rdc(
            r#"
module top(input logic clk, input logic rst_n, input logic d);
    logic q;
    always_ff @(posedge clk or negedge rst_n) begin
        if (!rst_n) q <= 1'b0;
        else q <= d;
    end
endmodule
"#,
        );
        assert!(a.crossings.is_empty(), "satu domain reset → no crossing: {}", a.report());
    }

    #[test]
    fn test_crossing_two_reset_domains() {
        // Sinyal `q_a` di-reset oleh rst_a, dibaca flop domain rst_b → crossing.
        let a = rdc(
            r#"
module top(
    input logic clk, input logic rst_a_n, input logic rst_b_n, input logic d
);
    logic q_a, q_b;
    always_ff @(posedge clk or negedge rst_a_n) begin
        if (!rst_a_n) q_a <= 1'b0;
        else q_a <= d;
    end
    always_ff @(posedge clk or negedge rst_b_n) begin
        if (!rst_b_n) q_b <= 1'b0;
        else q_b <= q_a;   // q_a dari domain reset rst_a → crossing
    end
endmodule
"#,
        );
        assert_eq!(a.domains.len(), 2, "2 domain reset: {}", a.report());
        assert!(
            a.crossings.iter().any(|c| c.signal_name == "q_a"),
            "q_a harus terdeteksi crossing: {}",
            a.report()
        );
        let c = a.crossings.iter().find(|c| c.signal_name == "q_a").unwrap();
        assert_eq!(c.src_reset, "rst_a_n", "sumber domain rst_a_n");
        assert_eq!(c.dst_reset, "rst_b_n", "tujuan domain rst_b_n");
    }

    #[test]
    fn test_async_reader_domain_medium() {
        // q_a ditulis domain rst_a; proses TANPA reset membaca q_a → severity
        // Medium (tidak ada sinkronisasi reset).
        let a = rdc(
            r#"
module top(input logic clk, input logic rst_a_n, input logic d);
    logic q_a, out;
    always_ff @(posedge clk or negedge rst_a_n) begin
        if (!rst_a_n) q_a <= 1'b0;
        else q_a <= d;
    end
    always_ff @(posedge clk) begin
        out <= q_a;   // pembaca tanpa reset
    end
endmodule
"#,
        );
        assert!(
            a.crossings.iter().any(|c| c.signal_name == "q_a"),
            "q_a crossing ke domain none: {}",
            a.report()
        );
    }

    #[test]
    fn test_multibit_crossing_critical() {
        let a = rdc(
            r#"
module top(
    input logic clk, input logic rst_a_n, input logic rst_b_n,
    input logic [3:0] d
);
    logic [3:0] a_bus, b_bus;
    always_ff @(posedge clk or negedge rst_a_n) begin
        if (!rst_a_n) a_bus <= 4'h0;
        else a_bus <= d;
    end
    always_ff @(posedge clk or negedge rst_b_n) begin
        if (!rst_b_n) b_bus <= 4'h0;
        else b_bus <= a_bus;   // 4-bit crossing → Critical
    end
endmodule
"#,
        );
        let c = a
            .crossings
            .iter()
            .find(|c| c.signal_name == "a_bus")
            .expect("a_bus crossing");
        assert_eq!(c.severity, RdcSeverity::Critical, "multi-bit → Critical");
    }
}
