//! Differential Executor — bandingkan Maria dengan simulator referensi.

use std::process::Command;
use std::io::Write;
use std::fs;
use tempfile::tempdir;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Simulator {
    Maria,
    Verilator,
    Icarus,
    VCS,
}

#[derive(Debug, Clone)]
pub struct SimulationResult {
    pub simulator: Simulator,
    pub success: bool,
    pub output: String,
    pub signals: Vec<(String, u64)>,
    pub error: Option<String>,
    pub compile_time_ms: u64,
    pub sim_time_ms: u64,
}

#[derive(Debug, Clone)]
pub struct DifferentialResult {
    pub testcase: String,
    pub maria: SimulationResult,
    pub references: Vec<SimulationResult>,
    pub mismatches: Vec<Mismatch>,
}

#[derive(Debug, Clone)]
pub struct Mismatch {
    pub signal: String,
    pub maria_value: u64,
    pub reference_value: u64,
    pub reference_sim: Simulator,
    pub kind: MismatchKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MismatchKind {
    CompileError,
    RuntimeError,
    SignalValue,
    NonDeterminism,
    Timeout,
}

pub struct DifferentialExecutor {
    pub enabled_sims: Vec<Simulator>,
    pub timeout_secs: u64,
    pub keep_temp: bool,
}

impl Default for DifferentialExecutor {
    fn default() -> Self {
        DifferentialExecutor {
            enabled_sims: vec![Simulator::Maria, Simulator::Verilator, Simulator::Icarus],
            timeout_secs: 60,
            keep_temp: false,
        }
    }
}

impl DifferentialExecutor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_simulators(mut self, sims: Vec<Simulator>) -> Self {
        self.enabled_sims = sims;
        self
    }

    pub fn run(&self, source: &str) -> DifferentialResult {
        let dir = tempdir().expect("failed to create temp dir");
        let file_path = dir.path().join("test.sv");
        fs::write(&file_path, source).expect("failed to write test file");

        let mut results = Vec::new();

        for sim in &self.enabled_sims {
            let result = self.run_simulator(*sim, &file_path, dir.path());
            results.push(result);
        }

        let maria_result = results.iter().find(|r| r.simulator == Simulator::Maria)
            .cloned()
            .unwrap_or(SimulationResult {
                simulator: Simulator::Maria,
                success: false,
                output: String::new(),
                signals: Vec::new(),
                error: Some("Maria not run".to_string()),
                compile_time_ms: 0,
                sim_time_ms: 0,
            });

        let references: Vec<SimulationResult> = results.into_iter()
            .filter(|r| r.simulator != Simulator::Maria)
            .collect();

        let mismatches = self.compare_results(&maria_result, &references);

        if !self.keep_temp {
            let _ = dir.close();
        }

        DifferentialResult {
            testcase: source.to_string(),
            maria: maria_result,
            references,
            mismatches,
        }
    }

    fn run_simulator(&self, sim: Simulator, file_path: &std::path::Path, work_dir: &std::path::Path) -> SimulationResult {
        match sim {
            Simulator::Maria => self.run_maria(file_path),
            Simulator::Verilator => self.run_verilator(file_path, work_dir),
            Simulator::Icarus => self.run_icarus(file_path, work_dir),
            Simulator::VCS => self.run_vcs(file_path, work_dir),
        }
    }

