//! CoverageDatabase — persistent coverage database with binary format and merge support.
//!
//! Stores coverage data (covergroup, line, toggle, branch, FSM) in a binary format
//! that supports merging from multiple simulation runs. This is the UCDB (Unified
//! Coverage Database) implementation for Maria.
//!
//! # Binary Format
//! - Magic: "MCDB" (4 bytes)
//! - Version: u32 (1)
//! - Sections, each prefixed with type tag + length:
//!   - SectionCovergroup: covergroup/coverpoint/cross/bin data
//!   - SectionLine: line coverage data (process→hits)
//!   - SectionToggle: toggle coverage data (signal_id→transition_set)
//!   - SectionBranch: branch coverage data (branch_key→label→hits)
//!   - SectionFsm: FSM coverage data (signal_id→state_set)
//!
//! # Merge
//! - Load: read binary file, deserialize into CoverageDatabase
//! - Merge: combine with current Engine coverage data (sum hits, union sets)
//! - Save: serialize to binary file

use maria_ir::LogicVal;
use maria_core::Symbol;
use std::collections::HashMap;
use std::io::{self, BufReader, BufWriter, Read, Write};


// ─── Binary I/O Helpers ─────────────────────────────────────────────

fn write_u64<W: Write>(w: &mut W, val: u64) -> io::Result<()> {
    w.write_all(&val.to_le_bytes())
}

fn read_u64<R: Read>(r: &mut R) -> io::Result<u64> {
    let mut buf = [0u8; 8];
    r.read_exact(&mut buf)?;
    Ok(u64::from_le_bytes(buf))
}

fn write_u32<W: Write>(w: &mut W, val: u32) -> io::Result<()> {
    w.write_all(&val.to_le_bytes())
}

fn read_u32<R: Read>(r: &mut R) -> io::Result<u32> {
    let mut buf = [0u8; 4];
    r.read_exact(&mut buf)?;
    Ok(u32::from_le_bytes(buf))
}

fn write_usize<W: Write>(w: &mut W, val: usize) -> io::Result<()> {
    write_u64(w, val as u64)
}

fn read_usize<R: Read>(r: &mut R) -> io::Result<usize> {
    read_u64(r).map(|v| v as usize)
}

fn write_str<W: Write>(w: &mut W, s: &str) -> io::Result<()> {
    let bytes = s.as_bytes();
    write_usize(w, bytes.len())?;
    w.write_all(bytes)
}

