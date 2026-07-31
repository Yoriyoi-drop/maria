/// Coverage tracking and reporting for SimulationEngine.
/// Manages covergroup sampling, coverage reporting, and UCIS XML export.
use crate::diagnostics::DiagCode;
use crate::error::SimError;
use crate::ir::*;
use crate::simulator::util::*;
use crate::Symbol;
use std::collections::{HashMap, HashSet};

use super::SimulationEngine;

/// Check if a value matches a wildcard bin pattern (supports ? and * wildcards).
#[allow(dead_code)]
fn wildcard_match(value: u64, pattern: &str) -> bool {
    let val_str = format!("{}", value);
    let p = pattern.trim();

    let pat_chars: Vec<char> = p.chars().collect();
    let val_chars: Vec<char> = val_str.chars().collect();

    let vlen = val_chars.len();
    let plen = pat_chars.len();
    let mut dp = vec![vec![false; plen + 1]; vlen + 1];
    dp[0][0] = true;

    for j in 1..=plen {
        if pat_chars[j - 1] == '*' {
            dp[0][j] = dp[0][j - 1];
        }
    }

    for i in 1..=vlen {
        for j in 1..=plen {
            if pat_chars[j - 1] == '*' {
                dp[i][j] = dp[i - 1][j] || dp[i][j - 1];
            } else if pat_chars[j - 1] == '?' {
                dp[i][j] = dp[i - 1][j - 1];
            } else if pat_chars[j - 1] == val_chars[i - 1] {
                dp[i][j] = dp[i - 1][j - 1];
            }
        }
    }
    dp[vlen][plen]
}

impl SimulationEngine {
    // ─── Line Coverage ─────────────────────────────────────────────
    
    /// Record that a source line was executed.
    pub(crate) fn record_line_hit(&mut self, stmt: &IrStmt, process_name: &str) {
        if !self.coverage_enabled {
            return;
        }
        let key = Symbol::intern(&format!("{}.{:?}", process_name, std::mem::discriminant(stmt)));
        *self.cover_line.entry(key).or_insert(0) += 1;
    }

    /// Create a unique branch key for a conditional statement.
    fn branch_key(process_name: &str, branch_type: &str, idx: usize) -> Symbol {
        Symbol::intern(&format!("{}.{}#{}", process_name, branch_type, idx))
    }

    /// Record a branch being taken (branch coverage).
    pub(crate) fn record_branch_hit(
        &mut self,
        branch_key: Symbol,
        label: &str,
    ) {
        if !self.coverage_enabled {
            return;
        }
        let branches = self.cover_branches.entry(branch_key).or_default();
        *branches.entry(Symbol::intern(label)).or_insert(0) += 1;
    }

    // ─── Toggle Coverage ───────────────────────────────────────────

    /// Record a signal toggle (transition between logic values).
    pub(crate) fn record_toggle(&mut self, sig_id: usize, old_val: &LogicVec, new_val: &LogicVec) {
        if !self.coverage_enabled {
            return;
        }
        let toggles = self.cover_toggle.entry(sig_id).or_default();
        for i in 0..old_val.width.min(new_val.width).min(64) {
            let old_bit = old_val.bits.get(i).copied().unwrap_or(LogicVal::X);
            let new_bit = new_val.bits.get(i).copied().unwrap_or(LogicVal::X);
            if old_bit != new_bit {
                toggles.insert((old_bit, new_bit));
            }
        }
    }

    // ─── FSM Coverage ──────────────────────────────────────────────

    /// Record a signal value for FSM state analysis.
    pub(crate) fn record_fsm_value(&mut self, sig_id: usize, val: &LogicVec) {
        if !self.coverage_enabled {
            return;
        }
        let uval = val.to_u64();
        self.cover_fsm.entry(sig_id).or_default().insert(uval);
    }

    // ─── Reporting ─────────────────────────────────────────────────

