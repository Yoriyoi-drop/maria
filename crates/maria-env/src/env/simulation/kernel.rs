use maria_ir::IrDesign;
use maria_simulator::simulator::{SimulationEngine, SimulationLimit};

/// SimulationKernel — membangun + menjalankan engine simulasi.
///
/// Kernel adalah satu-satunya pintu pembuat `SimulationEngine` di context.
#[derive(Debug, Clone, Copy)]
pub struct KernelOptions {
    pub max_time: Option<u64>,
    pub report_progress: bool,
    pub deep_debug: bool,
    pub snapshot_interval: u64,
}

impl Default for KernelOptions {
    fn default() -> Self {
        KernelOptions {
            max_time: None,
            report_progress: false,
            deep_debug: false,
            snapshot_interval: 1000,
        }
    }
}

pub struct SimulationKernel;

impl SimulationKernel {
    /// Bangun engine dengan batas waktu (None = unlimited).
    pub fn build(design: IrDesign, opts: &KernelOptions) -> SimulationEngine {
        let limit = opts
            .max_time
            .map(SimulationLimit::Finite)
            .unwrap_or(SimulationLimit::Unlimited);
        let mut engine = SimulationEngine::new_with_limit(design, limit);
        engine.report_progress = opts.report_progress;
        engine.debug_mode = if opts.deep_debug {
            maria_simulator::simulator::DebugMode::DeepDebug
        } else {
            maria_simulator::simulator::DebugMode::Normal
        };
        engine.snapshot_interval = opts.snapshot_interval;
        engine
    }

    /// Bangun + jalankan engine sampai selesai.
    pub fn run(
        design: IrDesign,
        opts: &KernelOptions,
    ) -> Result<(SimulationEngine, u64), maria_core::error::SimError> {
        let mut engine = Self::build(design, opts);
        engine.run()?;
        let final_time = engine.state.time;
        Ok((engine, final_time))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::compiler::{elaborate, lex, parse_strict};
    use maria_elaboration::ElaborateMode;

    #[test]
    fn test_kernel_runs_simple() {
        let design = parse_strict(
            lex("module top;\ninitial begin #5 $finish; end\nendmodule"),
            "t.sv",
        )
        .unwrap();
        let (ir, _diags) = elaborate(
            design,
            vec!["module top;\ninitial begin #5 $finish; end\nendmodule".to_string()],
            "t.sv".to_string(),
            Some("top"),
            ElaborateMode::StrictSimulation,
        )
        .unwrap();
        let opts = KernelOptions { max_time: Some(100), ..Default::default() };
        let (_engine, t) = SimulationKernel::run(ir, &opts).unwrap();
        assert!(t >= 5);
    }
}
