//! Regression test untuk bug EMI fuzzer (maria-fuzz kampanye seed 42):
//! evaluator parallel (`evaluate_expr_simple` di simulator/parallel.rs) mem-X-kan
//! SELURUH hasil part-select bila sebagian di luar batas, sedangkan evaluator
//! serial (engine/eval/expr.rs) menerapkan LRM 1800 §11.5.1 per-bit:
//! bit dalam batas = nilai asli, bit luar batas = X.
//!
//! Gejala asli: menambah deklarasi dead-code (`wire [7:0] _fuzz_dn; assign
//! _fuzz_dn = 8'h00;`) menambah 1 proses combinational sehingga jumlah proses
//! melewati `min_processes_parallel` (default 4) → jalur parallel aktif →
//! `__port_u_child_x = xxxx` alih-alih `xx01` → mismatch EMI.
//!
//! Fix: RangeSelect/ExprRangeSelect di parallel.rs kini per-bit §11.5.1,
//! identik dengan jalur serial.

use super::*;

/// SV sumber bug: part-select out-of-range `a[3:0]` pada `a` 2-bit.
/// Jalur serial & parallel wajib memberi hasil identik `xx01`.
const EMI_PARTSEL_SRC: &str = r#"
module child #(
  parameter CW = 4
) (
  input  logic [CW-1:0] x,
  output logic [CW-1:0] q
);
  assign q = ~x;
endmodule

module top (
  input  logic        clk,
  input  logic        rst_n,
  input  logic [1:0]  a,
  input  logic [1:0]  b,
  output logic [1:0]  y,
  output logic [1:0]  flag
);
  logic [1:0] wr;
  child #(.CW(4)) u_child (.x(a[3:0]), .q(wr));
  logic [1:0] r;
  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n)
      r <= '0;
    else
      r <= wr | b;
  end
  assign y = r;
  assign flag = (a > b) ? 2'd1 : (a == b) ? 2'd2 : 2'd0;
  initial begin clk = 0; forever #5 clk = ~clk; end
  initial begin rst_n = 0; a = 5; b = 3; #7 rst_n = 1; #3 a = 17; b = 9; end
  DEAD_CODE
endmodule
"#;

/// Jalankan EMI_PARTSEL_SRC pada konfigurasi parallel tertentu, kembalikan
/// nilai sinyal `name` dalam bentuk string bit (mis. "xx01").
fn run_emivariant(dead_code: &str, mut pcfg: maria_simulator::simulator::parallel::ParallelConfig) -> String {
    let source = EMI_PARTSEL_SRC.replace("  DEAD_CODE", dead_code);
    let design = compile_str(&source).unwrap();
    let mut engine = maria_simulator::simulator::SimulationEngine::new(design, 100);
    engine.set_parallel_config(pcfg);
    engine.run().unwrap();
    let sigs = engine.design.top.signals.clone();
    let idx = sigs
        .iter()
        .position(|s| s.name == "__port_u_child_x")
        .expect("sinyal __port_u_child_x harus ada");
    let val = engine.state.read_signal(idx);
    // Penyimpanan internal LSB-first; string ditulis MSB-first (sama dengan
    // --print-state/VCD) agar expected "xx01" terbaca alami.
    val.bits
        .iter()
        .rev()
        .map(|b| match b {
            maria_ir::LogicVal::Zero => '0',
            maria_ir::LogicVal::One => '1',
            maria_ir::LogicVal::X => 'x',
            maria_ir::LogicVal::Z => 'z',
        })
        .collect()
}

/// Jalur serial (parallel dimatikan) — referensi semantik §11.5.1.
#[test]
fn test_emi_partselect_serial_reference() {
    let mut pcfg = maria_simulator::simulator::parallel::ParallelConfig::default();
    pcfg.parallel_processes = false;
    let x = run_emivariant("", pcfg);
    assert_eq!(x, "xx01", "serial: a[3:0] pada a 2-bit → bit dalam batas nilai asli, sisanya X");
}

/// Bug asli fuzzer: proses comb ke-4+ mengaktifkan jalur parallel — hasil
/// wajib identik dengan jalur serial.
#[test]
fn test_emi_partselect_parallel_with_dead_code() {
    let pcfg = maria_simulator::simulator::parallel::ParallelConfig::default();
    let x = run_emivariant("  wire [7:0] _fuzz_dn;\n  assign _fuzz_dn = 8'h00;", pcfg);
    assert_eq!(
        x, "xx01",
        "parallel (threshold terlampaui via dead-code assign): harus identik dengan serial"
    );
}

/// Parallel dipaksa aktif untuk jumlah proses kecil.
#[test]
fn test_emi_partselect_parallel_forced() {
    let mut pcfg = maria_simulator::simulator::parallel::ParallelConfig::default();
    pcfg.min_processes_parallel = 1;
    let x = run_emivariant("", pcfg);
    assert_eq!(x, "xx01", "parallel dipaksa aktif: part-select OOB per-bit §11.5.1");
}

