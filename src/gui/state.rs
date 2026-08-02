//! State GUI — data workspace: project, file tree, editor tabs, diagnostics,
//! signals hasil simulasi, dan log console.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::gui::resource::ResourceState;
use crate::ir::IrDesign;

/// Node pohon file (rekursif).
#[derive(Debug, Clone)]
pub struct FileNode {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub children: Vec<FileNode>,
}

/// Satu deklarasi scope untuk Sticky Header (module/interface/package/
/// function/task/always/initial/begin). Disusun urut kemunculan di file;
/// scope terdalam (yang terakhir dengan `line <=` baris teratas terlihat)
/// ditampilkan menempel di atas editor saat scroll — deklarasi tetap terlihat.
#[derive(Debug, Clone)]
pub struct StickyScope {
    /// Baris deklarasi (1-based).
    pub line: usize,
    /// Kedalaman blok begin/end — menentukan indentasi di strip sticky.
    pub depth: usize,
    /// Jenis scope: "module" | "interface" | "package" | "function" |
    /// "task" | "always" | "initial" | "begin" (untuk ikon & warna).
    pub kind: String,
    /// Teks deklarasi (tanpa komentar `//`, dipotong ~64 karakter).
    pub text: String,
}

/// File yang terbuka di editor.
#[derive(Debug, Clone)]
pub struct OpenFile {
    pub path: PathBuf,
    pub name: String,
    pub content: String,
    pub dirty: bool,
    /// Offset scroll vertikal editor (pixel, positif = scroll ke bawah) —
    /// di-update dari ScrollAreaOutput tiap frame, dipakai Sticky Header.
    pub scroll_top: f32,
    /// Cache scope deklarasi (Sticky Header) — di-rebuild saat konten berubah
    /// (dideteksi via fingerprint FNV-1a), bukan tiap frame.
    pub sticky: Vec<StickyScope>,
    /// Fingerprint konten saat `sticky` terakhir dibangun (0 = belum pernah).
    pub sticky_fp: u64,
    /// Baris tujuan lompat berikutnya (Go To Definition) — 1-based, dikonsumsi
    /// (`take`) oleh ScrollArea editor pada frame berikutnya.
    pub pending_goto: Option<usize>,
    /// Sedang proses Rename Symbol (popup input terbuka).
    pub renaming: bool,
    /// Nama lama (identifier di bawah kursor/hover saat F2 ditekan).
    pub rename_old: String,
    /// Nama baru (diisi pengguna di popup rename).
    pub rename_new: String,
    /// Autocomplete: popup kandidat sedang terbuka (otomatis saat mengetik
    /// identifier, atau Ctrl+Space eksplisit).
    pub completing: bool,
    /// Kandidat yang sudah difilter & di-sort (urutan tampil di popup).
    pub completion_items: Vec<String>,
    /// Item yang sedang dipilih (navigasi ↑/↓).
    pub completion_selected: usize,
    /// Prefix yang dipakai saat kandidat terakhir dibangun — deteksi rebuild
    /// hanya saat prefix berubah (bukan setiap frame).
    pub completion_prefix: String,
    /// Byte offset awal kata yang sedang dilengkapi (batas kiri region
    /// penggantian saat accept).
    pub completion_insert: usize,
    /// Byte offset akhir kata — bertumbuh saat mengetik; membeku saat caret
    /// keluar dari kata (mis. navigasi ↑/↓).
    pub completion_end: usize,
}

/// Level diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagLevel {
    Error,
    Warning,
    Info,
}

/// Jenis perbaikan otomatis (Quick Fix) yang bisa diterapkan pada satu
/// diagnostic di Problems tab.
#[derive(Debug, Clone)]
pub enum QuickFixKind {
    /// Hapus seluruh baris deklarasi (mis. signal tak terpakai). Baris target
    /// = `DiagEntry.line`.
    RemoveLine,
    /// Ganti assignment blocking `=` pertama dengan non-blocking `<=` pada
    /// baris target (`DiagEntry.line`).
    BlockingToNonBlocking,
}

/// Aksi perbaikan otomatis yang ditawarkan pada satu diagnostic.
#[derive(Debug, Clone)]
pub struct QuickFix {
    /// Label tombol aksi di Problems tab (mis. "Hapus 'data'").
    pub action: String,
    pub kind: QuickFixKind,
}

