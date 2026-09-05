//! ENT-23: Design Migration Assistance — Verilog → SystemVerilog hints.
//!
//! Analisis kode Verilog dan menghasilkan rekomendasi migrasi ke
//! SystemVerilog. Bukan converter penuh, tapi rule-based analyzer
//! yang mengidentifikasi pola Verilog lama dan menyarankan modernisasi.
//!
//! Contoh output:
//! ```text
//! Line 12: reg [7:0] data → logic [7:0] data (use logic instead of reg)
//! Line 15: always @(posedge clk) → always_ff @(posedge clk)
//! Line 20: integer i → int i (use two-state types)
//! ```

use serde::{Deserialize, Serialize};

/// Severity dari migration hint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HintSeverity {
    Info,
    Warning,
    Suggestion,
}

impl std::fmt::Display for HintSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HintSeverity::Info => write!(f, "ℹ️"),
            HintSeverity::Warning => write!(f, "⚠️"),
            HintSeverity::Suggestion => write!(f, "💡"),
        }
    }
}

/// Migration hint untuk satu line.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationHint {
    pub line: u32,
    pub severity: HintSeverity,
    pub category: String,
    pub original: String,
    pub suggested: String,
    pub description: String,
}

/// Migration report untuk satu file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationReport {
    pub filename: String,
    pub hints: Vec<MigrationHint>,
}

impl MigrationReport {
    /// Analisis source code dan generate migration hints.
    pub fn analyze(filename: &str, source: &str) -> Self {
        let mut hints = Vec::new();

        for (idx, line) in source.lines().enumerate() {
            let line_num = idx as u32 + 1;
            let trimmed = line.trim();

            // Skip comments
            if trimmed.starts_with("//") || trimmed.starts_with("/*") {
                continue;
            }

            // Rule 1: reg → logic
            if let Some(rest) = trimmed.strip_prefix("reg ") {
                let after_reg = rest.trim_start();
                // reg [7:0] data → logic [7:0] data
                hints.push(MigrationHint {
                    line: line_num,
                    severity: HintSeverity::Suggestion,
                    category: "type-modernization".into(),
                    original: format!("reg {}", after_reg),
                    suggested: format!("logic {}", after_reg),
                    description: "Use 'logic' instead of 'reg' (IEEE 1800)".into(),
                });
            }

            // Rule 2: wire → logic
            if let Some(rest) = trimmed.strip_prefix("wire ") {
                let after_wire = rest.trim_start();
                hints.push(MigrationHint {
                    line: line_num,
                    severity: HintSeverity::Info,
                    category: "type-modernization".into(),
                    original: format!("wire {}", after_wire),
                    suggested: format!("logic {}", after_wire),
                    description: "Use 'logic' for nets (IEEE 1800)".into(),
                });
            }

            // Rule 3: always @(posedge clk) → always_ff
            if trimmed.contains("always @(") {
                if trimmed.contains("posedge") {
                    hints.push(MigrationHint {
                        line: line_num,
                        severity: HintSeverity::Suggestion,
                        category: "process-modernization".into(),
                        original: "always @(...)".into(),
                        suggested: "always_ff @(...)".into(),
                        description: "Use always_ff for sequential logic".into(),
                    });
                } else if !trimmed.contains("always @*") && !trimmed.contains("always @(*)") {
                    hints.push(MigrationHint {
                        line: line_num,
                        severity: HintSeverity::Suggestion,
                        category: "process-modernization".into(),
                        original: "always @(...)".into(),
                        suggested: "always_comb @(...) or always_ff @(...)".into(),
                        description: "Specify process type explicitly".into(),
                    });
                }
            }

            // Rule 4: always @* → always_comb
            if trimmed == "always @*" || trimmed == "always @(*)" {
                hints.push(MigrationHint {
                    line: line_num,
                    severity: HintSeverity::Suggestion,
                    category: "process-modernization".into(),
                    original: "always @*".into(),
                    suggested: "always_comb".into(),
                    description: "Use always_comb for combinational logic".into(),
                });
            }

            // Rule 5: integer → int
            if trimmed.contains("integer ") || trimmed.starts_with("integer ") {
                hints.push(MigrationHint {
                    line: line_num,
                    severity: HintSeverity::Suggestion,
                    category: "type-modernization".into(),
                    original: "integer".into(),
                    suggested: "int".into(),
                    description: "Use two-state 'int' instead of 'integer'".into(),
                });
            }

            // Rule 6: parameter → localparam (for non-port params)
            if trimmed.starts_with("parameter ") && !trimmed.contains("#(") {
                hints.push(MigrationHint {
                    line: line_num,
                    severity: HintSeverity::Info,
                    category: "declaration".into(),
                    original: "parameter".into(),
                    suggested: "localparam".into(),
                    description: "Consider localparam for module-internal constants".into(),
                });
            }

            // Rule 7: begin/end style (manual for loop)
            if trimmed.contains("for (") && trimmed.contains("integer ") {
                hints.push(MigrationHint {
                    line: line_num,
                    severity: HintSeverity::Suggestion,
                    category: "loop-modernization".into(),
                    original: "for (integer i = ...)".into(),
                    suggested: "for (int i = ...)".into(),
                    description: "Use two-state int in for loops".into(),
                });
            }

            // Rule 8: case with full case / parallel case pragmas
            if trimmed.contains("// synopsys full_case")
                || trimmed.contains("// synthesis full_case")
            {
                hints.push(MigrationHint {
                    line: line_num,
                    severity: HintSeverity::Suggestion,
                    category: "pragma-modernization".into(),
                    original: "// synopsys full_case".into(),
                    suggested: "Use 'unique case' or 'priority case'".into(),
                    description: "Replace synthesis pragmas with SystemVerilog keywords".into(),
                });
            }

            // Rule 9: defparam → parameter override
            if trimmed.starts_with("defparam ") {
                hints.push(MigrationHint {
                    line: line_num,
                    severity: HintSeverity::Warning,
                    category: "deprecated".into(),
                    original: "defparam".into(),
                    suggested: "Use #(param) instance override".into(),
                    description: "defparam is deprecated in IEEE 1800".into(),
                });
            }

            // Rule 10: initial block for clock gen
            if trimmed.contains("initial begin") && trimmed.contains("clk") {
                hints.push(MigrationHint {
                    line: line_num,
                    severity: HintSeverity::Info,
                    category: "testbench".into(),
                    original: "initial begin clk = 0; ... end".into(),
                    suggested: "Consider using clocking block".into(),
                    description: "Clocking blocks provide better timing control".into(),
                });
            }

            // Rule 11: tri / wand / wor → logic
            for net_type in &["tri", "wand", "wor", "tri0", "tri1", "trireg"] {
                if trimmed.starts_with(net_type) {
                    hints.push(MigrationHint {
                        line: line_num,
                        severity: HintSeverity::Suggestion,
                        category: "type-modernization".into(),
                        original: format!("{} ...", net_type),
                        suggested: "logic ...".into(),
                        description: "Use 'logic' for most net types".into(),
                    });
                }
            }
        }

        MigrationReport {
            filename: filename.to_string(),
            hints,
        }
    }

