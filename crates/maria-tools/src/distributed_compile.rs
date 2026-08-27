//! COMP-18: Distributed Compilation via Network.
//!
//! Framework untuk mendistribusikan kompilasi ke multiple mesin.
//! Menggunakan JSON-RPC protocol over TCP.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

/// Worker node info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerNode {
    pub id: String,
    pub hostname: String,
    pub port: u16,
    pub max_concurrent: u32,
    pub active_jobs: u32,
    pub capabilities: Vec<String>,
    pub last_heartbeat: u64,
}

/// Distributed compilation request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistCompileRequest {
    pub job_id: String,
    pub files: Vec<String>,
    pub worker_hint: Option<String>,
    pub priority: u8,
    pub timeout_secs: u64,
}

/// Distributed compilation response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistCompileResponse {
    pub job_id: String,
    pub worker_id: String,
    pub success: bool,
    pub output_files: Vec<String>,
    pub compile_time_ms: u64,
    pub error: Option<String>,
}

/// Compile job status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum JobStatus {
    Pending,
    Dispatched { worker_id: String },
    Running,
    Completed,
    Failed(String),
}

/// Compile job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompileJob {
    pub id: String,
    pub request: DistCompileRequest,
    pub status: JobStatus,
    pub result: Option<DistCompileResponse>,
}

/// Distributed compilation manager.
pub struct DistributedCompiler {
    workers: Arc<Mutex<HashMap<String, WorkerNode>>>,
    jobs: Arc<Mutex<HashMap<String, CompileJob>>>,
    completed: Arc<Mutex<Vec<String>>>,
}

impl DistributedCompiler {
    pub fn new() -> Self {
        DistributedCompiler {
            workers: Arc::new(Mutex::new(HashMap::new())),
            jobs: Arc::new(Mutex::new(HashMap::new())),
            completed: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Register a worker node.
    pub fn register_worker(&self, worker: WorkerNode) {
        self.workers
            .lock()
            .unwrap()
            .insert(worker.id.clone(), worker);
    }

    /// Remove a worker.
    pub fn remove_worker(&self, id: &str) -> bool {
        self.workers.lock().unwrap().remove(id).is_some()
    }

    /// Submit a compile job.
    pub fn submit_job(&self, request: DistCompileRequest) -> String {
        let job_id = request.job_id.clone();
        let job = CompileJob {
            id: job_id.clone(),
            request,
            status: JobStatus::Pending,
            result: None,
        };
        self.jobs.lock().unwrap().insert(job_id.clone(), job);
        job_id
    }

    /// Dispatch next pending job to best worker.
    pub fn dispatch_next(&self) -> Option<(String, DistCompileRequest)> {
        let mut jobs = self.jobs.lock().unwrap();
        let pending = jobs
            .values_mut()
            .find(|j| matches!(j.status, JobStatus::Pending))?;

        let workers = self.workers.lock().unwrap();

        // Find best worker (least load, has capacity)
        let best = workers
            .values()
            .filter(|w| w.active_jobs < w.max_concurrent)
            .min_by_key(|w| w.active_jobs)?;

        let worker_id = best.id.clone();
        let request = pending.request.clone();
        pending.status = JobStatus::Dispatched {
            worker_id: worker_id.clone(),
        };

        Some((worker_id, request))
    }

    /// Mark a job as running.
    pub fn mark_running(&self, job_id: &str) {
        if let Some(job) = self.jobs.lock().unwrap().get_mut(job_id) {
            job.status = JobStatus::Running;
        }
    }

    /// Complete a job.
    pub fn complete_job(&self, job_id: &str, response: DistCompileResponse) {
        let mut jobs = self.jobs.lock().unwrap();
        if let Some(job) = jobs.get_mut(job_id) {
            job.status = if response.success {
                JobStatus::Completed
            } else {
                JobStatus::Failed(response.error.clone().unwrap_or_default())
            };
            job.result = Some(response);
            self.completed.lock().unwrap().push(job_id.to_string());
        }
    }

    /// Get job status.
    pub fn get_job(&self, job_id: &str) -> Option<CompileJob> {
        self.jobs.lock().unwrap().get(job_id).cloned()
    }

    /// List all jobs.
    pub fn list_jobs(&self) -> Vec<CompileJob> {
        self.jobs.lock().unwrap().values().cloned().collect()
    }

    /// List active workers.
    pub fn list_workers(&self) -> Vec<WorkerNode> {
        self.workers.lock().unwrap().values().cloned().collect()
    }

    /// Worker count.
    pub fn worker_count(&self) -> usize {
        self.workers.lock().unwrap().len()
    }

    /// Summary.
    pub fn summary(&self) -> String {
        let workers = self.workers.lock().unwrap();
        let jobs = self.jobs.lock().unwrap();
        let pending = jobs.values().filter(|j| matches!(j.status, JobStatus::Pending)).count();
        let running = jobs.values().filter(|j| matches!(j.status, JobStatus::Running | JobStatus::Dispatched { .. })).count();
        let completed = jobs.values().filter(|j| matches!(j.status, JobStatus::Completed)).count();
        format!(
            "DistributedCompiler: {} workers, {} jobs (pending={}, running={}, completed={})",
            workers.len(),
            jobs.len(),
            pending,
            running,
            completed,
        )
    }
}

impl Default for DistributedCompiler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_worker(id: &str) -> WorkerNode {
        WorkerNode {
            id: id.to_string(),
            hostname: format!("worker-{}.local", id),
            port: 9000,
            max_concurrent: 4,
            active_jobs: 0,
            capabilities: vec!["sv".into()],
            last_heartbeat: 1000,
        }
    }

    #[test]
    fn test_register_workers() {
        let dc = DistributedCompiler::new();
        dc.register_worker(make_worker("w1"));
        dc.register_worker(make_worker("w2"));
        assert_eq!(dc.worker_count(), 2);
    }

    #[test]
    fn test_submit_and_dispatch() {
        let dc = DistributedCompiler::new();
        dc.register_worker(make_worker("w1"));

        let req = DistCompileRequest {
            job_id: "j1".into(),
            files: vec!["counter.sv".into()],
            worker_hint: None,
            priority: 0,
            timeout_secs: 60,
        };

        let job_id = dc.submit_job(req.clone());
        assert_eq!(job_id, "j1");

        let dispatch = dc.dispatch_next();
        assert!(dispatch.is_some());
        let (worker_id, _) = dispatch.unwrap();
        assert_eq!(worker_id, "w1");
    }

    #[test]
    fn test_complete_job() {
        let dc = DistributedCompiler::new();
        dc.register_worker(make_worker("w1"));

        let req = DistCompileRequest {
            job_id: "j1".into(),
            files: vec!["counter.sv".into()],
            worker_hint: None,
            priority: 0,
            timeout_secs: 60,
        };

        dc.submit_job(req);
        dc.dispatch_next();
        dc.mark_running("j1");

        dc.complete_job(
            "j1",
            DistCompileResponse {
                job_id: "j1".into(),
                worker_id: "w1".into(),
                success: true,
                output_files: vec!["counter_ir.json".into()],
                compile_time_ms: 150,
                error: None,
            },
        );

        let job = dc.get_job("j1").unwrap();
        assert!(matches!(job.status, JobStatus::Completed));
    }

    #[test]
    fn test_summary() {
        let dc = DistributedCompiler::new();
        dc.register_worker(make_worker("w1"));
        let s = dc.summary();
        assert!(s.contains("1 workers"));
    }
}