/// Satu baris diagnostic (Problems).
#[derive(Debug, Clone)]
pub struct DiagEntry {
    pub file: String,
    pub line: usize,
    pub message: String,
    pub level: DiagLevel,
    /// Aksi perbaikan otomatis (None = tidak ada Quick Fix).
    pub fix: Option<QuickFix>,
}

/// Pratinjau deklarasi untuk Peek Definition (Alt+Click di editor) — sesuai
/// desain: "tidak membuka tab baru, hanya popup".
#[derive(Debug, Clone)]
pub struct PeekInfo {
    pub file: PathBuf,
    pub name: String,
    /// Baris deklarasi (1-based).
    pub line: usize,
    /// (nomor baris, isi baris) konteks sekitar deklarasi untuk pratinjau.
    pub lines: Vec<(usize, String)>,
}

/// Satu baris log console.
#[derive(Debug, Clone)]
pub struct ConsoleLine {
    pub time: String,
    pub msg: String,
}

/// Satu baris output terminal (stdout/stderr dari perintah yang dijalankan).
#[derive(Debug, Clone)]
pub struct TermLine {
    pub text: String,
    /// true = stderr (ditampilkan merah), false = stdout.
    pub is_err: bool,
}

/// Maksimal baris output terminal yang ditahan di memori (anti-bloat).
pub const MAX_TERM_LINES: usize = 2000;

/// Satu baris sinyal hasil simulasi.
#[derive(Debug, Clone)]
pub struct SignalRow {
    pub name: String,
    pub width: usize,
    pub value: String,
    pub kind: String,
}

/// Satu dependency module → child module (module yang diinstansiasi).
#[derive(Debug, Clone)]
pub struct DepRow {
    pub module: String,
    /// (child_module, jumlah instance) — deduplikasi dari sub_instances.
    pub children: Vec<(String, usize)>,
}

/// Satu node hasil layout graf dependensi (tab Dependency visual) — posisi dan
/// ukuran di layar + daftar edge (index node target, jumlah instance).
#[derive(Debug, Clone)]
pub struct DepGraphNode {
    pub name: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// true bila node bagian dari siklus dependensi (ditandai merah).
    pub in_cycle: bool,
    /// (index node target, jumlah instance).
    pub edges: Vec<(usize, usize)>,
}

/// Hasil layout graf dependensi + kunci cache (hash ringkas dari graf). Layout
/// di-cache di `GuiState.dep_graph` agar tidak dihitung ulang tiap frame —
/// deteksi siklus (reachability) bisa mahal untuk desain ribuan module.
#[derive(Debug, Clone)]
pub struct DepGraphLayout {
    pub nodes: Vec<DepGraphNode>,
    pub width: f32,
    pub height: f32,
    /// Kunci cache — layout dipakai ulang bila graf tidak berubah (compile sama).
    pub key: u64,
}

/// Riwayat sampel resource realtime (CPU%, RAM GB, jumlah thread) untuk grafik
/// history di panel Benchmark — observatory "bukan sekadar angka, tetapi
/// histori": pengguna bisa melihat apakah perubahan terakhir membuat performa
/// membaik atau justru memburuk. Di-push oleh status bar / panel Benchmark
/// hanya saat sampel baru diambil (1Hz), di-cap agar memori terkendali.
#[derive(Debug, Clone)]
pub struct ResourceHistory {
    pub cpu: Vec<f32>,
    pub mem: Vec<f32>,
    pub threads: Vec<f32>,
    /// Maksimal entry per seri (180 sampel = 3 menit @1Hz).
    pub max: usize,
}

impl ResourceHistory {
    /// Tambah satu sampel; buang yang tertua jika melebihi `max`.
    pub fn push(&mut self, cpu: f32, mem_gb: f32, threads: f32) {
        self.cpu.push(cpu);
        self.mem.push(mem_gb);
        self.threads.push(threads);
        if self.cpu.len() > self.max {
            self.cpu.remove(0);
            self.mem.remove(0);
            self.threads.remove(0);
        }
    }
}

impl Default for ResourceHistory {
    fn default() -> Self {
        Self {
            cpu: Vec::new(),
            mem: Vec::new(),
            threads: Vec::new(),
            max: 180,
        }
    }
}

