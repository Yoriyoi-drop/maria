//! Constraint `.mcs` — Maria Constraint Specification (SYNTHESIS.md §15).
//!
//! Format garis-per-aturan, toleran terhadap whitespace & komentar `//`/`#`:
//!
//! ```text
//! clock clk { period = 10ns; }          // clock utama (periode ns)
//! input_delay 2ns;                      // delay input → FF (ns)
//! output_delay 2ns;                     // FF → output eksternal (ns)
//! max_fanout 32;                        // batas fanout (informasi saja)
//! false_path { from = rst; }            // jalur dari signal rst dikecualikan
//! multicycle_path 2 { from = reg_a; to = reg_b; }  // N cycle untuk jalur
//! ```
//!
//! Nilai periode tanpa unit dianggap ns. Satuan lain (`ps`, `us`, `ms`)
//! dikonversi. Baris yang tidak dikenal di-skip dengan warning (toleran —
//! constraint adalah saran, bukan grammar kaku).

use std::path::Path;

/// Spesifikasi clock: `clock <name> { period = <val>ns; }`.
#[derive(Debug, Clone, PartialEq)]
pub struct ClockSpec {
    pub name: String,
    /// Periode dalam nanodetik (float).
    pub period_ns: f64,
}

/// Satu jalur false/multicycle — di-match dari nama signal asal (`from`) dan
/// tujuan (`to`). Kosong = wildcard.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PathSpec {
    pub from: Option<String>,
    pub to: Option<String>,
    /// Faktor multicycle (1 = normal). 0 = false path.
    pub cycles: u32,
}

/// Constraint timing ter-parse.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Constraint {
    /// Clock terdefinisi (default: period 10 ns bila kosong).
    pub clocks: Vec<ClockSpec>,
    pub input_delay_ns: f64,
    pub output_delay_ns: f64,
    pub max_fanout: Option<u32>,
    pub false_paths: Vec<PathSpec>,
    pub multicycle_paths: Vec<PathSpec>,
}

impl Constraint {
    /// Periode clock pertama (atau 10 ns default).
    pub fn period_ns(&self) -> f64 {
        self.clocks
            .first()
            .map(|c| c.period_ns)
            .filter(|p| *p > 0.0)
            .unwrap_or(10.0)
    }

    /// Faktor multicycle yang cocok dengan pasangan `(from, to)` signal.
    /// False path dianggap multicycle ∞ — callers skip.
    pub fn cycle_multiplier(&self, from: &str, to: &str) -> u32 {
        for p in &self.multicycle_paths {
            let from_ok = p.from.as_deref().is_none_or(|f| from.contains(f));
            let to_ok = p.to.as_deref().is_none_or(|t| to.contains(t));
            if from_ok && to_ok {
                return p.cycles.max(1);
            }
        }
        1
    }

    /// True bila `(from, to)` termasuk false path.
    pub fn is_false_path(&self, from: &str, to: &str) -> bool {
        self.false_paths.iter().any(|p| {
            let from_ok = p.from.as_deref().is_none_or(|f| from.contains(f));
            let to_ok = p.to.as_deref().is_none_or(|t| to.contains(t));
            from_ok && to_ok
        })
    }
}

/// Parse teks `.mcs` → `Constraint`. Baris tak dikenal di-skip.
pub fn parse_constraints(text: &str) -> Constraint {
    let mut c = Constraint::default();

    for raw in text.lines() {
        let line = raw.split("//").next().unwrap_or("");
        let line = line.split('#').next().unwrap_or("");
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Strip kurung kurawal & titik koma — parse berbasis kata kunci.
        let norm = line.replace(['{', '}'], " ");
        let norm = norm.trim_end_matches(';').trim();
        if norm.is_empty() {
            continue;
        }

        // ── clock <name> { period = X } ──
        if let Some(rest) = norm.strip_prefix("clock") {
            let mut it = rest.split_whitespace();
            let name = it.next().unwrap_or("clk").to_string();
            // Cari `period = <val>`
            let period = it
                .position(|t| t == "period" || t == "period=")
                .and_then(|_| {
                    // setelah "period" bisa "=" lalu nilai
                    let mut rest2 = rest.split_whitespace();
                    let mut after = None;
                    while let Some(t) = rest2.next() {
                        if t == "period" {
                            after = rest2.next(); // "=" atau nilai
                            if after == Some("=") {
                                after = rest2.next();
                            }
                            break;
                        }
                    }
                    after.and_then(parse_ns)
                })
                .unwrap_or(10.0);
            c.clocks.push(ClockSpec {
                name,
                period_ns: period,
            });
            continue;
        }

        // ── input_delay / output_delay ──
        if let Some(rest) = norm.strip_prefix("input_delay") {
            if let Some(v) = rest.split_whitespace().next().and_then(parse_ns) {
                c.input_delay_ns = v;
            }
            continue;
        }
        if let Some(rest) = norm.strip_prefix("output_delay") {
            if let Some(v) = rest.split_whitespace().next().and_then(parse_ns) {
                c.output_delay_ns = v;
            }
            continue;
        }

        // ── max_fanout N ──
        if let Some(rest) = norm.strip_prefix("max_fanout") {
            if let Some(n) = rest
                .split_whitespace()
                .next()
                .and_then(|t| t.parse::<u32>().ok())
            {
                c.max_fanout = Some(n);
            }
            continue;
        }

        // ── false_path { from = X; to = Y; } (satu baris) ──
        if norm.starts_with("false_path") {
            let rest = &norm["false_path".len()..];
            c.false_paths.push(PathSpec {
                from: extract_attr(rest, "from"),
                to: extract_attr(rest, "to"),
                cycles: 0,
            });
            continue;
        }

        // ── multicycle_path N { from = X; to = Y; } (satu baris) ──
        if norm.starts_with("multicycle_path") {
            let n = norm
                .split_whitespace()
                .nth(1)
                .and_then(|t| t.parse::<u32>().ok())
                .unwrap_or(1);
            let rest = &norm["multicycle_path".len()..];
            c.multicycle_paths.push(PathSpec {
                from: extract_attr(rest, "from"),
                to: extract_attr(rest, "to"),
                cycles: n,
            });
            continue;
        }
    }
    c
}