    /// Print line coverage report.
    fn report_line_coverage(&self) {
        if self.cover_line.is_empty() {
            return;
        }
        eprintln!("\n=== Line Coverage ===");
        let mut sorted: Vec<(&str, u64)> = self.cover_line
            .iter()
            .map(|(k, v)| (k.as_str(), *v))
            .collect();
        sorted.sort_by_key(|a| std::cmp::Reverse(a.1));
        for (key, hits) in sorted.iter().take(20) {
            eprintln!("  {}: {} hits", key, hits);
        }
        eprintln!("  ({} total line items)", self.cover_line.len());
    }

    /// Print toggle coverage report.
    fn report_toggle_coverage(&self) {
        if self.cover_toggle.is_empty() {
            return;
        }
        eprintln!("\n=== Toggle Coverage ===");
        for (sig_id, toggles) in &self.cover_toggle {
            let sig_name = self.design.top.signals.get(*sig_id)
                .map(|s| s.name.as_str())
                .unwrap_or("<unknown>");
            eprintln!("  {}: {} transitions", sig_name, toggles.len());
            for (from, to) in toggles.iter() {
                eprintln!("    {:?}→{:?}", from, to);
            }
        }
        let total_signals = self.design.top.signals.len();
        eprintln!("  {} signals with toggles / {} total", self.cover_toggle.len(), total_signals);
    }

    /// Print branch coverage report.
    fn report_branch_coverage(&self) {
        if self.cover_branches.is_empty() {
            return;
        }
        eprintln!("\n=== Branch Coverage ===");
        for (key, branches) in &self.cover_branches {
            eprintln!("  {}:", key.as_str());
            let total: u64 = branches.values().sum();
            for (label, count) in branches {
                eprintln!("    {}: {} hits ({:.1}%)", label.as_str(), count, 
                    if total > 0 { *count as f64 / total as f64 * 100.0 } else { 0.0 });
            }
        }
    }

    /// Print FSM coverage report.
    fn report_fsm_coverage(&self) {
        if self.cover_fsm.is_empty() {
            return;
        }
        eprintln!("\n=== FSM Coverage ===");
        for (sig_id, states) in &self.cover_fsm {
            let sig_name = self.design.top.signals.get(*sig_id)
                .map(|s| s.name.as_str())
                .unwrap_or("<unknown>");
            let mut sorted_states: Vec<u64> = states.iter().copied().collect();
            sorted_states.sort();
            eprintln!("  {}: {} states visited: {:?}", sig_name, states.len(), sorted_states);
        }
    }

    /// Print combined coverage report.
    pub(crate) fn report_full_coverage(&self) {
        if !self.coverage_enabled {
            return;
        }
        eprintln!("\n═══════════════════════════════════════");
        eprintln!("   COVERAGE SUMMARY REPORT");
        eprintln!("═══════════════════════════════════════");
        
        // Line coverage percentage
        if !self.cover_line.is_empty() {
            let total_line_hits: u64 = self.cover_line.values().sum();
            eprintln!("  Line:     {} unique items, {} total hits", self.cover_line.len(), total_line_hits);
        } else {
            eprintln!("  Line:     (no line data)");
        }
        
        // Toggle coverage percentage
        if !self.cover_toggle.is_empty() {
            let mut total_signal_bits = 0usize;
            let mut covered_bits = 0usize;
            for toggles in self.cover_toggle.values() {
                let n_transitions = toggles.len();
                // Each bit can transition 0→1, 1→0, 0→X, X→0, etc (max 12 possible transitions per bit)
                covered_bits += n_transitions;
                total_signal_bits += toggles.len().max(1);
            }
            let pct = if total_signal_bits > 0 {
                (covered_bits as f64 / total_signal_bits as f64) * 100.0
            } else {
                0.0
            };
            eprintln!("  Toggle:   {} signals tracked, ~{:.1}% coverage", self.cover_toggle.len(), pct);
        } else {
            eprintln!("  Toggle:   (no toggle data)");
        }
        
        // Branch coverage percentage
        if !self.cover_branches.is_empty() {
            let mut total_branches = 0u64;
            let mut covered_branches = 0u64;
            for branches in self.cover_branches.values() {
                for count in branches.values() {
                    total_branches += 1;
                    if *count > 0 {
                        covered_branches += 1;
                    }
                }
            }
            let pct = if total_branches > 0 {
                (covered_branches as f64 / total_branches as f64) * 100.0
            } else {
                0.0
            };
            eprintln!("  Branch:   {}/{} branches covered ({:.1}%)", covered_branches, total_branches, pct);
        } else {
            eprintln!("  Branch:   (no branch data)");
        }
        
        // FSM coverage percentage
        if !self.cover_fsm.is_empty() {
            let mut total_states = 0usize;
            for states in self.cover_fsm.values() {
                total_states += states.len();
            }
            eprintln!("  FSM:      {} states visited across {} signals", total_states, self.cover_fsm.len());
        } else {
            eprintln!("  FSM:      (no FSM data)");
        }
        
        eprintln!("═══════════════════════════════════════\n");
        
        self.report_line_coverage();
        self.report_toggle_coverage();
        self.report_branch_coverage();
        self.report_fsm_coverage();
    }

