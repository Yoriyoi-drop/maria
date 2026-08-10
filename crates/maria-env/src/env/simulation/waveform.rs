use maria_simulator::simulator::SimulationEngine;
use maria_simulator::waveform::{FstWaveWriter, VcdWriter};

/// WaveformOptions — format output waveform.
#[derive(Debug, Clone)]
pub struct WaveformOptions {
    pub vcd: bool,
    pub fst: bool,
    pub base_path: String,
}

impl WaveformOptions {
    pub fn vcd_only(name: &str) -> Self {
        WaveformOptions { vcd: true, fst: false, base_path: name.to_string() }
    }

    pub fn all(name: &str) -> Self {
        WaveformOptions { vcd: true, fst: true, base_path: name.to_string() }
    }
}

/// Pasang VCD/FST ke engine. Error FST dianggap non-fatal (best-effort).
pub fn attach_waveforms(engine: &mut SimulationEngine, opts: &WaveformOptions) -> Vec<String> {
    let mut attached = Vec::new();
    let base = &opts.base_path;
    if opts.vcd {
        let path = format!("{}.vcd", base);
        match VcdWriter::new(&path, &engine.design) {
            Ok(vcd) => {
                engine.set_vcd(vcd);
                attached.push(path);
            }
            Err(e) => {
                eprintln!("warning: VCD tidak dapat dibuat '{}': {}", path, e);
            }
        }
    }
    if opts.fst {
        let path = format!("{}.fst", base);
        match FstWaveWriter::new(&path, &engine.design) {
            Ok(fst) => {
                engine.set_fst(fst);
                attached.push(path);
            }
            Err(e) => {
                eprintln!("warning: FST tidak dapat dibuat '{}': {}", path, e);
            }
        }
    }
    attached
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_waveform_options() {
        let o = WaveformOptions::vcd_only("top");
        assert!(o.vcd && !o.fst);
    }
}
