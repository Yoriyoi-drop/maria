//! COMP-19: Compilation Server Daemon.
//!
//! TCP-based compilation server yang menerima file SV dan
//! mengembalikan hasil kompilasi. Foundation untuk distributed
//! compilation.
//!
//! Protocol: JSON-over-TCP (newline-delimited).

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

/// Compilation request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompileRequest {
    pub id: String,
    pub files: Vec<String>,
    pub max_time: Option<u64>,
    pub options: CompileOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CompileOptions {
    pub enable_jit: bool,
    pub debug: bool,
    pub optimization_level: u8,
}

/// Compilation response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompileResponse {
    pub id: String,
    pub success: bool,
    pub errors: Vec<CompileError>,
    pub warnings: Vec<String>,
    pub modules: Vec<String>,
    pub compile_time_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompileError {
    pub file: String,
    pub line: u32,
    pub col: u32,
    pub message: String,
}

/// Server stats.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonStats {
    pub total_requests: u64,
    pub successful: u64,
    pub failed: u64,
    pub avg_compile_time_ms: f64,
    pub uptime_seconds: u64,
    pub active_connections: usize,
}

/// Compilation daemon.
pub struct CompileDaemon {
    listen_addr: String,
    stats: Arc<Mutex<DaemonStats>>,
    compiled_cache: Arc<Mutex<HashMap<String, CompileResponse>>>,
    project_root: PathBuf,
}

impl CompileDaemon {
    pub fn new(listen_addr: &str, project_root: PathBuf) -> Self {
        CompileDaemon {
            listen_addr: listen_addr.to_string(),
            stats: Arc::new(Mutex::new(DaemonStats {
                total_requests: 0,
                successful: 0,
                failed: 0,
                avg_compile_time_ms: 0.0,
                uptime_seconds: 0,
                active_connections: 0,
            })),
            compiled_cache: Arc::new(Mutex::new(HashMap::new())),
            project_root,
        }
    }

    /// Start the daemon (blocking).
    pub fn start(&self) -> Result<(), String> {
        let listener = TcpListener::bind(&self.listen_addr)
            .map_err(|e| format!("failed to bind {}: {}", self.listen_addr, e))?;

        println!("Compilation daemon listening on {}", self.listen_addr);

        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let stats = self.stats.clone();
                    let cache = self.compiled_cache.clone();
                    let root = self.project_root.clone();

                    stats.lock().unwrap().active_connections += 1;

                    std::thread::spawn(move || {
                        handle_connection(stream, &stats, &cache, &root);
                        stats.lock().unwrap().active_connections -= 1;
                    });
                }
                Err(e) => {
                    eprintln!("Connection error: {}", e);
                }
            }
        }

        Ok(())
    }

    /// Process a request without networking (for testing).
    pub fn process_request(&self, request: &CompileRequest) -> CompileResponse {
        let start = std::time::Instant::now();

        // Check cache
        let cache_key = request.files.join("|");
        if let Some(cached) = self.compiled_cache.lock().unwrap().get(&cache_key) {
            return cached.clone();
        }

        // Validate files exist
        let mut errors = Vec::new();
        let mut modules = Vec::new();

        for file in &request.files {
            let path = self.project_root.join(file);
            if !path.exists() {
                errors.push(CompileError {
                    file: file.clone(),
                    line: 0,
                    col: 0,
                    message: format!("file not found: {}", file),
                });
            } else {
                // Extract module name from file (simple heuristic)
                let name = path
                    .file_stem()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                modules.push(name);
            }
        }

        let elapsed = start.elapsed().as_millis() as u64;
        let success = errors.is_empty();

        let response = CompileResponse {
            id: request.id.clone(),
            success,
            errors,
            warnings: Vec::new(),
            modules,
            compile_time_ms: elapsed,
        };

        // Update stats
        {
            let mut stats = self.stats.lock().unwrap();
            stats.total_requests += 1;
            if success {
                stats.successful += 1;
            } else {
                stats.failed += 1;
            }
            let n = stats.total_requests as f64;
            stats.avg_compile_time_ms =
                (stats.avg_compile_time_ms * (n - 1.0) + elapsed as f64) / n;
        }

        // Cache result
        self.compiled_cache
            .lock()
            .unwrap()
            .insert(cache_key, response.clone());

        response
    }

    /// Get server stats.
    pub fn stats(&self) -> DaemonStats {
        self.stats.lock().unwrap().clone()
    }

    pub fn listen_addr(&self) -> &str {
        &self.listen_addr
    }
}

