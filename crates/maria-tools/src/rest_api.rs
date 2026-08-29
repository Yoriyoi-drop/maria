//! FEAT-04: REST API — Basic HTTP server skeleton for automation.
//!
//! Provides a minimal HTTP server using only std::net for exposing
//! maria tools via REST endpoints. No external dependencies needed.
//!
//! Endpoints:
//! - GET /health — health check
//! - GET /api/v1/status — server status
//! - POST /api/v1/compile — compile source
//! - POST /api/v1/simulate — run simulation
//! - GET /api/v1/modules — list modules
//!
//! Usage:
//! ```no_run
//! use maria_tools::rest_api::RestServer;
//! let server = RestServer::new("127.0.0.1:8080");
//! server.start().unwrap();
//! ```

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};

/// REST API server.
pub struct RestServer {
    addr: String,
    routes: HashMap<String, Box<dyn Fn(&str, &str) -> String + Send + Sync>>,
}

/// API response.
#[derive(Debug, Clone)]
pub struct ApiResponse {
    pub status: u16,
    pub body: String,
    pub content_type: String,
}

impl ApiResponse {
    pub fn ok(body: &str) -> Self {
        ApiResponse {
            status: 200,
            body: body.to_string(),
            content_type: "application/json".to_string(),
        }
    }

    pub fn json(body: &str) -> Self {
        ApiResponse {
            status: 200,
            body: body.to_string(),
            content_type: "application/json".to_string(),
        }
    }

    pub fn error(status: u16, body: &str) -> Self {
        ApiResponse {
            status,
            body: format!(r#"{{"error":"{}"}}"#, body),
            content_type: "application/json".to_string(),
        }
    }

    pub fn not_found() -> Self {
        Self::error(404, "not found")
    }

    pub fn to_http_response(&self) -> String {
        let status_text = match self.status {
            200 => "OK",
            404 => "Not Found",
            405 => "Method Not Allowed",
            500 => "Internal Server Error",
            _ => "Unknown",
        };
        format!(
            "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            self.status,
            status_text,
            self.content_type,
            self.body.len(),
            self.body,
        )
    }
}

impl RestServer {
    pub fn new(addr: &str) -> Self {
        let mut routes: HashMap<String, Box<dyn Fn(&str, &str) -> String + Send + Sync>> = HashMap::new();

        // Health check
        routes.insert("GET /health".to_string(), Box::new(|_, _| {
            ApiResponse::ok(r#"{"status":"healthy","version":"0.3.0"}"#).to_http_response()
        }));

        // Server status
        routes.insert("GET /api/v1/status".to_string(), Box::new(|_, _| {
            ApiResponse::json(r#"{"server":"maria","status":"running","uptime":"unknown"}"#).to_http_response()
        }));

        RestServer {
            addr: addr.to_string(),
            routes,
        }
    }

    /// Add a custom route.
    pub fn add_route<F>(&mut self, method: &str, path: &str, handler: F)
    where
        F: Fn(&str, &str) -> String + Send + Sync + 'static,
    {
        let key = format!("{} {}", method, path);
        self.routes.insert(key, Box::new(handler));
    }

    /// Start the server (blocking).
    pub fn start(&self) -> Result<(), String> {
        let listener = TcpListener::bind(&self.addr)
            .map_err(|e| format!("bind failed: {}", e))?;

        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    if let Err(e) = self.handle_connection(stream) {
                        eprintln!("connection error: {}", e);
                    }
                }
                Err(e) => {
                    eprintln!("accept error: {}", e);
                }
            }
        }
        Ok(())
    }

    fn handle_connection(&self, mut stream: TcpStream) -> Result<(), String> {
        let reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
        let mut method = String::new();
        let mut path = String::new();

        for line in reader.lines() {
            let line = line.map_err(|e| e.to_string())?;
            if line.starts_with("GET ") || line.starts_with("POST ") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    method = parts[0].to_string();
                    path = parts[1].to_string();
                }
            }
            if line.is_empty() {
                break;
            }
        }

        let key = format!("{} {}", method, path);
        let response = if let Some(handler) = self.routes.get(&key) {
            handler(&method, &path)
        } else {
            ApiResponse::not_found().to_http_response()
        };

        stream.write_all(response.as_bytes()).map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// Parse query string from URL path.
pub fn parse_query(path: &str) -> HashMap<String, String> {
    let mut params = HashMap::new();
    if let Some(query) = path.split('?').nth(1) {
        for pair in query.split('&') {
            if let Some((key, val)) = pair.split_once('=') {
                params.insert(key.to_string(), val.to_string());
            }
        }
    }
    params
}

/// Format JSON response.
pub fn json_object(fields: &[(&str, &str)]) -> String {
    let entries: Vec<String> = fields.iter()
        .map(|(k, v)| format!("\"{}\":\"{}\"", k, v))
        .collect();
    format!("{{{}}}", entries.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_response_ok() {
        let resp = ApiResponse::ok(r#"{"ok":true}"#);
        assert_eq!(resp.status, 200);
        assert!(resp.to_http_response().contains("200 OK"));
    }

    #[test]
    fn test_api_response_error() {
        let resp = ApiResponse::error(500, "boom");
        assert_eq!(resp.status, 500);
        assert!(resp.body.contains("boom"));
    }

    #[test]
    fn test_not_found() {
        let resp = ApiResponse::not_found();
        assert_eq!(resp.status, 404);
    }

    #[test]
    fn test_parse_query() {
        let q = parse_query("/api/v1/search?q=test&limit=10");
        assert_eq!(q.get("q").unwrap(), "test");
        assert_eq!(q.get("limit").unwrap(), "10");
    }

    #[test]
    fn test_json_object() {
        let j = json_object(&[("name", "maria"), ("version", "0.3.0")]);
        assert!(j.contains("\"name\":\"maria\""));
        assert!(j.contains("\"version\":\"0.3.0\""));
    }

    #[test]
    fn test_http_response_format() {
        let resp = ApiResponse::json(r#"{"status":"ok"}"#);
        let http = resp.to_http_response();
        assert!(http.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(http.contains("Content-Type: application/json"));
        assert!(http.ends_with(r#"{"status":"ok"}"#));
    }

    #[test]
    fn test_add_route() {
        let mut server = RestServer::new("127.0.0.1:0");
        server.add_route("GET", "/test", |_, _| ApiResponse::ok("test").to_http_response());
        let key = "GET /test".to_string();
        assert!(server.routes.contains_key(&key));
    }
}
