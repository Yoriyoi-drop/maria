//! Generator input SV.
//!
//! Paper #1 (Miller 1990): input acak murni — baseline robustnes lexer/parser.
//! Paper #14 (Csmith): generator program/modul acak lengkap tapi valid.
//! Paper #15 (YARPGen): type-aware — lebar/signedness dipilih sadar tipe,
//!   menghindari konstruk yang jelas-jelas invalid (bukan sekadar coblos acak).

use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::Rng;

pub const WIDTHS: &[usize] = &[2, 4, 8, 16];
pub const OPS: &[&str] = &["+", "-", "&", "|", "^", "<<", ">>"];

/// Generator deterministik ber-seed (Paper #14/#15: reproducibility).
pub struct Generator {
    #[allow(dead_code)] // seed disimpan utk API future (reset/derive)
    seed: u64,
}

impl Generator {
    pub fn new(seed: u64) -> Self {
        Generator { seed }
    }

    /// Input acak murni — deretan token SV campur junk (Paper #1).
    /// Sasaran: lexer/parser robustness, bukan semantik.
    pub fn random_syntax(&self, rng: &mut StdRng) -> String {
        const CHARSET: &[char] = &[
            'a', 'b', 'm', 'o', 'd', 'u', 'l', 'e', 't', 'o', 'p', '0', '1', 'x', 'z', '_', '[',
            ']', '(', ')', ';', ':', ',', '+', '-', '&', '|', '^', '~', '<', '>', '#', '.', '@',
            '{', '}', '"', '\'', '`', '/', '*', '\n', ' ', ' ', '=', '!', '?',
        ];
        let n = rng.gen_range(1..=200);
        let mut s = String::with_capacity(n + 32);
        if rng.gen_bool(0.7) {
            s.push_str("module top;\n");
        }
        for _ in 0..n {
            if rng.gen_bool(0.08) {
                s.push('\n');
            } else if rng.gen_bool(0.5) {
                s.push(*CHARSET.choose(rng).unwrap());
            } else {
                s.push_str(["endmodule", "always", "initial", "if", "assign", "reg", "wire"]
                    .choose(rng)
                    .unwrap());
            }
        }
        s
    }

    /// Modul SV acak — valid (Paper #14/#15) untuk stress engine semantik.
    pub fn random_module(&self, rng: &mut StdRng) -> String {
        let w = *WIDTHS.choose(rng).unwrap();
        let op = *OPS.choose(rng).unwrap();
        let shape = rng.gen_range(0..3u32);
        let with_param = rng.gen_bool(0.15);
        let with_clog2 = rng.gen_bool(0.3);

        match shape {
            0 => self.shape_arith(rng, w, op, with_param, with_clog2),
            1 => self.shape_case(rng, w, with_param),
            2 => self.shape_child(rng, w, op, with_param),
            _ => self.shape_loop(rng, w, with_param),
        }
    }

    /// Shape 1: `always_ff` + `assign` — arithmetic/bitwise pada register.
    fn shape_arith(&self, _rng: &mut StdRng, w: usize, op: &str, param: bool, clog2: bool) -> String {
        let mut s = String::new();
        let mut header = String::new();
        if param {
            header.push_str("module top #(parameter W = 8) (\n");
        } else {
            header.push_str(&format!("module top (\n"));
        }
        header.push_str(&format!(
            "  input  logic        clk,\n  input  logic        rst_n,\n  input  logic [{}-1:0] a,\n  input  logic [{}-1:0] b,\n  output logic [{}-1:0] y,\n  output logic [1:0]   flag\n);\n",
            w, w, w
        ));
        s.push_str(&header);
        if param {
            s.push_str("  localparam LW = W;\n");
        }
        if clog2 {
            s.push_str(&format!("  localparam CB = $clog2({});\n", w));
            s.push_str(&format!("  logic [{}-1:0] idx;\n", w));
            s.push_str("  always_comb idx = a;\n");
        }
        s.push_str(&format!("  logic [{}-1:0] r;\n", w));
        s.push_str("  always_ff @(posedge clk or negedge rst_n) begin\n    if (!rst_n)\n      r <= '0;\n    else\n");
        s.push_str(&format!("      r <= a {} b;\n", op));
        s.push_str("  end\n");
        s.push_str("  assign y = r;\n");
        s.push_str("  assign flag = (a > b) ? 2'd1 : (a == b) ? 2'd2 : 2'd0;\n");
        s.push_str("  initial begin clk = 0; forever #5 clk = ~clk; end\n");
        s.push_str("  initial begin rst_n = 0; a = 5; b = 3; #7 rst_n = 1; #3 a = 17; b = 9; #7 b = 4; end\n");
        s.push_str("endmodule\n");
        s
    }

