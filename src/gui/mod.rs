//! GUI native (egui) — Engineering Control Center untuk RTL.
//!
//! Menggantikan frontend Tauri (React/TS). Tidak ada lapisan IPC:
//! GUI memanggil API library `maria` langsung dari proses yang sama.
//!
//! Filosofi desain: Engineering Dashboard + IDE + Observatory.
//! Tenang, sedikit warna, banyak informasi.

pub mod app;
pub mod backend;
pub mod panels;
pub mod state;
pub mod sv_syntax;

pub use app::MariaApp;

/// Jalankan aplikasi GUI (blocking sampai window ditutup).
pub fn run() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Maria — RTL Engineering Control Center")
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([1024.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "maria",
        options,
        Box::new(|cc| Ok(Box::new(MariaApp::new(cc)))),
    )
}
