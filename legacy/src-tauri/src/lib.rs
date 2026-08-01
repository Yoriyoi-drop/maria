use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::State;

use maria::frontend::compile_session::{CompileSession, SessionConfig};
use maria::frontend::module_index::EntryKind;
use maria::ir::{IrDesign, LogicVal, LogicVec};

pub struct AppState {
    pub session: Mutex<Option<CompileSession>>,
    pub project_root: Mutex<Option<PathBuf>>,
    pub current_design: Mutex<Option<IrDesign>>,
    /// Signal values from the last simulation run (for post-sim queries)
    pub last_sim_signals: Mutex<Option<Vec<LogicVec>>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            session: Mutex::new(None),
            project_root: Mutex::new(None),
            current_design: Mutex::new(None),
            last_sim_signals: Mutex::new(None),
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct CompileResult {
    pub success: bool,
    pub modules: Vec<String>,
    pub packages: Vec<String>,
    pub interfaces: Vec<String>,
    pub classes: Vec<String>,
    pub errors: Vec<Diagnostic>,
    pub warnings: Vec<Diagnostic>,
    pub parse_time_ms: f64,
    pub preprocess_time_ms: f64,
    pub lex_time_ms: f64,
    pub elab_time_ms: f64,
    pub index_time_ms: f64,
    pub total_time_ms: f64,
    pub cached_files: usize,
    pub processed_files: usize,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Diagnostic {
    pub file: String,
    pub line: usize,
    pub column: usize,
    pub message: String,
    pub level: String,
}

#[derive(Serialize, Deserialize)]
pub struct SignalInfo {
    pub name: String,
    pub width: usize,
    pub value: String,
    pub kind: String,
    pub is_input: bool,
    pub is_output: bool,
}

#[derive(Serialize, Deserialize)]
pub struct SimResult {
    pub success: bool,
    pub signals: Vec<SignalInfo>,
    pub cycles: u64,
    pub sim_time_ms: f64,
}

#[derive(Serialize, Deserialize)]
pub struct ModuleInfo {
    pub name: String,
    pub file: String,
    pub line: usize,
    pub kind: String,
    pub ports: Vec<PortInfo>,
    pub params: Vec<ParamInfo>,
    pub instances: Vec<InstanceInfo>,
}

#[derive(Serialize, Deserialize)]
pub struct PortInfo {
    pub name: String,
    pub direction: String,
    pub width: usize,
    pub is_signed: bool,
}

#[derive(Serialize, Deserialize)]
pub struct ParamInfo {
    pub name: String,
    pub has_default: bool,
    pub is_type: bool,
    pub is_local: bool,
}

#[derive(Serialize, Deserialize)]
pub struct InstanceInfo {
    pub name: String,
    pub module_name: String,
}

#[derive(Serialize, Deserialize)]
pub struct HierarchyNode {
    pub name: String,
    pub kind: String,
    pub file: Option<String>,
    pub line: Option<usize>,
    pub children: Vec<HierarchyNode>,
}

#[derive(Serialize, Deserialize)]
pub struct FileTreeNode {
    pub name: String,
    pub path: String,
    pub kind: String,
    pub children: Option<Vec<FileTreeNode>>,
}

#[derive(Serialize, Deserialize)]
pub struct SearchResult {
    pub file: String,
    pub line: usize,
    pub column: usize,
    pub text: String,
    pub match_type: String,
}

#[derive(Serialize, Deserialize)]
pub struct BenchmarkData {
    pub parse_time_ms: f64,
    pub preprocess_time_ms: f64,
    pub lex_time_ms: f64,
    pub parse_ms: f64,
    pub elab_time_ms: f64,
    pub index_time_ms: f64,
    pub total_time_ms: f64,
    pub cached_files: usize,
    pub processed_files: usize,
    pub tokens_lexed: u64,
    pub modules_count: usize,
    pub signals_count: usize,
}

#[derive(Serialize, Deserialize)]
pub struct CoverageData {
    pub statement: f64,
    pub branch: f64,
    pub toggle: f64,
    pub fsm: f64,
    pub assertion: f64,
    pub function: f64,
}

#[derive(Serialize, Deserialize)]
pub struct ProjectInfo {
    pub name: String,
    pub root: String,
    pub files: Vec<String>,
}

#[derive(Serialize, Deserialize)]
pub struct ModuleDependency {
    pub from: String,
    pub to: String,
}

// ── Helpers ──

/// Convert LogicVec ke hex string. Mendukung sinyal >64-bit dengan iterasi penuh bit vector.
fn logicvec_to_hex(lv: &LogicVec) -> String {
    if lv.width == 0 {
        return "0".into();
    }
    // Check for X/Z in any bit
    let all_x = lv.bits.iter().all(|b| *b == LogicVal::X);
    let all_z = lv.bits.iter().all(|b| *b == LogicVal::Z);
    if all_x {
        return format!("'x{}", lv.width);
    }
    if all_z {
        return format!("'z{}", lv.width);
    }

    // Build hex from MSB to LSB (4 bits per nibble)
    let nibbles = (lv.width + 3) / 4;
    let mut hex = String::with_capacity(nibbles);
    for nib in (0..nibbles).rev() {
        let mut val = 0u8;
        let mut has_x = false;
        let mut has_z = false;
        for bit in 0..4 {
            let idx = nib * 4 + bit;
            if idx < lv.width {
                match lv.bits[idx] {
                    LogicVal::One => val |= 1 << bit,
                    LogicVal::X => has_x = true,
                    LogicVal::Z => has_z = true,
                    LogicVal::Zero => {}
                }
            }
        }
        if has_x {
            hex.push('x');
        } else if has_z {
            hex.push('z');
        } else {
            hex.push(std::char::from_digit(val as u32, 16).unwrap_or('0'));
        }
    }
    // Trim leading zeros but keep at least one digit
    let trimmed = hex.trim_start_matches('0');
    if trimmed.is_empty() { "0".into() } else { trimmed.to_string() }
}

fn find_signal_id<'a>(design: &'a IrDesign, name: &str) -> Option<(usize, &'a maria::ir::SignalInfo)> {
    design
        .top
        .signals
        .iter()
        .enumerate()
        .find(|(_, s)| s.name.as_str() == name)
}

/// Scan directory for .sv files (synchronous, iterative).
fn scan_sv_files_sync(dir: &Path) -> Result<Vec<String>, String> {
    let mut files = Vec::new();
    let mut stack: Vec<PathBuf> = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let entries = std::fs::read_dir(&current)
            .map_err(|e| format!("Failed to read directory '{}': {}", current.display(), e))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|s| s.to_str()) == Some("sv") {
                files.push(path.to_string_lossy().to_string());
            }
        }
    }
    Ok(files)
}

