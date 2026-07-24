import { invoke } from "@tauri-apps/api/core";

export interface CompileResult {
  success: boolean;
  modules: string[];
  errors: string[];
  parse_time_ms: number;
  elab_time_ms: number;
}

export interface SimResult {
  success: boolean;
  signals: { name: string; width: number; value: string }[];
  cycles: number;
  sim_time_ms: number;
}

export async function compileFile(path: string): Promise<CompileResult> {
  return invoke("compile_file", { path });
}

export async function runSimulation(maxTime: number): Promise<SimResult> {
  return invoke("run_simulation", { maxTime });
}

export async function getSignalValue(name: string): Promise<string> {
  return invoke("get_signal_value", { name });
}

export async function listPackages(): Promise<string[]> {
  return invoke("list_packages");
}

export async function getDiagnostics(): Promise<string[]> {
  return invoke("get_diagnostics");
}