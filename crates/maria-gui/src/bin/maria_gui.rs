//! Binary GUI: `cargo run --features gui --bin maria-gui`
//!
//! Maria — RTL Engineering Control Center (native egui, pengganti Tauri).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> eframe::Result<()> {
    maria_gui::run()
}
