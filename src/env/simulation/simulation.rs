use crate::env::simulation::{DpiInfo, KernelOptions, WaveformOptions};

/// SimulationContext — service simulasi: kernel, waveform, dpi.
///
/// Context tidak menahan engine (engine berumur per-run); ia menyimpan
/// service yang dipakai tiap run.
#[derive(Debug, Clone)]
pub struct SimulationContext {
    pub kernel_opts: KernelOptions,
    pub waveform_opts: Option<WaveformOptions>,
    pub dpi: DpiInfo,
}

impl SimulationContext {
    pub fn new() -> Self {
        SimulationContext {
            kernel_opts: KernelOptions::default(),
            waveform_opts: None,
            dpi: DpiInfo::detect(),
        }
    }

    pub fn with_max_time(mut self, max_time: Option<u64>) -> Self {
        self.kernel_opts.max_time = max_time;
        self
    }

    pub fn with_waveform(mut self, opts: WaveformOptions) -> Self {
        self.waveform_opts = Some(opts);
        self
    }

    pub fn summary(&self) -> String {
        format!(
            "max_time={:?} waveform={} dpi={}",
            self.kernel_opts.max_time,
            self.waveform_opts.is_some(),
            self.dpi.available,
        )
    }
}

impl Default for SimulationContext {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simulation_context() {
        let ctx = SimulationContext::new().with_max_time(Some(1000));
        assert_eq!(ctx.kernel_opts.max_time, Some(1000));
        assert!(ctx.summary().contains("dpi="));
    }
}
