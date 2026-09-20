//! Binary GUI: `cargo run --features gui --bin mivon-gui`
//!
//! Mivon — RTL Engineering Control Center (native egui, pengganti Tauri).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> eframe::Result<()> {
    mivon_gui::run()
}