/// Varian ExprRangeSelect: `a[base+:4]` pada `a` 2-bit, base konstan 0.
#[test]
fn test_emi_expr_partselect_parallel() {
    let source = r#"
module tb;
    reg [1:0] a;
    wire [3:0] w;
    assign w = a[0 +: 4];
    initial begin
        a = 2'b01;
        #1 $finish;
    end
endmodule
"#;
    let design = compile_str(source).unwrap();
    // Serial
    let mut engine = maria_simulator::simulator::SimulationEngine::new(design.clone(), 10);
    let mut ser = maria_simulator::simulator::parallel::ParallelConfig::default();
    ser.parallel_processes = false;
    engine.set_parallel_config(ser);
    engine.run().unwrap();
    let sigs = engine.design.top.signals.clone();
    let idx = sigs.iter().position(|s| s.name == "w").unwrap();
    let serial = engine.state.read_signal(idx).clone();
    // Parallel dipaksa
    let mut engine = maria_simulator::simulator::SimulationEngine::new(design, 10);
    let mut par = maria_simulator::simulator::parallel::ParallelConfig::default();
    par.min_processes_parallel = 1;
    engine.set_parallel_config(par);
    engine.run().unwrap();
    let parallel = engine.state.read_signal(idx).clone();
    assert_eq!(serial.width, 4);
    assert_eq!(serial.bits[0], maria_ir::LogicVal::One, "bit 0 dalam batas = nilai asli");
    assert_eq!(serial.bits[1], maria_ir::LogicVal::Zero, "bit 1 dalam batas = nilai asli");
    assert_eq!(serial.bits[2], maria_ir::LogicVal::X, "bit 2 OOB = X");
    assert_eq!(parallel, serial, "parallel harus identik dengan serial");
}

/// Regresi hang parser (ditemukan maria-fuzz, bugdb Hang opentitan kmac):
/// fragmen `clocking cb @(posedge tck);` TERPOTONG tanpa `endclocking` di level
/// unit (stray decl masuk module implisit) → `parse_clocking_block` catch-all
/// `_ => advance()` memutar di EOF selamanya. `advance()` hanya clamp pos ke
/// len melewati step-limit; loop TIDAK pernah break → infinite loop yang
/// melewati safety counter (`parse_steps`/`peek_count` — keduanya di-reset,
/// pos tidak pernah berubah → stuck detection di parse_design level-atas tak
/// pernah tercapai karena kontrol tidak pernah kembali ke sana).
///
/// Fix: specify.rs `parse_clocking_block` — arm `_` break bila `Token::Eof`.
#[test]
fn test_parser_no_hang_on_truncated_clocking_fragment() {
    let src = r#"
class kmac_smoke_vseq extends kmac_base_vseq;
  task body();
    fork
      begin : send_kmac_req
        wait (cond);
      end
    join
  endtask
endclass
wire [16-1:0] fz_863;
clocking cb @(posedge tck);
input tms;
"#;
    // Watchdog 10s: sebelum fix input ini hang >170 s (infinite loop parser).
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(compile_str(&src));
    });
    let result = rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("compile truncated clocking block harus selesai <10s (dulu hang? regresi)");
    // Hang = regresi (timeout panik di atas). Selesai cepat = ok/err biasa.
    let _ = result;
}

/// Regresi panic elaborasi (ditemukan maria-fuzz seed 42, corpus opentitan,
/// 6× `attempt to add with overflow`): concat `{ sig[0+:(IDW-STIDW)], ... }`
/// dengan `IDW = top_pkg::TL_AIW` (symbol package tak-resolved → 0) dan
/// `STIDW = $clog2(M)` (2) → `IDW - STIDW` = -2 → const-eval memberi i64
/// negatif → `as usize` mem-bungkus ke 2^64-2 → `ExprRangeSelect(hi=u64::MAX-1)`
/// → `expr_approx_width` concat `.sum()` overflow panic.
///
/// Fix dua lapis (maria-elaboration):
/// 1. `stmt.rs expr_approx_width` — Concat/Replicate kini saturasi (lebar
///    hanya perkiraan utk konteks sizing — tidak boleh panic).
/// 2. `expr.rs RangeSelect/ExprRangeSelect` — bound const `max(0)` sebelum
///    `as usize`, negatif = OOB (engine mengisi X per §11.5.1).
#[test]
fn test_no_panic_concat_negative_range_select() {
    let src = r#"
module tlul_socket_m1 #(
  parameter int unsigned  M         = 4,
) (
);
  localparam int unsigned IDW   = top_pkg::TL_AIW;
  localparam int unsigned STIDW = $clog2(M);
    assign shifted_id = {
      tl_h_i[i].a_source[0+:(IDW-STIDW)],
      reqid_sub
    };
endmodule
"#;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(compile_str(&src));
    });
    let r = rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("compile concat lebar negatif harus selesai <10s (dulu panic/regresi?)");
    assert!(r.is_ok(), "part-select lebar negatif/unresolved harus error elegan, bukan panic");
}
