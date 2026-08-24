//! ──────────────────────────────────────────────────────────────────────────────
//! Global Diagnostic Engine (GDE) — satu-satunya tempat seluruh komponen Maria
//! melaporkan diagnostic (lexer, parser, elaborator, simulator, tools, dll).
//!
//! Semua `DiagSink::push()` diteruskan ke engine ini, jadi komponen TIDAK perlu
//! memanggil engine global secara langsung — cukup lewat sink masing-masing.
//! `DiagSink` tetap dipakai untuk alur per-komponen (bounded queue, sort), dan
//! GDE adalah agregator global yang mencatat SEMUA diagnostic lintas fase.
//!
//! API ringkas:
//!   - `diag_global()`        → akses singleton global
//!   - `report()`             → single entry point (dipanggil DiagSink::push)
//!   - `report_error/warning/fatal/note(...)` → helper
//!   - `all()`                → semua diagnostic, terurut per file/posisi
//!   - `errors()/warnings()/fatal()/notes()`  → filter per level
//!   - `by_code()/by_source()`                → filter per kode / per file
//!   - `unpositioned()`       → diagnostic TANPA file:line:col (coverage posisi)
//!   - `uncoded()`            → diagnostic dengan kode yang TIDAK terdaftar
//!                              di registry `all_codes()` (coverage registry)
//!   - `code_usage()/uncovered_codes()` → analisis coverage kode error
//!   - `summary()/coverage_report()`     → laporan ringkas
//! ──────────────────────────────────────────────────────────────────────────────

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use super::codes::all_codes;
use super::diagnostic::{DiagCode, DiagLevel, Diagnostic};

/// Kapasitas maksimum koleksi global (anti-leak pada sesi yang sangat panjang).
const GLOBAL_CAP: usize = 1_000_000;

struct GlobalState {
    collected: Vec<Diagnostic>,
    by_code: HashMap<DiagCode, usize>,
    by_level: HashMap<DiagLevel, usize>,
}

/// Global Diagnostic Engine — agregator diagnostic lintas seluruh komponen.
pub struct GlobalDiagnosticEngine {
    state: Mutex<GlobalState>,
    /// Total diagnostic yang dilaporkan.
    total: AtomicUsize,
    /// Diagnostic yang dibuang karena kapasitas tercapai.
    dropped: AtomicUsize,
    started: Instant,
}

impl Default for GlobalDiagnosticEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl GlobalDiagnosticEngine {
    pub fn new() -> Self {
        GlobalDiagnosticEngine {
            state: Mutex::new(GlobalState {
                collected: Vec::new(),
                by_code: HashMap::new(),
                by_level: HashMap::new(),
            }),
            total: AtomicUsize::new(0),
            dropped: AtomicUsize::new(0),
            started: Instant::now(),
        }
    }

    // ── Pelaporan (single entry point) ──

    /// Laporkan diagnostic ke engine global. Semua komponen masuk lewat sini
    /// (melalui `DiagSink::push`). Thread-safe.
    pub fn report(&self, diag: Diagnostic) {
        self.total.fetch_add(1, Ordering::Relaxed);
        let mut st = self.state.lock().unwrap();
        if st.collected.len() >= GLOBAL_CAP {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        }
        st.collected.push(diag.clone());
        *st.by_code.entry(diag.code).or_insert(0) += 1;
        *st.by_level.entry(diag.level).or_insert(0) += 1;
    }

    /// Helper: laporkan error dengan kode.
    pub fn report_error(&self, code: DiagCode, message: impl Into<String>) {
        self.report(Diagnostic::new(DiagLevel::Error, code, message.into()));
    }

    /// Helper: laporkan warning dengan kode.
    pub fn report_warning(&self, code: DiagCode, message: impl Into<String>) {
        self.report(Diagnostic::new(DiagLevel::Warning, code, message.into()));
    }

    /// Helper: laporkan fatal dengan kode.
    pub fn report_fatal(&self, code: DiagCode, message: impl Into<String>) {
        self.report(Diagnostic::new(DiagLevel::Fatal, code, message.into()));
    }

    /// Helper: laporkan note (tanpa kode error).
    pub fn report_note(&self, message: impl Into<String>) {
        self.report(Diagnostic::new(
            DiagLevel::Note,
            DiagCode::SimulationError,
            message.into(),
        ));
    }

    // ── Koleksi ──

    /// Kunci posisi untuk sorting: (file, line, col) — fallback ke spans.
    fn pos_key(&self, d: &Diagnostic) -> (String, usize, usize) {
        if let Some(s) = &d.source_snippet {
            return (s.file.clone(), s.line, s.col);
        }
        if let Some(sp) = d.spans.first() {
            return (sp.file.as_str().to_string(), sp.start as usize, 0);
        }
        (String::new(), 0, 0)
    }

    /// Semua diagnostic terurut per file → posisi.
    pub fn all(&self) -> Vec<Diagnostic> {
        let st = self.state.lock().unwrap();
        let mut all = st.collected.clone();
        drop(st);
        all.sort_by(|a, b| self.pos_key(a).cmp(&self.pos_key(b)));
        all
    }

