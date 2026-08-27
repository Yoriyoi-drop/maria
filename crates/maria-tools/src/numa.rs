//! LINUX-03: NUMA-aware memory allocation.
//!
//! Detects NUMA topology and provides hints for memory allocation
//! to optimize performance on multi-socket systems.

use std::collections::HashMap;

/// NUMA node information.
#[derive(Debug, Clone)]
pub struct NumaNode {
    pub id: u32,
    pub cpus: Vec<u32>,
    pub memory_mb: u64,
    pub distance: HashMap<u32, u32>,
}

/// NUMA topology.
#[derive(Debug, Clone)]
pub struct NumaTopology {
    pub nodes: Vec<NumaNode>,
    pub num_sockets: usize,
    pub is_numa: bool,
}

impl NumaTopology {
    /// Detect NUMA topology (Linux only, fallback for non-Linux).
    pub fn detect() -> Self {
        #[cfg(target_os = "linux")]
        {
            detect_linux()
        }
        #[cfg(not(target_os = "linux"))]
        {
            NumaTopology {
                nodes: vec![NumaNode {
                    id: 0,
                    cpus: (0..std::thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(1)).collect(),
                    memory_mb: 0,
                    distance: HashMap::new(),
                }],
                num_sockets: 1,
                is_numa: false,
            }
        }
    }

    /// Get recommended NUMA node for a CPU.
    pub fn node_for_cpu(&self, cpu: u32) -> Option<u32> {
        self.nodes
            .iter()
            .find(|n| n.cpus.contains(&cpu))
            .map(|n| n.id)
    }

    /// Get closest NUMA node to a given node.
    pub fn closest_node(&self, from: u32) -> Option<u32> {
        let node = self.nodes.iter().find(|n| n.id == from)?;
        node.distance
            .iter()
            .filter(|(&id, _)| id != from)
            .min_by_key(|(_, &dist)| dist)
            .map(|(&id, _)| id)
    }

    /// Summary.
    pub fn summary(&self) -> String {
        format!(
            "NUMA topology: {} nodes, {} sockets, {}",
            self.nodes.len(),
            self.num_sockets,
            if self.is_numa { "NUMA" } else { "UMA" },
        )
    }
}

#[cfg(target_os = "linux")]
fn detect_linux() -> NumaTopology {
    let mut nodes = Vec::new();

    // Try reading sysfs
    if let Ok(entries) = std::fs::read_dir("/sys/devices/system/node/") {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if let Some(id) = name.strip_prefix("node") {
                if let Ok(node_id) = id.parse::<u32>() {
                    let cpus = read_node_cpus(node_id);
                    let memory_mb = read_node_memory(node_id);
                    let distance = read_node_distance(node_id);

                    nodes.push(NumaNode {
                        id: node_id,
                        cpus,
                        memory_mb,
                        distance,
                    });
                }
            }
        }
    }

    if nodes.is_empty() {
        nodes.push(NumaNode {
            id: 0,
            cpus: (0..std::thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(1)).collect(),
            memory_mb: 0,
            distance: HashMap::new(),
        });
    }

    let is_numa = nodes.len() > 1;
    let num_sockets = if is_numa { nodes.len() } else { 1 };

    NumaTopology {
        nodes,
        num_sockets,
        is_numa,
    }
}

#[cfg(target_os = "linux")]
fn read_node_cpus(node_id: u32) -> Vec<u32> {
    let path = format!("/sys/devices/system/node/node{}/cpulist", node_id);
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| parse_cpu_list(&s))
        .unwrap_or_default()
}

#[cfg(target_os = "linux")]
fn read_node_memory(node_id: u32) -> u64 {
    let path = format!("/sys/devices/system/node/node{}/meminfo", node_id);
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| {
            for line in s.lines() {
                if line.starts_with("MemTotal:") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 2 {
                        return parts[1].parse::<u64>().ok().map(|kb| kb / 1024);
                    }
                }
            }
            None
        })
        .unwrap_or(0)
}

#[cfg(target_os = "linux")]
fn read_node_distance(node_id: u32) -> HashMap<u32, u32> {
    let path = format!(
        "/sys/devices/system/node/node{}/distance",
        node_id
    );
    let mut distances = HashMap::new();
    if let Ok(content) = std::fs::read_to_string(&path) {
        let vals: Vec<u32> = content
            .split_whitespace()
            .filter_map(|s| s.parse().ok())
            .collect();
        for (i, &v) in vals.iter().enumerate() {
            distances.insert(i as u32, v);
        }
    }
    distances
}

fn parse_cpu_list(list: &str) -> Option<Vec<u32>> {
    let mut cpus = Vec::new();
    for part in list.trim().split(',') {
        if let Some((start, end)) = part.split_once('-') {
            let s: u32 = start.parse().ok()?;
            let e: u32 = end.parse().ok()?;
            cpus.extend(s..=e);
        } else {
            cpus.push(part.trim().parse().ok()?);
        }
    }
    Some(cpus)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_topology() {
        let topo = NumaTopology::detect();
        assert!(!topo.nodes.is_empty());
        assert!(topo.nodes[0].cpus.len() > 0);
    }

    #[test]
    fn test_node_for_cpu() {
        let topo = NumaTopology::detect();
        if let Some(cpu) = topo.nodes[0].cpus.first() {
            let node = topo.node_for_cpu(*cpu);
            assert!(node.is_some());
        }
    }

    #[test]
    fn test_summary() {
        let topo = NumaTopology::detect();
        let s = topo.summary();
        assert!(s.contains("nodes"));
        assert!(s.contains("sockets"));
    }

    #[test]
    fn test_parse_cpu_list() {
        let result = parse_cpu_list("0-3,8-11").unwrap();
        assert_eq!(result, vec![0, 1, 2, 3, 8, 9, 10, 11]);
    }
}