/// Statistik MICD (Maria Incremental Compilation Database) — untuk panel
/// Pipeline. Menggambarkan seberapa efektif cache incremental compile:
/// berapa AST di-restore (parse di-skip) vs berapa file yang berubah.
#[derive(Debug, Clone)]
pub struct MicdInfo {
    /// Database ditemukan & dibaca (file metadata.mdb ada di disk).
    pub present: bool,
    /// Jumlah file terdaftar di database.
    pub files: usize,
    /// File yang AST-nya di-restore dari MICD (lexer+parser di-skip).
    pub restored_ast: usize,
    /// File yang berubah pada build ini (perlu di-recompile).
    pub changed_files: usize,
    /// Hit verification cache.
    pub verify_hits: usize,
    /// Miss verification cache.
    pub verify_misses: usize,
    /// Jumlah snapshot build tersedia (rollback).
    pub snapshots: usize,
    /// Ukuran database di disk (bytes, dari seluruh file .mdb).
    pub db_bytes: u64,
}

/// Nama tahap Compile Pipeline — dipakai sebagai konstanta bersama oleh
/// `backend.rs` (saat membangun pipeline) dan `app.rs` (saat menandai tahap
/// Simulator selesai setelah simulasi). Hindari string hardcode ganda yang
/// bisa divergen bila nama tahap diubah.
pub const STAGE_SIMULATOR: &str = "Simulator";

/// Satu tahap Compile Pipeline (Discovery/Lexer/Parser/Elaborator/Optimizer/
/// Simulator) dengan waktu ukur — untuk panel Pipeline.
#[derive(Debug, Clone)]
pub struct PipelineStage {
    pub name: String,
    /// Waktu tahap (ms).
    pub ms: u64,
    /// Status: "ok" | "waiting" | "running".
    pub status: String,
}

/// Hasil compile + elaborate.
#[derive(Debug, Clone)]
pub struct CompileInfo {
    pub success: bool,
    pub modules: Vec<String>,
    pub packages: Vec<String>,
    pub interfaces: Vec<String>,
    pub total_time_ms: f64,
    /// Module → file path (dari module_index) untuk klik-ke-buka di arsitektur.
    pub module_files: std::collections::HashMap<String, PathBuf>,
    /// Graf dependency module → module yang diinstansiasi (untuk tab Dependency).
    pub deps: Vec<DepRow>,
    /// Module → jumlah instansiasi di seluruh design (precompute sekali saat
    /// compile; dipakai Code Lens di editor — hindari iterasi design per frame).
    pub ref_counts: std::collections::HashMap<String, usize>,
    /// Signal → (tipe, lebar bit) dari SEMUA module di design (precompute saat
    /// compile). Dipakai Hover tooltip editor ("logic · 8 bit").
    pub signal_info: std::collections::HashMap<String, (String, usize)>,
    /// Symbol → file asal (module/interface/package dari module_index) untuk
    /// Go To Definition (Ctrl+Click) — precompute sekali saat compile.
    pub symbol_files: std::collections::HashMap<String, PathBuf>,
    /// Lint warning dari scan source (unused signal, blocking assignment di
    /// blok sequential, dll) — ditampilkan di Problems tab dengan Quick Fix.
    pub lint: Vec<DiagEntry>,
    /// Statistik MICD (incremental compilation database) setelah compile —
    /// None bila database tidak tersedia (belum pernah compile / tidak ada
    /// project root). Dipakai panel Pipeline (Incremental Cache).
    pub micd: Option<MicdInfo>,
    /// Timing per tahap compile (Discovery → Preprocessor → Lexer → Parser →
    /// Elaborator → Optimizer → Simulator) — dipakai panel Pipeline.
    pub pipeline: Vec<PipelineStage>,
    /// File yang di-cache (checksum cocok, tidak diproses ulang) — dari
    /// session.timing, untuk ringkasan panel Pipeline.
    pub cached_files: usize,
    /// File yang benar-benar diproses pada compile ini.
    pub processed_files: usize,
}

/// Satu sinyal waveform dengan trace transisi nilai (dari VCD).
#[derive(Debug, Clone)]
pub struct WaveformSignal {
    pub name: String,
    pub width: usize,
    /// Transisi (time, nilai) terurut naik. Nilai = string biner ("0101...",
    /// bisa mengandung x/z). Nilai berlaku sejak time tsb sampai transisi berikut.
    pub trace: Vec<(u64, String)>,
}

/// Satu baris hasil coverage covergroup.
#[derive(Debug, Clone)]
pub struct CovergroupRow {
    pub name: String,
    pub total: u64,
    pub hits: u64,
}

