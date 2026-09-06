//! Feature map — peta fitur bahasa yang sudah tereksekusi (paper #6 CollAFL:
//! coverage sensitif-lintasan; #7 VUzzer: fitur dataflow op/lebar/outcome).
//!
//! Fitur = operator, konstruk, lebar sinyal, stage pipeline, error code.
//! Feature baru = seed menarik (corpus) + nambah energi (guide.rs).

use std::collections::HashMap;

/// Operator / konstruk SystemVerilog yang dilacak.
pub const TRACKED: &[&str] = &[
    "+", "-", "*", "/", "%", "&", "|", "^", "~", "<<", ">>", "==", "!=", "<", ">", "<=",
    ">=", "&&", "||", "!", "? :", "case", "if", "else", "for", "while", "repeat", "forever",
    "always_ff", "always_comb", "always_latch", "always", "initial", "final", "fork", "join",
    "join_any", "join_none", "assign", "posedge", "negedge", "interface", "modport", "class",
    "package", "import", "covergroup", "coverpoint", "assert", "$display", "$clog2", "$bits",
    "genvar", "generate", "parameter", "localparam", "`ifdef", "`define", "typedef", "enum",
    "struct", "logic", "reg", "wire", "signed", "unsigned", "`timescale", "task", "function",
];

/// Peta hitungan fitur (frequency) — dasar energy scheduling (Paper #4/#5).
#[derive(Debug, Default, Clone)]
pub struct FeatureMap {
    pub counts: HashMap<String, u64>,
}

impl FeatureMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ekstrak fitur bahasa dari source (op, konstruk, lebar).
    pub fn extract(source: &str) -> Vec<String> {
        let mut feats: Vec<String> = TRACKED
            .iter()
            .filter(|t| source.contains(*t))
            .map(|t| t.to_string())
            .collect();
        // Lebar sinyal `[<n>:<m>]` (#7: lebar = fitur dataflow).
        // Scanner: `[` lalu `hi : lo` di dalam kurung → width = hi-lo+1.
        let chars: Vec<char> = source.chars().collect();
        let mut i = 0usize;
        while i < chars.len() {
            if chars[i] == '[' {
                if let Some(close) = chars[i + 1..].iter().position(|&c| c == ']') {
                    let inner: String = chars[i + 1..i + 1 + close].iter().collect();
                    if let Some((hi_s, lo_s)) = inner.split_once(':') {
                        let hi: i64 = hi_s.trim().parse().unwrap_or(-1);
                        let lo: i64 = lo_s.trim().parse().unwrap_or(-1);
                        if hi >= 0 && lo >= 0 && hi >= lo {
                            let w = hi - lo + 1;
                            if w > 1 {
                                feats.push(format!("width:{}", w));
                            }
                        }
                    }
                    i += close + 1;
                    continue;
                }
            }
            i += 1;
        }
        feats.sort();
        feats.dedup();
        feats
    }

    /// Catat hitungan fitur.
    pub fn record(&mut self, feats: &[String]) {
        for f in feats {
            *self.counts.entry(f.clone()).or_insert(0) += 1;
        }
    }

    /// Ada fitur baru (belum pernah terlihat)?
    pub fn has_new(&self, feats: &[String]) -> bool {
        feats.iter().any(|f| !self.counts.contains_key(f))
    }

    /// Fitur langka (frekuensi 1) — target mutasi FairFuzz (Paper #5).
    pub fn rare(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .counts
            .iter()
            .filter(|(_, c)| **c == 1)
            .map(|(k, _)| k.clone())
            .collect();
        v.sort();
        v
    }

    /// Jumlah fitur yang pernah terlihat.
    pub fn covered(&self) -> usize {
        self.counts.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
module top(input logic [7:0] a, output logic [7:0] y);
  assign y = a + 1;
  always_comb begin
    y = y & a;
  end
endmodule
"#;

    #[test]
    fn extract_finds_ops_and_widths() {
        let f = FeatureMap::extract(SAMPLE);
        assert!(f.contains(&"+".to_string()));
        assert!(f.contains(&"&".to_string()));
        assert!(f.contains(&"always_comb".to_string()));
        assert!(f.contains(&"width:8".to_string()));
        assert!(f.contains(&"assign".to_string()));
    }

    #[test]
    fn record_and_has_new() {
        let mut m = FeatureMap::new();
        let feats = FeatureMap::extract(SAMPLE);
        assert!(m.has_new(&feats));
        m.record(&feats);
        assert!(!m.has_new(&feats));
        assert_eq!(m.covered(), feats.len());
        let mut extra = feats.clone();
        extra.push("width:32".to_string());
        assert!(m.has_new(&extra), "width baru = fitur baru");
    }

    #[test]
    fn rare_after_single_record() {
        let mut m = FeatureMap::new();
        let feats = FeatureMap::extract(SAMPLE);
        m.record(&feats);
        let rare = m.rare();
        assert_eq!(rare.len(), feats.len(), "semua fitur baru = langka");
    }
}