    /// Filter per level.
    pub fn by_level(&self, level: DiagLevel) -> Vec<Diagnostic> {
        self.all()
            .into_iter()
            .filter(|d| d.level == level)
            .collect()
    }

    pub fn errors(&self) -> Vec<Diagnostic> {
        self.by_level(DiagLevel::Error)
    }

    pub fn warnings(&self) -> Vec<Diagnostic> {
        self.by_level(DiagLevel::Warning)
    }

    pub fn fatal(&self) -> Vec<Diagnostic> {
        self.by_level(DiagLevel::Fatal)
    }

    pub fn notes(&self) -> Vec<Diagnostic> {
        self.by_level(DiagLevel::Note)
    }

    /// Filter per kode error.
    pub fn by_code(&self, code: DiagCode) -> Vec<Diagnostic> {
        self.all().into_iter().filter(|d| d.code == code).collect()
    }

    /// Filter per nama file sumber (cocok pada source_snippet atau spans).
    pub fn by_source(&self, file: &str) -> Vec<Diagnostic> {
        self.all()
            .into_iter()
            .filter(|d| {
                d.source_snippet
                    .as_ref()
                    .map(|s| s.file.contains(file))
                    .unwrap_or(false)
                    || d.spans
                        .first()
                        .map(|sp| sp.file.as_str().contains(file))
                        .unwrap_or(false)
            })
            .collect()
    }

    // ── Coverage / penyaringan ──

    /// Diagnostic yang TIDAK punya posisi (tidak ada `source_snippet` maupun
    /// `spans`) — tidak "kena cover" oleh file:line:col. Inilah yang dalam
    /// output tidak menampilkan `--> file:line:col` sama sekali.
    pub fn unpositioned(&self) -> Vec<Diagnostic> {
        let st = self.state.lock().unwrap();
        st.collected
            .iter()
            .filter(|d| d.source_snippet.is_none() && d.spans.is_empty())
            .cloned()
            .collect()
    }

    /// Diagnostic dengan kode yang TIDAK terdaftar di registry `all_codes()`.
    /// Kode seperti ini tidak punya entri resmi di codes.rs — "tidak kena
    /// cover" oleh registry error code. Biasanya menandakan kode baru yang
    /// belum didaftarkan di `DiagCode::as_str()`.
    pub fn uncoded(&self) -> Vec<Diagnostic> {
        let registry: Vec<DiagCode> = all_codes().iter().map(|(c, _)| *c).collect();
        let st = self.state.lock().unwrap();
        st.collected
            .iter()
            .filter(|d| !registry.contains(&d.code))
            .cloned()
            .collect()
    }

    /// Statistik pemakaian kode: (kode-string, jumlah), terurut menurun.
    pub fn code_usage(&self) -> Vec<(String, usize)> {
        let st = self.state.lock().unwrap();
        let mut v: Vec<(String, usize)> = st
            .by_code
            .iter()
            .map(|(c, n)| (c.as_str().to_string(), *n))
            .collect();
        drop(st);
        v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        v
    }

    /// Kode di registry yang TIDAK pernah dipakai komponen mana pun
    /// ("uncovered" oleh pemakaian nyata).
    pub fn uncovered_codes(&self) -> Vec<String> {
        let st = self.state.lock().unwrap();
        all_codes()
            .into_iter()
            .filter(|(c, _)| !st.by_code.contains_key(c))
            .map(|(_, s)| s.to_string())
            .collect()
    }

    // ── Statistik & laporan ──

