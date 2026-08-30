use std::collections::HashMap;
use std::fs;
use std::io::Write;

use flate2::write::GzEncoder;
use flate2::Compression;
use maria_ir::{IrDesign, LogicVal};

/// Pesan untuk writer thread background (WAV-19).
enum BgMsg {
    /// Tulis byte ke file.
    Bytes(Vec<u8>),
    /// Flush buffer + kirim ack balik (sinkronisasi flush()).
    Flush(std::sync::mpsc::Sender<()>),
}

/// Writer thread background: pemilik eksklusif file; menulis semua byte
/// dari channel sampai channel tertutup, lalu flush + exit.
struct BgWriter {
    tx: Option<std::sync::mpsc::Sender<BgMsg>>,
    handle: Option<std::thread::JoinHandle<()>>,
}

/// Tujuan tulisan VCD: file langsung (default, sinkron) atau writer thread
/// background (opt-in via enable_background — dump tidak lagi blocking sim).
/// WAV-04: compressed output via GzEncoder.
enum Out {
    File(fs::File),
    Compressed(GzEncoder<fs::File>),
    Bg(BgWriter),
    /// Placeholder transien saat mengambil File dari self.out
    /// (hanya ada di dalam enable_background).
    Detached,
}

/// Pre-computed info untuk satu "dump target" — satu non-array signal atau
/// satu elemen array. Menggantikan HashMap lookup per-cycle jadi flat
/// array access (O(1) tanpa hash computation).
struct DumpTarget {
    /// VCD code (mis. "s0", "s1", ...).
    code: String,
    /// is_one_bit flag (width == 1).
    is_one_bit: bool,
    /// Index signal di design.top.signals.
    signal_idx: usize,
    /// Index elemen dalam array (None untuk non-array).
    elem_idx: Option<usize>,
    /// Lebar elemen (untuk elem_val).
    elem_width: usize,
}

/// VCD waveform writer.
pub struct VcdWriter {
    out: Out,
    /// Flat array: nilai string terakhir per dump_target — O(1) access tanpa
    /// HashMap lookup. Untuk desain 146K signal, hemat ~20-40MB vs HashMap.
    last_values: Vec<String>,
    /// Pre-computed dump targets — dihitung sekali saat write_header, lalu
    /// dipakai di setiap dump_state tanpa parse_scope/code_for_signal.
    dump_targets: Vec<DumpTarget>,
    pub enabled: bool,
    pub max_dump_size: Option<u64>,
    total_written: u64,
    /// Flush to disk every N state dumps (0 = never, 1 = every dump)
    pub stream_flush_interval: u64,
    flush_counter: u64,
    /// Fallback HashMap untuk backward-compat (reopen/header internals).
    code_by_key: HashMap<(Vec<String>, String), String>,
}

// DEBT-20: Debug konsisten — tampilkan status ringkas.
impl std::fmt::Debug for VcdWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VcdWriter")
            .field("enabled", &self.enabled)
            .field("max_dump_size", &self.max_dump_size)
            .field("total_written", &self.total_written)
            .field("stream_flush_interval", &self.stream_flush_interval)
            .field("dump_targets", &self.dump_targets.len())
            .finish()
    }
}

impl VcdWriter {
    pub fn new(path: &str, design: &IrDesign) -> Result<Self, String> {
        let file = fs::File::create(path)
            .map_err(|e| format!("cannot create VCD file '{}': {}", path, e))?;

        let mut writer = VcdWriter {
            out: Out::File(file),
            last_values: Vec::new(),
            dump_targets: Vec::new(),
            enabled: true,
            max_dump_size: None,
            total_written: 0,
            stream_flush_interval: 0, // 0 = no periodic flush (default)
            flush_counter: 0,
            code_by_key: HashMap::new(),
        };

        writer.write_header(design)?;
        Ok(writer)
    }

    /// WAV-04: Enable gzip compression for VCD output.
    /// Replaces File output with Compressed(GzEncoder) — all subsequent
    /// writes will be gzip-compressed. Compression level: default (6).
    pub fn enable_compression(&mut self) -> Result<(), String> {
        match std::mem::replace(&mut self.out, Out::Detached) {
            Out::File(file) => {
                let enc = GzEncoder::new(file, Compression::default());
                self.out = Out::Compressed(enc);
                Ok(())
            }
            other => {
                self.out = other;
                Err("enable_compression: output is not a File".to_string())
            }
        }
    }