    fn run_maria(&self, file_path: &std::path::Path) -> SimulationResult {
        let start = std::time::Instant::now();
        let output = Command::new("cargo")
            .args(["run", "--quiet", "--", file_path.to_str().unwrap()])
            .current_dir("/home/whale-d/maria")
            .output();

        let compile_time = start.elapsed().as_millis() as u64;
        let sim_start = std::time::Instant::now();

        match output {
            Ok(out) => {
                let sim_time = sim_start.elapsed().as_millis() as u64;
                let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                let stderr = String::from_utf8_lossy(&out.stderr).to_string();

                if out.status.success() {
                    let signals = self.parse_maria_output(&stdout);
                    SimulationResult {
                        simulator: Simulator::Maria,
                        success: true,
                        output: stdout,
                        signals,
                        error: if stderr.is_empty() { None } else { Some(stderr) },
                        compile_time_ms: compile_time,
                        sim_time_ms: sim_time,
                    }
                } else {
                    SimulationResult {
                        simulator: Simulator::Maria,
                        success: false,
                        output: stdout,
                        signals: Vec::new(),
                        error: Some(stderr),
                        compile_time_ms: compile_time,
                        sim_time_ms: sim_time,
                    }
                }
            }
            Err(e) => SimulationResult {
                simulator: Simulator::Maria,
                success: false,
                output: String::new(),
                signals: Vec::new(),
                error: Some(e.to_string()),
                compile_time_ms: compile_time,
                sim_time_ms: 0,
            },
        }
    }

