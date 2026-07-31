//! SignalHistoryStore — memory-managed signal history with disk spill.
//!
//! Maintains a configurable in-memory ring buffer per signal. When the buffer
//! fills, oldest entries are spilled to a disk file instead of being discarded.
//! Queries transparently merge data from memory + disk.
//!
//! Spill file format:
//!   [magic: b"HIST"][version: u32][total_count: u64]
//!   Repeated entry blocks: [name_len: u64][name_bytes][time: u64][LogicVec binary]

use std::collections::{HashMap, VecDeque};
use std::io::{self, BufReader, BufWriter, Read, Seek, Write};
use std::path::PathBuf;

use crate::ir::{LogicVal, LogicVec};
use crate::Symbol;

// ─── Binary I/O helpers (mirror checkpoint.rs) ───

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
        _ => Err(io::Error::new(io::ErrorKind::InvalidData, "invalid LogicVal byte")),
    }
}

fn write_logic_vec<W: Write>(w: &mut W, lv: &LogicVec) -> io::Result<()> {
    write_usize(w, lv.width)?;
    write_usize(w, lv.bits.len())?;
    for b in &lv.bits {
        write_logic_val(w, b)?;
    }
    Ok(())
}

fn read_logic_vec<R: Read>(r: &mut R) -> io::Result<LogicVec> {
    let width = read_usize(r)?;
    let len = read_usize(r)?;
    let mut bits = Vec::with_capacity(len);
    for _ in 0..len {
        bits.push(read_logic_val(r)?);
    }
    Ok(LogicVec { bits, width })
}

// ─── Spill Entry ───

/// A single history entry stored on disk.
#[derive(Debug, Clone)]
struct SpillEntry {
    pub name: Symbol,
    pub time: u64,
    pub value: LogicVec,
}

// ─── SignalHistoryStore ───

/// Memory-managed signal history store with optional disk spill.
///
/// # Memory Management
/// - Each signal gets a ring buffer of up to `memory_per_signal` entries.
/// - When the buffer is full, the OLDEST entry is spilled to disk (not discarded).
/// - Queries transparently merge memory + disk data.
///
/// # Disk Spill Format
/// Single file append-only. Format: magic + version + total_count + entries.
/// Entries are scanned linearly on query (acceptable for debugger use).
pub struct SignalHistoryStore {
    /// In-memory ring buffer per signal
    memory: HashMap<Symbol, VecDeque<(u64, LogicVec)>>,
    /// Max in-memory entries per signal before spill
    memory_per_signal: usize,
    /// Disk spill file path (None = no spill, old entries discarded)
    spill_path: Option<PathBuf>,
    /// Disk spill writer (append-only, created on first spill)
    spill_writer: Option<BufWriter<std::fs::File>>,
    /// Total entries spilled to disk (for stats/metrics)
    total_spilled: u64,
    /// Total entries in memory across all signals
    total_memory: u64,
    /// Whether spill is enabled
    spill_enabled: bool,
}

impl SignalHistoryStore {
    /// Create a new store with given memory budget per signal.
    /// If `spill_path` is Some, old entries are spilled to disk instead of dropped.
    pub fn new(memory_per_signal: usize, spill_path: Option<PathBuf>) -> Self {
        let spill_enabled = spill_path.is_some();
        SignalHistoryStore {
            memory: HashMap::new(),
            memory_per_signal,
            spill_path,
            spill_writer: None,
            total_spilled: 0,
            total_memory: 0,
            spill_enabled,
        }
    }

    /// Create a store with default settings: 10K in-memory entries, no disk spill.
    pub fn default() -> Self {
        Self::new(10000, None)
    }

    /// Enable disk spill with the given path.
    pub fn enable_spill(&mut self, path: PathBuf) {
        self.spill_path = Some(path);
        self.spill_enabled = true;
    }

    /// Disable disk spill (entries will be discarded when memory is full).
    pub fn disable_spill(&mut self) {
        self.spill_path = None;
        self.spill_enabled = false;
        self.spill_writer = None;
    }