    /// Return coverage statistics as structured data (useful for CLI/JSON output).
    pub fn coverage_stats(&self) -> HashMap<String, f64> {
        let mut stats = HashMap::new();
        
        // Line coverage
        let line_items = self.cover_line.len() as f64;
        let line_hits: f64 = self.cover_line.values().sum::<u64>() as f64;
        stats.insert("line_items".to_string(), line_items);
        stats.insert("line_total_hits".to_string(), line_hits);
        
        // Toggle coverage
        let toggle_signals = self.cover_toggle.len() as f64;
        let toggle_transitions: f64 = self.cover_toggle.values().map(|s| s.len() as f64).sum();
        stats.insert("toggle_signals".to_string(), toggle_signals);
        stats.insert("toggle_transitions".to_string(), toggle_transitions);
        
        // Branch coverage
        let mut total_branches = 0u64;
        let mut covered_branches = 0u64;
        for branches in self.cover_branches.values() {
            for count in branches.values() {
                total_branches += 1;
                if *count > 0 {
                    covered_branches += 1;
                }
            }
        }
        let branch_pct = if total_branches > 0 {
            (covered_branches as f64 / total_branches as f64) * 100.0
        } else {
            0.0
        };
        stats.insert("branch_total".to_string(), total_branches as f64);
        stats.insert("branch_covered".to_string(), covered_branches as f64);
        stats.insert("branch_percent".to_string(), branch_pct);
        
        // FSM coverage
        let fsm_signals = self.cover_fsm.len() as f64;
        let fsm_states: f64 = self.cover_fsm.values().map(|s| s.len() as f64).sum();
        stats.insert("fsm_signals".to_string(), fsm_signals);
        stats.insert("fsm_states".to_string(), fsm_states);
        
        stats
    }

    /// Record toggle and FSM coverage after commit_changes.
    /// Called each delta cycle from run() loop.
    pub(crate) fn record_coverage_after_commit(&mut self) {
        if !self.coverage_enabled {
            return;
        }
        // Clone snapshot and current values to avoid double borrow of self
        let old_vals: Vec<LogicVec> = self.signal_snapshot
            .as_ref()
            .map(|snap| snap.clone())
            .unwrap_or_default();
        let n = old_vals.len();
        for sig_id in 0..n {
            let old_val = &old_vals[sig_id];
            // Read current signal value (now we can borrow self.state immutably)
            let new_val = self.state.read_signal(sig_id).clone();
            if old_val != &new_val {
                // Toggle: record transition — clone values to avoid borrow conflict
                self.record_toggle(sig_id, old_val, &new_val);
                // FSM: record signal value as potential state
                self.record_fsm_value(sig_id, &new_val);
            }
        }
    }