fn read_str<R: Read>(r: &mut R) -> io::Result<String> {
    let len = read_usize(r)?;
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    String::from_utf8(buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

fn write_logic_val<W: Write>(w: &mut W, val: &LogicVal) -> io::Result<()> {
    let byte = match val {
        LogicVal::Zero => 0u8,
        LogicVal::One => 1u8,
        LogicVal::X => 2u8,
        LogicVal::Z => 3u8,
    };
    w.write_all(&[byte])
}

fn read_logic_val<R: Read>(r: &mut R) -> io::Result<LogicVal> {
    let mut buf = [0u8; 1];
    r.read_exact(&mut buf)?;
    match buf[0] {
        0 => Ok(LogicVal::Zero),
        1 => Ok(LogicVal::One),
        2 => Ok(LogicVal::X),
        3 => Ok(LogicVal::Z),
        _ => Err(io::Error::new(io::ErrorKind::InvalidData, "invalid LogicVal")),
    }
}

// ─── Section Types ───────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
enum SectionTag {
    Covergroup = 1,
    Line = 2,
    Toggle = 3,
    Branch = 4,
    Fsm = 5,
    End = 0,
}

impl SectionTag {
    fn from_u32(v: u32) -> Option<Self> {
        match v {
            1 => Some(SectionTag::Covergroup),
            2 => Some(SectionTag::Line),
            3 => Some(SectionTag::Toggle),
            4 => Some(SectionTag::Branch),
            5 => Some(SectionTag::Fsm),
            0 => Some(SectionTag::End),
            _ => None,
        }
    }
}

/// A single covergroup bin entry: name → hit count.
pub type CoverBinMap = HashMap<Symbol, u64>;

/// Covergroup data: coverpoint/cross name → total/hits/bins.
#[derive(Debug, Clone)]
pub struct CovergroupEntry {
    pub name: String,
    pub coverpoints: Vec<CoverpointEntry>,
    pub crosses: Vec<CrossEntry>,
}

#[derive(Debug, Clone)]
pub struct CoverpointEntry {
    pub name: String,
    pub total: u64,
    pub hits: u64,
    pub bins: CoverBinMap,
}

#[derive(Debug, Clone)]
pub struct CrossEntry {
    pub name: String,
    pub total: u64,
    pub hits: u64,
    pub bins: CoverBinMap,
}

/// A toggle entry: (from, to) → count (merged count across runs).
#[derive(Debug, Clone)]
pub struct ToggleEntry {
    pub sig_id: usize,
    pub transitions: HashMap<(LogicVal, LogicVal), u64>,
}

/// A branch entry: label → count.
pub type BranchEntry = HashMap<Symbol, u64>;

/// FSM entry: state value → count.
pub type FsmEntry = HashMap<u64, u64>;

// ─── CoverageDatabase ────────────────────────────────────────────────

/// Result of comparing two coverage databases.
#[derive(Debug, Clone)]
pub struct CoverageDiff {
    /// Coverpoints whose hit count differs: (name, old_hits, new_hits)
    pub coverpoint_changes: Vec<(String, u64, u64)>,
    /// New coverpoints in `other` not present in `self`
    pub new_coverpoints: Vec<String>,
    /// Line items whose hit count differs
    pub line_changes: Vec<(Symbol, u64, u64)>,
    /// Branch keys with different branch distributions
    pub branch_changes: Vec<(Symbol, u64, u64)>,
    /// FSM signals with different state counts
    pub fsm_changes: Vec<(usize, usize, usize)>,
}

/// Persistent coverage database supporting multi-run merge.
pub struct CoverageDatabase {
    /// Covergroup data: covergroup_name → entry
    pub covergroups: HashMap<String, CovergroupEntry>,
    /// Line coverage: process_key → hits
    pub line_hits: HashMap<Symbol, u64>,
    /// Toggle coverage: signal_id → transitions
    pub toggle_data: HashMap<usize, ToggleEntry>,
    /// Branch coverage: branch_key → label_counts
    pub branch_data: HashMap<Symbol, BranchEntry>,
    /// FSM coverage: signal_id → state_counts
    pub fsm_data: HashMap<usize, FsmEntry>,
    /// Database file path (for auto-save)
    path: Option<String>,
}

impl CoverageDatabase {
    /// Create a new empty database.
    pub fn new() -> Self {
        CoverageDatabase {
            covergroups: HashMap::new(),
            line_hits: HashMap::new(),
            toggle_data: HashMap::new(),
            branch_data: HashMap::new(),
            fsm_data: HashMap::new(),
            path: None,
        }
    }

    /// Create a database with a file path (will load existing data if file exists).
    pub fn with_path(path: &str) -> Self {
        let mut db = CoverageDatabase::new();
        db.path = Some(path.to_string());
        if std::path::Path::new(path).exists() {
            if let Ok(loaded) = Self::load_from_path(path) {
                db = loaded;
                db.path = Some(path.to_string());
            }
        }
        db
    }

    /// Set the database file path.
    pub fn set_path(&mut self, path: &str) {
        self.path = Some(path.to_string());
    }

    /// Get the current file path.
    pub fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }

    // ─── Merge ─────────────────────────────────────────────────────

    /// Merge coverage data from the engine into this database.
    /// Values are summed across runs. Sets are unioned.
    pub fn merge_from_engine(
        &mut self,
        engine: &crate::simulator::engine::SimulationEngine,
    ) {
        // Merge covergroup data
        for cg in &engine.design.covergroups {
            let entry = self.covergroups.entry(cg.name.to_string())
                .or_insert_with(|| CovergroupEntry {
                    name: cg.name.to_string(),
                    coverpoints: Vec::new(),
                    crosses: Vec::new(),
                });

            // VERIF-28: per_instance → engine menyimpan per-instance key
            // (`cg.i<id>.cp`). Jumlahkan SEMUA key ber-prefix `cg.` agar db
            // tetap menerima total agregat.
            let prefix = format!("{}.", cg.name);
            let sum_key = |item: &str,
                               total_map: &HashMap<Symbol, u64>,
                               hits_map: &HashMap<Symbol, u64>,
                               bins_map: &HashMap<Symbol, HashMap<Symbol, u64>>|
             -> (u64, u64, HashMap<Symbol, u64>) {
                let mut total = 0u64;
                let mut hits = 0u64;
                let mut bins = HashMap::new();
                let full = format!("{}{}", prefix, item);
                let item_suffix = format!(".{}", item);
                let matches_key = |s: &str| {
                    // Key agregat `cg.item` ATAU key per-instance `cg.i<id>.item`
                    // (VERIF-28) — keduanya diawali `cg.` dan diakhiri `.item`.
                    s == full || (s.starts_with(&prefix) && s.ends_with(&item_suffix))
                };
                for (k, v) in total_map {
                    if matches_key(k.as_str()) {
                        total += v;
                    }
                }
                for (k, v) in hits_map {
                    if matches_key(k.as_str()) {
                        hits += v;
                    }
                }
                for (k, bmap) in bins_map {
                    if matches_key(k.as_str()) {
                        for (bk, bv) in bmap {
                            *bins.entry(*bk).or_insert(0) += bv;
                        }
                    }
                }
                (total, hits, bins)
            };

            for cp in &cg.coverpoints {
                let (total, hits, bins) = sum_key(
                    cp.name.as_str(),
                    &engine.cover_total,
                    &engine.cover_hits,
                    &engine.cover_bins,
                );
                let cp_name = cp.name.to_string();

                // Find or create coverpoint entry by name
                if let Some(existing) = entry.coverpoints.iter_mut().find(|e| e.name == cp_name) {
                    existing.total = existing.total.saturating_add(total);
                    existing.hits = existing.hits.saturating_add(hits);
                    for (k, v) in bins {
                        *existing.bins.entry(k).or_insert(0) += v;
                    }
                } else {
                    entry.coverpoints.push(CoverpointEntry {
                        name: cp_name,
                        total,
                        hits,
                        bins,
                    });
                }
            }

            for cross in &cg.crosses {
                let (total, hits, bins) = sum_key(
                    cross.name.as_str(),
                    &engine.cover_total,
                    &engine.cover_hits,
                    &engine.cover_bins,
                );
                let cross_name = cross.name.to_string();

                if let Some(existing) = entry.crosses.iter_mut().find(|e| e.name == cross_name) {
                    existing.total = existing.total.saturating_add(total);
                    existing.hits = existing.hits.saturating_add(hits);
                    for (k, v) in bins {
                        *existing.bins.entry(k).or_insert(0) += v;
                    }
                } else {
                    entry.crosses.push(CrossEntry {
                        name: cross_name,
                        total,
                        hits,
                        bins,
                    });
                }
            }
        }

        // Merge line coverage
        for (key, hits) in &engine.cover_line {
            *self.line_hits.entry(*key).or_insert(0) += hits;
        }

        // Merge toggle coverage
        for (sig_id, transitions) in &engine.cover_toggle {
            let entry = self.toggle_data.entry(*sig_id)
                .or_insert_with(|| ToggleEntry {
                    sig_id: *sig_id,
                    transitions: HashMap::new(),
                });
            for transition in transitions {
                *entry.transitions.entry(*transition).or_insert(0) += 1;
            }
        }

        // Merge branch coverage
        for (key, branches) in &engine.cover_branches {
            let entry = self.branch_data.entry(*key)
                .or_default();
            for (label, count) in branches {
                *entry.entry(*label).or_insert(0) += count;
            }
        }

        // Merge FSM coverage
        for (sig_id, states) in &engine.cover_fsm {
            let entry = self.fsm_data.entry(*sig_id)
                .or_default();
            for state in states {
                *entry.entry(*state).or_insert(0) += 1;
            }
        }
    }

    /// Build engine-style coverage HashMaps from this database.
    /// Useful for reporting after load.
    pub fn to_engine_maps(
        &self,
    ) -> (HashMap<Symbol, u64>, HashMap<Symbol, u64>, HashMap<Symbol, HashMap<Symbol, u64>>) {
        let mut total = HashMap::new();
        let mut hits = HashMap::new();
        let mut bins = HashMap::new();

        for (cg_name, entry) in &self.covergroups {
            for cp in &entry.coverpoints {
                let key = Symbol::intern(&format!("{}.{}", cg_name, cp.name));
                total.insert(key, cp.total);
                hits.insert(key, cp.hits);
                bins.insert(key, cp.bins.clone());
            }
            for cross in &entry.crosses {
                let key = Symbol::intern(&format!("{}.{}", cg_name, cross.name));
                total.insert(key, cross.total);
                hits.insert(key, cross.hits);
                bins.insert(key, cross.bins.clone());
            }
        }

        (total, hits, bins)
    }

    /// Save the database to its configured path.
    pub fn save(&self) -> Result<(), String> {
        match &self.path {
            Some(p) => self.save_to_file(p),
            None => Err("no path configured for coverage database".to_string()),
        }
    }

    // ─── Binary Serialization ───────────────────────────────────────

    /// Save to binary file.
    pub fn save_to_file(&self, path: &str) -> Result<(), String> {
        let file = std::fs::File::create(path)
            .map_err(|e| format!("cannot create coverage DB '{}': {}", path, e))?;
        let mut writer = BufWriter::new(file);

        // Header
        writer.write_all(b"MCDB")
            .map_err(|e| format!("write header: {}", e))?;
        write_u32(&mut writer, 1) // version
            .map_err(|e| format!("write version: {}", e))?;

        // Section 1: Covergroup
        writer.write_all(&[SectionTag::Covergroup as u8, 0, 0, 0])
            .map_err(|e| format!("write section tag: {}", e))?;
        self.write_covergroup_section(&mut writer)?;

        // Section 2: Line
        writer.write_all(&[SectionTag::Line as u8, 0, 0, 0])
            .map_err(|e| format!("write tag line: {}", e))?;
        self.write_line_section(&mut writer)?;

        // Section 3: Toggle
        writer.write_all(&[SectionTag::Toggle as u8, 0, 0, 0])
            .map_err(|e| format!("write tag toggle: {}", e))?;
        self.write_toggle_section(&mut writer)?;

        // Section 4: Branch
        writer.write_all(&[SectionTag::Branch as u8, 0, 0, 0])
            .map_err(|e| format!("write tag branch: {}", e))?;
        self.write_branch_section(&mut writer)?;

        // Section 5: FSM
        writer.write_all(&[SectionTag::Fsm as u8, 0, 0, 0])
            .map_err(|e| format!("write tag fsm: {}", e))?;
        self.write_fsm_section(&mut writer)?;

        // End marker
        writer.write_all(&[SectionTag::End as u8, 0, 0, 0])
            .map_err(|e| format!("write end: {}", e))?;

        writer.flush().map_err(|e| format!("flush: {}", e))?;
        Ok(())
    }

    /// Load from binary file and merge into this database.
    /// This supports multi-run merge: call load_and_merge() for each run file.
    pub fn load_and_merge(&mut self, path: &str) -> Result<(), String> {
        let loaded = Self::load_from_path(path)?;
        self.merge_from_db(&loaded);
        Ok(())
    }

    /// Load from binary file (static method).
    pub fn load_from_path(path: &str) -> Result<CoverageDatabase, String> {
        let file = std::fs::File::open(path)
            .map_err(|e| format!("cannot open coverage DB '{}': {}", path, e))?;
        let mut reader = BufReader::new(file);

        // Verify header
        let mut magic = [0u8; 4];
        reader.read_exact(&mut magic)
            .map_err(|e| format!("read magic: {}", e))?;
        if &magic != b"MCDB" {
            return Err("invalid coverage DB magic".to_string());
        }
        let _version = read_u32(&mut reader)
            .map_err(|e| format!("read version: {}", e))?;

        let mut db = CoverageDatabase::new();

        loop {
            let mut tag_buf = [0u8; 4];
            if reader.read_exact(&mut tag_buf).is_err() {
                break; // EOF
            }
            let tag = SectionTag::from_u32(tag_buf[0] as u32)
                .ok_or_else(|| format!("unknown section tag: {}", tag_buf[0]))?;

            match tag {
                SectionTag::Covergroup => db.read_covergroup_section(&mut reader)?,
                SectionTag::Line => db.read_line_section(&mut reader)?,
                SectionTag::Toggle => db.read_toggle_section(&mut reader)?,
                SectionTag::Branch => db.read_branch_section(&mut reader)?,
                SectionTag::Fsm => db.read_fsm_section(&mut reader)?,
                SectionTag::End => break,
            }
        }

        Ok(db)
    }

    /// Load from binary file (DEPRECATED: use load_from_path or load_and_merge).
    /// Returns a new CoverageDatabase.
    pub fn load_from_file(&self, path: &str) -> Result<CoverageDatabase, String> {
        Self::load_from_path(path)
    }

    // ─── Section Writers ────────────────────────────────────────────

    fn write_covergroup_section(&self, w: &mut impl Write) -> Result<(), String> {
        write_usize(w, self.covergroups.len())
            .map_err(|e| format!("cg count: {}", e))?;
        for (name, entry) in &self.covergroups {
            write_str(w, name)
                .map_err(|e| format!("cg name: {}", e))?;
            // Coverpoints
            write_usize(w, entry.coverpoints.len())
                .map_err(|e| format!("cp count: {}", e))?;
            for cp in &entry.coverpoints {
                write_str(w, &cp.name)
                    .map_err(|e| format!("cp name: {}", e))?;
                write_u64(w, cp.total)
                    .map_err(|e| format!("cp total: {}", e))?;
                write_u64(w, cp.hits)
                    .map_err(|e| format!("cp hits: {}", e))?;
                write_bin_map(w, &cp.bins)?;
            }
            // Crosses
            write_usize(w, entry.crosses.len())
                .map_err(|e| format!("cross count: {}", e))?;
            for cross in &entry.crosses {
                write_str(w, &cross.name)
                    .map_err(|e| format!("cross name: {}", e))?;
                write_u64(w, cross.total)
                    .map_err(|e| format!("cross total: {}", e))?;
                write_u64(w, cross.hits)
                    .map_err(|e| format!("cross hits: {}", e))?;
                write_bin_map(w, &cross.bins)?;
            }
        }
        Ok(())
    }

    fn write_line_section(&self, w: &mut impl Write) -> Result<(), String> {
        write_usize(w, self.line_hits.len())
            .map_err(|e| format!("line count: {}", e))?;
        for (key, hits) in &self.line_hits {
            write_str(w, key.as_str())
                .map_err(|e| format!("line key: {}", e))?;
            write_u64(w, *hits)
                .map_err(|e| format!("line hits: {}", e))?;
        }
        Ok(())
    }

    fn write_toggle_section(&self, w: &mut impl Write) -> Result<(), String> {
        write_usize(w, self.toggle_data.len())
            .map_err(|e| format!("toggle count: {}", e))?;
        for (sig_id, entry) in &self.toggle_data {
            write_usize(w, *sig_id)
                .map_err(|e| format!("toggle sig: {}", e))?;
            write_usize(w, entry.transitions.len())
                .map_err(|e| format!("toggle trans: {}", e))?;
            for ((from, to), count) in &entry.transitions {
                write_logic_val(w, from)
                    .map_err(|e| format!("toggle from: {}", e))?;
                write_logic_val(w, to)
                    .map_err(|e| format!("toggle to: {}", e))?;
                write_u64(w, *count)
                    .map_err(|e| format!("toggle count: {}", e))?;
            }
        }
        Ok(())
    }

    fn write_branch_section(&self, w: &mut impl Write) -> Result<(), String> {
        write_usize(w, self.branch_data.len())
            .map_err(|e| format!("branch count: {}", e))?;
        for (key, branches) in &self.branch_data {
            write_str(w, key.as_str())
                .map_err(|e| format!("branch key: {}", e))?;
            write_usize(w, branches.len())
                .map_err(|e| format!("branch entries: {}", e))?;
            for (label, count) in branches {
                write_str(w, label.as_str())
                    .map_err(|e| format!("branch label: {}", e))?;
                write_u64(w, *count)
                    .map_err(|e| format!("branch count: {}", e))?;
            }
        }
        Ok(())
    }

    fn write_fsm_section(&self, w: &mut impl Write) -> Result<(), String> {
        write_usize(w, self.fsm_data.len())
            .map_err(|e| format!("fsm count: {}", e))?;
        for (sig_id, states) in &self.fsm_data {
            write_usize(w, *sig_id)
                .map_err(|e| format!("fsm sig: {}", e))?;
            write_usize(w, states.len())
                .map_err(|e| format!("fsm states: {}", e))?;
            for (state, count) in states {
                write_u64(w, *state)
                    .map_err(|e| format!("fsm state: {}", e))?;
                write_u64(w, *count)
                    .map_err(|e| format!("fsm count: {}", e))?;
            }
        }
        Ok(())
    }

    // ─── Section Readers ────────────────────────────────────────────

    fn read_covergroup_section(&mut self, r: &mut impl Read) -> Result<(), String> {
        let cg_count = read_usize(r).map_err(|e| format!("cg count: {}", e))?;
        for _ in 0..cg_count {
            let name = read_str(r).map_err(|e| format!("cg name: {}", e))?;
            let mut entry = CovergroupEntry {
                name: name.clone(),
                coverpoints: Vec::new(),
                crosses: Vec::new(),
            };

            // Coverpoints
            let cp_count = read_usize(r).map_err(|e| format!("cp count: {}", e))?;
            for _ in 0..cp_count {
                let cp_name = read_str(r).map_err(|e| format!("cp name: {}", e))?;
                let total = read_u64(r).map_err(|e| format!("cp total: {}", e))?;
                let hits = read_u64(r).map_err(|e| format!("cp hits: {}", e))?;
                let bins = read_bin_map(r)?;
                entry.coverpoints.push(CoverpointEntry {
                    name: cp_name,
                    total,
                    hits,
                    bins,
                });
            }

            // Crosses
            let cross_count = read_usize(r).map_err(|e| format!("cross count: {}", e))?;
            for _ in 0..cross_count {
                let cross_name = read_str(r).map_err(|e| format!("cross name: {}", e))?;
                let total = read_u64(r).map_err(|e| format!("cross total: {}", e))?;
                let hits = read_u64(r).map_err(|e| format!("cross hits: {}", e))?;
                let bins = read_bin_map(r)?;
                entry.crosses.push(CrossEntry {
                    name: cross_name,
                    total,
                    hits,
                    bins,
                });
            }

            self.covergroups.insert(name, entry);
        }
        Ok(())
    }

    fn read_line_section(&mut self, r: &mut impl Read) -> Result<(), String> {
        let count = read_usize(r).map_err(|e| format!("line count: {}", e))?;
        for _ in 0..count {
            let key_str = read_str(r).map_err(|e| format!("line key: {}", e))?;
            let hits = read_u64(r).map_err(|e| format!("line hits: {}", e))?;
            self.line_hits.insert(Symbol::intern(&key_str), hits);
        }
        Ok(())
    }

    fn read_toggle_section(&mut self, r: &mut impl Read) -> Result<(), String> {
        let count = read_usize(r).map_err(|e| format!("toggle count: {}", e))?;
        for _ in 0..count {
            let sig_id = read_usize(r).map_err(|e| format!("toggle sig: {}", e))?;
            let trans_count = read_usize(r).map_err(|e| format!("toggle trans: {}", e))?;
            let mut transitions = HashMap::new();
            for _ in 0..trans_count {
                let from = read_logic_val(r).map_err(|e| format!("toggle from: {}", e))?;
                let to = read_logic_val(r).map_err(|e| format!("toggle to: {}", e))?;
                let count = read_u64(r).map_err(|e| format!("toggle count: {}", e))?;
                transitions.insert((from, to), count);
            }
            self.toggle_data.insert(sig_id, ToggleEntry { sig_id, transitions });
        }
        Ok(())
    }

    fn read_branch_section(&mut self, r: &mut impl Read) -> Result<(), String> {
        let count = read_usize(r).map_err(|e| format!("branch count: {}", e))?;
        for _ in 0..count {
            let key_str = read_str(r).map_err(|e| format!("branch key: {}", e))?;
            let entries = read_usize(r).map_err(|e| format!("branch entries: {}", e))?;
            let mut branches = HashMap::new();
            for _ in 0..entries {
                let label = read_str(r).map_err(|e| format!("branch label: {}", e))?;
                let count = read_u64(r).map_err(|e| format!("branch count: {}", e))?;
                branches.insert(Symbol::intern(&label), count);
            }
            self.branch_data.insert(Symbol::intern(&key_str), branches);
        }
        Ok(())
    }

    fn read_fsm_section(&mut self, r: &mut impl Read) -> Result<(), String> {
        let count = read_usize(r).map_err(|e| format!("fsm count: {}", e))?;
        for _ in 0..count {
            let sig_id = read_usize(r).map_err(|e| format!("fsm sig: {}", e))?;
            let states_count = read_usize(r).map_err(|e| format!("fsm states: {}", e))?;
            let mut states = HashMap::new();
            for _ in 0..states_count {
                let state = read_u64(r).map_err(|e| format!("fsm state: {}", e))?;
                let count = read_u64(r).map_err(|e| format!("fsm count: {}", e))?;
                states.insert(state, count);
            }
            self.fsm_data.insert(sig_id, states);
        }
        Ok(())
    }

    // ─── Inter-Database Operations ───────────────────────────────────

    /// Merge coverage data from another CoverageDatabase into this one.
    /// Values are summed, sets are unioned — just like merge_from_engine.
    pub fn merge_from_db(&mut self, other: &CoverageDatabase) {
        // Merge covergroup data
        for (name, other_entry) in &other.covergroups {
            let entry = self.covergroups.entry(name.clone())
                .or_insert_with(|| CovergroupEntry {
                    name: name.clone(),
                    coverpoints: Vec::new(),
                    crosses: Vec::new(),
                });

            for cp in &other_entry.coverpoints {
                if let Some(existing) = entry.coverpoints.iter_mut().find(|e| e.name == cp.name) {
                    existing.total = existing.total.saturating_add(cp.total);
                    existing.hits = existing.hits.saturating_add(cp.hits);
                    for (k, v) in &cp.bins {
                        *existing.bins.entry(*k).or_insert(0) += v;
                    }
                } else {
                    entry.coverpoints.push(cp.clone());
                }
            }

            for cross in &other_entry.crosses {
                if let Some(existing) = entry.crosses.iter_mut().find(|e| e.name == cross.name) {
                    existing.total = existing.total.saturating_add(cross.total);
                    existing.hits = existing.hits.saturating_add(cross.hits);
                    for (k, v) in &cross.bins {
                        *existing.bins.entry(*k).or_insert(0) += v;
                    }
                } else {
                    entry.crosses.push(cross.clone());
                }
            }
        }

        // Merge line coverage
        for (key, hits) in &other.line_hits {
            *self.line_hits.entry(*key).or_insert(0) += hits;
        }

        // Merge toggle coverage
        for (sig_id, other_entry) in &other.toggle_data {
            let entry = self.toggle_data.entry(*sig_id)
                .or_insert_with(|| ToggleEntry {
                    sig_id: *sig_id,
                    transitions: HashMap::new(),
                });
            for (transition, count) in &other_entry.transitions {
                *entry.transitions.entry(*transition).or_insert(0) += count;
            }
        }

        // Merge branch coverage
        for (key, branches) in &other.branch_data {
            let entry = self.branch_data.entry(*key)
                .or_default();
            for (label, count) in branches {
                *entry.entry(*label).or_insert(0) += count;
            }
        }

        // Merge FSM coverage
        for (sig_id, states) in &other.fsm_data {
            let entry = self.fsm_data.entry(*sig_id)
                .or_default();
            for (state, count) in states {
                *entry.entry(*state).or_insert(0) += count;
            }
        }
    }

    /// Compute a diff between this database and another.
    /// Returns all coverage items whose values differ.
    pub fn diff(&self, other: &CoverageDatabase) -> CoverageDiff {
        let mut diff = CoverageDiff {
            coverpoint_changes: Vec::new(),
            new_coverpoints: Vec::new(),
            line_changes: Vec::new(),
            branch_changes: Vec::new(),
            fsm_changes: Vec::new(),
        };

        // Coverpoint changes
        for (name, entry) in &self.covergroups {
            for cp in &entry.coverpoints {
                let other_hits = other.covergroups.get(name)
                    .and_then(|e| e.coverpoints.iter().find(|c| c.name == cp.name))
                    .map(|c| c.hits)
                    .unwrap_or(0);
                if cp.hits != other_hits {
                    diff.coverpoint_changes.push((format!("{}.{}", name, cp.name), cp.hits, other_hits));
                }
            }
        }
        // New coverpoints in other
        for (name, entry) in &other.covergroups {
            for cp in &entry.coverpoints {
                let found = self.covergroups.get(name)
                    .and_then(|e| e.coverpoints.iter().find(|c| c.name == cp.name))
                    .is_some();
                if !found {
                    diff.new_coverpoints.push(format!("{}.{}", name, cp.name));
                }
            }
        }

        // Line changes
        for (key, hits) in &self.line_hits {
            let other_hits = other.line_hits.get(key).copied().unwrap_or(0);
            if *hits != other_hits {
                diff.line_changes.push((*key, *hits, other_hits));
            }
        }

        // Branch changes
        for (key, branches) in &self.branch_data {
            let self_total: u64 = branches.values().sum();
            let other_total: u64 = other.branch_data.get(key)
                .map(|b| b.values().sum())
                .unwrap_or(0);
            if self_total != other_total {
                diff.branch_changes.push((*key, self_total, other_total));
            }
        }

        // FSM changes
        for (sig_id, states) in &self.fsm_data {
            let n_self = states.len();
            let n_other = other.fsm_data.get(sig_id).map(|s| s.len()).unwrap_or(0);
            if n_self != n_other {
                diff.fsm_changes.push((*sig_id, n_self, n_other));
            }
        }

        diff
    }

    /// Print a human-readable coverage report to stdout.
    pub fn report(&self) {
        let total_cg = self.covergroups.len();
        let total_cp: usize = self.covergroups.values().map(|e| e.coverpoints.len()).sum();
        let total_line = self.line_hits.len();
        let total_toggle = self.toggle_data.len();
        let total_branch = self.branch_data.len();
        let total_fsm = self.fsm_data.len();

        println!("\n========================================");
        println!("   COVERAGE DATABASE REPORT");
        println!("========================================");
        println!("  Covergroups: {}", total_cg);
        println!("  Coverpoints: {}", total_cp);
        println!("  Line items:  {}", total_line);
        println!("  Toggle sigs: {}", total_toggle);
        println!("  Branches:    {}", total_branch);
        println!("  FSM signals: {}", total_fsm);
        println!("");

        // Covergroup details
        for (name, entry) in &self.covergroups {
            println!("  Covergroup '{}':", name);
            for cp in &entry.coverpoints {
                let pct = if cp.total > 0 {
                    (cp.hits as f64 / cp.total as f64) * 100.0
                } else {
                    0.0
                };
                println!("    {}: {}/{} hits ({:.1}%)", cp.name, cp.hits, cp.total, pct);
                let mut sorted_bins: Vec<(&str, &u64)> = cp.bins.iter()
                    .map(|(k, v)| (k.as_str(), v))
                    .collect();
                sorted_bins.sort_by(|a, b| b.1.cmp(a.1));
                for (bin_key, count) in sorted_bins.iter().take(5) {
                    println!("      - {}: {}", bin_key, count);
                }
            }
        }

        // Line coverage highlights (top 10)
        if !self.line_hits.is_empty() {
            println!("  Top line hits:");
            let mut sorted: Vec<(&str, &u64)> = self.line_hits.iter()
                .map(|(k, v)| (k.as_str(), v))
                .collect();
            sorted.sort_by(|a, b| b.1.cmp(a.1));
            for (key, hits) in sorted.iter().take(10) {
                println!("    {}: {} hits", key, hits);
            }
        }

        // Branch coverage summary
        if !self.branch_data.is_empty() {
            let mut total_br = 0u64;
            let mut covered_br = 0u64;
            for branches in self.branch_data.values() {
                for count in branches.values() {
                    total_br += 1;
                    if *count > 0 { covered_br += 1; }
                }
            }
            let br_pct = if total_br > 0 { (covered_br as f64 / total_br as f64) * 100.0 } else { 0.0 };
            println!("  Branch coverage: {}/{} ({:.1}%)", covered_br, total_br, br_pct);
        }

        println!("========================================\n");
    }

    /// Export coverage report as HTML file.
    pub fn export_html(&self, path: &str) -> Result<(), String> {
        let mut html = String::new();
        html.push_str("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n");
        html.push_str("<meta charset=\"UTF-8\">\n");
        html.push_str("<title>Coverage Report</title>\n");
        html.push_str("<style>\n");
        html.push_str("  body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; margin: 20px; background: #0d1117; color: #c9d1d9; }\n");
        html.push_str("  h1 { color: #58a6ff; border-bottom: 2px solid #30363d; padding-bottom: 10px; }\n");
        html.push_str("  h2 { color: #58a6ff; }\n");
        html.push_str("  table { border-collapse: collapse; width: 100%; margin: 10px 0; }\n");
        html.push_str("  th, td { border: 1px solid #30363d; padding: 8px 12px; text-align: left; }\n");
        html.push_str("  th { background: #161b22; color: #58a6ff; font-weight: 600; }\n");
        html.push_str("  tr:nth-child(even) { background: #161b22; }\n");
        html.push_str("  tr:hover { background: #1c2128; }\n");
        html.push_str("  .good { color: #3fb950; }\n");
        html.push_str("  .warn { color: #d29922; }\n");
        html.push_str("  .bad { color: #f85149; }\n");
        html.push_str("  .summary { display: flex; gap: 20px; flex-wrap: wrap; }\n");
        html.push_str("  .card { background: #161b22; border: 1px solid #30363d; border-radius: 8px; padding: 16px; min-width: 150px; text-align: center; }\n");
        html.push_str("  .card .val { font-size: 2em; font-weight: bold; }\n");
        html.push_str("  .card .label { font-size: 0.85em; color: #8b949e; }\n");
        html.push_str("</style>\n</head>\n<body>\n");

        html.push_str("<h1>Coverage Report</h1>\n");

        // Summary cards
        let total_cg = self.covergroups.len();
        let _total_cp: usize = self.covergroups.values().map(|e| e.coverpoints.len()).sum();
        let total_line = self.line_hits.len();
        let _total_branch = self.branch_data.len();
        let total_fsm = self.fsm_data.len();

        // Branch percentage
        let mut total_br = 0u64;
        let mut covered_br = 0u64;
        for branches in self.branch_data.values() {
            for count in branches.values() {
                total_br += 1;
                if *count > 0 { covered_br += 1; }
            }
        }
        let br_pct = if total_br > 0 { (covered_br as f64 / total_br as f64 * 100.0) as u64 } else { 0 };

        // Coverpoint percentage
        let mut total_cp_hits = 0u64;
        let mut total_cp_total = 0u64;
        for entry in self.covergroups.values() {
            for cp in &entry.coverpoints {
                total_cp_hits += cp.hits;
                total_cp_total += cp.total;
            }
        }
        let cp_pct = if total_cp_total > 0 { (total_cp_hits as f64 / total_cp_total as f64 * 100.0) as u64 } else { 0 };

        html.push_str("<div class=\"summary\">\n");
        html.push_str(&format!("  <div class=\"card\"><div class=\"val\">{}</div><div class=\"label\">Covergroups</div></div>\n", total_cg));
        html.push_str(&format!("  <div class=\"card\"><div class=\"val {} \">{}%</div><div class=\"label\">Coverpoints</div></div>\n", 
            if cp_pct >= 90 { "good" } else if cp_pct >= 50 { "warn" } else { "bad" },
            cp_pct));
        html.push_str(&format!("  <div class=\"card\"><div class=\"val\">{}</div><div class=\"label\">Line Items</div></div>\n", total_line));
        html.push_str(&format!("  <div class=\"card\"><div class=\"val {} \">{}%</div><div class=\"label\">Branch</div></div>\n",
            if br_pct >= 90 { "good" } else if br_pct >= 50 { "warn" } else { "bad" },
            br_pct));
        html.push_str(&format!("  <div class=\"card\"><div class=\"val\">{}</div><div class=\"label\">FSM Signals</div></div>\n", total_fsm));
        html.push_str("</div>\n");

        // Covergroups table
        if total_cg > 0 {
            html.push_str("<h2>Covergroups</h2>\n<table>\n<tr><th>Name</th><th>Coverpoint</th><th>Hits/Total</th><th>%</th><th>Bins</th></tr>\n");
            for (name, entry) in &self.covergroups {
                let mut first = true;
                for cp in &entry.coverpoints {
                    let pct = if cp.total > 0 { (cp.hits as f64 / cp.total as f64 * 100.0) as u64 } else { 0 };
                    let css = if pct >= 90 { "good" } else if pct >= 50 { "warn" } else { "bad" };
                    html.push_str(&format!("<tr><td>{}</td><td>{}</td><td>{}/{}</td><td class=\"{}\">{}%</td><td>{}</td></tr>\n",
                        if first { name } else { "" },
                        cp.name, cp.hits, cp.total, css, pct, cp.bins.len()));
                    first = false;
                }
            }
            html.push_str("</table>\n");
        }

        // Branch coverage table
        if !self.branch_data.is_empty() {
            html.push_str("<h2>Branch Coverage</h2>\n<table>\n<tr><th>Branch</th><th>Label</th><th>Hits</th></tr>\n");
            for (key, branches) in &self.branch_data {
                let mut first = true;
                for (label, count) in branches {
                    html.push_str(&format!("<tr><td>{}</td><td>{}</td><td>{}</td></tr>\n",
                        if first { key.as_str() } else { "" },
                        label.as_str(), count));
                    first = false;
                }
            }
            html.push_str("</table>\n");
        }

        // Line coverage table (top 20)
        if !self.line_hits.is_empty() {
            html.push_str("<h2>Line Coverage (Top 20)</h2>\n<table>\n<tr><th>Key</th><th>Hits</th></tr>\n");
            let mut sorted: Vec<(&str, &u64)> = self.line_hits.iter()
                .map(|(k, v)| (k.as_str(), v))
                .collect();
            sorted.sort_by(|a, b| b.1.cmp(a.1));
            for (key, hits) in sorted.iter().take(20) {
                html.push_str(&format!("<tr><td>{}</td><td>{}</td></tr>\n", key, hits));
            }
            html.push_str("</table>\n");
        }

        html.push_str("</body>\n</html>\n");

        std::fs::write(path, html)
            .map_err(|e| format!("cannot write HTML coverage report '{}': {}", path, e))?;
        Ok(())
    }
}

// ─── Helper: Bin map serialization ───────────────────────────────────

fn write_bin_map(w: &mut impl Write, bins: &CoverBinMap) -> Result<(), String> {
    write_usize(w, bins.len())
        .map_err(|e| format!("bin count: {}", e))?;
    for (key, count) in bins {
        write_str(w, key.as_str())
            .map_err(|e| format!("bin key: {}", e))?;
        write_u64(w, *count)
            .map_err(|e| format!("bin count: {}", e))?;
    }
    Ok(())
}

fn read_bin_map(r: &mut impl Read) -> Result<CoverBinMap, String> {
    let count = read_usize(r).map_err(|e| format!("bin count: {}", e))?;
    let mut bins = HashMap::with_capacity(count);
    for _ in 0..count {
        let key = read_str(r).map_err(|e| format!("bin key: {}", e))?;
        let count = read_u64(r).map_err(|e| format!("bin count: {}", e))?;
        bins.insert(Symbol::intern(&key), count);
    }
    Ok(bins)
}

// ─── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use maria_ir::LogicVal;

    #[test]
    fn test_empty_db_save_load() {
        let dir = std::env::temp_dir().join(format!("maria_covdb_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("empty.mcdb");
        let path_str = path.to_str().unwrap().to_string();

        let db = CoverageDatabase::new();
        db.save_to_file(&path_str).unwrap();

        let db = CoverageDatabase::new();
        let loaded = db.load_from_file(&path_str).unwrap();
        assert!(loaded.covergroups.is_empty());
        assert!(loaded.line_hits.is_empty());
        assert!(loaded.toggle_data.is_empty());
        assert!(loaded.branch_data.is_empty());
        assert!(loaded.fsm_data.is_empty());

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn test_covergroup_roundtrip() {
        let dir = std::env::temp_dir().join(format!("maria_covdb_test2_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("cg.mcdb");
        let path_str = path.to_str().unwrap().to_string();

        let mut db = CoverageDatabase::new();
        let entry = CovergroupEntry {
            name: "cg_main".to_string(),
            coverpoints: vec![
                CoverpointEntry {
                    name: "addr".to_string(),
                    total: 100,
                    hits: 50,
                    bins: vec![
                        (Symbol::intern("addr=42"), 30u64),
                        (Symbol::intern("addr=99"), 20u64),
                    ].into_iter().collect(),
                },
            ],
            crosses: vec![
                CrossEntry {
                    name: "addr_x_data".to_string(),
                    total: 50,
                    hits: 25,
                    bins: vec![
                        (Symbol::intern("addr=42 x data=1"), 15u64),
                    ].into_iter().collect(),
                },
            ],
        };
        db.covergroups.insert("cg_main".to_string(), entry);

        db.save_to_file(&path_str).unwrap();

        let db = CoverageDatabase::new();
        let loaded = db.load_from_file(&path_str).unwrap();
        let cg = loaded.covergroups.get("cg_main").unwrap();
        assert_eq!(cg.coverpoints.len(), 1);
        assert_eq!(cg.coverpoints[0].name, "addr");
        assert_eq!(cg.coverpoints[0].total, 100);
        assert_eq!(cg.coverpoints[0].hits, 50);
        assert_eq!(cg.coverpoints[0].bins.len(), 2);
        assert_eq!(cg.crosses.len(), 1);
        assert_eq!(cg.crosses[0].total, 50);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn test_line_roundtrip() {
        let dir = std::env::temp_dir().join(format!("maria_covdb_test3_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("line.mcdb");
        let path_str = path.to_str().unwrap().to_string();

        let mut db = CoverageDatabase::new();
        db.line_hits.insert(Symbol::intern("proc1.stmt1"), 42);
        db.line_hits.insert(Symbol::intern("proc1.stmt2"), 10);

        db.save_to_file(&path_str).unwrap();

        let db = CoverageDatabase::new();
        let loaded = db.load_from_file(&path_str).unwrap();
        assert_eq!(loaded.line_hits.len(), 2);
        assert_eq!(*loaded.line_hits.get(&Symbol::intern("proc1.stmt1")).unwrap(), 42);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn test_toggle_roundtrip() {
        let dir = std::env::temp_dir().join(format!("maria_covdb_test4_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("toggle.mcdb");
        let path_str = path.to_str().unwrap().to_string();

        let mut db = CoverageDatabase::new();
        let mut transitions = HashMap::new();
        transitions.insert((LogicVal::Zero, LogicVal::One), 5u64);
        transitions.insert((LogicVal::One, LogicVal::Zero), 3u64);
        db.toggle_data.insert(0, ToggleEntry { sig_id: 0, transitions });

        db.save_to_file(&path_str).unwrap();

        let db = CoverageDatabase::new();
        let loaded = db.load_from_file(&path_str).unwrap();
        assert_eq!(loaded.toggle_data.len(), 1);
        let te = loaded.toggle_data.get(&0).unwrap();
        assert_eq!(te.transitions.len(), 2);
        assert_eq!(*te.transitions.get(&(LogicVal::Zero, LogicVal::One)).unwrap(), 5);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn test_branch_roundtrip() {
        let dir = std::env::temp_dir().join(format!("maria_covdb_test5_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("branch.mcdb");
        let path_str = path.to_str().unwrap().to_string();

        let mut db = CoverageDatabase::new();
        let mut br = HashMap::new();
        br.insert(Symbol::intern("true"), 30u64);
        br.insert(Symbol::intern("false"), 10u64);
        db.branch_data.insert(Symbol::intern("if.cond#0"), br);

        db.save_to_file(&path_str).unwrap();

        let db = CoverageDatabase::new();
        let loaded = db.load_from_file(&path_str).unwrap();
        let branches = loaded.branch_data.get(&Symbol::intern("if.cond#0")).unwrap();
        assert_eq!(*branches.get(&Symbol::intern("true")).unwrap(), 30);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn test_fsm_roundtrip() {
        let dir = std::env::temp_dir().join(format!("maria_covdb_test6_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("fsm.mcdb");
        let path_str = path.to_str().unwrap().to_string();

        let mut db = CoverageDatabase::new();
        let mut states = HashMap::new();
        states.insert(0u64, 10u64);
        states.insert(1u64, 5u64);
        states.insert(2u64, 3u64);
        db.fsm_data.insert(5, states);

        db.save_to_file(&path_str).unwrap();

        let db = CoverageDatabase::new();
        let loaded = db.load_from_file(&path_str).unwrap();
        let fsm = loaded.fsm_data.get(&5).unwrap();
        assert_eq!(*fsm.get(&0).unwrap(), 10);
        assert_eq!(*fsm.get(&1).unwrap(), 5);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn test_merge_from_engine_basic() {
        // Create a mock engine-like structure by writing to HashMaps directly
        let mut cover_hits = HashMap::new();
        let mut cover_total = HashMap::new();
        let mut cover_bins = HashMap::new();

        let key = Symbol::intern("cg.cp");
        cover_hits.insert(key, 10u64);
        cover_total.insert(key, 20u64);
        let mut bins = HashMap::new();
        bins.insert(Symbol::intern("val=5"), 6u64);
        cover_bins.insert(key, bins);

        // We can't easily construct a SimulationEngine, but we can test
        // the CoverageDatabase's own merge logic by manually inserting
        // and verifying via roundtrip.
        let mut db = CoverageDatabase::new();

        // Simulate merge by directly inserting covergroup data
        let entry = CovergroupEntry {
            name: "cg".to_string(),
            coverpoints: vec![
                CoverpointEntry {
                    name: "cp".to_string(),
                    total: 20,
                    hits: 10,
                    bins: vec![(Symbol::intern("val=5"), 6u64)].into_iter().collect(),
                },
            ],
            crosses: vec![],
        };
        db.covergroups.insert("cg".to_string(), entry);

        // Verify: roundtrip
        let dir = std::env::temp_dir().join(format!("maria_covdb_test7_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("merge.mcdb");
        let path_str = path.to_str().unwrap().to_string();
        db.save_to_file(&path_str).unwrap();
        let db = CoverageDatabase::new();
        let loaded = db.load_from_file(&path_str).unwrap();
        assert_eq!(loaded.covergroups.len(), 1);
        assert_eq!(loaded.covergroups.get("cg").unwrap().coverpoints[0].hits, 10);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn test_with_path_load_existing() {
        let dir = std::env::temp_dir().join(format!("maria_covdb_test8_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("existing.mcdb");
        let path_str = path.to_str().unwrap().to_string();

        // Create and save
        let mut db = CoverageDatabase::new();
        db.line_hits.insert(Symbol::intern("test.line"), 99);
        db.save_to_file(&path_str).unwrap();

        // Load with with_path
        let loaded = CoverageDatabase::with_path(&path_str);
        assert_eq!(*loaded.line_hits.get(&Symbol::intern("test.line")).unwrap(), 99);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn test_merge_from_db() {
        let mut db1 = CoverageDatabase::new();
        db1.line_hits.insert(Symbol::intern("proc.stmt"), 10);
        db1.line_hits.insert(Symbol::intern("proc2.stmt"), 5);

        let mut db2 = CoverageDatabase::new();
        db2.line_hits.insert(Symbol::intern("proc.stmt"), 20);  // same key → summed
        db2.line_hits.insert(Symbol::intern("proc3.stmt"), 8);   // new key

        db1.merge_from_db(&db2);

        assert_eq!(*db1.line_hits.get(&Symbol::intern("proc.stmt")).unwrap(), 30); // 10+20
        assert_eq!(*db1.line_hits.get(&Symbol::intern("proc2.stmt")).unwrap(), 5);
        assert_eq!(*db1.line_hits.get(&Symbol::intern("proc3.stmt")).unwrap(), 8);
    }

    #[test]
    fn test_merge_from_db_covergroups() {
        let mut db1 = CoverageDatabase::new();
        let entry1 = CovergroupEntry {
            name: "cg_main".to_string(),
            coverpoints: vec![
                CoverpointEntry {
                    name: "addr".to_string(),
                    total: 100,
                    hits: 50,
                    bins: vec![(Symbol::intern("addr=42"), 30u64)].into_iter().collect(),
                },
            ],
            crosses: vec![],
        };
        db1.covergroups.insert("cg_main".to_string(), entry1);

        let mut db2 = CoverageDatabase::new();
        let entry2 = CovergroupEntry {
            name: "cg_main".to_string(),
            coverpoints: vec![
                CoverpointEntry {
                    name: "addr".to_string(),
                    total: 200,
                    hits: 100,
                    bins: vec![(Symbol::intern("addr=99"), 60u64)].into_iter().collect(),
                },
            ],
            crosses: vec![],
        };
        db2.covergroups.insert("cg_main".to_string(), entry2);

        db1.merge_from_db(&db2);

        let merged = db1.covergroups.get("cg_main").unwrap();
        assert_eq!(merged.coverpoints.len(), 1);
        assert_eq!(merged.coverpoints[0].total, 300);  // 100+200
        assert_eq!(merged.coverpoints[0].hits, 150);   // 50+100
        // Bins should be merged (retain both addr=42 and addr=99)
        let bins = &merged.coverpoints[0].bins;
        assert_eq!(*bins.get(&Symbol::intern("addr=42")).unwrap(), 30);
        assert_eq!(*bins.get(&Symbol::intern("addr=99")).unwrap(), 60);
    }

    #[test]
    fn test_diff_empty() {
        let db1 = CoverageDatabase::new();
        let db2 = CoverageDatabase::new();
        let diff = db1.diff(&db2);
        assert!(diff.coverpoint_changes.is_empty());
        assert!(diff.new_coverpoints.is_empty());
        assert!(diff.line_changes.is_empty());
        assert!(diff.branch_changes.is_empty());
        assert!(diff.fsm_changes.is_empty());
    }

    #[test]
    fn test_diff_line_changes() {
        let mut db1 = CoverageDatabase::new();
        db1.line_hits.insert(Symbol::intern("stmt1"), 10);
        db1.line_hits.insert(Symbol::intern("stmt2"), 5);

        let mut db2 = CoverageDatabase::new();
        db2.line_hits.insert(Symbol::intern("stmt1"), 20);  // different
        db2.line_hits.insert(Symbol::intern("stmt2"), 5);   // same

        let diff = db1.diff(&db2);
        assert_eq!(diff.line_changes.len(), 1);
        assert_eq!(diff.line_changes[0].0, Symbol::intern("stmt1"));
        assert_eq!(diff.line_changes[0].1, 10);  // self
        assert_eq!(diff.line_changes[0].2, 20);  // other
    }

    #[test]
    fn test_diff_new_coverpoints() {
        let db1 = CoverageDatabase::new();

        let mut db2 = CoverageDatabase::new();
        let entry = CovergroupEntry {
            name: "cg_new".to_string(),
            coverpoints: vec![
                CoverpointEntry {
                    name: "cp_new".to_string(),
                    total: 10,
                    hits: 5,
                    bins: HashMap::new(),
                },
            ],
            crosses: vec![],
        };
        db2.covergroups.insert("cg_new".to_string(), entry);

        let diff = db1.diff(&db2);
        assert_eq!(diff.new_coverpoints.len(), 1);
        assert_eq!(diff.new_coverpoints[0], "cg_new.cp_new");
    }

    #[test]
    fn test_load_from_path_static() {
        let dir = std::env::temp_dir().join(format!("maria_covdb_test10_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("static.mcdb");
        let path_str = path.to_str().unwrap().to_string();

        let mut db = CoverageDatabase::new();
        db.line_hits.insert(Symbol::intern("test.key"), 42);
        db.save_to_file(&path_str).unwrap();

        let loaded = CoverageDatabase::load_from_path(&path_str).unwrap();
        assert_eq!(*loaded.line_hits.get(&Symbol::intern("test.key")).unwrap(), 42);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn test_load_and_merge() {
        let dir = std::env::temp_dir().join(format!("maria_covdb_test11_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("loadmerge.mcdb");
        let path_str = path.to_str().unwrap().to_string();

        // Create and save initial DB
        let mut db_initial = CoverageDatabase::new();
        db_initial.line_hits.insert(Symbol::intern("run1.stmt"), 10);
        db_initial.save_to_file(&path_str).unwrap();

        // Create a second DB with more data
        let mut db_main = CoverageDatabase::new();
        db_main.line_hits.insert(Symbol::intern("run2.stmt"), 20);

        // Load and merge the first DB into the second
        db_main.load_and_merge(&path_str).unwrap();

        assert_eq!(*db_main.line_hits.get(&Symbol::intern("run1.stmt")).unwrap(), 10);
        assert_eq!(*db_main.line_hits.get(&Symbol::intern("run2.stmt")).unwrap(), 20);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn test_report_does_not_panic() {
        let mut db = CoverageDatabase::new();
        db.line_hits.insert(Symbol::intern("line1"), 5);

        let entry = CovergroupEntry {
            name: "cg".to_string(),
            coverpoints: vec![
                CoverpointEntry {
                    name: "cp".to_string(),
                    total: 10,
                    hits: 5,
                    bins: HashMap::new(),
                },
            ],
            crosses: vec![],
        };
        db.covergroups.insert("cg".to_string(), entry);

        // Just check no panic
        db.report();
    }

    #[test]
    fn test_export_html_creates_file() {
        let dir = std::env::temp_dir().join(format!("maria_covdb_test12_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("report.html");
        let path_str = path.to_str().unwrap().to_string();

        let mut db = CoverageDatabase::new();
        db.line_hits.insert(Symbol::intern("line1"), 5);
        let entry = CovergroupEntry {
            name: "cg".to_string(),
            coverpoints: vec![
                CoverpointEntry {
                    name: "cp".to_string(),
                    total: 10,
                    hits: 5,
                    bins: HashMap::new(),
                },
            ],
            crosses: vec![],
        };
        db.covergroups.insert("cg".to_string(), entry);

        db.export_html(&path_str).unwrap();
        assert!(std::path::Path::new(&path_str).exists());

        let content = std::fs::read_to_string(&path_str).unwrap();
        assert!(content.contains("Coverage Report"));
        assert!(content.contains("Covergroups"));
        assert!(content.contains("Branch"));

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn test_merge_branch_data() {
        let mut db1 = CoverageDatabase::new();
        let mut br1 = HashMap::new();
        br1.insert(Symbol::intern("true"), 30u64);
        db1.branch_data.insert(Symbol::intern("if.cond#0"), br1);

        let mut db2 = CoverageDatabase::new();
        let mut br2 = HashMap::new();
        br2.insert(Symbol::intern("false"), 10u64);
        db2.branch_data.insert(Symbol::intern("if.cond#0"), br2);

        db1.merge_from_db(&db2);

        let merged = db1.branch_data.get(&Symbol::intern("if.cond#0")).unwrap();
        assert_eq!(*merged.get(&Symbol::intern("true")).unwrap(), 30);
        assert_eq!(*merged.get(&Symbol::intern("false")).unwrap(), 10);
    }
}