    /// Record a signal value at a given time.
    pub fn record(&mut self, time: u64, name: Symbol, value: LogicVec) {
        // Extract oldest entry BEFORE mutable borrow on self.memory
        let spill_candidate = if self.spill_enabled {
            let deque = self.memory.entry(name).or_insert_with(|| {
                VecDeque::with_capacity(self.memory_per_signal.min(1024))
            });
            if deque.len() >= self.memory_per_signal {
                deque.pop_front()
            } else {
                None
            }
        } else {
            let deque = self.memory.entry(name).or_insert_with(|| {
                VecDeque::with_capacity(self.memory_per_signal.min(1024))
            });
            if deque.len() >= self.memory_per_signal {
                deque.pop_front()
            } else {
                None
            }
        };

        // Spill to disk (separate from memory borrow)
        if let Some((old_time, old_val)) = spill_candidate {
            if let Err(e) = self.spill_entry(name, old_time, &old_val) {
                eprintln!("warning: signal history spill failed: {}", e);
            }
            self.total_spilled += 1;
        }

        // Now insert new entry (fresh borrow)
        let deque = self.memory.entry(name).or_insert_with(|| {
            VecDeque::with_capacity(self.memory_per_signal.min(1024))
        });
        deque.push_back((time, value));
        self.total_memory = self.memory.values().map(|d| d.len() as u64).sum();
    }

    /// Get all history for a signal (memory + disk merged, sorted by time).
    pub fn get_history(&self, name: &Symbol) -> Vec<(u64, LogicVec)> {
        let mut result = Vec::new();

        // Read from memory
        if let Some(deque) = self.memory.get(name) {
            result.extend(deque.iter().cloned());
        }

        // Read from disk
        if let Some(ref path) = self.spill_path {
            if path.exists() {
                if let Ok(entries) = self.read_disk_history(name) {
                    result.extend(entries);
                }
            }
        }

        // Sort by time (stable: disk entries come first, then memory)
        result.sort_by_key(|(t, _)| *t);
        result.dedup_by_key(|(t, _)| *t); // Remove duplicate times
        result
    }

    /// Get history for a signal within a time range.
    pub fn get_history_range(&self, name: &Symbol, from: u64, to: u64) -> Vec<(u64, LogicVec)> {
        self.get_history(name)
            .into_iter()
            .filter(|(t, _)| *t >= from && *t <= to)
            .collect()
    }

    /// Get the latest value for a signal (fast: checks memory first, then disk).
    pub fn get_latest(&self, name: &Symbol) -> Option<(u64, LogicVec)> {
        // Check memory first (most recent entries are in memory)
        if let Some(deque) = self.memory.get(name) {
            if let Some(last) = deque.back() {
                return Some(last.clone());
            }
        }
        // Fall back to disk scan
        if let Some(ref path) = self.spill_path {
            if path.exists() {
                if let Ok(mut entries) = self.read_disk_history(name) {
                    entries.sort_by_key(|(t, _)| *t);
                    return entries.last().cloned();
                }
            }
        }
        None
    }

    /// Get history length for a signal (memory + disk).
    pub fn history_len(&self, name: &Symbol) -> usize {
        let mem_len = self.memory.get(name).map(|d| d.len()).unwrap_or(0);
        let disk_len = if let Some(ref path) = self.spill_path {
            if path.exists() {
                if let Ok(entries) = self.read_disk_history(name) {
                    entries.len()
                } else {
                    0
                }
            } else {
                0
            }
        } else {
            0
        };
        mem_len + disk_len
    }

    /// Clear all history for a signal.
    pub fn clear(&mut self, name: &Symbol) {
        self.memory.remove(name);
    }

    /// Clear ALL history (memory + disk).
    pub fn clear_all(&mut self) {
        self.memory.clear();
        self.total_memory = 0;
        self.total_spilled = 0;
        // Remove spill file
        if let Some(ref path) = self.spill_path {
            if path.exists() {
                let _ = std::fs::remove_file(path);
            }
        }
        self.spill_writer = None;
    }

    /// Get stats for debugging/profiling.
    pub fn stats(&self) -> SignalHistoryStats {
        SignalHistoryStats {
            total_signals: self.memory.len(),
            total_memory_entries: self.total_memory,
            total_spilled_entries: self.total_spilled,
            memory_per_signal: self.memory_per_signal,
            spill_enabled: self.spill_enabled,
        }
    }

    // ─── Private: Disk I/O ───

    /// Spill a single entry to disk.
    fn spill_entry(&mut self, name: Symbol, time: u64, value: &LogicVec) -> io::Result<()> {
        let path = match &self.spill_path {
            Some(p) => p.clone(),
            None => return Err(io::Error::new(io::ErrorKind::Other, "spill not configured")),
        };

        // Initialize writer on first use (check metadata BEFORE creating BufWriter)
        if self.spill_writer.is_none() {
            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)?;
            let is_new = file.metadata()?.len() == 0;
            let mut writer = BufWriter::new(file);

            if is_new {
                writer.write_all(b"HIST")?;
                write_u32(&mut writer, 1)?; // version
            }
            self.spill_writer = Some(writer);
        }