/// Build file tree (synchronous, iterative).
fn build_tree_sync(dir: &Path) -> Result<Vec<FileTreeNode>, String> {
    let mut nodes = Vec::new();
    let entries = std::fs::read_dir(dir)
        .map_err(|e| format!("Failed to read directory '{}': {}", dir.display(), e))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let kind = if path.is_dir() { "directory" } else { "file" };

        let children = if path.is_dir() {
            Some(build_tree_sync(&path).unwrap_or_default())
        } else {
            None
        };

        nodes.push(FileTreeNode {
            name,
            path: path.to_string_lossy().to_string(),
            kind: kind.to_string(),
            children,
        });
    }

    nodes.sort_by(|a, b| match (a.kind.as_str(), b.kind.as_str()) {
        ("directory", "file") => std::cmp::Ordering::Less,
        ("file", "directory") => std::cmp::Ordering::Greater,
        _ => a.name.cmp(&b.name),
    });

    Ok(nodes)
}

/// Build CompileResult from compile+elaborate timing data.
fn make_compile_result(
    session: &CompileSession,
    total_time: f64,
    processed: usize,
) -> CompileResult {
    let t = &session.timing;
    let pp_ms = t.preprocess_ms as f64;
    let lex_ms = t.lex_ms as f64;
    let parse_ms = t.parse_ms as f64;
    let idx_ms = t.index_ms as f64;
    let cached = t.cached_files;

    // Estimate elaboration time: total - known phases
    let known_ms = pp_ms + lex_ms + parse_ms + idx_ms;
    let elab_ms = (total_time - known_ms).max(0.0);

    // Collect module/package/interface names from module_index
    let modules: Vec<String> = session
        .module_index
        .iter()
        .filter(|(_, kind, _)| *kind == EntryKind::Module)
        .map(|(name, _, _)| name.to_string())
        .collect();

    let packages: Vec<String> = session
        .module_index
        .iter()
        .filter(|(_, kind, _)| *kind == EntryKind::Package)
        .map(|(name, _, _)| name.to_string())
        .collect();

    let interfaces: Vec<String> = session
        .module_index
        .iter()
        .filter(|(_, kind, _)| *kind == EntryKind::Interface)
        .map(|(name, _, _)| name.to_string())
        .collect();

    CompileResult {
        success: true,
        modules,
        packages,
        interfaces,
        classes: vec![],
        errors: vec![],
        warnings: vec![],
        parse_time_ms: pp_ms,
        preprocess_time_ms: pp_ms,
        lex_time_ms: lex_ms,
        elab_time_ms: elab_ms,
        index_time_ms: idx_ms,
        total_time_ms: total_time,
        cached_files: cached,
        processed_files: processed,
    }
}