    /// WAV-04: Create a new VcdWriter with gzip compression from the start.
    pub fn new_compressed(path: &str, design: &IrDesign) -> Result<Self, String> {
        let file = fs::File::create(path)
            .map_err(|e| format!("cannot create VCD file '{}': {}", path, e))?;
        let enc = GzEncoder::new(file, Compression::default());
        let mut writer = VcdWriter {
            out: Out::Compressed(enc),
            last_values: Vec::new(),
            dump_targets: Vec::new(),
            enabled: true,
            max_dump_size: None,
            total_written: 0,
            stream_flush_interval: 0,
            flush_counter: 0,
            code_by_key: HashMap::new(),
        };
        writer.write_header(design)?;
        Ok(writer)
    }

    pub fn reopen(
        &mut self,
        path: &str,
        design: &IrDesign,
        state: &[maria_ir::LogicVec],
    ) -> Result<(), String> {
        self.close_inner()?;
        let file = fs::File::create(path)
            .map_err(|e| format!("cannot create VCD file '{}': {}", path, e))?;
        self.out = Out::File(file);
        self.last_values.clear();
        self.dump_targets.clear();
        self.code_by_key.clear();
        self.total_written = 0;
        self.enabled = true;
        self.write_header(design)?;
        self.dump_all(design, state)
    }

    fn write_raw(&mut self, buf: &[u8]) -> Result<(), String> {
        if let Some(limit) = self.max_dump_size {
            if self.total_written + buf.len() as u64 > limit {
                self.enabled = false;
                return Ok(());
            }
        }
        match &mut self.out {
            Out::File(f) => f
                .write_all(buf)
                .map_err(|e| format!("VCD write error: {}", e))?,
            Out::Compressed(enc) => enc
                .write_all(buf)
                .map_err(|e| format!("VCD write error: {}", e))?,
            Out::Bg(bg) => {
                if let Some(tx) = &bg.tx {
                    // Kirim byte ke writer thread (non-blocking bagi sim
                    // kecuali buffer channel penuh — backpressure alami).
                    if tx.send(BgMsg::Bytes(buf.to_vec())).is_err() {
                        // Thread mati → matikan dump agar tidak silent-loss.
                        self.enabled = false;
                    }
                }
            }
            Out::Detached => {}
        }
        self.total_written += buf.len() as u64;
        Ok(())
    }

    fn write_vals_force(
        &mut self,
        sig_val: &maria_ir::LogicVec,
        code: &str,
        is_one_bit: bool,
    ) -> Result<(), String> {
        let val_str = vec_to_vcd(sig_val);
        let line = if is_one_bit {
            format!("{}{}\n", val_str, code)
        } else {
            format!("b{} {}\n", val_str, code)
        };
        self.write_raw(line.as_bytes())?;
        Ok(())
    }