    pub fn len(&self) -> usize {
        self.state.lock().unwrap().collected.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Total diagnostic yang dilaporkan (termasuk yang dibuang).
    pub fn total(&self) -> usize {
        self.total.load(Ordering::Relaxed)
    }

    pub fn dropped(&self) -> usize {
        self.dropped.load(Ordering::Relaxed)
    }

    pub fn elapsed(&self) -> std::time::Duration {
        self.started.elapsed()
    }

    /// Kosongkan koleksi global (untuk test / sesi baru).
    pub fn reset(&self) {
        let mut st = self.state.lock().unwrap();
        st.collected.clear();
        st.by_code.clear();
        st.by_level.clear();
        st.collected.reserve(0);
    }

    /// Ringkasan singkat: total per level + posisi/registry coverage.
    pub fn summary(&self) -> String {
        let st = self.state.lock().unwrap();
        let e = st.by_level.get(&DiagLevel::Error).copied().unwrap_or(0);
        let w = st.by_level.get(&DiagLevel::Warning).copied().unwrap_or(0);
        let f = st.by_level.get(&DiagLevel::Fatal).copied().unwrap_or(0);
        let n = st.by_level.get(&DiagLevel::Note).copied().unwrap_or(0);
        let up = st
            .collected
            .iter()
            .filter(|d| d.source_snippet.is_none() && d.spans.is_empty())
            .count();
        let registry = all_codes().len();
        drop(st);
        format!(
            "Global diagnostics: total={} (pushed={} dropped={}) error={} warning={} fatal={} note={} | unpositioned={} | codes_emitted={}/{}",
            self.len(),
            self.total(),
            self.dropped(),
            e,
            w,
            f,
            n,
            up,
            self.code_usage().len(),
            registry,
        )
    }

    /// Laporan coverage lengkap: per-level, top kode, kode yang belum terpakai.
    pub fn coverage_report(&self) -> String {
        let mut out = String::new();
        out.push_str(&self.summary());
        out.push('\n');
        let usage = self.code_usage();
        if !usage.is_empty() {
            out.push_str("  Top codes:\n");
            for (code, n) in usage.iter().take(20) {
                out.push_str(&format!("    {} : {}\n", code, n));
            }
        }
        let uncovered = self.uncovered_codes();
        if !uncovered.is_empty() {
            out.push_str(&format!(
                "  Codes never emitted ({}): {}\n",
                uncovered.len(),
                uncovered.join(", ")
            ));
        }
        let up = self.unpositioned();
        if !up.is_empty() {
            out.push_str(&format!("  Unpositioned diagnostics ({}):\n", up.len()));
            for d in up.iter().take(20) {
                out.push_str(&format!("    [{}] {}\n", d.code.as_str(), d.message));
            }
        }
        out
    }
}

/// Singleton global engine.
pub fn diag_global() -> &'static GlobalDiagnosticEngine {
    static GLOBAL: OnceLock<GlobalDiagnosticEngine> = OnceLock::new();
    GLOBAL.get_or_init(GlobalDiagnosticEngine::new)
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::DiagSink;

    #[test]
    fn test_report_and_collect() {
        let engine = GlobalDiagnosticEngine::new();
        assert!(engine.is_empty());
        engine.report_error(DiagCode::UndefinedSignal, "a not found");
        engine.report_warning(DiagCode::WidthMismatchWarning, "width mismatch");
        assert_eq!(engine.len(), 2);
        assert_eq!(engine.errors().len(), 1);
        assert_eq!(engine.warnings().len(), 1);
        assert_eq!(engine.by_code(DiagCode::UndefinedSignal).len(), 1);
        assert_eq!(engine.total(), 2);
    }

    #[test]
    fn test_unpositioned() {
        let engine = GlobalDiagnosticEngine::new();
        engine.report_error(DiagCode::UndefinedSignal, "no position");
        let mut d = Diagnostic::new(DiagLevel::Error, DiagCode::TypeMismatch, "with position");
        d = d.with_source_snippet(crate::diagnostics::SourceSnippet::new(
            "a.sv", 10, 5, "x = 1;",
        ));
        engine.report(d);
        let up = engine.unpositioned();
        assert_eq!(up.len(), 1);
        assert_eq!(up[0].message, "no position");
    }

    #[test]
    fn test_coverage_counts() {
        let engine = GlobalDiagnosticEngine::new();
        engine.report_error(DiagCode::UndefinedSignal, "x");
        engine.report_error(DiagCode::UndefinedSignal, "y");
        engine.report_warning(DiagCode::WidthMismatchWarning, "z");
        let usage = engine.code_usage();
        assert_eq!(usage[0].0, DiagCode::UndefinedSignal.as_str());
        assert_eq!(usage[0].1, 2);
        // Kode yang pernah dipakai tidak masuk uncovered.
        let uncovered = engine.uncovered_codes();
        assert!(!uncovered.contains(&DiagCode::UndefinedSignal.as_str().to_string()));
    }

    #[test]
    fn test_summary_has_counts() {
        let engine = GlobalDiagnosticEngine::new();
        engine.report_error(DiagCode::UndefinedSignal, "x");
        let s = engine.summary();
        assert!(s.contains("error=1"));
        assert!(s.contains("pushed="));
    }

    #[test]
    fn test_diag_sink_forwards_to_global() {
        // DiagSink adalah jalur utama komponen → harus terlihat di singleton.
        diag_global().reset();
        let sink = DiagSink::new();
        sink.push(Diagnostic::new(
            DiagLevel::Error,
            DiagCode::UndefinedSignal,
            "forwarded",
        ));
        sink.push(Diagnostic::new(
            DiagLevel::Warning,
            DiagCode::WidthMismatchWarning,
            "forwarded2",
        ));
        // Catatan: singleton global dibagi antar-test yang berjalan paralel —
        // test lain di modul ini ikut mem-push ke singleton yang sama. Assert
        // berdasarkan pesan (deterministik), bukan berdasarkan jumlah total
        // (yang bisa terkontaminasi test paralel).
        let all = diag_global().all();
        let fwd_err = all.iter().find(|d| d.message == "forwarded");
        let fwd_warn = all.iter().find(|d| d.message == "forwarded2");
        assert!(fwd_err.is_some(), "error harus diteruskan ke global");
        assert!(fwd_warn.is_some(), "warning harus diteruskan ke global");
        assert_eq!(fwd_err.unwrap().level, DiagLevel::Error);
        assert_eq!(fwd_warn.unwrap().level, DiagLevel::Warning);
        diag_global().reset();
    }
}
