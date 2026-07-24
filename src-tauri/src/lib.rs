use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{Emitter, State};

use maria::frontend::compile_session::{CompileSession, SessionConfig};
use maria::frontend::discovery::FileDiscovery;
use maria::ir::IrDesign;
use maria::ir::LogicVec;
use maria::elaboration::Elaborator;

pub struct AppState {
    pub session: Mutex<Option<CompileSession>>,
    pub project_root: Mutex<Option<PathBuf>>,
    pub current_design: Mutex<Option<IrDesign>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            session: Mutex::new(None),
            project_root: Mutex::new(None),
            current_design: Mutex::new(None),
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

#[tauri::command]
async fn open_project(path: String, state: State<'_, AppState>) -> Result<ProjectInfo, String> {
    let path_buf = PathBuf::from(&path);
    
    if path_buf.is_dir() {
        // Scan for .maria file or .sv files
        let files = scan_sv_files(&path_buf).await?;
        let name = path_buf
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Untitled")
            .to_string();
        
        *state.project_root.lock().unwrap() = Some(path_buf);
        
        Ok(ProjectInfo {
            name,
            root: path,
            files,
        })
    } else if path_buf.extension().and_then(|s| s.to_str()) == Some("maria") {
        // Read .maria project file
        let content = tokio::fs::read_to_string(&path_buf).await
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

async fn scan_sv_files(dir: &Path) -> Result<Vec<String>, String> {
    let mut files = Vec::new();
    let mut entries = tokio::fs::read_dir(dir).await
        .map_err(|e| format!("Failed to read directory: {}", e))?;
    
    while let Some(entry) = entries.next_entry().await
        .map_err(|e| format!("Failed to read entry: {}", e))? {
        let path = entry.path();
        if path.is_dir() {
            files.extend(scan_sv_files(&path).await?);
        } else if path.extension().and_then(|s| s.to_str()) == Some("sv") {
            files.push(path.to_string_lossy().to_string());
        }
    }
    Ok(files)
}

#[tauri::command]
async fn get_file_tree(root: String) -> Result<Vec<FileTreeNode>, String> {
    build_tree(&PathBuf::from(root)).await
}

async fn build_tree(dir: &Path) -> Result<Vec<FileTreeNode>, String> {
    let mut nodes = Vec::new();
    let mut entries = tokio::fs::read_dir(dir).await
        .map_err(|e| format!("Failed to read directory: {}", e))?;
    
    while let Some(entry) = entries.next_entry().await
        .map_err(|e| format!("Failed to read entry: {}", e))? {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let kind = if path.is_dir() { "directory" } else { "file" };
        
        let children = if path.is_dir() {
            Some(build_tree(&path).await.unwrap_or_default())
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
    
    nodes.sort_by(|a, b| {
        match (a.kind.as_str(), b.kind.as_str()) {
            ("directory", "file") => std::cmp::Ordering::Less,
            ("file", "directory") => std::cmp::Ordering::Greater,
            _ => a.name.cmp(&b.name),
        }
    });
    
    Ok(nodes)
}

#[tauri::command]
async fn read_file(path: String) -> Result<String, String> {
    tokio::fs::read_to_string(&path).await
        .map_err(|e| format!("Failed to read file: {}", e))
}

#[tauri::command]
async fn write_file(path: String, content: String) -> Result<(), String> {
    tokio::fs::write(&path, content).await
        .map_err(|e| format!("Failed to write file: {}", e))
}

#[tauri::command]
async fn create_file(path: String) -> Result<(), String> {
    let path_buf = PathBuf::from(&path);
    if let Some(parent) = path_buf.parent() {
        tokio::fs::create_dir_all(parent).await
            .map_err(|e| format!("Failed to create directory: {}", e))?;
    }
    tokio::fs::write(&path, "").await
        .map_err(|e| format!("Failed to create file: {}", e))
}

#[tauri::command]
fn compile_project(paths: Vec<String>, state: State<AppState>) -> Result<CompileResult, String> {
    let mut config = SessionConfig::default();
    for p in &paths {
        config.add_source(p);
    }
    config.use_lazy_elab = true;
    
    let mut session = CompileSession::new(config);
    
    let start = std::time::Instant::now();
    let result = session.compile();
    let total_time = start.elapsed().as_secs_f64() * 1000.0;
    
    match result {
        Ok((design, module_index)) => {
            let modules: Vec<String> = design.modules.iter().map(|m| m.name.to_string()).collect();
            let packages: Vec<String> = design.packages.iter().map(|p| p.name.to_string()).collect();
            let interfaces: Vec<String> = design.interfaces.iter().map(|i| i.name.to_string()).collect();
            let classes: Vec<String> = design.classes.iter().map(|c| c.name.to_string()).collect();
            
            // Get diagnostics from parse errors
            let mut errors = Vec::new();
            let mut warnings = Vec::new();
            
            // Store session and design
            *state.session.lock().unwrap() = Some(session);
            *state.current_design.lock().unwrap() = None; // Will be set on elaborate
            
            Ok(CompileResult {
                success: true,
                modules,
                packages,
                interfaces,
                classes,
                errors,
                warnings,
                parse_time_ms: 0.0,
                preprocess_time_ms: 0.0,
                lex_time_ms: 0.0,
                index_time_ms: 0.0,
                total_time_ms: total_time,
                cached_files: 0,
                processed_files: paths.len(),
            })
        }
        Err(e) => {
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
                index_time_ms: 0.0,
                total_time_ms: total_time,
                cached_files: 0,
                processed_files: 0,
            })
        }
    }
}

#[tauri::command]
fn elaborate_design(state: State<AppState>) -> Result<(), String> {
    let mut session_guard = state.session.lock().unwrap();
    let session = session_guard.as_mut().ok_or("No compiled design")?;
    
    let (design, _) = session.compile().map_err(|e| e.to_string())?;
    let mut elaborator = Elaborator::new(design);
    let ir_design = elaborator.elaborate(None).map_err(|e| e.to_string())?;
    
    *state.current_design.lock().unwrap() = Some(ir_design);
    Ok(())
}

#[tauri::command]
fn get_modules(state: State<AppState>) -> Result<Vec<ModuleInfo>, String> {
    let session_guard = state.session.lock().unwrap();
    let session = session_guard.as_ref().ok_or("No compiled design")?;
    
    let modules = session.module_index.entries()
        .filter(|(_, meta)| meta.kind == maria::frontend::module_index::EntryKind::Module)
        .map(|(name, meta)| ModuleInfo {
            name: name.to_string(),
            file: meta.file.to_string_lossy().to_string(),
            line: 0, // TODO: get actual line from AST
            kind: "module".into(),
            ports: meta.ports.iter().map(|p| PortInfo {
                name: p.to_string(),
                direction: "inout".into(),
                width: 1,
                is_signed: false,
            }).collect(),
            params: meta.params.iter().map(|p| ParamInfo {
                name: p.name.to_string(),
                has_default: p.has_default,
                is_type: p.is_type_param,
                is_local: p.is_local,
            }).collect(),
            instances: meta.instances.iter().map(|i| InstanceInfo {
                name: "".into(), // Instance name not stored in ModuleMeta
                module_name: i.to_string(),
            }).collect(),
        })
        .collect();
    
    Ok(modules)
}

#[tauri::command]
fn get_hierarchy(state: State<AppState>) -> Result<HierarchyNode, String> {
    let design_guard = state.current_design.lock().unwrap();
    let design = design_guard.as_ref().ok_or("No elaborated design")?;
    
    fn build_hierarchy(design: &IrDesign, module_name: &str, depth: usize) -> HierarchyNode {
        let module = design.modules.get(&maria::intern::Symbol::intern(module_name));
        
        let children = module.map(|m| {
            m.sub_instances.iter().filter_map(|inst| {
                let child = build_hierarchy(design, &inst.module_name.to_string(), depth + 1);
                Some(child)
            }).collect()
        }).unwrap_or_default();
        
        HierarchyNode {
            name: module_name.into(),
            kind: "module".into(),
            file: module.as_ref().and_then(|m| {
                // Try to get file from session
                None
            }),
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
    for (name, meta) in &session.module_index.entries() {
        if meta.kind == maria::frontend::module_index::EntryKind::Module {
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

#[derive(Serialize, Deserialize)]
pub struct ModuleDependency {
    pub from: String,
    pub to: String,
}

#[tauri::command]
fn search_symbols(query: String, state: State<AppState>) -> Result<Vec<SearchResult>, String> {
    let session_guard = state.session.lock().unwrap();
    let session = session_guard.as_ref().ok_or("No compiled design")?;
    
    let mut results = Vec::new();
    let query_lower = query.to_lowercase();
    
    for (name, meta) in &session.module_index.entries() {
        if name.to_string().to_lowercase().contains(&query_lower) {
            results.push(SearchResult {
                file: meta.file.to_string_lossy().to_string(),
                line: 0,
                column: 0,
                text: format!("{} ({})", name, meta.kind),
                match_type: format!("{:?}", meta.kind),
            });
        }
    }
    
    Ok(results)
}

#[tauri::command]
fn run_simulation(
    max_time: u64,
    state: State<AppState>,
) -> Result<SimResult, String> {
    let design_guard = state.current_design.lock().unwrap();
    let design = design_guard.as_ref().ok_or("No elaborated design")?;
    
    let start = std::time::Instant::now();
    let mut engine = maria::simulator::SimulationEngine::new(design, 0);
    engine.run(max_time);
    let sim_time = start.elapsed().as_secs_f64() * 1000.0;
    
    let signals: Vec<SignalInfo> = engine
        .get_signal_names()
        .iter()
        .map(|name| {
            let lv = engine.get_signal_value(name).unwrap_or(LogicVec::from(0u64));
            let info = engine.get_signal_info(name);
            SignalInfo {
                name: name.clone(),
                width: lv.width(),
                value: lv.to_hex_string(),
                kind: info.map(|i| format!("{:?}", i.kind)).unwrap_or("unknown".into()),
                is_input: info.map(|i| i.is_input).unwrap_or(false),
                is_output: info.map(|i| i.is_output).unwrap_or(false),
            }
        })
        .collect();
    
    Ok(SimResult {
        success: true,
        signals,
        cycles: engine.current_time(),
        sim_time_ms: sim_time,
    })
}

#[tauri::command]
fn get_benchmark_data(state: State<AppState>) -> Result<BenchmarkData, String> {
    let session_guard = state.session.lock().unwrap();
    let session = session_guard.as_ref().ok_or("No session")?;
    
    Ok(BenchmarkData {
        parse_time_ms: session.timing.parse_ms as f64,
        preprocess_time_ms: session.timing.preprocess_ms as f64,
        lex_time_ms: session.timing.lex_ms as f64,
        parse_ms: session.timing.parse_ms as f64,
        index_time_ms: session.timing.index_ms as f64,
        total_time_ms: session.timing.total_ms as f64,
        cached_files: session.timing.cached_files,
        processed_files: session.timing.processed_files,
        tokens_lexed: 0, // Not tracked currently
        modules_count: session.module_index.len(),
        signals_count: 0,
    })
}

#[tauri::command]
fn get_coverage_data(_state: State<AppState>) -> Result<CoverageData, String> {
    // Coverage data would come from simulation run with coverage enabled
    Ok(CoverageData {
        statement: 0.0,
        branch: 0.0,
        toggle: 0.0,
        fsm: 0.0,
        assertion: 0.0,
        function: 0.0,
    })
}

#[tauri::command]
fn get_signal_value(name: String, state: State<AppState>) -> Result<String, String> {
    let design_guard = state.current_design.lock().unwrap();
    let design = design_guard.as_ref().ok_or("No elaborated design")?;
    
    let engine = maria::simulator::SimulationEngine::new(design, 0);
    let lv = engine.get_signal_value(&name).unwrap_or(LogicVec::from(0u64));
    Ok(lv.to_hex_string())
}

#[tauri::command]
async fn grep_search(pattern: String, path: String, include: Option<String>) -> Result<Vec<SearchResult>, String> {
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
    
    let output = cmd.output().await
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
async fn open_terminal_shell(cwd: String) -> Result<(), String> {
    // This would launch an external terminal or use a pty
    // For now, just return success
    Ok(())
}

#[tauri::command]
async fn run_command(command: String, args: Vec<String>, cwd: String) -> Result<String, String> {
    use tokio::process::Command;
    
    let mut cmd = Command::new(command);
    cmd.args(args).current_dir(cwd);
    
    let output = cmd.output().await
        .map_err(|e| format!("Failed to run command: {}", e))?;
    
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    
    if output.status.success() {
        Ok(stdout.to_string())
    } else {
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