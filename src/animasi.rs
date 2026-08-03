//! PipelineAnimator — animasi compile Maria sebagai waveform digital.
//!
//! Menampilkan fase pipeline (LEX/PAR/ELA/OPT/VER) sebagai sinyal digital ala
//! logic analyzer: setiap fase adalah satu baris yang terisi blok `█` saat
//! aktif, lalu menjadi `✓` ketika selesai. Baris CLK selalu berjalan meniru
//! clock yang menggerakkan pipeline.
//!
//! Render berjalan di background thread (±10 FPS) sehingga animasi tetap hidup
//! meski kompilasi berjalan sinkron di thread utama. Output digambar ulang di
//! area tetap tanpa menggulir terminal, lalu diakhiri panel ringkasan EDA.

use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

// ── ANSI ──
const RED: &str = "\x1b[31m";
const GREEN: &str = "\x1b[32m";
const BRIGHT_GREEN: &str = "\x1b[92m";
const YELLOW: &str = "\x1b[33m";
const GRAY: &str = "\x1b[90m";
const CYAN: &str = "\x1b[36m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";
const SAVE_CURSOR: &str = "\x1b7";
const RESTORE_CURSOR: &str = "\x1b8";
const HIDE_CURSOR: &str = "\x1b[?25l";
const SHOW_CURSOR: &str = "\x1b[?25h";
const CLEAR_LINE: &str = "\x1b[2K";

const BAR_WIDTH: usize = 20;
const CLK_WIDTH: usize = 24;
/// Tinggi area animasi (baris) — harus tetap agar output di bawah tidak rusak.
const AREA_LINES: usize = 12;

/// Fase pipeline yang ditampilkan.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Lex,
    Par,
    Ela,
    Opt,
    Ver,
}

const PHASE_NAMES: [&str; 5] = ["LEX", "PAR", "ELA", "OPT", "VER"];