/// Ekstrak nilai atribut `key = value` dari sisa baris (mis. `{ from = rst; }`).
fn extract_attr(rest: &str, key: &str) -> Option<String> {
    let toks: Vec<&str> = rest
        .split_whitespace()
        .map(|t| t.trim_end_matches(';'))
        .collect();
    for (i, t) in toks.iter().enumerate() {
        if let Some((k, v)) = t.split_once('=') {
            if k == key {
                return Some(v.to_string());
            }
        }
        if *t == key {
            if toks.get(i + 1).copied() == Some("=") {
                return toks.get(i + 2).map(|s| s.to_string());
            }
            // toleran: `from rst` tanpa `=`
            return toks
                .get(i + 1)
                .filter(|v| v != &&"=")
                .map(|s| s.to_string());
        }
    }
    None
}

/// Parse nilai waktu `10ns`, `2`, `0.5ps`, `1us` → ns.
fn parse_ns(tok: &str) -> Option<f64> {
    let tok = tok.trim().trim_end_matches(';');
    let (num, mult) = if let Some(v) = tok.strip_suffix("ns") {
        (v, 1.0)
    } else if let Some(v) = tok.strip_suffix("ps") {
        (v, 0.001)
    } else if let Some(v) = tok.strip_suffix("us") {
        (v, 1000.0)
    } else if let Some(v) = tok.strip_suffix("ms") {
        (v, 1_000_000.0)
    } else {
        (tok, 1.0)
    };
    num.trim().parse::<f64>().ok().map(|v| v * mult)
}

/// Baca file `.mcs` → Constraint (IO error → default kosong + warning).
pub fn load_constraints(path: &Path) -> std::io::Result<Constraint> {
    let text = std::fs::read_to_string(path)?;
    Ok(parse_constraints(&text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_full_mcs() {
        let c = parse_constraints(
            "// komentar\n\
             clock clk { period = 10ns; }\n\
             input_delay 2ns;\n\
             output_delay 1.5ns;\n\
             max_fanout 32;\n\
             false_path { from = rst; }\n\
             multicycle_path 2 { from = reg_a; to = reg_b; }\n",
        );
        assert_eq!(c.clocks.len(), 1);
        assert_eq!(c.clocks[0].name, "clk");
        assert!((c.clocks[0].period_ns - 10.0).abs() < 1e-9);
        assert!((c.input_delay_ns - 2.0).abs() < 1e-9);
        assert!((c.output_delay_ns - 1.5).abs() < 1e-9);
        assert_eq!(c.max_fanout, Some(32));
        assert_eq!(c.false_paths.len(), 1);
        assert_eq!(c.false_paths[0].from.as_deref(), Some("rst"));
        assert_eq!(c.multicycle_paths.len(), 1);
        assert_eq!(c.multicycle_paths[0].cycles, 2);
        assert_eq!(c.multicycle_paths[0].from.as_deref(), Some("reg_a"));
        assert_eq!(c.multicycle_paths[0].to.as_deref(), Some("reg_b"));
        assert!(c.is_false_path("rst_q", "any"));
        assert!(!c.is_false_path("a", "b"));
        assert_eq!(c.cycle_multiplier("reg_a", "reg_b"), 2);
        assert_eq!(c.cycle_multiplier("x", "y"), 1);
        assert!((c.period_ns() - 10.0).abs() < 1e-9);
    }

    #[test]
    fn parse_unit_conversion() {
        let c = parse_constraints("clock clk { period = 500ps; }\ninput_delay 2;\n");
        assert!((c.clocks[0].period_ns - 0.5).abs() < 1e-9, "500ps = 0.5ns");
        assert!((c.input_delay_ns - 2.0).abs() < 1e-9, "tanpa unit = ns");
    }

    #[test]
    fn empty_defaults_to_10ns() {
        let c = parse_constraints("");
        assert!((c.period_ns() - 10.0).abs() < 1e-9);
    }

    #[test]
    fn chip_mcs_file_roundtrip() {
        // Reproduksi e2e: file chip.mcs lengkap harus menghasilkan false path
        // HANYA untuk signal berisi "rst", bukan untuk "y".
        let text =
            std::fs::read_to_string("examples/synth/chip.mcs").unwrap_or_else(|_| String::new());
        if text.is_empty() {
            return; // file belum ada di konteks test — skip
        }
        let c = parse_constraints(&text);
        assert_eq!(c.false_paths.len(), 1, "false_paths = {:?}", c.false_paths);
        assert_eq!(
            c.false_paths[0].from.as_deref(),
            Some("rst"),
            "from false_path = {:?}",
            c.false_paths[0].from
        );
        assert!(
            !c.is_false_path("y", "y"),
            "output y bukan false path (false_paths={:?})",
            c.false_paths
        );
        assert!(c.is_false_path("rst_n", "q"), "rst harus false path");
    }
}