    fn run_verilator(&self, file_path: &std::path::Path, work_dir: &std::path::Path) -> SimulationResult {
        let start = std::time::Instant::now();

        let verilator_bin = std::env::var("VERILATOR").unwrap_or_else(|_| "verilator".to_string());
        let mut cmd = Command::new(&verilator_bin);
        cmd.args(["--cc", "--exe", "--build", "-j", "0", "--timing"])
            .arg(file_path)
            .current_dir(work_dir);

        let compile_time = start.elapsed().as_millis() as u64;

        let sim_start = std::time::Instant::now();
        let obj_dir = work_dir.join("obj_dir");
        let sim_bin = obj_dir.join("Vtest");

        let output = if sim_bin.exists() {
            Command::new(&sim_bin).output()
        } else {
            let compile_out = cmd.output();
            let compile_time_total = start.elapsed().as_millis() as u64;

            if let Ok(out) = compile_out {
                if !out.status.success() {
                    return SimulationResult {
                        simulator: Simulator::Verilator,
                        success: false,
                        output: String::from_utf8_lossy(&out.stdout).to_string(),
                        signals: Vec::new(),
                        error: Some(String::from_utf8_lossy(&out.stderr).to_string()),
                        compile_time_ms: compile_time_total,
                        sim_time_ms: 0,
                    };
                }
            }

            if sim_bin.exists() {
                Command::new(&sim_bin).output()
            } else {
                return SimulationResult {
                    simulator: Simulator::Verilator,
                    success: false,
                    output: String::new(),
                    signals: Vec::new(),
                    error: Some("Verilator build failed - no sim binary".to_string()),
                    compile_time_ms: compile_time_total,
                    sim_time_ms: 0,
                };
            }
        };

        let sim_time = sim_start.elapsed().as_millis() as u64;

        match output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                if out.status.success() {
                    let signals = self.parse_verilator_output(&stdout);
                    SimulationResult {
                        simulator: Simulator::Verilator,
                        success: true,
                        output: stdout,
                        signals,
                        error: None,
                        compile_time_ms: compile_time,
                        sim_time_ms: sim_time,
                    }
                } else {
                    SimulationResult {
                        simulator: Simulator::Verilator,
                        success: false,
                        output: stdout,
                        signals: Vec::new(),
                        error: Some(String::from_utf8_lossy(&out.stderr).to_string()),
                        compile_time_ms: compile_time,
                        sim_time_ms: sim_time,
                    }
                }
            }
            Err(e) => SimulationResult {
                simulator: Simulator::Verilator,
                success: false,
                output: String::new(),
                signals: Vec::new(),
                error: Some(e.to_string()),
                compile_time_ms: compile_time,
                sim_time_ms: sim_time,
            },
        }
    }

    fn run_icarus(&self, file_path: &std::path::Path, work_dir: &std::path::Path) -> SimulationResult {
        let start = std::time::Instant::now();

        let iverilog = std::env::var("IVERILOG").unwrap_or_else(|_| "iverilog".to_string());
        let vvp = std::env::var("VVP").unwrap_or_else(|_| "vvp".to_string());
        let out_bin = work_dir.join("test_icarus.out");

        let compile_out = Command::new(&iverilog)
            .args(["-o", out_bin.to_str().unwrap(), file_path.to_str().unwrap()])
            .current_dir(work_dir)
            .output();

        let compile_time = start.elapsed().as_millis() as u64;

        let sim_start = std::time::Instant::now();

        let result = match compile_out {
            Ok(out) if out.status.success() => {
                Command::new(&vvp).arg(&out_bin).output()
            }
            Ok(out) => {
                return SimulationResult {
                    simulator: Simulator::Icarus,
                    success: false,
                    output: String::from_utf8_lossy(&out.stdout).to_string(),
                    signals: Vec::new(),
                    error: Some(String::from_utf8_lossy(&out.stderr).to_string()),
                    compile_time_ms: compile_time,
                    sim_time_ms: 0,
                };
            }
            Err(e) => {
                return SimulationResult {
                    simulator: Simulator::Icarus,
                    success: false,
                    output: String::new(),
                    signals: Vec::new(),
                    error: Some(e.to_string()),
                    compile_time_ms: compile_time,
                    sim_time_ms: 0,
                };
            }
        };

        let sim_time = sim_start.elapsed().as_millis() as u64;

        match result {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                if out.status.success() {
                    let signals = self.parse_icarus_output(&stdout);
                    SimulationResult {
                        simulator: Simulator::Icarus,
                        success: true,
                        output: stdout,
                        signals,
                        error: None,
                        compile_time_ms: compile_time,
                        sim_time_ms: sim_time,
                    }
                } else {
                    SimulationResult {
                        simulator: Simulator::Icarus,
                        success: false,
                        output: stdout,
                        signals: Vec::new(),
                        error: Some(String::from_utf8_lossy(&out.stderr).to_string()),
                        compile_time_ms: compile_time,
                        sim_time_ms: sim_time,
                    }
                }
            }
            Err(e) => SimulationResult {
                simulator: Simulator::Icarus,
                success: false,
                output: String::new(),
                signals: Vec::new(),
                error: Some(e.to_string()),
                compile_time_ms: compile_time,
                sim_time_ms: sim_time,
            },
        }
    }

    fn run_vcs(&self, file_path: &std::path::Path, work_dir: &std::path::Path) -> SimulationResult {
        let start = std::time::Instant::now();

        let vcs = std::env::var("VCS").unwrap_or_else(|_| "vcs".to_string());
        let out_bin = work_dir.join("test_vcs.out");

        let compile_out = Command::new(&vcs)
            .args(["-o", out_bin.to_str().unwrap(), file_path.to_str().unwrap()])
            .current_dir(work_dir)
            .output();

        let compile_time = start.elapsed().as_millis() as u64;

        let sim_start = std::time::Instant::now();

        let result = match compile_out {
            Ok(out) if out.status.success() => {
                Command::new(&out_bin).output()
            }
            Ok(out) => {
                return SimulationResult {
                    simulator: Simulator::VCS,
                    success: false,
                    output: String::from_utf8_lossy(&out.stdout).to_string(),
                    signals: Vec::new(),
                    error: Some(String::from_utf8_lossy(&out.stderr).to_string()),
                    compile_time_ms: compile_time,
                    sim_time_ms: 0,
                };
            }
            Err(e) => {
                return SimulationResult {
                    simulator: Simulator::VCS,
                    success: false,
                    output: String::new(),
                    signals: Vec::new(),
                    error: Some(e.to_string()),
                    compile_time_ms: compile_time,
                    sim_time_ms: 0,
                };
            }
        };

        let sim_time = sim_start.elapsed().as_millis() as u64;

        match result {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                if out.status.success() {
                    let signals = self.parse_vcs_output(&stdout);
                    SimulationResult {
                        simulator: Simulator::VCS,
                        success: true,
                        output: stdout,
                        signals,
                        error: None,
                        compile_time_ms: compile_time,
                        sim_time_ms: sim_time,
                    }
                } else {
                    SimulationResult {
                        simulator: Simulator::VCS,
                        success: false,
                        output: stdout,
                        signals: Vec::new(),
                        error: Some(String::from_utf8_lossy(&out.stderr).to_string()),
                        compile_time_ms: compile_time,
                        sim_time_ms: sim_time,
                    }
                }
            }
            Err(e) => SimulationResult {
                simulator: Simulator::VCS,
                success: false,
                output: String::new(),
                signals: Vec::new(),
                error: Some(e.to_string()),
                compile_time_ms: compile_time,
                sim_time_ms: sim_time,
            },
        }
    }

    fn parse_maria_output(&self, output: &str) -> Vec<(String, u64)> {
        let mut signals = Vec::new();
        for line in output.lines() {
            if line.contains(" = ") && (line.contains("0x") || line.contains("0b") || line.chars().any(|c| c.is_ascii_digit())) {
                if let Some(eq_pos) = line.find(" = ") {
                    let name = line[..eq_pos].trim();
                    let value_str = line[eq_pos + 3..].trim();
                    if let Ok(value) = self.parse_value(value_str) {
                        signals.push((name.to_string(), value));
                    }
                }
            }
        }
        signals
    }

    fn parse_verilator_output(&self, output: &str) -> Vec<(String, u64)> {
        self.parse_maria_output(output)
    }

    fn parse_icarus_output(&self, output: &str) -> Vec<(String, u64)> {
        self.parse_maria_output(output)
    }

    fn parse_vcs_output(&self, output: &str) -> Vec<(String, u64)> {
        self.parse_maria_output(output)
    }

    fn parse_value(&self, s: &str) -> Result<u64, String> {
        let s = s.trim();
        if s.starts_with("0x") || s.starts_with("0X") {
            u64::from_str_radix(&s[2..], 16).map_err(|e| e.to_string())
        } else if s.starts_with("0b") || s.starts_with("0B") {
            u64::from_str_radix(&s[2..], 2).map_err(|e| e.to_string())
        } else if s.starts_with("'h") || s.starts_with("'H") {
            u64::from_str_radix(&s[2..], 16).map_err(|e| e.to_string())
        } else if s.starts_with("'b") || s.starts_with("'B") {
            u64::from_str_radix(&s[2..], 2).map_err(|e| e.to_string())
        } else {
            s.parse::<u64>().map_err(|e| e.to_string())
        }
    }

    fn compare_results(&self, maria: &SimulationResult, references: &[SimulationResult]) -> Vec<Mismatch> {
        let mut mismatches = Vec::new();

        for ref_sim in references {
            if maria.success != ref_sim.success {
                mismatches.push(Mismatch {
                    signal: "COMPILE/RUN STATUS".to_string(),
                    maria_value: if maria.success { 1 } else { 0 },
                    reference_value: if ref_sim.success { 1 } else { 0 },
                    reference_sim: ref_sim.simulator,
                    kind: MismatchKind::CompileError,
                });
            }

            if maria.success && ref_sim.success {
                let maria_signals: std::collections::HashMap<_, _> = maria.signals.iter().cloned().collect();
                let ref_signals: std::collections::HashMap<_, _> = ref_sim.signals.iter().cloned().collect();

                for (name, maria_val) in &maria_signals {
                    if let Some(ref_val) = ref_signals.get(name) {
                        if maria_val != ref_val {
                            mismatches.push(Mismatch {
                                signal: name.clone(),
                                maria_value: *maria_val,
                                reference_value: *ref_val,
                                reference_sim: ref_sim.simulator,
                                kind: MismatchKind::SignalValue,
                            });
                        }
                    }
                }
            }
        }

        mismatches
    }
}

impl DifferentialResult {
    pub fn has_bug(&self) -> bool {
        !self.mismatches.is_empty()
    }

    pub fn summary(&self) -> String {
        format!(
            "Maria: {} | Refs: {} | Mismatches: {}",
            if self.maria.success { "OK" } else { "FAIL" },
            self.references.iter().filter(|r| r.success).count(),
            self.mismatches.len()
        )
    }
}