        if let Some(ref mut writer) = self.spill_writer {
            write_str(writer, name.as_str())?;
            write_u64(writer, time)?;
            write_logic_vec(writer, value)?;
            writer.flush()?;
        }

        Ok(())
    }

    /// Read all disk entries for a specific signal.
    fn read_disk_history(&self, target: &Symbol) -> io::Result<Vec<(u64, LogicVec)>> {
        let path = match &self.spill_path {
            Some(p) => p,
            None => return Ok(Vec::new()),
        };

        if !path.exists() {
            return Ok(Vec::new());
        }

        let file = std::fs::File::open(path)?;
        let file_len = file.metadata()?.len();
        if file_len < 12 {
            return Ok(Vec::new()); // Too small for valid header
        }

        let mut reader = BufReader::new(file);

        // Read and verify header
        let mut magic = [0u8; 4];
        reader.read_exact(&mut magic)?;
        if &magic != b"HIST" {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "bad history magic"));
        }
        let _version = read_u32(&mut reader)?;
        // Note: no total_count field — entries follow immediately after version

        let target_str = target.as_str();
        let mut entries = Vec::new();

        // Read entries until EOF
        loop {
            // Peek if we can read another name length
            let pos_before = reader.stream_position()?;
            if pos_before >= file_len {
                break;
            }

            // Read name
            let name_len = match read_usize(&mut reader) {
                Ok(n) => n,
                Err(_) => break, // EOF
            };
            let mut name_buf = vec![0u8; name_len];
            if reader.read_exact(&mut name_buf).is_err() {
                break;
            }
            let name_str = String::from_utf8_lossy(&name_buf);

            // Read time
            let time = match read_u64(&mut reader) {
                Ok(t) => t,
                Err(_) => break,
            };

            // Read LogicVec
            let value = match read_logic_vec(&mut reader) {
                Ok(v) => v,
                Err(_) => break,
            };

            if name_str == target_str {
                entries.push((time, value));
            }
        }

        Ok(entries)
    }

    /// Flush spill writer.
    pub fn flush(&mut self) -> io::Result<()> {
        if let Some(ref mut writer) = self.spill_writer {
            writer.flush()?;
        }
        Ok(())
    }

    /// Dump all memory entries as HashMap (for checkpoint serialization).
    /// Does NOT include spilled entries (they are on disk and will persist).
    pub fn dump_memory_entries(&self) -> HashMap<Symbol, VecDeque<(u64, LogicVec)>> {
        self.memory.clone()
    }

    /// Restore memory entries from a HashMap (from checkpoint restore).
    pub fn restore_memory_entries(&mut self, entries: HashMap<Symbol, VecDeque<(u64, LogicVec)>>) {
        for (sym, deque) in entries {
            for (time, val) in deque {
                self.record(time, sym, val);
            }
        }
    }
}

// ─── Stats ───

#[derive(Debug, Clone)]
pub struct SignalHistoryStats {
    pub total_signals: usize,
    pub total_memory_entries: u64,
    pub total_spilled_entries: u64,
    pub memory_per_signal: usize,
    pub spill_enabled: bool,
}

