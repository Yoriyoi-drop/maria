//! Fuzzing terarah (directed) — fokus ke satu fitur/modul target.
//!
//! Paper #17 (DirectFuzz, CANACCI et al. DAC 2021): directed graybox —
//! seed diarahkan ke *site* tujuan ketimbang eksplorasi seluruh desain.
//! Di maria-fuzz: target = fitur bahasa (mis. ">>") atau module tertentu;
//! seed yang tidak relevan di-bias agar memakai fitur target.

/// Target fuzzing terarah.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// Fitur bahasa: operator/konstruk (mis. ">>", "case", "$clog2").
    Feature(String),
    /// Nama modul yang harus ada di seed.
    Module(String),
}

impl Target {
    pub fn feature(name: &str) -> Self {
        Target::Feature(name.to_string())
    }
    pub fn module(name: &str) -> Self {
        Target::Module(name.to_string())
    }
}

/// Apakah seed relevan untuk target (sudah memuatnya)?
pub fn is_relevant(source: &str, target: &str) -> bool {
    source.contains(target)
}

/// Fitur yang AMAN diarahkan oleh steering CDG — `bias_seed` punya
/// replacement/snippet yang valid. Target tanpa snippet (class/package/
/// typedef/enum/struct/import/… atau unary ~,!) di-skip — menghindari
/// mutasi invalid (`assign fz_q = fz_a while 1'b1;`).
pub fn is_steerable(target: &str) -> bool {
    is_operator(target)
        || matches!(
            target,
            "case" | "for" | "while" | "repeat" | "forever" | "fork" | "join"
                | "join_any" | "join_none" | "$clog2" | "$bits" | "$size" | "$display" | "final"
        )
}

/// Bias seed ke target: bila fitur target belum ada, ganti operator lain
/// dengan target (bila operator) atau sisipkan konstruk VALID (bila ada
/// snippet aman). `None` bila target tak steerable / sudah ada.
pub fn bias_seed(source: &str, target: &str) -> Option<String> {
    if !is_steerable(target) {
        return None;
    }
    if source.contains(target) {
        return None;
    }
    // 1) Coba ganti operator yang ada dengan target (bila target operator).
    if is_operator(target) {
        for op in ["+", "-", "&", "|", "^", "<<", ">>", "==", "<", ">"] {
            if op == target {
                continue;
            }
            if let Some(pos) = source.find(op) {
                let mut s = source.to_string();
                s.replace_range(pos..pos + op.len(), target);
                return Some(s);
            }
        }
    }
    // 2) Sisipkan konstruk target — snippet WAJIB valid (elab+sim). "forever"
    //    memakai delay agar tak jadi delta-storm (engine RT2001).
    let snippet = match target {
        "case" => "  always_comb begin case (fz_sel) 1'd0: fz_q = 1'b0; default: fz_q = 1'b1; endcase end\n".to_string(),
        "for" => "  initial begin for (fz_i = 0; fz_i < 4; fz_i = fz_i + 1) fz_q = fz_i[0]; end\n".to_string(),
        "$clog2" => "  localparam fz_cb = $clog2(16);\n".to_string(),
        "$bits" => "  localparam fz_n = $bits(fz_q);\n".to_string(),
        "$size" => "  localparam fz_s = $size(fz_q);\n".to_string(),
        "$display" => "  initial $display(\"fuzz\");\n".to_string(),
        "final" => "  final $display(\"fuzz\");\n".to_string(),
        "while" => "  initial begin fz_i = 0; while (fz_i < 4) begin fz_q = fz_i[0]; fz_i = fz_i + 1; end end\n".to_string(),
        "repeat" => "  initial begin repeat (4) fz_q = ~fz_q; end\n".to_string(),
        "forever" => "  initial begin forever #10 fz_q = ~fz_q; end\n".to_string(),
        "fork" => "  initial begin fork #1 fz_q = 1'b0; #2 fz_q = 1'b1; join end\n".to_string(),
        "join_any" => "  initial begin fork #1 fz_q = 1'b0; join_any end\n".to_string(),
        "join_none" => "  initial begin fork #1 fz_q = 1'b0; join_none end\n".to_string(),
        _ => format!("  assign fz_q = fz_a {} 1'b1;\n", target),
    };
    let mut s = source.to_string();
    if let Some(end) = s.rfind("endmodule") {
        s.insert_str(end, &snippet);
    } else {
        s.push_str(&snippet);
    }
    Some(s)
}

fn is_operator(target: &str) -> bool {
    ["+", "-", "&", "|", "^", "<<", ">>", "==", "!=", "<", ">", "<=", ">="].contains(&target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relevant_checks_substring() {
        assert!(is_relevant("assign y = a >> b;", ">>"));
        assert!(!is_relevant("assign y = a + b;", ">>"));
    }

    #[test]
    fn bias_shifts_operator() {
        let src = "module top;\n  assign y = a + b;\nendmodule\n";
        let out = bias_seed(src, ">>").unwrap();
        assert!(out.contains(">>"));
        assert!(!out.contains(" + "));
    }

    #[test]
    fn bias_already_relevant_returns_none() {
        let src = "module top;\n  assign y = a >> b;\nendmodule\n";
        assert!(bias_seed(src, ">>").is_none());
    }

    #[test]
    fn bias_non_operator_inserts() {
        let src = "module top;\n  assign y = a;\nendmodule\n";
        let out = bias_seed(src, "case").unwrap();
        assert!(out.contains("case"));
    }

    #[test]
    fn steerable_filters_safe_targets() {
        // Hanya target dengan replacement/snippet valid yang boleh diarahkan.
        assert!(is_steerable(">>"));
        assert!(is_steerable("case"));
        assert!(is_steerable("while"));
        assert!(is_steerable("fork"));
        assert!(is_steerable("$clog2"));
        assert!(!is_steerable("class"));
        assert!(!is_steerable("package"));
        assert!(!is_steerable("typedef"));
        assert!(!is_steerable("~"));
        assert!(!is_steerable("!"));
    }

    #[test]
    fn bias_unsteerable_returns_none() {
        let src = "module top;\n  assign y = a;\nendmodule\n";
        assert!(bias_seed(src, "class").is_none(), "target tak aman → tanpa bias");
    }

    #[test]
    fn bias_while_inserts_valid_snippet() {
        let src = "module top;\n  assign y = a;\nendmodule\n";
        let out = bias_seed(src, "while").unwrap();
        assert!(out.contains("while"), "snippet while harus tersisip: {}", out);
    }
}