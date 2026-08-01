import { invoke } from "@tauri-apps/api/core";

// ── Types matching backend structs ──

export interface ProjectInfo {
  name: string;
  root: string;
  files: string[];
}

export interface CompileResult {
  success: boolean;
  modules: string[];
  packages: string[];
  interfaces: string[];
  classes: string[];
  errors: Diagnostic[];
  warnings: Diagnostic[];
  parse_time_ms: number;
  preprocess_time_ms: number;
  lex_time_ms: number;
  index_time_ms: number;
  total_time_ms: number;
  cached_files: number;
  processed_files: number;
}

export interface Diagnostic {
  file: string;
  line: number;
  column: number;
  message: string;
  level: string;
}

export interface SignalInfo {
  name: string;
  width: number;
  value: string;
  kind: string;
  is_input: boolean;
  is_output: boolean;
}

export interface SimResult {
  success: boolean;
  signals: SignalInfo[];
  cycles: number;
  sim_time_ms: number;
}

export interface ModuleInfo {
  name: string;
  file: string;
  line: number;
  kind: string;
  ports: PortInfo[];
  params: ParamInfo[];
  instances: InstanceInfo[];
}

export interface PortInfo {
  name: string;
  direction: string;
  width: number;
  is_signed: boolean;
}

export interface ParamInfo {
  name: string;
  has_default: boolean;
  is_type: boolean;
  is_local: boolean;
}

export interface InstanceInfo {
  name: string;
  module_name: string;
}

export interface HierarchyNode {
  name: string;
  kind: string;
  file?: string;
  line?: number;
  children: HierarchyNode[];
}

export interface FileTreeNode {
  name: string;
  path: string;
  kind: string;
  children?: FileTreeNode[];
}

export interface SearchResult {
  file: string;
  line: number;
  column: number;
  text: string;
  match_type: string;
}

export interface BenchmarkData {
  parse_time_ms: number;
  preprocess_time_ms: number;
  lex_time_ms: number;
  parse_ms: number;
  index_time_ms: number;
  total_time_ms: number;
  cached_files: number;
  processed_files: number;
  tokens_lexed: number;
  modules_count: number;
  signals_count: number;
}

export interface CoverageData {
  statement: number;
  branch: number;
  toggle: number;
  fsm: number;
  assertion: number;
  function: number;
}

export interface ModuleDependency {
  from: string;
  to: string;
}

// ── Project & File Operations ──

export async function openProject(path: string): Promise<ProjectInfo> {
  return invoke("open_project", { path });
}

export async function getFileTree(root: string): Promise<FileTreeNode[]> {
  return invoke("get_file_tree", { root });
}

export async function readFile(path: string): Promise<string> {
  return invoke("read_file", { path });
}

export async function writeFile(path: string, content: string): Promise<void> {
  return invoke("write_file", { path, content });
}

export async function createFile(path: string): Promise<void> {
  return invoke("create_file", { path });
}

// ── Compilation & Elaboration ──

export async function compileProject(paths: string[]): Promise<CompileResult> {
  return invoke("compile_project", { paths });
}

export async function elaborateDesign(): Promise<void> {
  return invoke("elaborate_design");
}

// ── Module & Hierarchy Queries ──

export async function getModules(): Promise<ModuleInfo[]> {
  return invoke("get_modules");
}

export async function getHierarchy(): Promise<HierarchyNode> {
  return invoke("get_hierarchy");
}

export async function getDependencies(): Promise<ModuleDependency[]> {
  return invoke("get_dependencies");
}

// ── Simulation ──

export async function runSimulation(maxTime: number): Promise<SimResult> {
  return invoke("run_simulation", { maxTime });
}

export async function getSignalValue(name: string): Promise<string> {
  return invoke("get_signal_value", { name });
}

// ── Search ──

export async function searchSymbols(query: string): Promise<SearchResult[]> {
  return invoke("search_symbols", { query });
}

export async function grepSearch(pattern: string, path: string, include?: string): Promise<SearchResult[]> {
  return invoke("grep_search", { pattern, path, include });
}

// ── Benchmark & Coverage ──

export async function getBenchmarkData(): Promise<BenchmarkData> {
  return invoke("get_benchmark_data");
}

export async function getCoverageData(): Promise<CoverageData> {
  return invoke("get_coverage_data");
}

// ── Terminal ──

export async function runCommand(command: string, args: string[], cwd: string): Promise<string> {
  return invoke("run_command", { command, args, cwd });
}