impl Phase {
    fn idx(self) -> usize {
        match self {
            Phase::Lex => 0,
            Phase::Par => 1,
            Phase::Ela => 2,
            Phase::Opt => 3,
            Phase::Ver => 4,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PhaseStatus {
    Idle,
    Running,
    Done,
    Warn,
    Error,
}

struct PhaseInfo {
    status: PhaseStatus,
    progress: u8,
    warn_count: usize,
}

impl Default for PhaseInfo {
    fn default() -> Self {
        PhaseInfo {
            status: PhaseStatus::Idle,
            progress: 0,
            warn_count: 0,
        }
    }
}

struct AnimState {
    phases: [PhaseInfo; 5],
    files_total: u64,
    files_done: u64,
    modules: u64,
    tokens: u64,
    memory: u64,
    workers: u64,
    start: Instant,
    finished: bool,
    ok: bool,
    errors: usize,
    warnings: usize,
}

impl Default for AnimState {
    fn default() -> Self {
        AnimState {
            phases: std::array::from_fn(|_| PhaseInfo::default()),
            files_total: 0,
            files_done: 0,
            modules: 0,
            tokens: 0,
            memory: 0,
            workers: num_cpus::get() as u64,
            start: Instant::now(),
            finished: false,
            ok: true,
            errors: 0,
            warnings: 0,
        }
    }
}

/// Animator pipeline compile. `start()` mengembalikan `None` bila terminal
/// tidak mendukung (bukan TTY) atau animasi dinonaktifkan — caller tidak perlu
/// perubahan apa pun untuk jalur non-interaktif.
pub struct PipelineAnimator {
    state: Arc<Mutex<AnimState>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    active: bool,
}

/// Deteksi apakah stdout adalah TTY.
fn stdout_is_tty() -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        let fd = io::stdout().as_raw_fd();
        unsafe extern "C" {
            fn isatty(fd: i32) -> i32;
        }
        unsafe { isatty(fd) != 0 }
    }
    #[cfg(not(unix))]
    {
        false
    }
}

impl PipelineAnimator {
    /// Mulai animasi. `enabled` mengontrol apakah animasi diizinkan (mis.
    /// false saat `--quiet` / mode debug). Mengembalikan `None` bila tidak
    /// ada animasi (bukan TTY atau disabled).
    pub fn start(enabled: bool) -> Option<PipelineAnimator> {
        if !enabled || !stdout_is_tty() {
            return None;
        }
        let state = Arc::new(Mutex::new(AnimState::default()));
        let stop = Arc::new(AtomicBool::new(false));

        // Area pertama: simpan posisi kursor + sembunyikan kursor.
        let mut out = io::stdout();
        let _ = write!(out, "{}{}", SAVE_CURSOR, HIDE_CURSOR);
        let _ = out.flush();

        let render_state = Arc::clone(&state);
        let render_stop = Arc::clone(&stop);
        let handle = std::thread::spawn(move || {
            let mut frame: u64 = 0;
            loop {
                if render_stop.load(Ordering::Relaxed) {
                    break;
                }
                // Naikkan progress fase Running secara perlahan (indeterminate
                // hingga 95%) — lompat ke 100 saat fase ditandai selesai.
                {
                    let mut st = render_state.lock().unwrap();
                    if !st.finished {
                        for p in st.phases.iter_mut() {
                            if p.status == PhaseStatus::Running && p.progress < 95 {
                                p.progress = p.progress.saturating_add(3).min(95);
                            }
                        }
                        st.memory = read_mem_mb();
                    }
                }
                render_frame(&render_state, frame);
                frame = frame.wrapping_add(1);
                std::thread::sleep(Duration::from_millis(100));
            }
        });

        Some(PipelineAnimator {
            state,
            stop,
            handle: Some(handle),
            active: true,
        })
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn phase_running(&self, phase: Phase) {
        let mut st = self.state.lock().unwrap();
        let i = phase.idx();
        st.phases[i].status = PhaseStatus::Running;
        st.phases[i].progress = 0;
    }

    pub fn phase_done(&self, phase: Phase) {
        let mut st = self.state.lock().unwrap();
        let i = phase.idx();
        st.phases[i].status = PhaseStatus::Done;
        st.phases[i].progress = 100;
    }

    pub fn phase_warn(&self, phase: Phase, count: usize) {
        let mut st = self.state.lock().unwrap();
        let i = phase.idx();
        st.phases[i].status = PhaseStatus::Warn;
        st.phases[i].progress = 100;
        st.phases[i].warn_count = count;
    }

    pub fn phase_error(&self, phase: Phase) {
        let mut st = self.state.lock().unwrap();
        let i = phase.idx();
        st.phases[i].status = PhaseStatus::Error;
        st.phases[i].progress = st.phases[i].progress.max(100);
    }

    pub fn set_files(&self, total: u64, done: u64) {
        let mut st = self.state.lock().unwrap();
        st.files_total = total;
        st.files_done = done;
    }

    pub fn set_modules(&self, n: u64) {
        let mut st = self.state.lock().unwrap();
        st.modules = n;
    }

    pub fn set_tokens(&self, n: u64) {
        let mut st = self.state.lock().unwrap();
        st.tokens = n;
    }

    /// Hentikan thread render dan gambar panel ringkasan final (blok animasi
    /// diganti area statis; output berikutnya dicetak di bawahnya).
    pub fn finish(&mut self, ok: bool, errors: usize, warnings: usize) {
        if !self.active {
            return;
        }
        {
            let mut st = self.state.lock().unwrap();
            for p in st.phases.iter_mut() {
                if p.status == PhaseStatus::Running {
                    p.status = PhaseStatus::Done;
                    p.progress = 100;
                }
            }
            st.finished = true;
            st.ok = ok;
            st.errors = errors;
            st.warnings = warnings;
        }
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        render_summary(&self.state);
        let mut out = io::stdout();
        let _ = write!(out, "{}{}", SHOW_CURSOR, "\n");
        let _ = out.flush();
        self.active = false;
    }

    /// Abort karena error — panel menunjukkan fase Error dan batal.
    pub fn abort(&mut self, errors: usize, warnings: usize) {
        self.finish(false, errors, warnings);
    }
}

impl Drop for PipelineAnimator {
    fn drop(&mut self) {
        if self.active {
            // Jalur keluar tanpa finish() (error dini): bersihkan area animasi
            // agar tidak ada sisa baris waveform di terminal.
            self.stop.store(true, Ordering::Relaxed);
            if let Some(h) = self.handle.take() {
                let _ = h.join();
            }
            clear_area();
            let mut out = io::stdout();
            let _ = write!(out, "{}", SHOW_CURSOR);
            let _ = out.flush();
        }
    }
}

// ── Rendering ──

/// Render satu frame ke area tetap. Setiap frame: restore posisi awal lalu
/// tulis ulang AREA_LINES baris (dengan clear per baris). Output lain yang
/// ditulis thread utama muncul di bawah area dan tidak terganggu.
fn render_frame(state: &Arc<Mutex<AnimState>>, frame: u64) {
    let lines = build_lines(&state.lock().unwrap(), frame, false);
    let mut out = io::stdout();
    // Kembali ke posisi awal area.
    let _ = write!(out, "{}", RESTORE_CURSOR);
    for (i, line) in lines.iter().enumerate() {
        let suffix = if i + 1 < lines.len() { "\n" } else { "" };
        let _ = write!(out, "\r{}{}{}", CLEAR_LINE, line, suffix);
    }
    let _ = out.flush();
}

fn render_summary(state: &Arc<Mutex<AnimState>>) {
    let st = state.lock().unwrap();
    let lines = build_lines(&st, 0, true);
    let mut out = io::stdout();
    let _ = write!(out, "{}", RESTORE_CURSOR);
    for (i, line) in lines.iter().enumerate() {
        let suffix = if i + 1 < lines.len() { "\n" } else { "" };
        let _ = write!(out, "\r{}{}{}", CLEAR_LINE, line, suffix);
    }
    let _ = out.flush();
}

/// Bersihkan area animasi (hapus semua baris waveform) tanpa menggambar panel.
fn clear_area() {
    let mut out = io::stdout();
    let _ = write!(out, "{}", RESTORE_CURSOR);
    for _ in 0..AREA_LINES {
        let _ = write!(out, "\r{}\n", CLEAR_LINE);
    }
    let _ = out.flush();
}

fn build_lines(st: &AnimState, frame: u64, final_frame: bool) -> Vec<String> {
    let mut lines = Vec::with_capacity(AREA_LINES);
    lines.push(format!(
        "{}{}  Maria Hardware Activity Monitor  {}{}",
        CYAN, BOLD, RESET, DIM
    ));
    lines.push(format!("  ─────────────────────────────{}", RESET));
    lines.push(clock_line(frame));

    for (i, name) in PHASE_NAMES.iter().enumerate() {
        lines.push(phase_line(name, &st.phases[i]));
    }

    let files_s = format!("{}{}{}/{}{}", BOLD, st.files_done, RESET, GRAY, st.files_total);
    let mods_s = format!("{}{}{}", BOLD, st.modules, RESET);
    let toks_s = format!("{}{}{}", BOLD, fmt_count(st.tokens), RESET);
    let mem_s = format!("{}{}MB{}", BOLD, st.memory, RESET);
    let wrk_s = format!("{}{}{}", BOLD, st.workers, RESET);
    lines.push(format!(
        "  {}Files {}  {}Modules {}  {}Tokens {}  {}Mem {}  {}Workers {}{}",
        GRAY, files_s, GRAY, mods_s, GRAY, toks_s, GRAY, mem_s, GRAY, wrk_s, RESET,
    ));

    let elapsed = st.start.elapsed().as_secs_f64();
    let status = if final_frame {
        if st.ok {
            format!("{}{}✓ Compile Completed{}", GREEN, BOLD, RESET)
        } else {
            format!("{}{}✖ Compile Failed{}", RED, BOLD, RESET)
        }
    } else {
        format!("{}Elapsed {:.1}s{}", CYAN, elapsed, RESET)
    };
    lines.push(format!("  {}", status));

    if final_frame {
        let err_color = if st.errors > 0 { RED } else { GREEN };
        let warn_color = if st.warnings > 0 { YELLOW } else { GREEN };
        lines.push(format!(
            "  {}Errors {}{}{}  {}Warnings {}{}{}  {}Time {:.2}s{}",
            GRAY, err_color, st.errors, RESET,
            GRAY, warn_color, st.warnings, RESET,
            BOLD, elapsed, RESET,
        ));
    }
    lines
}

fn clock_line(frame: u64) -> String {
    const P: [&str; 4] = ["─", "▁", "─", "▔"];
    let mut s = format!("{}CLK {} ", DIM, RESET);
    let shift = (frame as usize) % 4;
    for i in 0..CLK_WIDTH {
        s.push_str(P[(i + shift) % 4]);
    }
    s.push(' ');
    s
}

fn phase_line(name: &str, info: &PhaseInfo) -> String {
    let filled = (info.progress as usize * BAR_WIDTH / 100).min(BAR_WIDTH);
    let mut bar = String::with_capacity(BAR_WIDTH);
    for i in 0..BAR_WIDTH {
        bar.push(if i < filled { '█' } else { '░' });
    }

    let (color, status_text) = match info.status {
        PhaseStatus::Idle => (GRAY, String::new()),
        PhaseStatus::Running => (BRIGHT_GREEN, format!("{}", info.progress)),
        PhaseStatus::Done => (GREEN, "✓".to_string()),
        PhaseStatus::Warn => (
            YELLOW,
            format!("▲ {} warn", info.warn_count),
        ),
        PhaseStatus::Error => (RED, "✖ error".to_string()),
    };

    let label = format!("{} ", name);
    format!(
        "{}{}{}{} {}{}{}",
        label, color, bar, RESET,
        color, BOLD, status_text,
    ) + RESET
}

fn fmt_count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// Baca RSS process (MB) dari /proc/self/statm. Fallback 0.
fn read_mem_mb() -> u64 {
    if let Ok(s) = std::fs::read_to_string("/proc/self/statm") {
        if let Some(resident) = s.split_whitespace().nth(1) {
            if let Ok(pages) = resident.parse::<u64>() {
                return pages * 4096 / (1024 * 1024);
            }
        }
    }
    0
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_phase_idx() {
        assert_eq!(Phase::Lex.idx(), 0);
        assert_eq!(Phase::Par.idx(), 1);
        assert_eq!(Phase::Ela.idx(), 2);
        assert_eq!(Phase::Opt.idx(), 3);
        assert_eq!(Phase::Ver.idx(), 4);
    }

    #[test]
    fn test_phase_line_running() {
        let mut info = PhaseInfo::default();
        info.status = PhaseStatus::Running;
        info.progress = 50;
        let line = phase_line("LEX", &info);
        assert!(line.starts_with("LEX "));
        assert!(line.contains('█'));
        assert!(line.contains('░'));
    }

    #[test]
    fn test_phase_line_done() {
        let mut info = PhaseInfo::default();
        info.status = PhaseStatus::Done;
        info.progress = 100;
        let line = phase_line("PAR", &info);
        assert!(line.contains('✓'));
        assert!(!line.contains('░'));
    }

    #[test]
    fn test_clock_line() {
        let l1 = clock_line(0);
        let l2 = clock_line(1);
        assert!(l1.contains("CLK"), "clock line harus memuat label CLK: {:?}", l1);
        assert!(l1.contains('▁') || l1.contains('▔'), "clock harus memuat bentuk gelombang");
        assert_ne!(l1, l2, "clock harus bergeser tiap frame");
    }

    #[test]
    fn test_fmt_count() {
        assert_eq!(fmt_count(0), "0");
        assert_eq!(fmt_count(999), "999");
        assert_eq!(fmt_count(1_500), "1.5K");
        assert_eq!(fmt_count(6_700_000), "6.7M");
    }

    #[test]
    fn test_read_mem_mb_ok() {
        // read_mem_mb tidak boleh panic (fallback 0).
        let _ = read_mem_mb();
    }

    #[test]
    fn test_animator_not_tty() {
        // Dalam test, stdout bukan TTY → start(false) harus None.
        assert!(PipelineAnimator::start(false).is_none());
    }
}
