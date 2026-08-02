//! Wizard "Generate Module" / "Create Interface" — buat skeleton RTL valid
//! dalam beberapa klik (sesuai desain Command Palette Maria). Pengguna mengisi
//! nama, parameter, dan port; source SystemVerilog digenerate otomatis dengan
//! pratinjau live, lalu ditulis ke project dan dibuka di editor.
//!
//! Generator source (`gen_module_source` / `gen_interface_source`) adalah
//! fungsi murni — mudah di-unit-test dan bisa dipakai ulang dari mana pun.

use eframe::egui;
use std::path::PathBuf;

use super::super::state::{GenKind, GenParam, GenPort, GuiState};

/// Render wizard (dipanggil dari App::ui bila `state.gen_open`).
/// Window di-bind ke `state.gen_open` (`Window::open`) — tombol ✕ di title
/// bar berfungsi, dan `create`/`cancel` cukup mengeset flag (window tutup
/// otomatis oleh egui saat `gen_open` false).
pub fn show(ui: &mut egui::Ui, state: &mut GuiState) {
    let ctx = ui.ctx().clone();
    let mut create = false;
    let mut cancel = false;

    // Pratinjau dihitung SEBELUM window: `.open(&mut state.gen_open)` meminjam
    // field `gen_open` selama window hidup, jadi memanggil `gen_source(state)`
    // (borrow seluruh `*state`) di dalam closure ditolak (E0500). Pratinjau
    // dari state frame ini dirender frame berikutnya — delay 1 frame tidak
    // terlihat (60 FPS).
    let preview = gen_source(state);

    egui::Window::new("Generate RTL")
        .id(egui::Id::new("gen_wizard"))
        .open(&mut state.gen_open)
        .collapsible(false)
        .resizable(true)
        .default_width(640.0)
        .default_height(560.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(&ctx, |ui| {
            // ── Jenis + nama ──
            ui.horizontal(|ui| {
                ui.label("Jenis:");
                ui.selectable_value(&mut state.gen_kind, GenKind::Module, "Module");
                ui.selectable_value(&mut state.gen_kind, GenKind::Interface, "Interface");
                ui.separator();
                ui.label("Nama:");
                ui.add(
                    egui::TextEdit::singleline(&mut state.gen_name)
                        .hint_text("mis. cache_controller")
                        .desired_width(240.0)
                        .font(egui::FontSelection::FontId(egui::FontId::monospace(
                            super::super::semantic::FONT_SIZE,
                        ))),
                );
            });
            ui.add_space(8.0);

            // ── Parameters ──
            ui.label(egui::RichText::new("Parameters").strong().size(11.0));
            let mut remove_param: Option<usize> = None;
            for (i, p) in state.gen_params.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut p.name)
                            .hint_text("Nama")
                            .desired_width(110.0)
                            .font(egui::FontSelection::FontId(egui::FontId::monospace(
                                super::super::semantic::FONT_SIZE,
                            ))),
                    );
                    ui.add(
                        egui::TextEdit::singleline(&mut p.ty)
                            .hint_text("tipe")
                            .desired_width(80.0)
                            .font(egui::FontSelection::FontId(egui::FontId::monospace(
                                super::super::semantic::FONT_SIZE,
                            ))),
                    );
                    ui.add(
                        egui::TextEdit::singleline(&mut p.default)
                            .hint_text("default")
                            .desired_width(80.0)
                            .font(egui::FontSelection::FontId(egui::FontId::monospace(
                                super::super::semantic::FONT_SIZE,
                            ))),
                    );
                    if ui.small_button("✕").on_hover_text("Hapus parameter").clicked() {
                        remove_param = Some(i);
                    }
                });
            }
            if let Some(i) = remove_param {
                state.gen_params.remove(i);
            }
            if ui.small_button("+ Parameter").clicked() {
                state.gen_params.push(GenParam::default());
            }
            ui.add_space(8.0);

            // ── Ports ──
            ui.label(egui::RichText::new("Ports").strong().size(11.0));
            let mut remove_port: Option<usize> = None;
            for (i, p) in state.gen_ports.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt(("gen_dir", i))
                        .selected_text(p.dir.clone())
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut p.dir, "input".to_string(), "input");
                            ui.selectable_value(&mut p.dir, "output".to_string(), "output");
                            ui.selectable_value(&mut p.dir, "inout".to_string(), "inout");
                        });
                    ui.add(
                        egui::TextEdit::singleline(&mut p.name)
                            .hint_text("Nama port")
                            .desired_width(140.0)
                            .font(egui::FontSelection::FontId(egui::FontId::monospace(
                                super::super::semantic::FONT_SIZE,
                            ))),
                    );
                    ui.add(
                        egui::TextEdit::singleline(&mut p.range)
                            .hint_text("range (kosong = 1 bit)")
                            .desired_width(150.0)
                            .font(egui::FontSelection::FontId(egui::FontId::monospace(
                                super::super::semantic::FONT_SIZE,
                            ))),
                    );
                    if ui.small_button("✕").on_hover_text("Hapus port").clicked() {
                        remove_port = Some(i);
                    }
                });
            }
            if let Some(i) = remove_port {
                state.gen_ports.remove(i);
            }
            if ui.small_button("+ Port").clicked() {
                state.gen_ports.push(GenPort::default());
            }
            ui.add_space(4.0);

            // clk/rst hanya relevan untuk module (interface tidak pakai).
            if state.gen_kind == GenKind::Module {
                ui.checkbox(&mut state.gen_clk_rst, "Tambahkan clk & rst_n otomatis");
                ui.add_space(8.0);
            }

            // ── Error (bila ada) — tidak memblokir pengeditan ──
            if !state.gen_error.is_empty() {
                ui.label(
                    egui::RichText::new(format!("⚠ {}", state.gen_error))
                        .color(egui::Color32::from_rgb(239, 68, 68))
                        .size(11.0),
                );
                ui.add_space(6.0);
            }

            // ── Pratinjau live ──
            ui.label(egui::RichText::new("Pratinjau").strong().size(11.0));
            egui::ScrollArea::vertical()
                .id_salt("genwiz_preview_scroll")
                .max_height(220.0)
                .show(ui, |ui| {
                    let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, _: f32| {
                        let job = super::super::semantic::highlight(buf.as_str());
                        ui.ctx().fonts_mut(|f| f.layout_job(job))
                    };
                    let mut buf = preview.clone();
                    ui.add(
                        egui::TextEdit::multiline(&mut buf)
                            .id_source("genwiz_preview")
                            .font(egui::FontSelection::FontId(egui::FontId::monospace(
                                super::super::semantic::FONT_SIZE,
                            )))
                            .interactive(false)
                            .desired_width(f32::INFINITY)
                            .desired_rows(12)
                            .layouter(&mut layouter),
                    );
                });

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui
                    .button(egui::RichText::new("✨ Buat & Buka").strong())
                    .on_hover_text("Tulis file .sv ke project & buka di editor")
                    .clicked()
                {
                    create = true;
                }
                if ui.button("Batal").clicked() {
                    cancel = true;
                }
            });
        });

    if create {
        create_file(state);
    }
    if cancel {
        state.gen_open = false;
        state.gen_error.clear();
    }
}

