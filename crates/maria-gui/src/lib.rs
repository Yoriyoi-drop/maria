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
pub mod resource;
pub mod semantic;
pub mod splitter;
pub mod state;
pub mod workspace;

pub use app::MariaApp;

/// Jalankan aplikasi GUI (blocking sampai window ditutup).
///
/// Renderer: wgpu (Vulkan/DX12/Metal) dipilih eksplisit untuk rendering GPU
/// yang tetap 60-144 FPS saat CPU sibuk mengompilasi. Build juga mengaktifkan
/// feature `glow` (OpenGL) sebagai fallback: jika inisialisasi wgpu gagal
/// (GPU/driver bermasalah, tidak ada Vulkan, dll), `run()` otomatis mencoba
/// lagi dengan renderer Glow sebelum menyerah.
/// Buat app creator untuk eframe (cocok dengan `AppCreator`).
/// `MariaApp::new` hanya membuat channel + GuiState (tanpa efek samping berat),
/// sehingga aman dipanggil ulang saat fallback renderer.
fn create_app(
    cc: &eframe::CreationContext<'_>,
) -> Result<Box<dyn eframe::App>, Box<dyn std::error::Error + Send + Sync>> {
    Ok(Box::new(MariaApp::new(cc)))
}

/// Jalankan aplikasi GUI (blocking sampai window ditutup).
///
/// Renderer: wgpu (Vulkan/DX12/Metal) dipilih eksplisit untuk rendering GPU
/// yang tetap 60-144 FPS saat CPU sibuk mengompilasi. Build juga mengaktifkan
/// feature `glow` (OpenGL) sebagai fallback: jika inisialisasi wgpu gagal
/// (GPU/driver bermasalah, tidak ada Vulkan, dll), `run()` otomatis mencoba
/// lagi dengan renderer Glow sebelum menyerah.
pub fn run() -> eframe::Result<()> {
    let mk_options = |renderer| eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Maria — RTL Engineering Control Center")
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([1024.0, 600.0]),
        renderer,
        ..Default::default()
    };

    // Pass 1: wgpu (renderer utama — GPU rendering untuk 60-144 FPS)
    match eframe::run_native(
        "maria",
        mk_options(eframe::Renderer::Wgpu),
        Box::new(create_app),
    ) {
        Ok(()) => Ok(()),
        Err(wgpu_err) => {
            eprintln!(
                "⚠ wgpu gagal inisialisasi ({}), fallback ke OpenGL (glow)...",
                wgpu_err
            );
            // Pass 2: glow — fallback otomatis bila GPU/driver wgpu bermasalah.
            eframe::run_native(
                "maria",
                mk_options(eframe::Renderer::Glow),
                Box::new(create_app),
            )
        }
    }
}