/// Ringkasan coverage hasil simulasi (dari engine.coverage_stats()).
#[derive(Debug, Clone, Default)]
pub struct CoverageInfo {
    pub line_items: u64,
    pub line_hits: u64,
    pub toggle_signals: u64,
    pub toggle_transitions: u64,
    pub branch_total: u64,
    pub branch_covered: u64,
    pub branch_percent: f64,
    pub fsm_signals: u64,
    pub fsm_states: u64,
    pub covergroups: Vec<CovergroupRow>,
}

/// Hasil simulasi (dikirim dari worker).
#[derive(Debug, Clone)]
pub struct SimInfo {
    pub signals: Vec<SignalRow>,
    pub cycles: u64,
    pub sim_time_ms: f64,
    /// Trace waveform (dari VCD) untuk Waveform viewer.
    pub waveform: Vec<WaveformSignal>,
    // ── Benchmark metrics (dari engine.sim_perf) ──
    pub delta_cycles: u64,
    pub events_processed: u64,
    pub processes_evaluated: u64,
    pub nba_commits: u64,
    pub sensitive_triggers: u64,
    pub events_per_delta: f64,
    // ── Coverage (dari engine.coverage_stats) ──
    pub coverage: CoverageInfo,
}

/// Tab sidebar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SidebarTab {
    Project,
    Symbols,
    Architecture,
    Dependency,
    Search,
}

/// Tab panel bawah.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BottomTab {
    Problems,
    Console,
    Signals,
    Waveform,
    Benchmark,
    Coverage,
    Terminal,
    /// Compile Pipeline + Incremental Cache (MICD) — statistik compile
    /// incremental database & timing per tahap (nilai jual Maria).
    Pipeline,
}

/// Event yang dikirim dari worker thread ke UI thread.
pub enum GuiEvent {
    /// Compile selesai → (info, design ter-elaborasi untuk simulasi) atau daftar
    /// diagnostics error (dengan lokasi file/line) untuk Problems tab & Mini Map.
    CompileDone(Result<(CompileInfo, IrDesign), Vec<DiagEntry>>),
    /// Simulasi selesai.
    SimDone(Result<SimInfo, String>),
    /// Satu baris output terminal (text, is_err).
    TermOutput(String, bool),
    /// Proses terminal selesai → exit code.
    TermExit(i32),
}

/// State utama GUI.
pub struct GuiState {
    // ── Project ──
    pub project_name: String,
    pub project_root: Option<PathBuf>,
    pub files: Vec<FileNode>,

    // ── Editor ──
    pub open_files: Vec<OpenFile>,
    pub active_file: Option<usize>,

    // ── Compile/Elaborate ──
    pub design: Option<IrDesign>,
    pub compile_info: Option<CompileInfo>,

    // ── Panels ──
    pub diagnostics: Vec<DiagEntry>,
    pub console: VecDeque<ConsoleLine>,
    pub signals: Vec<SignalRow>,

    // ── Simulasi ──
    pub is_running: bool,
    pub max_time: u64,
    pub sim_time_ms: f64,
    pub cycles: u64,
    /// Flag pembatalan bersama worker thread simulasi (tombol Stop).
    pub cancel_flag: Arc<AtomicBool>,

    // ── Waveform ──
    /// Trace waveform hasil simulasi terakhir.
    pub waveform: Vec<WaveformSignal>,
    /// Zoom horizontal (pixel per unit waktu).
    pub wave_zoom: f32,
    /// Nama signal yang DISEMBUNYIKAN dari tampilan waveform (toggle di pemilih
    /// signal panel) — filter visual tanpa mengubah data trace; berguna untuk
    /// desain dengan banyak signal.
    pub wave_hidden: std::collections::HashSet<String>,

    // ── Architecture ──
    /// State expand/collapse per node pohon arsitektur (key = path instance).
    pub arch_open: std::collections::HashMap<String, bool>,

    // ── Dependency ──
    /// Cache layout graf dependensi visual (None sampai compile pertama) —
    /// di-reset saat compile baru (graf berubah).
    pub dep_graph: Option<DepGraphLayout>,

    // ── Outline & Search ──
    /// Panel outline kanan terlihat/tidak.
    pub show_outline: bool,
    pub outline_filter: String,
    pub search_filter: String,

    // ── Benchmark (hasil sim terakhir) ──
    pub delta_cycles: u64,
    pub events_processed: u64,
    pub processes_evaluated: u64,
    pub nba_commits: u64,
    pub sensitive_triggers: u64,
    pub events_per_delta: f64,
    // ── Coverage (hasil sim terakhir) ──
    pub coverage: CoverageInfo,