    /// Summary statistics.
    pub fn summary(&self) -> MigrationSummary {
        let total = self.hints.len();
        let info = self
            .hints
            .iter()
            .filter(|h| h.severity == HintSeverity::Info)
            .count();
        let warning = self
            .hints
            .iter()
            .filter(|h| h.severity == HintSeverity::Warning)
            .count();
        let suggestion = self
            .hints
            .iter()
            .filter(|h| h.severity == HintSeverity::Suggestion)
            .count();

        let mut categories: Vec<(String, usize)> = Vec::new();
        for hint in &self.hints {
            if let Some(entry) = categories.iter_mut().find(|(c, _)| *c == hint.category) {
                entry.1 += 1;
            } else {
                categories.push((hint.category.clone(), 1));
            }
        }
        categories.sort_by(|a, b| b.1.cmp(&a.1));

        MigrationSummary {
            total_hints: total,
            info_count: info,
            warning_count: warning,
            suggestion_count: suggestion,
            categories,
        }
    }

    /// Generate text report.
    pub fn report(&self) -> String {
        let summary = self.summary();
        let mut out = format!(
            "Migration Report: {}\n\
             ========================\n\
             Total hints: {} (💡{} ⚠️{} ℹ️{})\n\n",
            self.filename,
            summary.total_hints,
            summary.suggestion_count,
            summary.warning_count,
            summary.info_count,
        );

        if !summary.categories.is_empty() {
            out.push_str("By category:\n");
            for (cat, count) in &summary.categories {
                out.push_str(&format!("  {}: {}\n", cat, count));
            }
            out.push('\n');
        }

        for hint in &self.hints {
            out.push_str(&format!(
                "{} Line {}: {} → {}\n  └─ {}\n\n",
                hint.severity, hint.line, hint.original, hint.suggested, hint.description,
            ));
        }

        out
    }
}

#[derive(Debug, Clone)]
pub struct MigrationSummary {
    pub total_hints: usize,
    pub info_count: usize,
    pub warning_count: usize,
    pub suggestion_count: usize,
    pub categories: Vec<(String, usize)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_analyze_verilog() {
        let source = r#"
module counter (
    input wire clk,
    input wire rst,
    output reg [7:0] count
);
    integer i;
    reg [7:0] temp;

    always @(posedge clk) begin
        if (rst)
            count <= 0;
        else
            count <= count + 1;
    end

    always @* begin
        temp = count + 1;
    end
endmodule
"#;
        let report = MigrationReport::analyze("counter.v", source);
        assert!(!report.hints.is_empty());
        assert!(report
            .hints
            .iter()
            .any(|h| h.category == "type-modernization"));
        assert!(report
            .hints
            .iter()
            .any(|h| h.category == "process-modernization"));
    }

    #[test]
    fn test_modern_sv_no_hints() {
        let source = r#"
module counter (
    input  logic       clk,
    input  logic       rst,
    output logic [7:0] count
);
    always_ff @(posedge clk) begin
        if (rst)
            count <= 0;
        else
            count <= count + 1;
    end

    always_comb begin
    end
endmodule
"#;
        let report = MigrationReport::analyze("counter.sv", source);
        // Modern SV should have minimal hints
        assert!(report.hints.len() < 3);
    }

    #[test]
    fn test_defparam_warning() {
        let source = "defparam u0.PARAM = 1;\n";
        let report = MigrationReport::analyze("test.v", source);
        assert!(report
            .hints
            .iter()
            .any(|h| h.severity == HintSeverity::Warning));
    }

    #[test]
    fn test_summary() {
        let source = "reg x;\ninteger i;\nalways @*\n";
        let report = MigrationReport::analyze("t.v", source);
        let summary = report.summary();
        assert!(summary.total_hints >= 3);
    }

    #[test]
    fn test_report_text() {
        let source = "reg x;\nwire y;\n";
        let report = MigrationReport::analyze("t.v", source);
        let text = report.report();
        assert!(text.contains("Migration Report"));
    }
}