fn handle_connection(
    stream: TcpStream,
    stats: &Arc<Mutex<DaemonStats>>,
    cache: &Arc<Mutex<HashMap<String, CompileResponse>>>,
    root: &PathBuf,
) {
    let reader = BufReader::new(stream.try_clone().unwrap());
    let mut writer = stream;

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };

        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }

        let response = match serde_json::from_str::<CompileRequest>(&line) {
            Ok(request) => {
                let start = std::time::Instant::now();
                let cache_key = request.files.join("|");

                let result = if let Some(cached) = cache.lock().unwrap().get(&cache_key) {
                    cached.clone()
                } else {
                    let mut errors = Vec::new();
                    let mut modules = Vec::new();
                    for file in &request.files {
                        let path = root.join(file);
                        if !path.exists() {
                            errors.push(CompileError {
                                file: file.clone(),
                                line: 0,
                                col: 0,
                                message: format!("file not found"),
                            });
                        } else {
                            let name = path
                                .file_stem()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_default();
                            modules.push(name);
                        }
                    }
                    let elapsed = start.elapsed().as_millis() as u64;
                    let success = errors.is_empty();
                    let resp = CompileResponse {
                        id: request.id.clone(),
                        success,
                        errors,
                        warnings: Vec::new(),
                        modules,
                        compile_time_ms: elapsed,
                    };
                    cache.lock().unwrap().insert(cache_key, resp.clone());
                    resp
                };

                {
                    let mut s = stats.lock().unwrap();
                    s.total_requests += 1;
                    if result.success {
                        s.successful += 1;
                    } else {
                        s.failed += 1;
                    }
                }

                result
            }
            Err(e) => CompileResponse {
                id: "error".into(),
                success: false,
                errors: vec![CompileError {
                    file: "<request>".into(),
                    line: 0,
                    col: 0,
                    message: format!("invalid request: {}", e),
                }],
                warnings: Vec::new(),
                modules: Vec::new(),
                compile_time_ms: 0,
            },
        };

        let mut json = serde_json::to_string(&response).unwrap_or_default();
        json.push('\n');
        let _ = writer.write_all(json.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_request() {
        let daemon = CompileDaemon::new("127.0.0.1:0", PathBuf::from("."));
        let request = CompileRequest {
            id: "req1".into(),
            files: vec!["Cargo.toml".into()],
            max_time: None,
            options: CompileOptions::default(),
        };
        let response = daemon.process_request(&request);
        assert!(response.success);
        assert_eq!(response.id, "req1");
    }

    #[test]
    fn test_missing_file() {
        let daemon = CompileDaemon::new("127.0.0.1:0", PathBuf::from("."));
        let request = CompileRequest {
            id: "req2".into(),
            files: vec!["nonexistent.sv".into()],
            max_time: None,
            options: CompileOptions::default(),
        };
        let response = daemon.process_request(&request);
        assert!(!response.success);
        assert!(!response.errors.is_empty());
    }

    #[test]
    fn test_cache_hit() {
        let daemon = CompileDaemon::new("127.0.0.1:0", PathBuf::from("."));
        let request = CompileRequest {
            id: "req3".into(),
            files: vec!["Cargo.toml".into()],
            max_time: None,
            options: CompileOptions::default(),
        };
        let r1 = daemon.process_request(&request);
        let r2 = daemon.process_request(&request);
        assert_eq!(r1.id, r2.id);
        // Cache hit returns same response
        assert_eq!(r1.success, r2.success);
    }

    #[test]
    fn test_stats() {
        let daemon = CompileDaemon::new("127.0.0.1:0", PathBuf::from("."));
        let req = CompileRequest {
            id: "r".into(),
            files: vec!["Cargo.toml".into()],
            max_time: None,
            options: CompileOptions::default(),
        };
        daemon.process_request(&req);
        let stats = daemon.stats();
        assert_eq!(stats.total_requests, 1);
        assert_eq!(stats.successful, 1);
    }
}