// ── Tauri Commands ──

#[tauri::command]
async fn open_project(path: String, state: State<'_, AppState>) -> Result<ProjectInfo, String> {
    let path_buf = PathBuf::from(&path);

    if path_buf.is_dir() {
        let files = scan_sv_files_sync(&path_buf)?;
        let name = path_buf
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Untitled")
            .to_string();

        *state.project_root.lock().unwrap() = Some(path_buf);

        Ok(ProjectInfo { name, root: path, files })
    } else if path_buf.extension().and_then(|s| s.to_str()) == Some("maria") {
        let content = tokio::fs::read_to_string(&path_buf)
            .await
            .map_err(|e| format!("Failed to read project file: {}", e))?;

        let base_dir = path_buf.parent().unwrap_or(Path::new("."));
        let files: Vec<String> = content
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| {
                let p = base_dir.join(l);
                p.to_string_lossy().to_string()
            })
            .collect();

        let name = path_buf
            .file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or("Untitled")
            .to_string();

        *state.project_root.lock().unwrap() = Some(base_dir.to_path_buf());

        Ok(ProjectInfo { name, root: path, files })
    } else {
        Err("Path must be a directory or .maria file".into())
    }
}

#[tauri::command]
async fn get_file_tree(root: String) -> Result<Vec<FileTreeNode>, String> {
    build_tree_sync(Path::new(&root))
}

#[tauri::command]
async fn read_file(path: String) -> Result<String, String> {
    tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| format!("Failed to read file: {}", e))
}

#[tauri::command]
async fn write_file(path: String, content: String) -> Result<(), String> {
    tokio::fs::write(&path, content)
        .await
        .map_err(|e| format!("Failed to write file: {}", e))
}

#[tauri::command]
async fn create_file(path: String) -> Result<(), String> {
    let path_buf = PathBuf::from(&path);
    if let Some(parent) = path_buf.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Failed to create directory: {}", e))?;
    }
    tokio::fs::write(&path, "")
        .await
        .map_err(|e| format!("Failed to create file: {}", e))
}