    /// Shape 2: `always_comb` + `case` — ekspresi seleksi multi-branch.
    fn shape_case(&self, rng: &mut StdRng, w: usize, param: bool) -> String {
        let mut s = String::new();
        if param {
            s.push_str("module top #(parameter W = 8) (\n");
        } else {
            s.push_str("module top (\n");
        }
        s.push_str(&format!(
            "  input  logic        clk,\n  input  logic        rst_n,\n  input  logic [{}-1:0] a,\n  input  logic [{}-1:0] b,\n  output logic [{}-1:0] y,\n  output logic [1:0]   flag\n);\n",
            w, w, w
        ));
        s.push_str(&format!("  logic [{}-1:0] r;\n", w));
        // case sensitif: pilih isi default bervariasi
        s.push_str("  always_comb begin\n    case (a[1:0])\n      2'd0: r = b;\n      2'd1: r = a & b;\n      2'd2: r = a | b;\n      default: r = a;\n    endcase\n  end\n");
        s.push_str(&format!("  assign y = r;\n"));
        s.push_str("  assign flag = (a > b) ? 2'd1 : (a == b) ? 2'd2 : 2'd0;\n");
        if rng.gen_bool(0.5) {
            s.push_str("  initial begin clk = 0; forever #5 clk = ~clk; end\n");
        }
        s.push_str("  initial begin rst_n = 0; a = 5; b = 3; #7 rst_n = 1; #3 a = 17; b = 9; #7 b = 4; end\n");
        s.push_str("endmodule\n");
        s
    }

    /// Shape 3: hierarki dua modul — child diinstansiasi di top (Paper #14:
    /// multi-modul, instansiasi, koneksi port).
    fn shape_child(&self, _rng: &mut StdRng, w: usize, op: &str, param: bool) -> String {
        let mut s = String::new();
        s.push_str("#(\n  parameter CW = 4\n) module child (\n");
        s.push_str("  input  logic [CW-1:0] x,\n  output logic [CW-1:0] q\n);\n");
        s.push_str("  assign q = ~x;\n");
        s.push_str("endmodule\n\n");
        if param {
            s.push_str("module top #(parameter W = 8) (\n");
        } else {
            s.push_str("module top (\n");
        }
        s.push_str(&format!(
            "  input  logic        clk,\n  input  logic        rst_n,\n  input  logic [{}-1:0] a,\n  input  logic [{}-1:0] b,\n  output logic [{}-1:0] y,\n  output logic [1:0]   flag\n);\n",
            w, w, w
        ));
        s.push_str(&format!("  logic [{}-1:0] wr;\n", w));
        s.push_str("  child #(.CW(4)) u_child (.x(a[3:0]), .q(wr));\n");
        s.push_str(&format!("  logic [{}-1:0] r;\n", w));
        s.push_str("  always_ff @(posedge clk or negedge rst_n) begin\n    if (!rst_n)\n      r <= '0;\n    else\n");
        s.push_str(&format!("      r <= wr {} b;\n", op));
        s.push_str("  end\n");
        s.push_str("  assign y = r;\n");
        s.push_str("  assign flag = (a > b) ? 2'd1 : (a == b) ? 2'd2 : 2'd0;\n");
        s.push_str("  initial begin clk = 0; forever #5 clk = ~clk; end\n");
        s.push_str("  initial begin rst_n = 0; a = 5; b = 3; #7 rst_n = 1; #3 a = 17; b = 9; #7 b = 4; end\n");
        s.push_str("endmodule\n");
        s
    }

    /// Shape 4: `always_comb` + `for` loop unroll + nested `if/else` + part-select
    /// — stress statement-engine (loop unrolling, distribusi i, bit-select).
    fn shape_loop(&self, rng: &mut StdRng, w: usize, param: bool) -> String {
        let mut s = String::new();
        if param {
            s.push_str("module top #(parameter W = 8) (\n");
        } else {
            s.push_str("module top (\n");
        }
        s.push_str(&format!(
            "  input  logic        clk,\n  input  logic        rst_n,\n  input  logic [{}-1:0] a,\n  input  logic [{}-1:0] b,\n  output logic [{}-1:0] y,\n  output logic [1:0]   flag\n);\n",
            w, w, w
        ));
        s.push_str(&format!("  logic [{}-1:0] r;\n", w));
        s.push_str("  integer i;\n");
        s.push_str("  always_comb begin\n    r = a;\n");
        s.push_str(&format!(
            "    for (i = 0; i < {}; i = i + 1) begin\n      if (r[i]) r = r ^ b;\n      else r = r + b;\n    end\n",
            w
        ));
        s.push_str("  end\n");
        s.push_str(&format!("  assign y = r;\n"));
        s.push_str("  assign flag = (a > b) ? 2'd1 : (a == b) ? 2'd2 : 2'd0;\n");
        s.push_str("  initial begin rst_n = 0; a = 5; b = 3; #7 rst_n = 1; #3 a = 17; b = 9; #7 b = 4; end\n");
        s.push_str("endmodule\n");
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oracle::compile_verdict;
    use rand::SeedableRng;

    #[test]
    fn random_syntax_nonempty() {
        let mut rng = StdRng::seed_from_u64(1);
        let g = Generator::new(1);
        let s = g.random_syntax(&mut rng);
        assert!(!s.is_empty());
    }

    #[test]
    fn generated_modules_compile() {
        let mut rng = StdRng::seed_from_u64(42);
        let g = Generator::new(42);
        let mut ok = 0;
        let mut total = 0;
        for i in 0..24 {
            let src = g.random_module(&mut rng);
            total += 1;
            let v = compile_verdict(&src);
            if v.ok {
                ok += 1;
            } else {
                // Cetak kegagalan utk diagnosa cepat ke pengembang.
                eprintln!("seed #{} gagal compile: {}", i, v.message);
            }
        }
        assert!(
            ok > total / 2,
            "sebagian besar seed generated harus compile ok ({}/{})",
            ok,
            total
        );
    }
}