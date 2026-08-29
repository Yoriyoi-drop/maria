//! ENT-18: Parallel Multi-User Simulation Server.
//!
//! Manages multiple concurrent simulation sessions for different users.
//! Each session runs independently with resource quotas.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Simulation session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimSession {
    pub id: String,
    pub user: String,
    pub status: SessionStatus,
    pub source_files: Vec<String>,
    pub max_time: u64,
    pub elapsed_cycles: u64,
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SessionStatus {
    Queued,
    Running,
    Completed,
    Failed(String),
    Cancelled,
}

/// User quota.
#[derive(Debug, Clone)]
pub struct UserQuota {
    pub max_concurrent_sessions: usize,
    pub max_memory_mb: u64,
    pub max_time_secs: u64,
    pub current_sessions: usize,
}

impl Default for UserQuota {
    fn default() -> Self {
        UserQuota {
            max_concurrent_sessions: 4,
            max_memory_mb: 2048,
            max_time_secs: 300,
            current_sessions: 0,
        }
    }
}

/// Server configuration.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub max_total_sessions: usize,
    pub max_workers: usize,
    pub session_timeout_secs: u64,
    pub user_quotas: HashMap<String, UserQuota>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            max_total_sessions: 32,
            max_workers: 8,
            session_timeout_secs: 600,
            user_quotas: HashMap::new(),
        }
    }
}

/// Simulation server — manages concurrent sessions.
pub struct SimServer {
    config: ServerConfig,
    sessions: Arc<Mutex<HashMap<String, SimSession>>>,
    queue: Arc<Mutex<Vec<String>>>,
    start_time: Instant,
}

impl SimServer {
    pub fn new(config: ServerConfig) -> Self {
        SimServer {
            config,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            queue: Arc::new(Mutex::new(Vec::new())),
            start_time: Instant::now(),
        }
    }

    /// Submit a new simulation session.
    pub fn submit(&self, user: &str, files: Vec<String>, max_time: u64) -> Result<String, String> {
        let sessions = self.sessions.lock().unwrap();
        let user_sessions: usize = sessions
            .values()
            .filter(|s| s.user == user && matches!(s.status, SessionStatus::Running | SessionStatus::Queued))
            .count();

        // Check user quota
        let quota = self.config.user_quotas.get(user);
        let max_concurrent = quota.map(|q| q.max_concurrent_sessions).unwrap_or(4);
        if user_sessions >= max_concurrent {
            return Err(format!("user {} has {} sessions (max {})", user, user_sessions, max_concurrent));
        }

        // Check total capacity
        let total_active: usize = sessions
            .values()
            .filter(|s| matches!(s.status, SessionStatus::Running | SessionStatus::Queued))
            .count();
        if total_active >= self.config.max_total_sessions {
            drop(sessions);
            let id = format!("sim_{}_{}", user, self.next_id());
            let session = SimSession {
                id: id.clone(),
                user: user.to_string(),
                status: SessionStatus::Queued,
                source_files: files,
                max_time,
                elapsed_cycles: 0,
                created_at: now_secs(),
            };
            self.sessions.lock().unwrap().insert(id.clone(), session);
            self.queue.lock().unwrap().push(id.clone());
            return Ok(id);
        }

        // Compute ID before re-locking
        drop(sessions);
        let id = format!("sim_{}_{}", user, self.next_id());
        let session = SimSession {
            id: id.clone(),
            user: user.to_string(),
            status: SessionStatus::Running,
            source_files: files,
            max_time,
            elapsed_cycles: 0,
            created_at: now_secs(),
        };
        self.sessions.lock().unwrap().insert(id.clone(), session);
        Ok(id)
    }

    /// Get session status.
    pub fn get_session(&self, id: &str) -> Option<SimSession> {
        self.sessions.lock().unwrap().get(id).cloned()
    }

    /// Cancel a session.
    pub fn cancel(&self, id: &str) -> Result<(), String> {
        let mut sessions = self.sessions.lock().unwrap();
        let session = sessions.get_mut(id).ok_or("session not found")?;
        match session.status {
            SessionStatus::Running | SessionStatus::Queued => {
                session.status = SessionStatus::Cancelled;
                Ok(())
            }
            _ => Err("session not in cancellable state".into()),
        }
    }

    /// List sessions for a user.
    pub fn list_user_sessions(&self, user: &str) -> Vec<SimSession> {
        self.sessions
            .lock()
            .unwrap()
            .values()
            .filter(|s| s.user == user)
            .cloned()
            .collect()
    }

    /// List all active sessions.
    pub fn list_active(&self) -> Vec<SimSession> {
        self.sessions
            .lock()
            .unwrap()
            .values()
            .filter(|s| matches!(s.status, SessionStatus::Running | SessionStatus::Queued))
            .cloned()
            .collect()
    }

    /// Queue depth.
    pub fn queue_depth(&self) -> usize {
        self.queue.lock().unwrap().len()
    }

    /// Summary.
    pub fn summary(&self) -> String {
        let sessions = self.sessions.lock().unwrap();
        let running = sessions.values().filter(|s| matches!(s.status, SessionStatus::Running)).count();
        let queued = sessions.values().filter(|s| matches!(s.status, SessionStatus::Queued)).count();
        let completed = sessions.values().filter(|s| matches!(s.status, SessionStatus::Completed)).count();
        let uptime = self.start_time.elapsed().as_secs();
        format!(
            "SimServer: {} sessions ({} running, {} queued, {} completed), uptime {}s",
            sessions.len(), running, queued, completed, uptime,
        )
    }

    fn next_id(&self) -> u64 {
        self.sessions.lock().unwrap().len() as u64 + 1
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_submit_session() {
        let server = SimServer::new(ServerConfig::default());
        let id = server.submit("alice", vec!["counter.sv".into()], 1000).unwrap();
        assert!(id.starts_with("sim_alice_"));
        assert_eq!(server.list_active().len(), 1);
    }

    #[test]
    fn test_user_quota() {
        let server = SimServer::new(ServerConfig::default());
        for _ in 0..4 {
            server.submit("alice", vec![], 100).unwrap();
        }
        let result = server.submit("alice", vec![], 100);
        assert!(result.is_err());
    }

    #[test]
    fn test_cancel() {
        let server = SimServer::new(ServerConfig::default());
        let id = server.submit("alice", vec![], 100).unwrap();
        server.cancel(&id).unwrap();
        let session = server.get_session(&id).unwrap();
        assert!(matches!(session.status, SessionStatus::Cancelled));
    }

    #[test]
    fn test_list_user_sessions() {
        let server = SimServer::new(ServerConfig::default());
        server.submit("alice", vec![], 100).unwrap();
        server.submit("alice", vec![], 100).unwrap();
        server.submit("bob", vec![], 100).unwrap();
        assert_eq!(server.list_user_sessions("alice").len(), 2);
        assert_eq!(server.list_user_sessions("bob").len(), 1);
    }

    #[test]
    fn test_summary() {
        let server = SimServer::new(ServerConfig::default());
        let s = server.summary();
        assert!(s.contains("SimServer"));
    }
}