/// Compile project + elaborate IR design in one pass.
/// Stores compiled session + elaborated IR design in AppState.
#[tauri::command]
fn compile_project(paths: Vec<String>, state: State<AppState>) -> Result<CompileResult, String> {
    use std::time::Instant;

    let mut config = SessionConfig::default();
    for p in &paths {
        config.sources.push(PathBuf::from(p));
    }
    config.use_lazy_elab = true;

    let mut session = CompileSession::new(config);
    let start = Instant::now();

    // Use compile_and_elaborate to get both compiled Design + elaborated IrDesign
    match session.compile_and_elaborate(None) {
        Ok((_design, ir_design, _index_len)) => {
            let total_time = start.elapsed().as_secs_f64() * 1000.0;
            let processed = paths.len();

            let result = make_compile_result(&session, total_time, processed);

            *state.session.lock().unwrap() = Some(session);
            *state.current_design.lock().unwrap() = Some(ir_design);

            Ok(result)
        }
        Err(e) => {
            let total_time = start.elapsed().as_secs_f64() * 1000.0;
            // Store session even on error for diagnostics access
            *state.session.lock().unwrap() = Some(session);

            Ok(CompileResult {
                success: false,
                modules: vec![],
                packages: vec![],
                interfaces: vec![],
                classes: vec![],
                errors: vec![Diagnostic {
                    file: "".into(),
                    line: 0,
                    column: 0,
                    message: e.to_string(),
                    level: "error".into(),
                }],
                warnings: vec![],
                parse_time_ms: 0.0,
                preprocess_time_ms: 0.0,
                lex_time_ms: 0.0,
                elab_time_ms: 0.0,
                index_time_ms: 0.0,
                total_time_ms: total_time,
                cached_files: 0,
                processed_files: 0,
            })
        }
    }
}

/// Elaborate design (from cached session or re-compile if needed).
/// Uses session's `compile_and_elaborate` which caches the IR internally.
#[tauri::command]
fn elaborate_design(state: State<AppState>) -> Result<(), String> {
    // Check if we already have an elaborated design
    {
        let design_guard = state.current_design.lock().unwrap();
        if design_guard.is_some() {
            return Ok(()); // Already elaborated
        }
    }

    // Need to elaborate
    let ir_design = {
        let mut session_guard = state.session.lock().unwrap();
        let session = session_guard.as_mut().ok_or("No compiled design")?;

        // Try cached IR first
        if let Some(cached) = session.get_cached_ir() {
            cached.clone()
        } else {
            // Compile + elaborate
            let (_, ir_design, _) =
                session.compile_and_elaborate(None).map_err(|e| e.to_string())?;
            ir_design
        }
    };

    *state.current_design.lock().unwrap() = Some(ir_design);
    Ok(())
}

#[tauri::command]
fn get_modules(state: State<AppState>) -> Result<Vec<ModuleInfo>, String> {
    let session_guard = state.session.lock().unwrap();
    let session = session_guard.as_ref().ok_or("No compiled design")?;

    let modules: Vec<ModuleInfo> = session
        .module_index
        .iter()
        .filter(|(_, kind, _)| *kind == EntryKind::Module)
        .map(|(name, _, meta)| ModuleInfo {
            name: name.to_string(),
            file: meta.file.to_string_lossy().to_string(),
            line: 0,
            kind: "module".into(),
            ports: meta
                .ports
                .iter()
                .map(|p| PortInfo {
                    name: p.to_string(),
                    direction: "inout".into(),
                    width: 1,
                    is_signed: false,
                })
                .collect(),
            params: meta
                .params
                .iter()
                .map(|p| ParamInfo {
                    name: p.name.to_string(),
                    has_default: p.has_default,
                    is_type: p.is_type,
                    is_local: p.is_local,
                })
                .collect(),
            instances: meta
                .instances
                .iter()
                .map(|i| InstanceInfo {
                    name: "".into(),
                    module_name: i.to_string(),
                })
                .collect(),
        })
        .collect();

    Ok(modules)
}