    // ── Command Palette ──
    pub palette_open: bool,
    /// true hanya pada frame pertama setelah dibuka (untuk request focus).
    pub palette_just_opened: bool,
    pub palette_filter: String,
    pub palette_selected: usize,

    // ── Terminal ──
    /// Output terminal (stdout/stderr perintah terakhir).
    pub term_lines: Vec<TermLine>,
    /// Input baris perintah saat ini.
    pub term_input: String,
    /// Riwayat perintah (untuk navigasi ↑/↓).
    pub term_history: Vec<String>,
    /// Posisi kursor riwayat (usize::MAX = di ujung, input baru).
    pub term_hist_idx: usize,
    /// Ada proses terminal yang sedang berjalan.
    pub term_running: bool,

    // ── Resource monitor (CPU/RAM realtime) ──
    pub resource: ResourceState,
    /// Riwayat sampel resource — grafik history di panel Benchmark.
    pub resource_hist: ResourceHistory,

    // ── Peek Definition ──
    /// Popup Peek Definition aktif (Alt+Click): data pratinjau.
    pub peek: Option<PeekInfo>,
    /// Posisi anchor popup peek (screen) — sedikit offset dari titik klik.
    pub peek_anchor: Option<(f32, f32)>,

    // ── Layout ──
    pub sidebar_tab: SidebarTab,
    pub bottom_tab: BottomTab,
    pub show_sidebar: bool,
    pub show_bottom: bool,

    /// Channel ke worker threads (panels spawn via ini).
    pub tx: Sender<GuiEvent>,
    /// Penerima event dari worker thread (dipoll di app).
    pub rx: Receiver<GuiEvent>,
}

impl GuiState {
    pub fn new(tx: Sender<GuiEvent>, rx: Receiver<GuiEvent>) -> Self {
        Self {
            project_name: String::new(),
            project_root: None,
            files: Vec::new(),
            open_files: Vec::new(),
            active_file: None,
            design: None,
            compile_info: None,
            diagnostics: Vec::new(),
            console: VecDeque::new(),
            signals: Vec::new(),
            is_running: false,
            max_time: 1000,
            cancel_flag: Arc::new(AtomicBool::new(false)),
            sim_time_ms: 0.0,
            cycles: 0,
            waveform: Vec::new(),
            wave_zoom: 4.0,
            wave_hidden: std::collections::HashSet::new(),
            arch_open: std::collections::HashMap::new(),
            dep_graph: None,
            show_outline: true,
            outline_filter: String::new(),
            search_filter: String::new(),
            delta_cycles: 0,
            events_processed: 0,
            processes_evaluated: 0,
            nba_commits: 0,
            sensitive_triggers: 0,
            events_per_delta: 0.0,
            coverage: CoverageInfo::default(),
            palette_open: false,
            palette_just_opened: false,
            palette_filter: String::new(),
            palette_selected: 0,
            term_lines: Vec::new(),
            term_input: String::new(),
            term_history: Vec::new(),
            term_hist_idx: usize::MAX,
            term_running: false,
            resource: ResourceState::default(),
            resource_hist: ResourceHistory::default(),
            peek: None,
            peek_anchor: None,
            sidebar_tab: SidebarTab::Project,
            bottom_tab: BottomTab::Console,
            show_sidebar: true,
            show_bottom: true,
            tx,
            rx,
        }
    }

    pub fn log(&mut self, msg: impl Into<String>) {
        let t = format!("t={}", self.cycles);
        self.console.push_back(ConsoleLine {
            time: t,
            msg: msg.into(),
        });
        while self.console.len() > 5000 {
            self.console.pop_front();
        }
    }

