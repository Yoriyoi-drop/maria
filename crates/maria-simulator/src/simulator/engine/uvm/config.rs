//! uvm_config_db — path matching & lookup (F19).
//! UVM `set`/`get` memakai inst_path dengan wildcard (`*.agent`,
//! `uvm_test_top.*`, `?` satu karakter). Lookup: exact match menang, lalu
//! wildcard pattern PALING SPESIFIK (paling sedikit wildcard; tie-break
//! prefix literal terpanjang, lalu pattern terpanjang).
//! 1 file = 1 tanggung jawab: hanya path matching + lookup config db.

use super::super::SimulationEngine;
use maria_compiler::hir::{LogicVec, ObjId};
use crate::simulator::types::*;
use crate::simulator::util::*;

/// Wildcard match — `*` match zero-or-more karakter (termasuk `.`), `?`
/// match satu karakter. DP iteratif dua baris (space O(len inst_path)).
pub(crate) fn config_db_path_match(pattern: &str, inst_path: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let s: Vec<char> = inst_path.chars().collect();
    // dp[j] = apakah p[..i] match s[..j] (i = pattern yang sudah diproses)
    let mut dp = vec![false; s.len() + 1];
    dp[0] = true;
    for &pc in &p {
        let mut next = vec![false; s.len() + 1];
        match pc {
            '*' => {
                // '*' match zero+ chars: next[j] = dp[j] (nol) || next[j-1] (satu+)
                let mut run = false;
                for j in 0..=s.len() {
                    run = run || dp[j];
                    next[j] = run;
                }
            }
            '?' => {
                for j in 1..=s.len() {
                    next[j] = dp[j - 1];
                }
            }
            c => {
                for j in 1..=s.len() {
                    next[j] = dp[j - 1] && s[j - 1] == c;
                }
            }
        }
        dp = next;
    }
    dp[s.len()]
}

/// Spesifisitas pattern — LEBIH KECIL = LEBIH spesifik:
/// (jumlah wildcard, negasi panjang prefix literal, negasi panjang pattern).
/// Exact match (`wildcards == 0`, prefix = len) selalu menang.
fn pattern_specificity(pattern: &str) -> (usize, usize, usize) {
    let wildcards = pattern.chars().filter(|c| *c == '*' || *c == '?').count();
    let literal_prefix = pattern.find(['*', '?']).unwrap_or(pattern.len());
    let len = pattern.len();
    (wildcards, usize::MAX - literal_prefix, usize::MAX - len)
}

impl SimulationEngine {
    /// Cari nilai config db: exact match dulu, lalu wildcard paling spesifik.
    pub(crate) fn config_db_find(
        &self,
        inst_name: &str,
        field_name: &str,
    ) -> Option<LogicVec> {
        // 1) Exact match — kemenangan penuh.
        if let Some(v) = self
            .uvm_config_db_data
            .get(&(inst_name.to_string(), field_name.to_string()))
        {
            return Some(v.clone());
        }
        // 2) Wildcard — pilih pattern paling spesifik yang match.
        let mut best: Option<((usize, usize, usize), LogicVec)> = None;
        for ((pat, f), v) in &self.uvm_config_db_data {
            if f != field_name {
                continue;
            }
            if config_db_path_match(pat, inst_name) {
                let sp = pattern_specificity(pat);
                let better = match &best {
                    None => true,
                    Some((bsp, _)) => sp < *bsp,
                };
                if better {
                    best = Some((sp, v.clone()));
                }
            }
        }
        best.map(|(_, v)| v)
    }

    /// Path hierarki penuh objek UVM (`uvm_test_top.env.agent`), dipakai
    /// `get` dengan inst_path kosong — `uvm_config_db::get(this, "", ...)`.
    /// Fallback ke `current_instance_path` (top module) bila objek tak punya
    /// nama hierarki.
    pub(crate) fn uvm_object_full_path(&self, obj_id: ObjId) -> String {
        let mut names = Vec::new();
        let mut current = Some(obj_id);
        while let Some(id) = current {
            let n = self
                .uvm_object_data
                .get(&id)
                .map(|d| d.name.clone())
                .unwrap_or_default();
            names.push(n);
            current = self.uvm_component_data.get(&id).and_then(|d| d.parent);
        }
        names.reverse();
        let full = names.join(".");
        if full.is_empty() {
            self.current_instance_path.clone().unwrap_or_default()
        } else {
            full
        }
    }
}