#[tauri::command]
fn get_hierarchy(state: State<AppState>) -> Result<HierarchyNode, String> {
    let design_guard = state.current_design.lock().unwrap();
    let design = design_guard.as_ref().ok_or("No elaborated design")?;

    fn build_hierarchy(design: &IrDesign, module_name: &str, _depth: usize) -> HierarchyNode {
        let module = design
            .modules
            .get(&maria::intern::Symbol::intern(module_name));

        let children = module
            .map(|m| {
                m.sub_instances
                    .iter()
                    .filter_map(|inst| {
                        let child =
                            build_hierarchy(design, &inst.module_name.to_string(), _depth + 1);
                        Some(child)
                    })
                    .collect()
            })
            .unwrap_or_default();

        HierarchyNode {
            name: module_name.into(),
            kind: "module".into(),
            file: None,
            line: None,
            children,
        }
    }

    let top = design.top.name.to_string();
    Ok(build_hierarchy(design, &top, 0))
}

#[tauri::command]
fn get_dependencies(state: State<AppState>) -> Result<Vec<ModuleDependency>, String> {
    let session_guard = state.session.lock().unwrap();
    let session = session_guard.as_ref().ok_or("No compiled design")?;

    let mut deps = Vec::new();
    for (name, kind, meta) in session.module_index.iter() {
        if kind == EntryKind::Module {
            for inst in &meta.instances {
                deps.push(ModuleDependency {
                    from: name.to_string(),
                    to: inst.to_string(),
                });
            }
        }
    }
    Ok(deps)
}

#[tauri::command]
fn search_symbols(query: String, state: State<AppState>) -> Result<Vec<SearchResult>, String> {
    let session_guard = state.session.lock().unwrap();
    let session = session_guard.as_ref().ok_or("No compiled design")?;

    let mut results = Vec::new();
    let query_lower = query.to_lowercase();

    for (name, kind, meta) in session.module_index.iter() {
        if name.to_string().to_lowercase().contains(&query_lower) {
            results.push(SearchResult {
                file: meta.file.to_string_lossy().to_string(),
                line: 0,
                column: 0,
                text: format!("{} ({:?})", name, kind),
                match_type: format!("{:?}", kind),
            });
        }
    }

    Ok(results)
}

/// Run simulation on the elaborated design.
/// Stores post-sim signal values in AppState for later query via get_signal_value.
#[tauri::command]
fn run_simulation(max_time: u64, state: State<AppState>) -> Result<SimResult, String> {
    let design_guard = state.current_design.lock().unwrap();
    let design = design_guard.as_ref().ok_or("No elaborated design")?;

    let start = std::time::Instant::now();
    let mut engine = maria::simulator::SimulationEngine::new(design.clone(), max_time);
    engine.run().map_err(|e| e.to_string())?;
    let sim_time = start.elapsed().as_secs_f64() * 1000.0;

    let signals: Vec<SignalInfo> = engine
        .design
        .top
        .signals
        .iter()
        .enumerate()
        .map(|(id, sig)| {
            let lv = engine.state.read_signal(id);
            SignalInfo {
                name: sig.name.to_string(),
                width: lv.width,
                value: logicvec_to_hex(lv),
                kind: format!("{:?}", sig.kind),
                is_input: matches!(sig.kind, maria::ir::SignalKind::Input),
                is_output: matches!(sig.kind, maria::ir::SignalKind::Output),
            }
        })
        .collect();

    // Store post-sim signal values for later queries
    let post_sim_values: Vec<LogicVec> = (0..design.top.signals.len())
        .map(|id| engine.state.read_signal(id).clone())
        .collect();
    *state.last_sim_signals.lock().unwrap() = Some(post_sim_values);

    Ok(SimResult {
        success: true,
        signals,
        cycles: engine.current_time,
        sim_time_ms: sim_time,
    })
}

