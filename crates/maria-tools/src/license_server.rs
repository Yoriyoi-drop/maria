//! FEAT-12: License Server — floating license management.
//!
//! Manages concurrent license checkout/checkin across multiple users.
//! Tracks license pool, heartbeat timeout, and lease expiration.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// License pool configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LicensePool {
    pub total: u32,
    pub available: u32,
    pub feature: String,
}

/// Active license lease.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LicenseLease {
    pub id: String,
    pub user: String,
    pub host: String,
    pub feature: String,
    pub granted_at: u64,
    pub expires_at: u64,
    pub heartbeat: u64,
}

/// License server.
pub struct LicenseServer {
    pools: HashMap<String, LicensePool>,
    leases: Vec<LicenseLease>,
    next_id: u32,
    heartbeat_timeout: u64,
}

impl LicenseServer {
    pub fn new(heartbeat_timeout_secs: u64) -> Self {
        LicenseServer {
            pools: HashMap::new(),
            leases: Vec::new(),
            next_id: 1,
            heartbeat_timeout: heartbeat_timeout_secs,
        }
    }

    /// Register a license pool.
    pub fn register_pool(&mut self, feature: &str, count: u32) {
        self.pools.insert(
            feature.to_string(),
            LicensePool {
                total: count,
                available: count,
                feature: feature.to_string(),
            },
        );
    }

    /// Checkout a license. Returns lease ID or error.
    pub fn checkout(&mut self, feature: &str, user: &str, host: &str) -> Result<String, String> {
        // Expire stale leases first
        self.expire_stale();

        let pool = self
            .pools
            .get_mut(feature)
            .ok_or_else(|| format!("unknown feature: {}", feature))?;

        if pool.available == 0 {
            return Err(format!("no licenses available for {}", feature));
        }

        pool.available -= 1;
        let now = now_secs();
        let id = format!("L-{:06}", self.next_id);
        self.next_id += 1;

        self.leases.push(LicenseLease {
            id: id.clone(),
            user: user.to_string(),
            host: host.to_string(),
            feature: feature.to_string(),
            granted_at: now,
            expires_at: now + 3600, // 1 hour default
            heartbeat: now,
        });

        Ok(id)
    }

    /// Checkin (release) a license.
    pub fn checkin(&mut self, lease_id: &str) -> Result<(), String> {
        let idx = self
            .leases
            .iter()
            .position(|l| l.id == lease_id)
            .ok_or_else(|| format!("lease {} not found", lease_id))?;
        let lease = self.leases.remove(idx);
        if let Some(pool) = self.pools.get_mut(&lease.feature) {
            pool.available += 1;
        }
        Ok(())
    }

    /// Send heartbeat to keep lease alive.
    pub fn heartbeat(&mut self, lease_id: &str) -> Result<(), String> {
        let lease = self
            .leases
            .iter_mut()
            .find(|l| l.id == lease_id)
            .ok_or_else(|| format!("lease {} not found", lease_id))?;
        let now = now_secs();
        lease.heartbeat = now;
        lease.expires_at = now + 3600;
        Ok(())
    }

    /// Expire stale leases (heartbeat timeout).
    fn expire_stale(&mut self) {
        let now = now_secs();
        let timeout = self.heartbeat_timeout;
        self.leases.retain(|l| {
            let stale = now.saturating_sub(l.heartbeat) > timeout;
            if stale {
                // Return license to pool
                // Can't return here since we're in retain, handle after
                false
            } else {
                true
            }
        });
        // Recompute available counts
        for pool in self.pools.values_mut() {
            pool.available = pool.total;
        }
        for l in &self.leases {
            if let Some(pool) = self.pools.get_mut(&l.feature) {
                pool.available = pool.available.saturating_sub(1);
            }
        }
    }

    /// List active leases.
    pub fn active_leases(&self) -> Vec<&LicenseLease> {
        self.leases.iter().collect()
    }

    /// Get pool status.
    pub fn pool_status(&self, feature: &str) -> Option<(u32, u32, u32)> {
        self.pools
            .get(feature)
            .map(|p| (p.total, p.available, p.total - p.available))
    }

    /// Summary.
    pub fn summary(&self) -> String {
        let pools: Vec<String> = self
            .pools
            .iter()
            .map(|(f, p)| format!("{}: {}/{} available", f, p.available, p.total))
            .collect();
        format!(
            "LicenseServer: {} pools, {} active leases [{}]",
            self.pools.len(),
            self.leases.len(),
            pools.join(", ")
        )
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_checkout_checkin() {
        let mut server = LicenseServer::new(300);
        server.register_pool("maria-sim", 2);
        let id1 = server.checkout("maria-sim", "alice", "host1").unwrap();
        let id2 = server.checkout("maria-sim", "bob", "host2").unwrap();
        assert!(server.checkout("maria-sim", "carol", "host3").is_err());
        server.checkin(&id1).unwrap();
        assert!(server.checkout("maria-sim", "carol", "host3").is_ok());
    }

    #[test]
    fn test_heartbeat() {
        let mut server = LicenseServer::new(300);
        server.register_pool("maria-sim", 1);
        let id = server.checkout("maria-sim", "alice", "host1").unwrap();
        assert!(server.heartbeat(&id).is_ok());
    }

    #[test]
    fn test_summary() {
        let mut server = LicenseServer::new(300);
        server.register_pool("maria-sim", 4);
        server.register_pool("maria-formal", 2);
        let s = server.summary();
        assert!(s.contains("2 pools"));
    }

    #[test]
    fn test_pool_status() {
        let mut server = LicenseServer::new(300);
        server.register_pool("maria-sim", 3);
        let id = server.checkout("maria-sim", "alice", "h").unwrap();
        let (total, avail, in_use) = server.pool_status("maria-sim").unwrap();
        assert_eq!(total, 3);
        assert_eq!(avail, 2);
        assert_eq!(in_use, 1);
    }
}