    fn elem_val<'a>(
        &self,
        sig_val: &'a maria_ir::LogicVec,
        elem: usize,
        elem_width: usize,
    ) -> maria_ir::LogicVec {
        let start = elem * elem_width;
        let mut bits = Vec::with_capacity(elem_width);
        for j in start..start + elem_width {
            bits.push(sig_val.bits.get(j).copied().unwrap_or(LogicVal::X));
        }
        maria_ir::LogicVec {
            width: elem_width,
            bits,
        }
    }

    fn code_for_signal(
        &self,
        sig_scope: &[String],
        sig_bare: &str,
        elem: Option<usize>,
    ) -> Option<String> {
        if let Some(e) = elem {
            let elem_name = format!("{}[{}]", sig_bare, e);
            self.code_by_key
                .get(&(sig_scope.to_vec(), elem_name))
                .cloned()
        } else {
            self.code_by_key
                .get(&(sig_scope.to_vec(), sig_bare.to_string()))
                .cloned()
        }
    }

    fn parse_scope(name: &str) -> (Vec<String>, String) {
        let parts: Vec<&str> = name.rsplitn(2, '.').collect();
        if parts.len() == 2 {
            let bare_name = parts[0].to_string();
            let scope_parts: Vec<String> = parts[1].split('.').map(|s| s.to_string()).collect();
            (scope_parts, bare_name)
        } else {
            (vec![], name.to_string())
        }
    }

    fn write_scopes(
        &mut self,
        current: &[String],
        target: &[String],
        sigs: &[(String, usize, usize)],
        entry_idx: &mut usize,
    ) -> Result<(), String> {
        let mut close_count = 0usize;
        for (i, p) in current.iter().enumerate() {
            if i >= target.len() || target[i] != *p {
                close_count = current.len() - i;
                break;
            }
        }
        for _ in 0..close_count {
            let _ = self.write_raw(b"$upscope $end\n");
        }

        let keep = current.len() - close_count;
        for p in &target[keep..] {
            self.write_raw(format!("$scope module {} $end\n", p).as_bytes())?;
        }

        for (bare_name, width, array_depth) in sigs {
            if *width == 0 {
                continue;
            } // skip dynamic/queue arrays before allocation
            if *array_depth > 1 {
                for elem in 0..*array_depth {
                    let code = format!("s{:x}", entry_idx);
                    *entry_idx += 1;
                    let elem_name = format!("{}[{}]", bare_name, elem);
                    let width_disp = if *width == 1 {
                        "1".to_string()
                    } else {
                        width.to_string()
                    };
                    let range = if *width == 1 {
                        String::new()
                    } else {
                        format!(" [{}:0]", width - 1)
                    };
                    self.write_raw(
                        format!(
                            "$var wire {} {} {} {} $end\n",
                            width_disp, code, elem_name, range
                        )
                        .as_bytes(),
                    )?;
                    self.code_by_key.insert((target.to_vec(), elem_name), code);
                }
            } else {
                let code = format!("s{:x}", entry_idx);
                *entry_idx += 1;
                let width_disp = if *width == 1 {
                    "1".to_string()
                } else {
                    width.to_string()
                };
                let range = if *width == 1 {
                    String::new()
                } else {
                    format!(" [{}:0]", width - 1)
                };
                self.write_raw(
                    format!(
                        "$var wire {} {} {} {} $end\n",
                        width_disp, code, bare_name, range
                    )
                    .as_bytes(),
                )?;
                self.code_by_key
                    .insert((target.to_vec(), bare_name.clone()), code);
            }
        }
        Ok(())
    }

    fn write_header(&mut self, design: &IrDesign) -> Result<(), String> {
        self.write_raw(b"$version Maria RTL Simulator v0.1.0 $end\n")?;
        let ts = if let Some((ref unit, _)) = design.timescale {
            format!("$timescale {} $end\n", unit)
        } else {
            "$timescale 1ns $end\n".to_string()
        };
        self.write_raw(ts.as_bytes())?;

        let mut scope_map: HashMap<Vec<String>, Vec<(String, usize, usize)>> = HashMap::new();
        for sig in &design.top.signals {
            // Class handle (objek/covergroup/virtual interface) BUKAN nilai 4-state —
            // Questa tidak pernah dump ke VCD. Skip biar VCD bersih.
            if sig.class_name.is_some() {
                continue;
            }
            let (scope_parts, bare_name) = Self::parse_scope(sig.name.as_str());
            scope_map
                .entry(scope_parts)
                .or_default()
                .push((bare_name, sig.width, sig.array_depth));
        }

        let mut sorted_scopes: Vec<Vec<String>> = scope_map.keys().cloned().collect();
        sorted_scopes.sort();

        self.write_raw(format!("$scope module {} $end\n", design.top.name).as_bytes())?;

        let mut current_scope: Vec<String> = Vec::new();
        let mut entry_idx = 0usize;

        for scope_path in &sorted_scopes {
            let sigs = scope_map.get(scope_path).unwrap();
            self.write_scopes(&current_scope, scope_path, sigs, &mut entry_idx)?;
            current_scope = scope_path.clone();
        }

        for _ in 0..current_scope.len() {
            self.write_raw(b"$upscope $end\n")?;
        }
        self.write_raw(b"$upscope $end\n")?;
        self.write_raw(b"$enddefinitions $end\n")?;
        self.write_raw(b"$dumpvars\n")?;

        // Build dump_targets flat array — pre-computed untuk semua signal.
        // Menggantikan parse_scope + code_for_signal lookup per cycle.
        self.dump_targets.clear();

        for (sig_idx, sig) in design.top.signals.iter().enumerate() {
            if sig.class_name.is_some() {
                continue;
            }
            let (sig_scope, sig_bare) = Self::parse_scope(sig.name.as_str());
            if sig.array_depth > 1 {
                for elem in 0..sig.array_depth {
                    if let Some(code) = self.code_for_signal(&sig_scope, &sig_bare, Some(elem)) {
                        let e_val = self.elem_val(&sig.init_val, elem, sig.elem_width);
                        self.write_vals_force(&e_val, &code, sig.elem_width == 1)?;
                        self.dump_targets.push(DumpTarget {
                            code,
                            is_one_bit: sig.elem_width == 1,
                            signal_idx: sig_idx,
                            elem_idx: Some(elem),
                            elem_width: sig.elem_width,
                        });
                    }
                }
            } else {
                if let Some(code) = self.code_for_signal(&sig_scope, &sig_bare, None) {
                    self.write_vals_force(&sig.init_val, &code, sig.width == 1)?;
                    self.dump_targets.push(DumpTarget {
                        code,
                        is_one_bit: sig.width == 1,
                        signal_idx: sig_idx,
                        elem_idx: None,
                        elem_width: sig.width,
                    });
                }
            }
        }

        // Init last_values flat array
        self.last_values = self.dump_targets.iter().map(|t| {
            let sig = &design.top.signals[t.signal_idx];
            let val = if let Some(elem) = t.elem_idx {
                let e_val = self.elem_val(&sig.init_val, elem, t.elem_width);
                vec_to_vcd(&e_val)
            } else {
                vec_to_vcd(&sig.init_val)
            };
            val
        }).collect();

        self.write_raw(b"$end\n")
    }

    pub fn write_time_header(&mut self, time: u64) -> Result<(), String> {
        if !self.enabled {
            return Ok(());
        }
        self.write_raw(format!("#{}\n", time).as_bytes())
    }

    pub fn dump_state(
        &mut self,
        _design: &IrDesign,
        state: &[maria_ir::LogicVec],
    ) -> Result<(), String> {
        if !self.enabled {
            return Ok(());
        }
        // Fast path: index-based loop — O(1) flat array access tanpa
        // HashMap lookup / parse_scope / code_for_signal per cycle.
        let n = self.dump_targets.len();
        for i in 0..n {
            let sig_val = &state[self.dump_targets[i].signal_idx];
            let val_str = if let Some(elem) = self.dump_targets[i].elem_idx {
                let e_val = self.elem_val(sig_val, elem, self.dump_targets[i].elem_width);
                vec_to_vcd(&e_val)
            } else {
                vec_to_vcd(sig_val)
            };
            if self.last_values[i] != val_str {
                let line = if self.dump_targets[i].is_one_bit {
                    format!("{}{}\n", val_str, self.dump_targets[i].code)
                } else {
                    format!("b{} {}\n", val_str, self.dump_targets[i].code)
                };
                self.write_raw(line.as_bytes())?;
                self.last_values[i] = val_str;
            }
        }
        Ok(())
    }

    pub fn dump_all(
        &mut self,
        _design: &IrDesign,
        state: &[maria_ir::LogicVec],
    ) -> Result<(), String> {
        if !self.enabled {
            return Ok(());
        }
        let n = self.dump_targets.len();
        for i in 0..n {
            let sig_val = &state[self.dump_targets[i].signal_idx];
            let val_str = if let Some(elem) = self.dump_targets[i].elem_idx {
                let e_val = self.elem_val(sig_val, elem, self.dump_targets[i].elem_width);
                vec_to_vcd(&e_val)
            } else {
                vec_to_vcd(sig_val)
            };
            let line = if self.dump_targets[i].is_one_bit {
                format!("{}{}\n", val_str, self.dump_targets[i].code)
            } else {
                format!("b{} {}\n", val_str, self.dump_targets[i].code)
            };
            self.write_raw(line.as_bytes())?;
            self.last_values[i] = val_str;
        }
        Ok(())
    }

    fn close_inner(&mut self) -> Result<(), String> {
        // Background: tutup channel → thread flush lalu exit; join agar
        // file benar-benar final sebelum fungsi ini kembali.
        if matches!(self.out, Out::Bg(_)) {
            if let Out::Bg(bg) = &mut self.out {
                bg.tx = None;
                if let Some(h) = bg.handle.take() {
                    let _ = h.join();
                }
            }
        }
        if let Out::File(f) = &mut self.out {
            let _ = f.flush();
        }
        // WAV-04: flush compressed encoder (writes gzip footer)
        if let Out::Compressed(_) = &self.out {
            if let Out::Compressed(enc) = std::mem::replace(&mut self.out, Out::Detached) {
                let _ = enc.finish();
            }
        }
        Ok(())
    }

    /// Aktifkan writer thread background (WAV-19): setelah ini semua byte
    /// dump dikirim via channel ke thread penulis terpisah — simulasi tidak
    /// lagi menunggu I/O disk di jalur panas. Opt-in (default sinkron).
    pub fn enable_background(&mut self) -> Result<(), String> {
        if matches!(self.out, Out::Bg(_)) {
            return Ok(()); // sudah aktif
        }
        // Ambil File dari self.out (placeholder Detached sementara).
        let taken = std::mem::replace(&mut self.out, Out::Detached);
        let file = match taken {
            Out::File(f) => f,
            other => {
                self.out = other;
                return Ok(());
            }
        };
        let (tx, rx) = std::sync::mpsc::channel::<BgMsg>();
        let handle = std::thread::Builder::new()
            .name("maria-vcd-writer".to_string())
            .spawn(move || {
                let mut out = std::io::BufWriter::new(file);
                for msg in rx {
                    match msg {
                        BgMsg::Bytes(b) => {
                            let _ = out.write_all(&b);
                        }
                        BgMsg::Flush(ack) => {
                            let _ = out.flush();
                            let _ = ack.send(());
                        }
                    }
                }
                // Channel tertutup → finalisasi file.
                let _ = out.flush();
            })
            .map_err(|e| format!("cannot spawn VCD writer thread: {}", e))?;
        self.out = Out::Bg(BgWriter {
            tx: Some(tx),
            handle: Some(handle),
        });
        Ok(())
    }

    /// Flush file buffer to disk (for streaming mode).
    pub fn flush(&mut self) -> Result<(), String> {
        match &mut self.out {
            Out::File(f) => f
                .flush()
                .map_err(|e| format!("VCD flush error: {}", e)),
            Out::Compressed(enc) => enc
                .flush()
                .map_err(|e| format!("VCD flush error: {}", e)),
            Out::Bg(bg) => {
                if let Some(tx) = &bg.tx {
                    // Sinkron: tunggu ack dari writer thread sehingga
                    // setelah flush() kembali, semua antrean sudah di-disk.
                    let (ack_tx, ack_rx) = std::sync::mpsc::channel();
                    if tx.send(BgMsg::Flush(ack_tx)).is_ok() {
                        let _ = ack_rx.recv();
                    } else {
                        bg.tx = None;
                        self.enabled = false;
                    }
                }
                Ok(())
            }
            Out::Detached => Ok(()),
        }
    }

    /// Flush to disk if stream_flush_interval reached.
    pub fn maybe_flush(&mut self) -> Result<(), String> {
        if self.stream_flush_interval == 0 {
            return Ok(());
        }
        self.flush_counter += 1;
        if self.flush_counter >= self.stream_flush_interval {
            self.flush()?;
            self.flush_counter = 0;
        }
        Ok(())
    }

    pub fn close(&mut self) -> Result<(), String> {
        self.flush()?;
        self.close_inner()
    }
}