    /// Sample a named covergroup: evaluate coverpoints, update hit counts and bins.
    pub(crate) fn sample_covergroup(&mut self, cg_name: &str) -> Result<(), SimError> {
        let cg = self
            .design
            .covergroups
            .iter()
            .find(|c| c.name == cg_name)
            .cloned();
        if let Some(cg) = cg {
            let mut cp_values: HashMap<String, u64> = HashMap::new();
            for cp in &cg.coverpoints {
                let key = format!("{}.{}", cg.name, cp.name);
                let key_sym = Symbol::intern(&key);
                let total = self.cover_total.entry(key_sym).or_insert(0);
                *total += 1;
                let val = self
                    .evaluate_expr(&cp.expr)
                    .unwrap_or(LogicVec::from_u64(0, 32));
                cp_values.insert(cp.name.as_str().to_string(), val.to_u64());

                // Default bin: just record the actual value
                let bin_key = format!("{}={}", cp.name, val.to_u64());
                let bin_key_sym = Symbol::intern(&bin_key);
                let bins = self
                    .cover_bins
                    .entry(key_sym)
                    .or_default();
                let entry = bins.entry(bin_key_sym).or_insert(0);
                *entry += 1;
                let hits = self.cover_hits.entry(key_sym).or_insert(0);
                *hits += 1;
            }
            // Cross coverage
            for cross in &cg.crosses {
                let key = format!("{}.{}", cg.name, cross.name);
                let key_sym = Symbol::intern(&key);
                let total = self.cover_total.entry(key_sym).or_insert(0);
                *total += 1;
                let mut parts: Vec<String> = Vec::new();
                for cp_name in &cross.coverpoints {
                    let val = cp_values.get(cp_name.as_str()).copied().unwrap_or(0);
                    parts.push(format!("{}={}", cp_name, val));
                }
                let bin_key = parts.join(" x ");
                let bin_key_sym = Symbol::intern(&bin_key);
                let bins = self
                    .cover_bins
                    .entry(key_sym)
                    .or_default();
                let entry = bins.entry(bin_key_sym).or_insert(0);
                *entry += 1;
                let hits = self.cover_hits.entry(key_sym).or_insert(0);
                *hits += 1;
            }
        }
        Ok(())
    }

    /// Print coverage report to stderr.
    pub(crate) fn report_coverage(&self) {
        if self.design.covergroups.is_empty() {
            return;
        }
        eprintln!("\n=== Coverage Report ===");
        for cg in &self.design.covergroups {
            eprintln!("Covergroup: {}", cg.name);
            for cp in &cg.coverpoints {
                let key = format!("{}.{}", cg.name, cp.name);
                let key_sym = Symbol::intern(&key);
                let total = self.cover_total.get(&key_sym).copied().unwrap_or(0);
                let hits = self.cover_hits.get(&key_sym).copied().unwrap_or(0);
                let bins = self.cover_bins.get(&key_sym);
                let pct = if total > 0 {
                    (hits as f64 / total as f64) * 100.0
                } else {
                    0.0
                };
                eprintln!(
                    "  {}: {} hits / {} samples ({:.1}%)",
                    cp.name, hits, total, pct
                );
                if let Some(bins) = bins {
                    for (bin_key, count) in bins.iter() {
                        eprintln!("    - {}: {} hits", bin_key, count);
                    }
                }
            }
            for cross in &cg.crosses {
                let key = format!("{}.{}", cg.name, cross.name);
                let key_sym = Symbol::intern(&key);
                let total = self.cover_total.get(&key_sym).copied().unwrap_or(0);
                let hits = self.cover_hits.get(&key_sym).copied().unwrap_or(0);
                let bins = self.cover_bins.get(&key_sym);
                let pct = if total > 0 {
                    (hits as f64 / total as f64) * 100.0
                } else {
                    0.0
                };
                eprintln!(
                    "  {} (cross): {} hits / {} samples ({:.1}%)",
                    cross.name, hits, total, pct
                );
                if let Some(bins) = bins {
                    for (bin_key, count) in bins.iter() {
                        eprintln!("    - {}: {} hits", bin_key, count);
                    }
                }
            }
        }
    }