    /// Buka file di editor (read + tampilkan). Jika sudah terbuka, aktifkan.
    pub fn open_file(&mut self, path: PathBuf) {
        if let Some(i) = self.open_files.iter().position(|f| f.path == path) {
            self.active_file = Some(i);
            return;
        }
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string());
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        self.open_files.push(OpenFile {
            path,
            name,
            content,
            dirty: false,
            scroll_top: 0.0,
            sticky: Vec::new(),
            sticky_fp: 0,
            pending_goto: None,
            renaming: false,
            rename_old: String::new(),
            rename_new: String::new(),
            completing: false,
            completion_items: Vec::new(),
            completion_selected: 0,
            completion_prefix: String::new(),
            completion_insert: 0,
            completion_end: 0,
        });
        self.active_file = Some(self.open_files.len() - 1);
    }

    /// Simpan file aktif (jika dirty) ke disk. Mengembalikan true jika sukses.
    pub fn save_active_file(&mut self) -> bool {
        let Some(idx) = self.active_file else {
            return false;
        };
        // Clone hanya `name` (untuk pesan error). Write dilakukan dalam scoped
        // borrow — jangan memegang & ke `open_files` saat memanggil `self.log()`
        // (borrow E0502) dan jangan clone seluruh isi file.
        let name = match self.open_files.get(idx) {
            Some(f) => f.name.clone(),
            None => return false,
        };
        let result = match self.open_files.get(idx) {
            Some(f) => std::fs::write(&f.path, &f.content),
            None => return false,
        };
        match result {
            Ok(()) => {
                if let Some(f) = self.open_files.get_mut(idx) {
                    f.dirty = false;
                }
                true
            }
            Err(e) => {
                self.log(format!("❌ Gagal menyimpan {}: {}", name, e));
                false
            }
        }
    }

    pub fn close_file(&mut self, idx: usize) {
        if idx >= self.open_files.len() {
            return;
        }
        self.open_files.remove(idx);
        match self.active_file {
            Some(a) if a == idx => {
                self.active_file = if self.open_files.is_empty() {
                    None
                } else {
                    Some(idx.min(self.open_files.len() - 1))
                };
            }
            Some(a) if a > idx => self.active_file = Some(a - 1),
            _ => {}
        }
    }

    /// Terapkan Quick Fix untuk diagnostic pada `diag_idx` (Problems tab).
    /// Menulis perubahan ke file terbuka yang cocok (atau membaca disk bila
    /// belum terbuka), lalu menghapus diagnostic dari daftar. Mengembalikan
    /// true jika fix berhasil diterapkan.
    pub fn apply_quick_fix(&mut self, diag_idx: usize) -> bool {
        let Some(d) = self.diagnostics.get(diag_idx) else {
            return false;
        };
        let fix = match &d.fix {
            Some(f) => f.clone(),
            None => return false,
        };
        let file = d.file.clone();
        let line = d.line;

        // Cari file yang cocok di antara file terbuka; jika belum terbuka,
        // baca dari disk lalu buka setelah fix diterapkan.
        let open_idx = self
            .open_files
            .iter()
            .position(|of| diag_matches_file(&file, &of.path.display().to_string()));

        let (applied, ok) = match open_idx {
            Some(i) => {
                let mut content = self.open_files[i].content.clone();
                let applied = match &fix.kind {
                    QuickFixKind::RemoveLine => remove_line(&mut content, line),
                    QuickFixKind::BlockingToNonBlocking => fix_blocking_assign(&mut content, line),
                };
                if !applied {
                    return false;
                }
                let ok = std::fs::write(&self.open_files[i].path, &content).is_ok();
                self.open_files[i].content = content;
                self.open_files[i].dirty = !ok;
                (applied, ok)
            }
            None => {
                let path = PathBuf::from(&file);
                let Ok(mut content) = std::fs::read_to_string(&path) else {
                    return false;
                };
                let applied = match &fix.kind {
                    QuickFixKind::RemoveLine => remove_line(&mut content, line),
                    QuickFixKind::BlockingToNonBlocking => fix_blocking_assign(&mut content, line),
                };
                if !applied {
                    return false;
                }
                let ok = std::fs::write(&path, &content).is_ok();
                if ok {
                    self.open_file(path);
                }
                (applied, ok)
            }
        };
        if !applied {
            return false;
        }
        self.diagnostics.remove(diag_idx);
        if ok {
            self.log(format!("💡 Quick fix '{}' → {}:{}", fix.action, file, line));
        } else {
            self.log(format!("⚠ Quick fix '{}' gagal menulis {}:{}", fix.action, file, line));
        }
        ok
    }

    /// Kumpulkan semua path file .sv/.svh dari pohon proyek.
    pub fn collect_sv_files(&self) -> Vec<PathBuf> {
        fn walk(nodes: &[FileNode], out: &mut Vec<PathBuf>) {
            for n in nodes {
                if n.is_dir {
                    walk(&n.children, out);
                } else {
                    let ext = n
                        .path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("");
                    if ext == "sv" || ext == "svh" {
                        out.push(n.path.clone());
                    }
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.files, &mut out);
        out
    }
}

// ─────────────────────────── Helper text (Quick Fix) ───────────────────────────

/// Karakter pembentuk identifier SV (`[A-Za-z0-9_$]`).
fn is_word_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

/// Jumlah kemunculan `word` sebagai kata utuh di `text` (boundary aman).
/// Dipakai lint unused signal (backend) — dihitung per file.
pub fn word_count(text: &str, word: &str) -> usize {
    if word.is_empty() {
        return 0;
    }
    let b = text.as_bytes();
    let wb = word.as_bytes();
    let n = b.len();
    let mut count = 0usize;
    let mut i = 0usize;
    while i + wb.len() <= n {
        if b[i] == wb[0] {
            let prev_ok = i == 0 || !is_word_char(b[i - 1]);
            let end = i + wb.len();
            let next_ok = end >= n || !is_word_char(b[end]);
            if prev_ok && next_ok && &b[i..end] == wb {
                count += 1;
                i = end;
                continue;
            }
        }
        i += 1;
    }
    count
}

/// Cocokkan file diagnostic (dari compile/lint) dengan file yang sedang
/// dibuka. Nama file dibandingkan (bukan path penuh) — path bisa berbeda
/// format antara compiler dan tree project.
pub fn diag_matches_file(diag_file: &str, open_path: &str) -> bool {
    if diag_file.is_empty() {
        return false;
    }
    // Fallback pertama: path persis sama (paling andal).
    if diag_file == open_path {
        return true;
    }
    // Kedua: nama file sama — cegah mis-attribute bila ada dua file bernama
    // sama di direktori berbeda (bandingkan nama saja tetap lebih baik dari
    // path penuh yang formatnya bisa beda antara compiler & tree project).
    let a = std::path::Path::new(diag_file)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let b = std::path::Path::new(open_path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    !a.is_empty() && a == b
}

/// Hapus baris `line` (1-based) dari konten. Mengembalikan true jika baris
/// ditemukan dan dihapus.
fn remove_line(content: &mut String, line: usize) -> bool {
    if line == 0 {
        return false;
    }
    let mut out = String::with_capacity(content.len());
    let mut removed = false;
    for (i, l) in content.split('\n').enumerate() {
        if i + 1 == line {
            removed = true;
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(l);
    }
    if !removed {
        return false;
    }
    *content = out;
    true
}

/// Ganti assignment blocking `=` pertama (bukan `==`/`<=`/`>=`/`!=`/`+=` dll)
/// pada baris `line` dengan `<=`. Mengembalikan true jika ada yang diganti.
fn fix_blocking_assign(content: &mut String, line: usize) -> bool {
    if line == 0 {
        return false;
    }
    let mut out = String::with_capacity(content.len());
    let mut changed = false;
    for (i, l) in content.split('\n').enumerate() {
        if i + 1 == line {
            if let Some(pos) = blocking_assign_pos(l) {
                // Separator baris SEBELUM baris yang diganti tetap harus
                // ditulis (jangan `continue` tanpa push newline — baris
                // sebelumnya akan menempel ke baris ini).
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&l[..pos]);
                out.push_str("<=");
                out.push_str(&l[pos + 1..]);
                changed = true;
                continue;
            }
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(l);
    }
    if changed {
        *content = out;
    }
    changed
}

/// Byte offset `=` pertama di baris yang merupakan assignment blocking murni
/// (bukan `==`, `<=`, `>=`, `!=`, `+=`, `-=`, dll). Dipakai lint blocking
/// assignment di blok sequential & Quick Fix (ganti `=` → `<=`).
pub fn blocking_assign_pos(line: &str) -> Option<usize> {
    let b = line.as_bytes();
    let n = b.len();
    let mut i = 0usize;
    while i < n {
        if b[i] == b'=' {
            let prev = if i > 0 { b[i - 1] } else { 0 };
            let next = if i + 1 < n { b[i + 1] } else { 0 };
            // Bukan assignment: `==` (perbandingan), `<=`/`>=`/`!=` (relasional),
            // atau operator majemuk `+=` `-=` dst.
            let is_compound_or_compare = matches!(
                prev,
                b'=' | b'<' | b'>' | b'!' | b'+' | b'-' | b'*' | b'/' | b'&' | b'|' | b'^'
            ) || next == b'=';
            if !is_compound_or_compare {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}
