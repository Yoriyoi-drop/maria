//! Dependency-aware DAG Scheduler.
//!
//! Task scheduling berdasarkan dependency graph.
//! Task hanya bisa dijalankan jika semua dependensi sudah selesai.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use super::work_stealing::Task;

// ─── Node ID ───

/// Unique identifier untuk node di dependency graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(pub usize);

// ─── Dependency Graph ───

/// Dependency graph untuk task scheduling.
pub struct DependencyGraph {
    /// Number of nodes
    node_count: AtomicUsize,
    /// Edges: node → set of dependencies
    edges: Mutex<HashMap<NodeId, HashSet<NodeId>>>,
    /// Reverse edges: node → set of dependents
    reverse_edges: Mutex<HashMap<NodeId, HashSet<NodeId>>>,
    /// Node → Task mapping
    tasks: Mutex<HashMap<NodeId, Task>>,
    /// Completed nodes
    completed: Mutex<HashSet<NodeId>>,
}

impl DependencyGraph {
    pub fn new() -> Self {
        DependencyGraph {
            node_count: AtomicUsize::new(0),
            edges: Mutex::new(HashMap::new()),
            reverse_edges: Mutex::new(HashMap::new()),
            tasks: Mutex::new(HashMap::new()),
            completed: Mutex::new(HashSet::new()),
        }
    }

    /// Add a node with its task.
    pub fn add_node(&self, task: Task) -> NodeId {
        let id = NodeId(self.node_count.fetch_add(1, Ordering::SeqCst));
        self.tasks.lock().unwrap().insert(id, task);
        self.edges.lock().unwrap().insert(id, HashSet::new());
        self.reverse_edges
            .lock()
            .unwrap()
            .insert(id, HashSet::new());
        id
    }

    /// Add dependency edge: `from` depends on `to`.
    pub fn add_edge(&self, from: NodeId, to: NodeId) {
        self.edges
            .lock()
            .unwrap()
            .entry(from)
            .or_default()
            .insert(to);
        self.reverse_edges
            .lock()
            .unwrap()
            .entry(to)
            .or_default()
            .insert(from);
    }

    /// Mark a node as completed and return newly-ready nodes.
    pub fn complete(&self, node: NodeId) -> Vec<NodeId> {
        self.completed.lock().unwrap().insert(node);

        let reverse = self.reverse_edges.lock().unwrap();
        let completed = self.completed.lock().unwrap();

        // Find nodes whose dependencies are all completed
        let mut ready = Vec::new();
        if let Some(dependents) = reverse.get(&node) {
            for &dep_id in dependents {
                if completed.contains(&dep_id) {
                    continue;
                }
                let deps = self.edges.lock().unwrap();
                if let Some(deps_of_dep) = deps.get(&dep_id) {
                    if deps_of_dep.iter().all(|d| completed.contains(d)) {
                        ready.push(dep_id);
                    }
                }
            }
        }

        ready
    }

    /// Get all nodes with no uncompleted dependencies (initial ready set).
    pub fn initial_ready(&self) -> Vec<NodeId> {
        let edges = self.edges.lock().unwrap();
        let completed = self.completed.lock().unwrap();

        edges
            .iter()
            .filter(|(_, deps)| deps.is_empty() || deps.iter().all(|d| completed.contains(d)))
            .map(|(&id, _)| id)
            .collect()
    }

    /// Get topological order of all nodes.
    pub fn topo_order(&self) -> Vec<NodeId> {
        let edges = self.edges.lock().unwrap();
        let mut visited = HashSet::new();
        let mut order = Vec::new();
        let mut stack: Vec<NodeId> = edges.keys().copied().collect();

        // Simple DFS-based topological sort
        while let Some(node) = stack.pop() {
            if visited.contains(&node) {
                continue;
            }
            let deps = edges.get(&node).cloned().unwrap_or_default();
            if deps.iter().all(|d| visited.contains(d)) {
                visited.insert(node);
                order.push(node);
            } else {
                stack.push(node);
                for dep in &deps {
                    if !visited.contains(dep) {
                        stack.push(*dep);
                    }
                }
            }
        }

        order
    }

    /// Get task for a node.
    pub fn get_task(&self, node: NodeId) -> Option<Task> {
        self.tasks.lock().unwrap().get(&node).cloned()
    }

    /// Number of nodes.
    pub fn node_count(&self) -> usize {
        self.node_count.load(Ordering::Relaxed)
    }

    /// Number of completed nodes.
    pub fn completed_count(&self) -> usize {
        self.completed.lock().unwrap().len()
    }
}

impl Default for DependencyGraph {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dep_graph_basic() {
        let graph = DependencyGraph::new();
        let a = graph.add_node(Task::ParseFile("a.sv".into()));
        let b = graph.add_node(Task::ParseFile("b.sv".into()));
        graph.add_edge(b, a); // b depends on a

        let ready = graph.initial_ready();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0], a);
    }

    #[test]
    fn test_dep_graph_topo_order() {
        let graph = DependencyGraph::new();
        let a = graph.add_node(Task::ParseFile("a.sv".into()));
        let b = graph.add_node(Task::ParseFile("b.sv".into()));
        let c = graph.add_node(Task::ParseFile("c.sv".into()));
        graph.add_edge(b, a);
        graph.add_edge(c, b);

        let order = graph.topo_order();
        let pos_a = order.iter().position(|&x| x == a).unwrap();
        let pos_b = order.iter().position(|&x| x == b).unwrap();
        let pos_c = order.iter().position(|&x| x == c).unwrap();
        assert!(pos_a < pos_b);
        assert!(pos_b < pos_c);
    }
}