/// Generate source dari state wizard saat ini (pratinjau & file).
fn gen_source(state: &GuiState) -> String {
    match state.gen_kind {
        GenKind::Module => gen_module_source(
            &state.gen_name,
            &state.gen_params,
            &state.gen_ports,
            state.gen_clk_rst,
        ),
        GenKind::Interface => {
            gen_interface_source(&state.gen_name, &state.gen_params, &state.gen_ports)
        }
    }
}

/// Tulis file .sv hasil generate ke project root & buka di editor. Error
/// (nama kosong / gagal menulis) dilaporkan via `state.gen_error` — dialog
/// tetap terbuka sehingga pengguna bisa memperbaiki tanpa kehilangan input.
fn create_file(state: &mut GuiState) {
    // Nama file memakai clean_name yang SAMA dengan nama module di source —
    // kalau tidak, "cache controller" menghasilkan file "cache controller.sv"
    // berisi module "cache_controller" (mismatch nama file vs module).
    let name = clean_name(&state.gen_name);
    if state.gen_name.trim().is_empty() {
        state.gen_error = "Nama entitas kosong".to_string();
        return;
    }
    let src = gen_source(state);
    let root = state
        .project_root
        .clone()
        .unwrap_or_else(|| PathBuf::from("."));
    let path = root.join(format!("{}.sv", name));
    match std::fs::write(&path, &src) {
        Ok(()) => {
            let label = match state.gen_kind {
                GenKind::Module => "Module",
                GenKind::Interface => "Interface",
            };
            state.gen_error.clear();
            state.gen_open = false;
            state.open_file(path.clone());
            state.log(format!("✨ {} '{}' dibuat → {}", label, name, path.display()));
        }
        Err(e) => {
            state.gen_error = format!("Gagal menulis {}: {}", path.display(), e);
        }
    }
}

