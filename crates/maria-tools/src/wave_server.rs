//! ENT-26: Waveform Database Server — HTTP API for VCD/FST access.
//!
//! Menyediakan akses remote ke waveform files via HTTP API.
//! Client dapat query signal values, browse hierarchy, dan
//! download waveform subsets.
//!
//! API endpoints:
//! - GET /api/v1/files — list available waveform files
//! - GET /api/v1/files/:id/hierarchy — browse scope hierarchy
//! - GET /api/v1/files/:id/signals — list signals in scope
//! - GET /api/v1/files/:id/values/:signal — get signal values
//! - GET /api/v1/files/:id/range?t1=N&t2=M — get values in time range

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// Waveform file record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaveformFile {
    pub id: String,
    pub name: String,
    pub path: String,
    pub format: String, // "vcd" or "fst"
    pub size_bytes: u64,
    pub signal_count: usize,
    pub time_range: (u64, u64),
    pub uploaded_at: String,
}

/// Hierarchy scope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HierarchyScope {
    pub name: String,
    pub full_path: String,
    pub child_scopes: Vec<String>,
    pub signal_count: usize,
}

/// Signal info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalInfo {
    pub name: String,
    pub width: u32,
    pub scope: String,
    pub signal_type: String, // "wire", "reg", "logic"
}

/// Signal value at a specific time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalValue {
    pub time: u64,
    pub value: String,
}

/// API response wrapper.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiResponse<T: Serialize> {
    pub success: bool,
    pub data: Option<T>,
    pub error: Option<String>,
}

impl<T: Serialize> ApiResponse<T> {
    pub fn ok(data: T) -> Self {
        ApiResponse {
            success: true,
            data: Some(data),
            error: None,
        }
    }

    pub fn err(msg: &str) -> Self {
        ApiResponse {
            success: false,
            data: None,
            error: Some(msg.to_string()),
        }
    }
}

/// Waveform database server — manages waveform files dan handles queries.
pub struct WaveformServer {
    files: Mutex<HashMap<String, WaveformFile>>,
    data_dir: PathBuf,
}

impl WaveformServer {
    pub fn new(data_dir: PathBuf) -> Self {
        WaveformServer {
            files: Mutex::new(HashMap::new()),
            data_dir,
        }
    }

    /// Register waveform file.
    pub fn register_file(&self, path: &Path) -> Result<WaveformFile, String> {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_string())
            .unwrap_or_default();
        let metadata = std::fs::metadata(path)
            .map_err(|e| format!("failed to read metadata: {}", e))?;

        let id = format!("wf_{}", name.replace('.', "_"));
        let file = WaveformFile {
            id: id.clone(),
            name,
            path: path.to_string_lossy().to_string(),
            format: ext,
            size_bytes: metadata.len(),
            signal_count: 0, // would be parsed from file
            time_range: (0, 0),
            uploaded_at: chrono_now(),
        };