#[tauri::command]
fn get_benchmark_data(state: State<AppState>) -> Result<BenchmarkData, String> {
    let session_guard = state.session.lock().unwrap();
    let session = session_guard.as_ref().ok_or("No session")?;

    let design_guard = state.current_design.lock().unwrap();
    let sig_count = design_guard.as_ref().map(|d| d.top.signals.len()).unwrap_or(0);

    Ok(BenchmarkData {
        parse_time_ms: session.timing.parse_ms as f64,
        preprocess_time_ms: session.timing.preprocess_ms as f64,
        lex_time_ms: session.timing.lex_ms as f64,
        parse_ms: session.timing.parse_ms as f64,
        elab_time_ms: session.timing.index_ms as f64,
        index_time_ms: session.timing.index_ms as f64,
        total_time_ms: session.timing.total_ms as f64,
        cached_files: session.timing.cached_files,
        processed_files: session.timing.processed_files,
        tokens_lexed: 0,
        modules_count: session.module_index.len(),
        signals_count: sig_count,
    })
}

#[tauri::command]
fn get_coverage_data(_state: State<AppState>) -> Result<CoverageData, String> {
    Ok(CoverageData {
        statement: 0.0,
        branch: 0.0,
        toggle: 0.0,
        fsm: 0.0,
        assertion: 0.0,
        function: 0.0,
    })
}

/// Get signal value — reads post-sim values if available, otherwise reads from design initial values.
#[tauri::command]
fn get_signal_value(name: String, state: State<AppState>) -> Result<String, String> {
    let design_guard = state.current_design.lock().unwrap();
    let design = design_guard.as_ref().ok_or("No elaborated design")?;

    // Find signal ID
    let (sig_id, _) = find_signal_id(design, &name).ok_or_else(|| format!("Signal '{}' not found", name))?;

    // Try post-sim values first (if simulation was run)
    let last_sim = state.last_sim_signals.lock().unwrap();
    if let Some(ref post_sim) = *last_sim {
        if sig_id < post_sim.len() {
            return Ok(logicvec_to_hex(&post_sim[sig_id]));
        }
    }

    // Fall back to initial values from design
    if sig_id < design.top.signals.len() {
        return Ok(logicvec_to_hex(&design.top.signals[sig_id].init_val));
    }

    Err(format!("Signal '{}' not found", name))
}

#[tauri::command]
async fn grep_search(
    pattern: String,
    path: String,
    include: Option<String>,
) -> Result<Vec<SearchResult>, String> {
    use tokio::process::Command;

    let mut cmd = Command::new("rg");
    cmd.arg("--json")
        .arg("--line-number")
        .arg("--column")
        .arg(&pattern);

    if let Some(inc) = include {
        cmd.arg("--glob").arg(inc);
    }

    cmd.arg(&path);

    let output = cmd
        .output()
        .await
        .map_err(|e| format!("Failed to run ripgrep: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut results = Vec::new();

    for line in stdout.lines() {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(line) {
            if json["type"] == "match" {
                let data = &json["data"];
                results.push(SearchResult {
                    file: data["path"]["text"].as_str().unwrap_or("").to_string(),
                    line: data["line_number"].as_u64().unwrap_or(0) as usize,
                    column: data["submatches"][0]["start"].as_u64().unwrap_or(0) as usize,
                    text: data["lines"]["text"].as_str().unwrap_or("").trim().to_string(),
                    match_type: "grep".into(),
                });
            }
        }
    }

    Ok(results)
}

#[tauri::command]
async fn open_terminal_shell(_cwd: String) -> Result<(), String> {
    Ok(())
}

#[tauri::command]
async fn run_command(command: String, args: Vec<String>, cwd: String) -> Result<String, String> {
    use tokio::process::Command;

    let mut cmd = Command::new(command);
    cmd.args(args).current_dir(cwd);

    let output = cmd
        .output()
        .await
        .map_err(|e| format!("Failed to run command: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);

    if output.status.success() {
        Ok(stdout.to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(stderr.to_string())
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_process::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            open_project,
            get_file_tree,
            read_file,
            write_file,
            create_file,
            compile_project,
            elaborate_design,
            get_modules,
            get_hierarchy,
            get_dependencies,
            search_symbols,
            run_simulation,
            get_benchmark_data,
            get_coverage_data,
            get_signal_value,
            grep_search,
            open_terminal_shell,
            run_command,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