// ───────────────────────────── Generator source ─────────────────────────────

/// Skeleton module SystemVerilog: parameter, port (clk/rst otomatis bila
/// `clk_rst`), always_comb + always_ff kosong yang siap diisi.
pub fn gen_module_source(
    name: &str,
    params: &[GenParam],
    ports: &[GenPort],
    clk_rst: bool,
) -> String {
    let name = clean_name(name);
    let mut s = String::new();
    s.push_str(&format!("// ── Module: {} ──\n", name));
    s.push_str("// Generated by Maria — RTL Engineering Control Center\n\n");

    // Header parameter (atau port langsung bila tidak ada parameter).
    if params.is_empty() {
        s.push_str(&format!("module {} (\n", name));
    } else {
        s.push_str(&format!("module {} #(\n", name));
        for (i, p) in params.iter().enumerate() {
            let ty = if p.ty.is_empty() { "int" } else { p.ty.as_str() };
            let comma = if i + 1 == params.len() { "" } else { "," };
            let default = if p.default.is_empty() {
                String::new()
            } else {
                format!(" = {}", p.default)
            };
            s.push_str(&format!(
                "    parameter {} {}{}{}\n",
                ty, p.name, default, comma
            ));
        }
        s.push_str(") (\n");
    }

    // Ports: clk/rst otomatis di posisi pertama.
    let mut all: Vec<GenPort> = ports.to_vec();
    if clk_rst {
        all.insert(
            0,
            GenPort {
                dir: "input".into(),
                name: "clk".into(),
                range: String::new(),
            },
        );
        all.insert(
            1,
            GenPort {
                dir: "input".into(),
                name: "rst_n".into(),
                range: String::new(),
            },
        );
    }

    // Rata kiri kolom nama: max lebar "dir + tipe/range".
    let rendered: Vec<(String, String)> = all
        .iter()
        .map(|p| {
            let tr = if p.range.is_empty() {
                "logic".to_string()
            } else {
                format!("logic [{}]", p.range)
            };
            (format!("{} {}", p.dir, tr), p.name.clone())
        })
        .collect();
    let max_pre = rendered
        .iter()
        .map(|(pre, _)| pre.len())
        .max()
        .unwrap_or(0);

    for (i, (pre, nm)) in rendered.iter().enumerate() {
        let comma = if i + 1 == rendered.len() { "" } else { "," };
        let pad = " ".repeat(max_pre.saturating_sub(pre.len()));
        s.push_str(&format!("    {} {} {}{}\n", pre, pad, nm, comma));
    }
    s.push_str(");\n\n");

    // Body.
    s.push_str("    // ── Internal signals ──\n\n");
    s.push_str("    // ── Kombinasional ──\n");
    s.push_str("    always_comb begin\n");
    s.push_str("    end\n\n");
    if clk_rst {
        s.push_str("    // ── Sekuensial ──\n");
        s.push_str("    always_ff @(posedge clk or negedge rst_n) begin\n");
        s.push_str("        if (!rst_n) begin\n");
        s.push_str("        end else begin\n");
        s.push_str("        end\n");
        s.push_str("    end\n\n");
    }
    s.push_str(&format!("endmodule : {}\n", name));
    s
}

