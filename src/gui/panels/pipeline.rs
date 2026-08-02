//! Pipeline tab — Compile Pipeline (timing per tahap) + Incremental Cache
//! (statistik MICD). Ini nilai jual Maria: pengguna bisa melihat tahap mana
//! yang lambat (lexer/parser/elaborator) dan seberapa efektif cache
//! incremental compile (berapa AST di-restore vs berapa file berubah).

use eframe::egui;

use super::super::state::{GuiState, MicdInfo, PipelineStage};

const BAR_W: f32 = 240.0;
const BAR_H: f32 = 14.0;

pub fn show(ui: &mut egui::Ui, state: &mut GuiState) {
    let Some(info) = state.compile_info.as_ref() else {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("Compile project untuk melihat pipeline & statistik cache")
                .weak()
                .italics(),
        );
        return;
    };

    // ── Compile Pipeline ──
    ui.label(egui::RichText::new("Compile Pipeline").strong().size(12.0));
    ui.add_space(4.0);
    let max_ms = info
        .pipeline
        .iter()
        .map(|s| s.ms)
        .fold(1u64, u64::max)
        .max(1);
    for stage in &info.pipeline {
        stage_row(ui, stage, max_ms);
    }
    ui.label(
        egui::RichText::new(format!(
            "Total: {:.2} ms · {} module · {} cached / {} processed",
            info.total_time_ms,
            info.modules.len(),
            info.cached_files,
            info.processed_files,
        ))
        .weak()
        .size(11.0),
    );

    ui.add_space(12.0);
    ui.separator();
    ui.add_space(4.0);

    // ── Incremental Cache (MICD) ──
    ui.label(egui::RichText::new("Incremental Cache (MICD)").strong().size(12.0));
    ui.add_space(4.0);
    match &info.micd {
        Some(m) => cache_view(ui, m),
        None => {
            ui.label(
                egui::RichText::new(
                    "Database MICD belum tersedia — buka project (.maria/database) lalu compile",
                )
                .weak()
                .italics(),
            );
        }
    }
}

/// Satu baris tahap pipeline: nama + bar proporsional + status/waktu.
fn stage_row(ui: &mut egui::Ui, s: &PipelineStage, max_ms: u64) {
    ui.horizontal(|ui| {
        ui.add_sized(
            [130.0, 18.0],
            egui::Label::new(egui::RichText::new(&s.name).size(11.0)),
        );
        let (rect, _) = ui.allocate_exact_size(egui::vec2(BAR_W, BAR_H), egui::Sense::hover());
        let bg = ui.visuals().widgets.noninteractive.bg_fill;
        ui.painter().rect_filled(rect, egui::CornerRadius::ZERO, bg);
        let frac = (s.ms as f32 / max_ms as f32).clamp(0.0, 1.0);
        if frac > 0.0 {
            let fill_w = (rect.width() * frac).max(2.0);
            let color = match s.status.as_str() {
                "ok" => ui.visuals().selection.bg_fill,
                _ => egui::Color32::from_rgb(148, 163, 184),
            };
            ui.painter().rect_filled(
                egui::Rect::from_min_size(rect.min, egui::vec2(fill_w, rect.height())),
                egui::CornerRadius::ZERO,
                color,
            );
        }
        let text = match s.status.as_str() {
            "ok" if s.ms > 0 => format!("✓ {:.1} ms", s.ms),
            "ok" => "✓".to_string(),
            "waiting" => "…".to_string(),
            other => other.to_string(),
        };
        ui.label(egui::RichText::new(text).monospace().size(11.0));
    });
}

/// Tampilan statistik Incremental Cache (MICD): ringkasan + bar hit rate.
fn cache_view(ui: &mut egui::Ui, m: &MicdInfo) {
    // Hit rate = AST yang di-restore (tidak perlu di-parse ulang).
    let hit_rate = if m.files > 0 {
        m.restored_ast as f32 / m.files as f32
    } else {
        0.0
    };
    ui.horizontal(|ui| {
        ui.add_sized(
            [130.0, 18.0],
            egui::Label::new(egui::RichText::new("Cache hit").size(11.0)),
        );
        let (rect, _) = ui.allocate_exact_size(egui::vec2(BAR_W, BAR_H), egui::Sense::hover());
        let bg = ui.visuals().widgets.noninteractive.bg_fill;
        ui.painter().rect_filled(rect, egui::CornerRadius::ZERO, bg);
        let fill_w = (rect.width() * hit_rate).max(if hit_rate > 0.0 { 2.0 } else { 0.0 });
        ui.painter().rect_filled(
            egui::Rect::from_min_size(rect.min, egui::vec2(fill_w, rect.height())),
            egui::CornerRadius::ZERO,
            egui::Color32::from_rgb(34, 197, 94),
        );
        ui.label(
            egui::RichText::new(format!("{:.1}%", hit_rate * 100.0))
                .monospace()
                .size(11.0),
        );
    });

    ui.add_space(4.0);
    let rows: Vec<(&str, String)> = vec![
        ("Files registered", m.files.to_string()),
        ("AST restored", m.restored_ast.to_string()),
        ("Recompiled", m.changed_files.to_string()),
        ("Verify hits", m.verify_hits.to_string()),
        ("Verify misses", m.verify_misses.to_string()),
        ("Snapshots", m.snapshots.to_string()),
        ("DB size", format_bytes(m.db_bytes)),
    ];
    for (label, val) in &rows {
        ui.horizontal(|ui| {
            ui.add_sized(
                [130.0, 18.0],
                egui::Label::new(egui::RichText::new(*label).size(11.0)),
            );
            ui.label(egui::RichText::new(val).monospace().size(11.0));
        });
    }

    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(format!(
            "Restored {} dari {} file — file tak berubah tidak di-lex/di-parse ulang.",
            m.restored_ast, m.files
        ))
        .weak()
        .size(11.0),
    );
}

/// Format ukuran byte → B/KB/MB.
fn format_bytes(b: u64) -> String {
    if b >= 1024 * 1024 {
        format!("{:.1} MB", b as f64 / (1024.0 * 1024.0))
    } else if b >= 1024 {
        format!("{:.1} KB", b as f64 / 1024.0)
    } else {
        format!("{} B", b)
    }
}