    /// Export all coverage data to UCIS XML format (IEEE 1800 UCIS schema).
    /// Includes: covergroup, line, toggle, branch, and FSM coverage.
    pub fn export_coverage_ucis(&self, path: &str) -> Result<(), SimError> {
        let mut xml = String::new();
        xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        xml.push_str("<coverageDatabase xmlns=\"urn:ucis:0.1\">\n");
        xml.push_str(&format!(
            "  <design name=\"{}\">\n",
            escape_xml(self.design.top.name.as_str())
        ));

        // ── Covergroup Coverage ──
        if !self.design.covergroups.is_empty() {
            xml.push_str("    <functionalCoverage>\n");
            for cg in &self.design.covergroups {
                xml.push_str(&format!("      <covergroup name=\"{}\">\n", escape_xml(cg.name.as_str())));

                for cp in &cg.coverpoints {
                    let key = format!("{}.{}", cg.name, cp.name);
                    let key_sym = Symbol::intern(&key);
                    let total = self.cover_total.get(&key_sym).copied().unwrap_or(0);
                    let hits = self.cover_hits.get(&key_sym).copied().unwrap_or(0);
                    let pct = if total > 0 { (hits as f64 / total as f64) * 100.0 } else { 0.0 };

                    xml.push_str(&format!(
                        "        <coverpoint name=\"{}\" total=\"{}\" hits=\"{}\" coverage=\"{:.1}\">\n",
                        escape_xml(cp.name.as_str()), total, hits, pct
                    ));

                    if let Some(bins) = self.cover_bins.get(&key_sym) {
                        for (bin_key, count) in bins.iter() {
                            xml.push_str(&format!(
                                "          <bin name=\"{}\" hits=\"{}\"/>\n",
                                escape_xml(bin_key.as_str()), count
                            ));
                        }
                    }

                    xml.push_str("        </coverpoint>\n");
                }

                for cross in &cg.crosses {
                    let key = format!("{}.{}", cg.name, cross.name);
                    let key_sym = Symbol::intern(&key);
                    let total = self.cover_total.get(&key_sym).copied().unwrap_or(0);
                    let hits = self.cover_hits.get(&key_sym).copied().unwrap_or(0);
                    let pct = if total > 0 { (hits as f64 / total as f64) * 100.0 } else { 0.0 };

                    xml.push_str(&format!(
                        "        <cross name=\"{}\" total=\"{}\" hits=\"{}\" coverage=\"{:.1}\">\n",
                        escape_xml(cross.name.as_str()), total, hits, pct
                    ));

                    if let Some(bins) = self.cover_bins.get(&key_sym) {
                        for (bin_key, count) in bins.iter() {
                            xml.push_str(&format!(
                                "          <bin name=\"{}\" hits=\"{}\"/>\n",
                                escape_xml(bin_key.as_str()), count
                            ));
                        }
                    }

                    xml.push_str("        </cross>\n");
                }

                xml.push_str("      </covergroup>\n");
            }
            xml.push_str("    </functionalCoverage>\n");
        }

        // ── Line Coverage ──
        if !self.cover_line.is_empty() {
            xml.push_str("    <lineCoverage>\n");
            let mut sorted_line: Vec<(&str, &u64)> = self.cover_line.iter()
                .map(|(k, v)| (k.as_str(), v))
                .collect();
            sorted_line.sort_by(|a, b| b.1.cmp(a.1));
            xml.push_str(&format!(
                "      <summary totalItems=\"{}\" totalHits=\"{}\"/>\n",
                self.cover_line.len(),
                self.cover_line.values().sum::<u64>()
            ));
            for (key, hits) in sorted_line {
                xml.push_str(&format!(
                    "      <lineItem key=\"{}\" hits=\"{}\"/>\n",
                    escape_xml(key), hits
                ));
            }
            xml.push_str("    </lineCoverage>\n");
        }

        // ── Toggle Coverage ──
        if !self.cover_toggle.is_empty() {
            xml.push_str("    <toggleCoverage>\n");
            let mut sorted_toggle: Vec<(&usize, &HashSet<(LogicVal, LogicVal)>)> = self.cover_toggle.iter().collect();
            sorted_toggle.sort_by_key(|(id, _)| *id);
            xml.push_str(&format!(
                "      <summary totalSignals=\"{}\"/>\n",
                self.cover_toggle.len()
            ));
            for (sig_id, transitions) in sorted_toggle {
                let sig_name = self.design.top.signals.get(*sig_id)
                    .map(|s| s.name.as_str())
                    .unwrap_or("<unknown>");
                xml.push_str(&format!(
                    "      <signal id=\"{}\" name=\"{}\" transitions=\"{}\">\n",
                    sig_id, escape_xml(sig_name), transitions.len()
                ));
                for (from, to) in transitions.iter() {
                    xml.push_str(&format!(
                        "        <transition from=\"{:?}\" to=\"{:?}\"/>\n", from, to
                    ));
                }
                xml.push_str("      </signal>\n");
            }
            xml.push_str("    </toggleCoverage>\n");
        }

        // ── Branch Coverage ──
        if !self.cover_branches.is_empty() {
            xml.push_str("    <branchCoverage>\n");
            let mut total_br = 0u64;
            let mut covered_br = 0u64;
            for branches in self.cover_branches.values() {
                for count in branches.values() {
                    total_br += 1;
                    if *count > 0 { covered_br += 1; }
                }
            }
            let br_pct = if total_br > 0 { (covered_br as f64 / total_br as f64) * 100.0 } else { 0.0 };
            xml.push_str(&format!(
                "      <summary totalBranches=\"{}\" coveredBranches=\"{}\" coverage=\"{:.1}\"/>\n",
                total_br, covered_br, br_pct
            ));
            let mut sorted_branch: Vec<(&str, &HashMap<Symbol, u64>)> = self.cover_branches.iter()
                .map(|(k, v)| (k.as_str(), v))
                .collect();
            sorted_branch.sort_by(|a, b| a.0.cmp(b.0));
            for (key, branches) in sorted_branch {
                xml.push_str(&format!(
                    "      <branchItem key=\"{}\">\n",
                    escape_xml(key)
                ));
                for (label, count) in branches {
                    xml.push_str(&format!(
                        "        <branch label=\"{}\" hits=\"{}\"/>\n",
                        escape_xml(label.as_str()), count
                    ));
                }
                xml.push_str("      </branchItem>\n");
            }
            xml.push_str("    </branchCoverage>\n");
        }

        // ── FSM Coverage ──
        if !self.cover_fsm.is_empty() {
            xml.push_str("    <fsmCoverage>\n");
            xml.push_str(&format!(
                "      <summary totalSignals=\"{}\" totalStates=\"{}\"/>\n",
                self.cover_fsm.len(),
                self.cover_fsm.values().map(|s| s.len()).sum::<usize>()
            ));
            let mut sorted_fsm: Vec<(&usize, &HashSet<u64>)> = self.cover_fsm.iter().collect();
            sorted_fsm.sort_by_key(|(id, _)| *id);
            for (sig_id, states) in sorted_fsm {
                let sig_name = self.design.top.signals.get(*sig_id)
                    .map(|s| s.name.as_str())
                    .unwrap_or("<unknown>");
                let mut sorted_states: Vec<u64> = states.iter().copied().collect();
                sorted_states.sort();
                xml.push_str(&format!(
                    "      <signal id=\"{}\" name=\"{}\" states=\"{}\">\n",
                    sig_id, escape_xml(sig_name), states.len()
                ));
                for state in &sorted_states {
                    xml.push_str(&format!(
                        "        <state value=\"{}\"/>\n", state
                    ));
                }
                xml.push_str("      </signal>\n");
            }
            xml.push_str("    </fsmCoverage>\n");
        }

        xml.push_str("  </design>\n");
        xml.push_str("</coverageDatabase>\n");

        std::fs::write(path, xml)
            .map_err(|e| SimError::with_diag(DiagCode::IoError, format!("cannot write UCIS file '{}': {}", path, e)))?;

        // Print summary to stderr
        let line_count = self.cover_line.len();
        let toggle_count = self.cover_toggle.len();
        let branch_count = self.cover_branches.len();
        let fsm_count = self.cover_fsm.len();
        let cg_count = self.design.covergroups.len();
        eprintln!("UCIS exported: covergroups={} line={} toggle={} branch={} fsm={}",
            cg_count, line_count, toggle_count, branch_count, fsm_count);

        Ok(())
    }
}