/// Skeleton interface SystemVerilog: parameter, port, modport master/slave
/// (input/output otomatis dari daftar port) siap diisi.
pub fn gen_interface_source(name: &str, params: &[GenParam], ports: &[GenPort]) -> String {
    let name = clean_name(name);
    let mut s = String::new();
    s.push_str(&format!("// ── Interface: {} ──\n", name));
    s.push_str("// Generated by Maria — RTL Engineering Control Center\n\n");

    if params.is_empty() {
        s.push_str(&format!("interface {} (\n", name));
    } else {
        s.push_str(&format!("interface {} #(\n", name));
        for (i, p) in params.iter().enumerate() {
            let ty = if p.ty.is_empty() { "int" } else { p.ty.as_str() };
            let comma = if i + 1 == params.len() { "" } else { "," };
            let default = if p.default.is_empty() {
                String::new()
            } else {
                format!(" = {}", p.default)
            };
            s.push_str(&format!(
                "    parameter {} {}{}{}\n",
                ty, p.name, default, comma
            ));
        }
        s.push_str(") (\n");
    }

    let rendered: Vec<(String, String)> = ports
        .iter()
        .map(|p| {
            let tr = if p.range.is_empty() {
                "logic".to_string()
            } else {
                format!("logic [{}]", p.range)
            };
            (format!("{} {}", p.dir, tr), p.name.clone())
        })
        .collect();
    let max_pre = rendered
        .iter()
        .map(|(pre, _)| pre.len())
        .max()
        .unwrap_or(0);
    for (i, (pre, nm)) in rendered.iter().enumerate() {
        let comma = if i + 1 == rendered.len() { "" } else { "," };
        let pad = " ".repeat(max_pre.saturating_sub(pre.len()));
        s.push_str(&format!("    {} {} {}{}\n", pre, pad, nm, comma));
    }
    s.push_str(");\n\n");

    // Modports: master = input dari perspektif initiator; slave = kebalikan.
    let ins: Vec<String> = ports
        .iter()
        .filter(|p| p.dir == "input")
        .map(|p| p.name.clone())
        .collect();
    let outs: Vec<String> = ports
        .iter()
        .filter(|p| p.dir == "output")
        .map(|p| p.name.clone())
        .collect();
    s.push_str("    // ── Modport master ──\n");
    s.push_str("    modport master (\n");
    for (i, n) in ins.iter().enumerate() {
        let comma = if i + 1 == ins.len() && outs.is_empty() {
            ""
        } else {
            ","
        };
        s.push_str(&format!("        input {}{}\n", n, comma));
    }
    for (i, n) in outs.iter().enumerate() {
        let comma = if i + 1 == outs.len() { "" } else { "," };
        s.push_str(&format!("        output {}{}\n", n, comma));
    }
    s.push_str("    );\n\n");
    s.push_str("    // ── Modport slave ──\n");
    s.push_str("    modport slave (\n");
    for (i, n) in outs.iter().enumerate() {
        let comma = if i + 1 == outs.len() && ins.is_empty() {
            ""
        } else {
            ","
        };
        s.push_str(&format!("        input {}{}\n", n, comma));
    }
    for (i, n) in ins.iter().enumerate() {
        let comma = if i + 1 == ins.len() { "" } else { "," };
        s.push_str(&format!("        output {}{}\n", n, comma));
    }
    s.push_str("    );\n\n");

    s.push_str(&format!("endinterface : {}\n", name));
    s
}

/// Bersihkan nama entitas (trim + hanya `[A-Za-z0-9_]`; sisanya jadi `_`).
fn clean_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        out.push_str("rtl_unit");
    }
    out
}