        self.files.lock().unwrap().insert(id, file.clone());
        Ok(file)
    }

    /// List all registered files.
    pub fn list_files(&self) -> Vec<WaveformFile> {
        self.files.lock().unwrap().values().cloned().collect()
    }

    /// Get file by ID.
    pub fn get_file(&self, id: &str) -> Option<WaveformFile> {
        self.files.lock().unwrap().get(id).cloned()
    }

    /// Get hierarchy for a file (stub — real impl would parse VCD/FST).
    pub fn get_hierarchy(&self, id: &str) -> Result<Vec<HierarchyScope>, String> {
        if self.files.lock().unwrap().contains_key(id) {
            // Stub: return example hierarchy
            Ok(vec![HierarchyScope {
                name: "top".into(),
                full_path: "top".into(),
                child_scopes: vec!["top.u_cpu".into(), "top.u_mem".into()],
                signal_count: 5,
            }])
        } else {
            Err(format!("file '{}' not found", id))
        }
    }

    /// Get signals in a scope (stub).
    pub fn get_signals(&self, id: &str, _scope: &str) -> Result<Vec<SignalInfo>, String> {
        if self.files.lock().unwrap().contains_key(id) {
            Ok(vec![
                SignalInfo {
                    name: "clk".into(),
                    width: 1,
                    scope: "top".into(),
                    signal_type: "logic".into(),
                },
                SignalInfo {
                    name: "data".into(),
                    width: 32,
                    scope: "top".into(),
                    signal_type: "logic".into(),
                },
            ])
        } else {
            Err(format!("file '{}' not found", id))
        }
    }

    /// Get signal values (stub).
    pub fn get_signal_values(
        &self,
        id: &str,
        _signal: &str,
        t1: Option<u64>,
        t2: Option<u64>,
    ) -> Result<Vec<SignalValue>, String> {
        if self.files.lock().unwrap().contains_key(id) {
            let start = t1.unwrap_or(0);
            let end = t2.unwrap_or(100);
            let values: Vec<SignalValue> = (start..=end)
                .step_by(10)
                .map(|t| SignalValue {
                    time: t,
                    value: format!("{:08x}", t & 0xFFFF_FFFF),
                })
                .collect();
            Ok(values)
        } else {
            Err(format!("file '{}' not found", id))
        }
    }

    /// Generate OpenAPI spec (JSON).
    pub fn openapi_spec(&self) -> String {
        serde_json::json!({
            "openapi": "3.0.0",
            "info": {
                "title": "Maria Waveform Server",
                "version": "0.3.0"
            },
            "paths": {
                "/api/v1/files": {
                    "get": {"summary": "List waveform files"}
                },
                "/api/v1/files/{id}/hierarchy": {
                    "get": {"summary": "Browse scope hierarchy"}
                },
                "/api/v1/files/{id}/signals": {
                    "get": {"summary": "List signals in scope"}
                },
                "/api/v1/files/{id}/values/{signal}": {
                    "get": {"summary": "Get signal values"}
                }
            }
        })
        .to_string()
    }

    /// Summary stats.
    pub fn summary(&self) -> String {
        let files = self.files.lock().unwrap();
        format!(
            "WaveformServer: {} files registered, data dir: {}",
            files.len(),
            self.data_dir.display()
        )
    }
}

fn chrono_now() -> String {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}s", d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_register_and_list() {
        let dir = TempDir::new().unwrap();
        let vcd = dir.path().join("test.vcd");
        std::fs::write(&vcd, "$timescale 1ns $end").unwrap();
        let server = WaveformServer::new(dir.path().to_path_buf());
        let file = server.register_file(&vcd).unwrap();
        assert_eq!(server.list_files().len(), 1);
        assert_eq!(file.format, "vcd");
    }

    #[test]
    fn test_get_hierarchy() {
        let dir = TempDir::new().unwrap();
        let vcd = dir.path().join("test.vcd");
        std::fs::write(&vcd, "$timescale 1ns $end").unwrap();
        let server = WaveformServer::new(dir.path().to_path_buf());
        let file = server.register_file(&vcd).unwrap();
        let hierarchy = server.get_hierarchy(&file.id).unwrap();
        assert!(!hierarchy.is_empty());
    }

    #[test]
    fn test_get_signals() {
        let dir = TempDir::new().unwrap();
        let vcd = dir.path().join("test.vcd");
        std::fs::write(&vcd, "$timescale 1ns $end").unwrap();
        let server = WaveformServer::new(dir.path().to_path_buf());
        let file = server.register_file(&vcd).unwrap();
        let signals = server.get_signals(&file.id, "top").unwrap();
        assert_eq!(signals.len(), 2);
    }

    #[test]
    fn test_get_signal_values() {
        let dir = TempDir::new().unwrap();
        let vcd = dir.path().join("test.vcd");
        std::fs::write(&vcd, "$timescale 1ns $end").unwrap();
        let server = WaveformServer::new(dir.path().to_path_buf());
        let file = server.register_file(&vcd).unwrap();
        let values = server
            .get_signal_values(&file.id, "clk", Some(0), Some(50))
            .unwrap();
        assert!(!values.is_empty());
    }

    #[test]
    fn test_not_found() {
        let dir = TempDir::new().unwrap();
        let server = WaveformServer::new(dir.path().to_path_buf());
        assert!(server.get_hierarchy("nonexistent").is_err());
    }

    #[test]
    fn test_openapi_spec() {
        let dir = TempDir::new().unwrap();
        let server = WaveformServer::new(dir.path().to_path_buf());
        let spec = server.openapi_spec();
        assert!(spec.contains("openapi"));
        assert!(spec.contains("files"));
    }

    #[test]
    fn test_summary() {
        let dir = TempDir::new().unwrap();
        let server = WaveformServer::new(dir.path().to_path_buf());
        let s = server.summary();
        assert!(s.contains("WaveformServer"));
        assert!(s.contains("0 files"));
    }
}