impl Drop for VcdWriter {
    fn drop(&mut self) {
        // DEBT-19: pastikan buffer ter-flush walau engine return early
        // (mis. error sim) tanpa sempat memanggil close() eksplisit.
        // WAV-19: flush (tunggu ack antrean background) lalu join thread —
        // file finalisasi meski drop tanpa close().
        let _ = self.flush();
        let _ = self.close_inner();
    }
}

fn vec_to_vcd(val: &maria_ir::LogicVec) -> String {
    let mut s = String::with_capacity(val.width);
    for bit in val.bits.iter().rev() {
        match bit {
            LogicVal::Zero => s.push('0'),
            LogicVal::One => s.push('1'),
            LogicVal::X => s.push('x'),
            LogicVal::Z => s.push('z'),
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use maria_core::intern::Symbol;
    use maria_ir::*;

    fn make_design() -> IrDesign {
        let mut design = IrDesign::default();
        design.top.name = Symbol::intern("bg_top");
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

    fn state(clk: u64, data: u64) -> Vec<LogicVec> {
        vec![LogicVec::from_u64(clk, 1), LogicVec::from_u64(data, 8)]
    }

    #[test]
    fn test_background_mode_writes_identical_output() {
        let dir = std::env::temp_dir().join("maria_vcd_bg_test");
        let _ = std::fs::create_dir_all(&dir);
        let design = make_design();

        // Sinkron (baseline).
        let sync_path = dir.join("sync.vcd");
        {
            let mut w = VcdWriter::new(sync_path.to_str().unwrap(), &design).unwrap();
            w.dump_all(&design, &state(0, 42)).unwrap();
            w.write_time_header(10).unwrap();
            w.dump_state(&design, &state(1, 99)).unwrap();
            w.close().unwrap();
        }

        // Background (WAV-19) — output harus identik.
        let bg_path = dir.join("bg.vcd");
        {
            let mut w = VcdWriter::new(bg_path.to_str().unwrap(), &design).unwrap();
            w.enable_background().unwrap();
            w.dump_all(&design, &state(0, 42)).unwrap();
            w.write_time_header(10).unwrap();
            w.dump_state(&design, &state(1, 99)).unwrap();
            w.close().unwrap(); // join thread → file final
        }

        let sync_out = std::fs::read_to_string(&sync_path).unwrap();
        let bg_out = std::fs::read_to_string(&bg_path).unwrap();
        assert_eq!(sync_out, bg_out, "background output identik dengan sinkron");
        assert!(bg_out.contains("#10"), "time header tertulis");
        assert!(bg_out.contains("$enddefinitions $end"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_background_flush_synchronizes() {
        let dir = std::env::temp_dir().join("maria_vcd_bg_flush");
        let _ = std::fs::create_dir_all(&dir);
        let design = make_design();
        let path = dir.join("flush.vcd");

        let mut w = VcdWriter::new(path.to_str().unwrap(), &design).unwrap();
        w.enable_background().unwrap();
        w.dump_all(&design, &state(0, 7)).unwrap();

        // flush() sinkron — setelah kembali, semua byte harus sudah di-disk.
        w.flush().unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(
            content.contains("$enddefinitions $end"),
            "header lengkap setelah flush"
        );
        assert!(content.contains("b00000111"), "nilai data=7 ter-flush");

        w.close().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_background_drop_finalizes_file() {
        let dir = std::env::temp_dir().join("maria_vcd_bg_drop");
        let _ = std::fs::create_dir_all(&dir);
        let design = make_design();
        let path = dir.join("drop.vcd");

        {
            let mut w = VcdWriter::new(path.to_str().unwrap(), &design).unwrap();
            w.enable_background().unwrap();
            w.dump_all(&design, &state(0, 3)).unwrap();
            // Drop TANPA close() — Drop impl harus join thread + flush.
        }
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("b00000011"), "Drop mem-finalisasi file bg");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── WAV-04: Compressed waveform database tests ──

    #[test]
    fn test_compressed_vcd_is_valid_gzip() {
        let dir = std::env::temp_dir().join("maria_vcd_gzip_test");
        let _ = std::fs::create_dir_all(&dir);
        let design = make_design();
        let path = dir.join("compressed.vcd.gz");

        {
            let mut w = VcdWriter::new_compressed(path.to_str().unwrap(), &design).unwrap();
            w.dump_all(&design, &state(0, 42)).unwrap();
            w.write_time_header(10).unwrap();
            w.dump_state(&design, &state(1, 99)).unwrap();
            w.close().unwrap();
        }

        // File harus ada dan valid gzip (magic bytes 0x1f 0x8b)
        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.len() > 2, "file gzip tidak kosong");
        assert_eq!(bytes[0], 0x1f, "gzip magic byte 1");
        assert_eq!(bytes[1], 0x8b, "gzip magic byte 2");

        // Decompress dan verifikasi isi VCD valid
        use flate2::read::GzDecoder;
        use std::io::Read;
        let mut decoder = GzDecoder::new(&bytes[..]);
        let mut decompressed = String::new();
        decoder.read_to_string(&mut decompressed).unwrap();
        assert!(decompressed.contains("$enddefinitions $end"), "decompressed VCD header valid");
        assert!(decompressed.contains("b00101010"), "decompressed VCD data valid (42 = 00101010)");
        assert!(decompressed.contains("#10"), "decompressed VCD time header valid");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_compressed_vcd_new_compressed() {
        let dir = std::env::temp_dir().join("maria_vcd_gzip_new");
        let _ = std::fs::create_dir_all(&dir);
        let design = make_design();
        let path = dir.join("new_compressed.vcd.gz");

        {
            let mut w = VcdWriter::new_compressed(path.to_str().unwrap(), &design).unwrap();
            w.dump_all(&design, &state(0, 7)).unwrap();
            w.close().unwrap();
        }

        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes[0], 0x1f, "new_compressed: gzip magic byte 1");
        assert_eq!(bytes[1], 0x8b, "new_compressed: gzip magic byte 2");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