// ───────────────────────────── Unit tests ─────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::state::{GenKind, GenParam, GenPort, GuiState};

    fn param(name: &str, ty: &str, default: &str) -> GenParam {
        GenParam {
            name: name.into(),
            ty: ty.into(),
            default: default.into(),
        }
    }

    fn port(dir: &str, name: &str, range: &str) -> GenPort {
        GenPort {
            dir: dir.into(),
            name: name.into(),
            range: range.into(),
        }
    }

    /// Helper: cari baris yang mengandung semua token (urutan bebas spasi) —
    /// generator merata-kiri kolom port dengan padding spasi, jadi assertion
    /// `contains` dengan 1 spasi pasti gagal. Normalisasi whitespace per baris.
    fn has_line_tokens(src: &str, tokens: &[&str]) -> bool {
        src.lines().any(|l| {
            let ws: Vec<&str> = l.split_whitespace().collect();
            tokens.iter().all(|t| ws.contains(t))
        })
    }

    #[test]
    fn module_source_has_decl_params_and_ports() {
        let src = gen_module_source(
            "cache_ctrl",
            &[param("WIDTH", "int", "32")],
            &[port("input", "data_in", "WIDTH-1:0"), port("output", "data_out", "WIDTH-1:0")],
            false,
        );
        assert!(src.contains("module cache_ctrl #("), "header parameter:\n{}", src);
        assert!(src.contains("parameter int WIDTH = 32"), "parameter:\n{}", src);
        assert!(has_line_tokens(&src, &["input", "logic", "[WIDTH-1:0]", "data_in,"]), "input port:\n{}", src);
        assert!(has_line_tokens(&src, &["output", "logic", "[WIDTH-1:0]", "data_out"]), "output port:\n{}", src);
        assert!(src.contains("endmodule : cache_ctrl"), "footer:\n{}", src);
    }

    #[test]
    fn module_clk_rst_inserted_first() {
        let src = gen_module_source("top", &[], &[port("output", "done", "")], true);
        assert!(src.contains("module top ("), "tanpa parameter:\n{}", src);
        assert!(has_line_tokens(&src, &["input", "logic", "clk,"]), "clk otomatis:\n{}", src);
        assert!(has_line_tokens(&src, &["input", "logic", "rst_n,"]), "rst_n otomatis:\n{}", src);
        // clk harus mendahului port user.
        let clk_pos = src.find("clk").unwrap();
        let done_pos = src.find("done").unwrap();
        assert!(clk_pos < done_pos, "clk sebelum done:\n{}", src);
    }

    #[test]
    fn interface_source_has_modports() {
        let src = gen_interface_source(
            "bus_if",
            &[param("WIDTH", "int", "32")],
            &[port("input", "data_in", "WIDTH-1:0"), port("output", "data_out", "WIDTH-1:0")],
        );
        assert!(src.contains("interface bus_if #("), "header:\n{}", src);
        assert!(has_line_tokens(&src, &["modport", "master", "("]), "modport master:\n{}", src);
        assert!(has_line_tokens(&src, &["modport", "slave", "("]), "modport slave:\n{}", src);
        assert!(src.contains("endinterface : bus_if"), "footer:\n{}", src);
    }

    #[test]
    fn clean_name_sanitizes() {
        assert_eq!(clean_name("cache controller"), "cache_controller");
        assert_eq!(clean_name("  top  "), "top");
        assert_eq!(clean_name(""), "rtl_unit");
        assert_eq!(clean_name("a-b/c"), "a_b_c");
    }

    #[test]
    fn gen_source_matches_kind() {
        // state.gen_kind = Module → gen_module_source dipanggil.
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut st = GuiState::new(tx, _rx);
        st.gen_kind = GenKind::Interface;
        st.gen_name = "my_if".into();
        let src = gen_source(&st);
        assert!(src.contains("interface my_if"), "interface dipilih:\n{}", src);
    }
}