impl std::fmt::Display for SignalHistoryStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "SignalHistory: {} signals, {} mem entries, {} spilled, {} mem/sig, spill={}",
            self.total_signals,
            self.total_memory_entries,
            self.total_spilled_entries,
            self.memory_per_signal,
            if self.spill_enabled { "ON" } else { "OFF" }
        )
    }
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_record_and_get() {
        let mut store = SignalHistoryStore::new(100, None);
        let sig = Symbol::intern("clk");

        store.record(0, sig, LogicVec::from_u64(0, 1));
        store.record(1, sig, LogicVec::from_u64(1, 1));
        store.record(2, sig, LogicVec::from_u64(0, 1));

        let history = store.get_history(&sig);
        assert_eq!(history.len(), 3);
        assert_eq!(history[0].0, 0);
        assert_eq!(history[0].1.to_u64(), 0);
        assert_eq!(history[1].0, 1);
        assert_eq!(history[1].1.to_u64(), 1);
        assert_eq!(history[2].0, 2);
        assert_eq!(history[2].1.to_u64(), 0);
    }

    #[test]
    fn test_discard_when_full_no_spill() {
        let mut store = SignalHistoryStore::new(3, None); // only 3 in memory
        let sig = Symbol::intern("data");

        store.record(0, sig, LogicVec::from_u64(10, 8));
        store.record(1, sig, LogicVec::from_u64(20, 8));
        store.record(2, sig, LogicVec::from_u64(30, 8));
        store.record(3, sig, LogicVec::from_u64(40, 8)); // should discard entry 0

        let history = store.get_history(&sig);
        assert_eq!(history.len(), 3); // Only 3 entries (entry at time 0 discarded)
        assert_eq!(history[0].0, 1); // First entry is time 1
        assert_eq!(history[0].1.to_u64(), 20);
    }

    #[test]
    fn test_spill_to_disk() {
        let dir = std::env::temp_dir().join(format!("maria_sh_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let spill_path = dir.join("spill.hist");

        let mut store = SignalHistoryStore::new(3, Some(spill_path.clone()));
        let sig = Symbol::intern("wide_bus");

        store.record(0, sig, LogicVec::from_u64(0xA, 8));
        store.record(1, sig, LogicVec::from_u64(0xB, 8));
        store.record(2, sig, LogicVec::from_u64(0xC, 8));
        store.record(3, sig, LogicVec::from_u64(0xD, 8)); // time 0 spills to disk

        let history = store.get_history(&sig);
        assert_eq!(history.len(), 4);
        // Must have all 4 entries
        let times: Vec<u64> = history.iter().map(|(t, _)| *t).collect();
        assert_eq!(times, vec![0, 1, 2, 3]);
        // Verify values
        assert_eq!(history[0].1.to_u64(), 0xA); // from disk
        assert_eq!(history[1].1.to_u64(), 0xB); // from memory
        assert_eq!(history[2].1.to_u64(), 0xC);
        assert_eq!(history[3].1.to_u64(), 0xD);

        // Verify spill file exists and has correct stats
        assert!(spill_path.exists());
        assert_eq!(store.total_spilled, 1);

        let _ = std::fs::remove_file(&spill_path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn test_multi_signal_spill() {
        let dir = std::env::temp_dir().join(format!("maria_sh_test2_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let spill_path = dir.join("multi.hist");

        let mut store = SignalHistoryStore::new(2, Some(spill_path.clone()));
        let sig_a = Symbol::intern("sig_a");
        let sig_b = Symbol::intern("sig_b");

        store.record(0, sig_a, LogicVec::from_u64(1, 4));
        store.record(0, sig_b, LogicVec::from_u64(10, 4));
        store.record(1, sig_a, LogicVec::from_u64(2, 4));
        store.record(1, sig_b, LogicVec::from_u64(20, 4));
        store.record(2, sig_a, LogicVec::from_u64(3, 4)); // spills sig_a time 0
        store.record(2, sig_b, LogicVec::from_u64(30, 4)); // spills sig_b time 0

        let hist_a = store.get_history(&sig_a);
        let hist_b = store.get_history(&sig_b);

        assert_eq!(hist_a.len(), 3);
        assert_eq!(hist_b.len(), 3);
        assert_eq!(hist_a[0].1.to_u64(), 1); // from disk
        assert_eq!(hist_b[0].1.to_u64(), 10); // from disk

        assert_eq!(store.total_spilled, 2);

        let _ = std::fs::remove_file(&spill_path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn test_get_history_range() {
        let mut store = SignalHistoryStore::new(100, None);
        let sig = Symbol::intern("counter");

        for i in 0..10 {
            store.record(i, sig, LogicVec::from_u64(i * 10, 16));
        }

        let range = store.get_history_range(&sig, 3, 7);
        assert_eq!(range.len(), 5);
        assert_eq!(range[0].0, 3);
        assert_eq!(range[0].1.to_u64(), 30);
        assert_eq!(range[4].0, 7);
        assert_eq!(range[4].1.to_u64(), 70);
    }

    #[test]
    fn test_get_latest() {
        let mut store = SignalHistoryStore::new(100, None);
        let sig = Symbol::intern("q");

        store.record(0, sig, LogicVec::from_u64(0, 4));
        store.record(5, sig, LogicVec::from_u64(5, 4));
        store.record(10, sig, LogicVec::from_u64(10, 4));

        let latest = store.get_latest(&sig).unwrap();
        assert_eq!(latest.0, 10);
        assert_eq!(latest.1.to_u64(), 10);
    }

    #[test]
    fn test_clear() {
        let mut store = SignalHistoryStore::new(100, None);
        let sig = Symbol::intern("temp");

        store.record(0, sig, LogicVec::from_u64(42, 8));
        assert_eq!(store.history_len(&sig), 1);

        store.clear(&sig);
        assert_eq!(store.history_len(&sig), 0);
    }

    #[test]
    fn test_stats() {
        let store = SignalHistoryStore::new(5000, None);
        let stats = store.stats();
        assert_eq!(stats.memory_per_signal, 5000);
        assert!(!stats.spill_enabled);
    }
}
