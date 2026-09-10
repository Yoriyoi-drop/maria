//! Triage: dedup crash + render laporan (Taming Compiler Fuzzers-style).

use crate::{bugs_dir, reports_dir, CaseResult};
use std::collections::BTreeMap;

/// Grup bug yang di-dedup berdasarkan signature.
#[derive(Debug, Clone)]
pub struct BugGroup {
    pub signature: String,
    pub count: usize,
    pub representative: CaseResult,
}

/// Dedup daftar bug → grup, sorted count desc lalu sig asc.
pub fn triage(results: &[CaseResult]) -> Vec<BugGroup> {
    let mut by_sig: BTreeMap<String, Vec<&CaseResult>> = BTreeMap::new();
    for r in results {
        by_sig.entry(r.signature()).or_default().push(r);
    }

    let mut groups: Vec<BugGroup> = by_sig
        .into_iter()
        .map(|(sig, rs)| BugGroup {
            count: rs.len(),
            signature: sig.clone(),
            representative: rs[0].clone(),
        })
        .collect();

    groups.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.signature.cmp(&b.signature))
    });
    groups
}

/// Render laporan ringkas ke String.
pub fn render_report(results: &[CaseResult]) -> String {
    let groups = triage(results);
    let mut out = String::new();
    out.push_str(&format!("BUG GROUPS: {}\n", groups.len()));
    out.push_str(&format!("TOTAL BUGS: {}\n", results.len()));

    for g in &groups {
        let r = &g.representative;
        out.push_str(&format!(
            "\n[{}x] {} | {} | {}\n",
            g.count,
            g.signature,
            r.target.as_str(),
            r.oracle
        ));
        out.push_str(&format!("  detail: {}\n", r.detail.lines().next().unwrap_or("")));
        out.push_str(&format!(
            "  source ({:?} bytes): {}\n",
            r.source.len(),
            truncate_preview(&r.source, 200)
        ));
    }
    out
}

/// Simpan laporan timestamped ke fuzz/reports/.
pub fn save_report(results: &[CaseResult]) -> std::io::Result<std::path::PathBuf> {
    let dir = reports_dir();
    std::fs::create_dir_all(&dir)?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = dir.join(format!("report_{stamp}.txt"));
    std::fs::write(&path, render_report(results))?;
    Ok(path)
}

/// Tulis bug yang sudah di-minimize (reproduksi) ke bugs dir.
pub fn save_repro(name: &str, source: &str) -> std::io::Result<std::path::PathBuf> {
    let dir = bugs_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(name);
    std::fs::write(&path, source)?;
    Ok(path)
}

fn truncate_preview(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        return s.to_string();
    }
    let head: String = chars[..max].iter().collect();
    format!("{head}…[+{} chars]", chars.len() - max)
}