//! CSV Waveform Writer — export signal values to CSV for analysis/diff.
//!
//! Format:
//!   Time, signal1, signal2, ...
//!   0, 0, x, ...
//!   1, 1, 0, ...
//!
//! Setiap baris = satu time step. Nilai ditulis sebagai hex (untuk multi-bit)
//! atau 0/1/x/z (untuk single-bit).
//!
//! Gunakan `--waveform-csv <path>` untuk mengaktifkan.

use maria_ir::{IrDesign, LogicVec};

/// CSV waveform writer.
#[derive(Debug)]
pub struct CsvWaveWriter {
    /// Output file path
    path: String,
    /// Buffered CSV content
    content: String,
    /// Signal names (header row)
    signal_names: Vec<String>,
    /// Whether CSV writing is enabled
    pub enabled: bool,
    /// Whether header has been written
    header_written: bool,
}

impl CsvWaveWriter {
    /// Create a new CSV waveform writer.
    pub fn new(path: &str, design: &IrDesign) -> Result<Self, String> {
        // Collect signal names
        let signal_names: Vec<String> = design
            .top
            .signals
            .iter()
            .map(|s| s.name.to_string())
            .collect();

        if signal_names.is_empty() {
            return Err("no signals to dump".to_string());
        }

        let mut writer = CsvWaveWriter {
            path: path.to_string(),
            content: String::new(),
            signal_names,
            enabled: true,
            header_written: false,
        };

        // Write header immediately
        writer.write_header();

        Ok(writer)
    }

    /// Write CSV header row.
    fn write_header(&mut self) {
        if self.header_written {
            return;
        }
        self.content.push_str("Time");
        for name in &self.signal_names {
            self.content.push(',');
            self.content.push_str(name);
        }
        self.content.push('\n');
        self.header_written = true;
    }

    /// Convert a LogicVec to CSV string (hex for multi-bit, char for single-bit).
    fn vec_to_csv(val: &LogicVec) -> String {
        if val.width <= 4 {
            // Single bit or small: use char representation
            if val.width == 1 {
                match val.bits.first() {
                    Some(maria_ir::LogicVal::Zero) => "0".to_string(),
                    Some(maria_ir::LogicVal::One) => "1".to_string(),
                    Some(maria_ir::LogicVal::X) => "x".to_string(),
                    Some(maria_ir::LogicVal::Z) => "z".to_string(),
                    None => "x".to_string(),
                }
            } else {
                // Up to 4 bits: use hex
                let u64_val = val.to_u64();
                let hex_width = val.width.div_ceil(4);
                format!("{:0width$x}", u64_val, width = hex_width)
            }
        } else {
            // Multi-bit: use hex
            let u64_val = val.to_u64();
            let hex_width = val.width.div_ceil(4);
            format!("{:0width$x}", u64_val, width = hex_width)
        }
    }

    /// Write one row of signal values at current time.
    pub fn dump_state(&mut self, time: u64, state: &[LogicVec]) -> Result<(), String> {
        if !self.enabled {
            return Ok(());
        }

        self.content.push_str(&time.to_string());
        for sig_val in state {
            self.content.push(',');
            self.content.push_str(&Self::vec_to_csv(sig_val));
        }
        self.content.push('\n');

        // Flush periodically to avoid memory issues
        if self.content.len() > 1024 * 1024 {
            // 1MB buffer
            self.flush()?;
        }

        Ok(())
    }

    /// Flush buffered content to disk.
    pub fn flush(&mut self) -> Result<(), String> {
        if self.content.is_empty() {
            return Ok(());
        }

        // Append to file
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| format!("CSV append error: {}", e))?;

        file.write_all(self.content.as_bytes())
            .map_err(|e| format!("CSV write error: {}", e))?;

        self.content.clear();
        Ok(())
    }

    /// Flush and close the writer.
    pub fn close(&mut self) -> Result<(), String> {
        self.flush()?;
        Ok(())
    }
}

impl Drop for CsvWaveWriter {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maria_core::intern::Symbol;
    use maria_ir::*;

    fn make_design() -> IrDesign {
        let mut design = IrDesign::default();
        design.top.name = Symbol::intern("test_top");
        design.top.signals = vec![
            SignalInfo {
                name: Symbol::intern("clk"),
                width: 1,
                ..Default::default()
            },
            SignalInfo {
                name: Symbol::intern("data"),
                width: 8,
                ..Default::default()
            },
        ];
        design
    }

    fn make_state() -> Vec<LogicVec> {
        vec![
            LogicVec::from_u64(0, 1),  // clk = 0
            LogicVec::from_u64(42, 8), // data = 42
        ]
    }

    #[test]
    fn test_csv_new() {
        let design = make_design();
        let dir = std::env::temp_dir().join("maria_csv_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test.csv");

        let writer = CsvWaveWriter::new(path.to_str().unwrap(), &design).unwrap();
        assert_eq!(writer.signal_names.len(), 2);
        assert!(writer.enabled);
        assert!(writer.header_written);
        assert!(writer.content.starts_with("Time,clk,data"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_csv_empty_signals() {
        let design = IrDesign::default();
        let dir = std::env::temp_dir().join("maria_csv_test2");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("empty.csv");

        let result = CsvWaveWriter::new(path.to_str().unwrap(), &design);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no signals"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_csv_dump_and_flush() {
        let design = make_design();
        let dir = std::env::temp_dir().join("maria_csv_test3");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("dump_test.csv");

        let mut writer = CsvWaveWriter::new(path.to_str().unwrap(), &design).unwrap();
        let state = make_state();

        // Dump at time 0
        writer.dump_state(0, &state).unwrap();

        // Dump at time 5 with new values
        let state2 = vec![
            LogicVec::from_u64(1, 1),   // clk = 1
            LogicVec::from_u64(255, 8), // data = 255
        ];
        writer.dump_state(5, &state2).unwrap();

        // Flush and read back
        writer.flush().unwrap();
        let content = std::fs::read_to_string(&path).unwrap();

        assert!(content.contains("Time,clk,data"), "header");
        assert!(content.contains("0,0,2a"), "time 0: clk=0, data=42=0x2a");
        assert!(content.contains("5,1,ff"), "time 5: clk=1, data=255=0xff");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_csv_vec_to_csv() {
        assert_eq!(CsvWaveWriter::vec_to_csv(&LogicVec::from_u64(0, 1)), "0");
        assert_eq!(CsvWaveWriter::vec_to_csv(&LogicVec::from_u64(1, 1)), "1");
        assert_eq!(CsvWaveWriter::vec_to_csv(&LogicVec::from_u64(10, 4)), "a");
        assert_eq!(CsvWaveWriter::vec_to_csv(&LogicVec::from_u64(255, 8)), "ff");
        assert_eq!(
            CsvWaveWriter::vec_to_csv(&LogicVec::from_u64(0xabcd, 16)),
            "abcd"
        );
    }

    #[test]
    fn test_csv_disabled_does_not_write() {
        let design = make_design();
        let dir = std::env::temp_dir().join("maria_csv_test4");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("disabled.csv");

        let mut writer = CsvWaveWriter::new(path.to_str().unwrap(), &design).unwrap();
        writer.enabled = false;

        let state = make_state();
        writer.dump_state(0, &state).unwrap();
        writer.flush().unwrap();

        // Should only have header
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content.trim(), "Time,clk,data");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
