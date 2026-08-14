//! ──────────────────────────────────────────────────────────────────────────────
//! CATATAN: File ini adalah bagian dari pemisahan util.rs (SRP Refactoring).
//! Tanggung jawab: Statistik optimasi elaborator untuk cache pipeline
//! (db.md "6. optimize/" + "10. expression/").
//!
//! `OptStats` adalah penghitung `Cell` (bisa di-increment dari method `&self`
//! — `elaborate_expr` menerima `&self`, bukan `&mut self`). `snapshot()` \
//! mengembalikan [`OptimizeSnapshot`] yang aman dikirim lintas crate dan
//! disimpan ke cache `optimize/` + `expression/`.
//!
//! ──────────────────────────────────────────────────────────────────────────────

use std::cell::{Cell, RefCell};

/// Penghitung optimasi elaborator (db.md "6. optimize/").
/// Memakai `Cell<usize>` agar dapat di-increment dari method `&self` tanpa
/// mengubah ribuan signature `elaborate_expr`/`elaborate_stmt`.
#[derive(Debug, Default)]
pub struct OptStats {
    /// Jumlah constant folding yang berhasil (db.md "6. optimize/").
    pub const_folds: Cell<usize>,
    /// Jumlah loop for yang di-unroll saat elaborasi.
    pub loop_unrolls: Cell<usize>,
    /// Jumlah statement hasil unroll (ukuran kerja unroll).
    pub unrolled_stmts: Cell<usize>,
    /// Jumlah panggilan `elaborate_expr` (evaluasi ekspresi, db.md
    /// "10. expression/" — compiler modern melakukannya jutaan kali).
    pub expr_evals: Cell<usize>,
    /// Sampel hasil evaluasi ekspresi konstanta (db.md "10. expression/":
    /// `4+5 → 9`). Disimpan sebagai (teks ekspresi, nilai) — maksimal 8
    /// sampel pertama agar payload cache tetap kecil.
    pub expr_samples: RefCell<Vec<(String, i64)>>,
}

impl OptStats {
    /// Catat satu evaluasi ekspresi (dipanggil di awal `elaborate_expr`).
    pub fn record_expr_eval(&self) {
        self.expr_evals.set(self.expr_evals.get() + 1);
    }

    /// Catat satu constant folding berhasil + sampel (ekspresi, nilai) bila
    /// kuota sampel belum penuh.
    pub fn record_const_fold(&self, expr_text: String, value: i64) {
        self.const_folds.set(self.const_folds.get() + 1);
        let mut samples = self.expr_samples.borrow_mut();
        if samples.len() < 8 {
            samples.push((expr_text, value));
        }
    }

    /// Catat satu loop for yang berhasil di-unroll.
    pub fn record_loop_unroll(&self, stmt_count: usize) {
        self.loop_unrolls.set(self.loop_unrolls.get() + 1);
        self.unrolled_stmts
            .set(self.unrolled_stmts.get() + stmt_count);
    }

    /// Snapshot statistik untuk disimpan ke cache (bukan Cell, aman
    /// di-serialize / dikirim lintas thread).
    pub fn snapshot(&self) -> OptimizeSnapshot {
        OptimizeSnapshot {
            const_folds: self.const_folds.get(),
            loop_unrolls: self.loop_unrolls.get(),
            unrolled_stmts: self.unrolled_stmts.get(),
            expr_evals: self.expr_evals.get(),
            expr_samples: self.expr_samples.borrow().clone(),
        }
    }
}

/// Snapshot statistik optimasi — disimpan ke cache `optimize/` + `expression/`
/// (db.md "6." + "10."). Bukan `Cell`, aman diserialize dan dibaca tool lain.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OptimizeSnapshot {
    /// Jumlah constant folding berhasil.
    pub const_folds: usize,
    /// Jumlah loop for di-unroll.
    pub loop_unrolls: usize,
    /// Jumlah statement hasil unroll.
    pub unrolled_stmts: usize,
    /// Jumlah panggilan `elaborate_expr` (evaluasi ekspresi).
    pub expr_evals: usize,
    /// Sampel (ekspresi, nilai) hasil evaluasi konstanta.
    pub expr_samples: Vec<(String, i64)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_accumulate_and_snapshot() {
        let s = OptStats::default();
        s.record_expr_eval();
        s.record_expr_eval();
        s.record_const_fold("4+5".into(), 9);
        s.record_const_fold("WIDTH*8".into(), 256);
        s.record_loop_unroll(12);
        let snap = s.snapshot();
        assert_eq!(snap.expr_evals, 2);
        assert_eq!(snap.const_folds, 2);
        assert_eq!(snap.loop_unrolls, 1);
        assert_eq!(snap.unrolled_stmts, 12);
        assert_eq!(snap.expr_samples.len(), 2);
        assert_eq!(snap.expr_samples[0], ("4+5".to_string(), 9));
    }

    #[test]
    fn samples_capped_at_8() {
        let s = OptStats::default();
        for i in 0..20 {
            s.record_const_fold(format!("expr{}", i), i);
        }
        assert_eq!(s.snapshot().expr_samples.len(), 8);
        assert_eq!(s.const_folds.get(), 20, "penghitung tetap akurat");
    }
}
