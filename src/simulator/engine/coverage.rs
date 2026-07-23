/// Coverage tracking and reporting for SimulationEngine.
/// Manages covergroup sampling, coverage reporting, and UCIS XML export.
use crate::error::SimError;
use crate::ir::*;
use crate::simulator::util::*;
use crate::Symbol;
use std::collections::HashMap;

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
                    .or_insert_with(HashMap::new);
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
                    .or_insert_with(HashMap::new);
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

    /// Export coverage data to UCIS XML format.
    pub fn export_coverage_ucis(&self, path: &str) -> Result<(), SimError> {
        if self.design.covergroups.is_empty() {
            return Ok(());
        }

        let mut xml = String::new();
        xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        xml.push_str("<ucis xmlns=\"urn:ucis:0.1\">\n");
        xml.push_str(&format!(
            "  <scope name=\"{}\" type=\"module\">\n",
            self.design.top.name
        ));

        for cg in &self.design.covergroups {
            xml.push_str(&format!("    <covergroup name=\"{}\">\n", cg.name));

            for cp in &cg.coverpoints {
                let key = format!("{}.{}", cg.name, cp.name);
                let key_sym = Symbol::intern(&key);
                let total = self.cover_total.get(&key_sym).copied().unwrap_or(0);
                let hits = self.cover_hits.get(&key_sym).copied().unwrap_or(0);
                let bins = self.cover_bins.get(&key_sym);

                xml.push_str(&format!(
                    "      <coverpoint name=\"{}\" total=\"{}\" hits=\"{}\">\n",
                    cp.name, total, hits
                ));

                if let Some(bins) = bins {
                    for (bin_key, count) in bins.iter() {
                        xml.push_str(&format!(
                            "        <bin name=\"{}\" hits=\"{}\"/>\n",
                            escape_xml(bin_key.as_str()),
                            count
                        ));
                    }
                }

                xml.push_str("      </coverpoint>\n");
            }

            for cross in &cg.crosses {
                let key = format!("{}.{}", cg.name, cross.name);
                let key_sym = Symbol::intern(&key);
                let total = self.cover_total.get(&key_sym).copied().unwrap_or(0);
                let hits = self.cover_hits.get(&key_sym).copied().unwrap_or(0);
                let bins = self.cover_bins.get(&key_sym);

                xml.push_str(&format!(
                    "      <cross name=\"{}\" total=\"{}\" hits=\"{}\">\n",
                    cross.name, total, hits
                ));

                if let Some(bins) = bins {
                    for (bin_key, count) in bins.iter() {
                        xml.push_str(&format!(
                            "        <bin name=\"{}\" hits=\"{}\"/>\n",
                            escape_xml(bin_key.as_str()),
                            count
                        ));
                    }
                }

                xml.push_str("      </cross>\n");
            }

            xml.push_str("    </covergroup>\n");
        }

        xml.push_str("  </scope>\n");
        xml.push_str("</ucis>\n");

        std::fs::write(path, xml)
            .map_err(|e| SimError::waveform(format!("cannot write UCIS file '{}': {}", path, e)))?;

        Ok(())
    }
}
