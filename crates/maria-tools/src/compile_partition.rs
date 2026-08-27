//! COMP-17: Design partitioning for parallel compilation.
//!
//! Membagi desain menjadi independen partitions yang bisa di-compile
//! secara parallel. Menggunakan dependency graph antar modules untuk
//! menentukan partition boundaries dan compile order.
//!
//! # Strategy
//! 1. Build module dependency graph (module A instance module B → A depends on B)
//! 2. Tarjan SCC → condensation DAG
//! 3. Layer-based partitioning (same layer = parallel-safe)
//! 4. Cross-partition dependencies → serial compile order

use std::collections::{HashMap, HashSet, VecDeque};

/// Module dalam dependency graph.
#[derive(Debug, Clone)]
pub struct CompileModule {
    pub name: String,
    pub file_path: Option<String>,
    pub dependencies: Vec<String>, // module names this module depends on
    pub estimated_size: usize,     // rough LOC estimate
}

/// Partition dari modules yang bisa di-compile parallel.
#[derive(Debug, Clone)]
pub struct CompilePartition {
    pub id: usize,
    pub modules: Vec<String>,
    pub depends_on: Vec<usize>, // partition IDs this partition depends on
    pub estimated_size: usize,
}

/// Hasil partitioning.
#[derive(Debug, Clone)]
pub struct PartitionResult {
    pub partitions: Vec<CompilePartition>,
    pub compile_order: Vec<usize>, // topological order of partition IDs
    pub cross_partition_deps: Vec<(String, String)>, // (from_module, to_module)
}

/// Design partitioner untuk parallel compilation.
pub struct CompilePartitioner;

impl CompilePartitioner {
    /// Partition modules menjadi parallel-safe groups.
    pub fn partition(modules: &[CompileModule], max_partitions: usize) -> PartitionResult {
        if modules.is_empty() {
            return PartitionResult {
                partitions: Vec::new(),
                compile_order: Vec::new(),
                cross_partition_deps: Vec::new(),
            };
        }

        // 1. Build dependency graph
        let name_to_idx: HashMap<&str, usize> = modules
            .iter()
            .enumerate()
            .map(|(i, m)| (m.name.as_str(), i))
            .collect();

        // 2. Tarjan SCC → find strongly connected components
        let sccs = tarjan_scc(modules, &name_to_idx);

        // 3. Build condensation DAG (SCC → single node)
        let scc_map: HashMap<usize, usize> = sccs
            .iter()
            .enumerate()
            .flat_map(|(scc_id, scc)| scc.iter().map(move |&idx| (idx, scc_id)))
            .collect();

        let scc_deps = build_scc_deps(modules, &name_to_idx, &scc_map, &sccs);

        // 4. Layer-based assignment (BFS levels)
        let layers = compute_layers(&scc_deps, sccs.len());

        // 5. Assign partitions (combine small layers)
        let partitions = assign_partitions(&layers, &sccs, modules, max_partitions);

        // 6. Compute cross-partition deps
        let cross_deps = compute_cross_partition_deps(modules, &name_to_idx, &partitions, &scc_map);

        // 7. Topological order
        let compile_order = topological_sort_partitions(&partitions);

        PartitionResult {
            partitions,
            compile_order,
            cross_partition_deps: cross_deps,
        }
    }

    /// Quick stats about the partitioning.
    pub fn stats(result: &PartitionResult) -> String {
        let total_modules: usize = result.partitions.iter().map(|p| p.modules.len()).sum();
        let max_size = result
            .partitions
            .iter()
            .map(|p| p.estimated_size)
            .max()
            .unwrap_or(0);
        let min_size = result
            .partitions
            .iter()
            .map(|p| p.estimated_size)
            .min()
            .unwrap_or(0);

        format!(
            "{} partitions, {} modules, size range {}-{} LOC, {} cross-deps, compile depth {}",
            result.partitions.len(),
            total_modules,
            min_size,
            max_size,
            result.cross_partition_deps.len(),
            result.compile_order.len(),
        )
    }
}

/// Tarjan's algorithm for SCC.
fn tarjan_scc(modules: &[CompileModule], name_to_idx: &HashMap<&str, usize>) -> Vec<Vec<usize>> {
    let n = modules.len();
    let mut index = vec![0i32; n];
    let mut lowlink = vec![0i32; n];
    let mut on_stack = vec![false; n];
    let mut stack = Vec::new();
    let mut indices = Vec::new();
    let mut idx_counter = 0;
    let mut sccs = Vec::new();

    fn strongconnect(
        v: usize,
        modules: &[CompileModule],
        name_to_idx: &HashMap<&str, usize>,
        index: &mut Vec<i32>,
        lowlink: &mut Vec<i32>,
        on_stack: &mut Vec<bool>,
        stack: &mut Vec<usize>,
        indices: &mut Vec<usize>,
        idx_counter: &mut i32,
        sccs: &mut Vec<Vec<usize>>,
    ) {
        index[v] = *idx_counter;
        lowlink[v] = *idx_counter;
        *idx_counter += 1;
        stack.push(v);
        on_stack[v] = true;
        indices.push(v);

        for dep_name in &modules[v].dependencies {
            if let Some(&w) = name_to_idx.get(dep_name.as_str()) {
                if index[w] == 0 {
                    strongconnect(
                        w,
                        modules,
                        name_to_idx,
                        index,
                        lowlink,
                        on_stack,
                        stack,
                        indices,
                        idx_counter,
                        sccs,
                    );
                    lowlink[v] = lowlink[v].min(lowlink[w]);
                } else if on_stack[w] {
                    lowlink[v] = lowlink[v].min(index[w]);
                }
            }
        }

        if lowlink[v] == index[v] {
            let mut component = Vec::new();
            loop {
                let w = stack.pop().unwrap();
                on_stack[w] = false;
                component.push(w);
                if w == v {
                    break;
                }
            }
            sccs.push(component);
        }
    }

    for v in 0..n {
        if index[v] == 0 {
            strongconnect(
                v,
                modules,
                name_to_idx,
                &mut index,
                &mut lowlink,
                &mut on_stack,
                &mut stack,
                &mut indices,
                &mut idx_counter,
                &mut sccs,
            );
        }
    }

    sccs
}

/// Build dependencies between SCCs.
fn build_scc_deps(
    modules: &[CompileModule],
    name_to_idx: &HashMap<&str, usize>,
    scc_map: &HashMap<usize, usize>,
    sccs: &[Vec<usize>],
) -> Vec<HashSet<usize>> {
    let num_sccs = sccs.len();
    let mut deps: Vec<HashSet<usize>> = vec![HashSet::new(); num_sccs];

    for scc_id in 0..num_sccs {
        for &module_idx in &sccs[scc_id] {
            for dep_name in &modules[module_idx].dependencies {
                if let Some(&dep_idx) = name_to_idx.get(dep_name.as_str()) {
                    let dep_scc = scc_map[&dep_idx];
                    if dep_scc != scc_id {
                        deps[scc_id].insert(dep_scc);
                    }
                }
            }
        }
    }

    deps
}

/// BFS layer computation on DAG.
fn compute_layers(deps: &[HashSet<usize>], n: usize) -> Vec<Vec<usize>> {
    let mut in_degree = vec![0usize; n];
    for scc_id in 0..n {
        for &dep in &deps[scc_id] {
            // scc_id depends on dep → dep is a prerequisite
            // In compile order, dep comes BEFORE scc_id
            // Layer: dep should have LOWER layer number
        }
    }

    // Actually we need reverse: which SCCs depend on which
    // deps[i] = {j | i depends on j} → j must compile before i
    // So j → i in DAG
    let mut reverse_in_degree = vec![0usize; n];
    for i in 0..n {
        for &dep in &deps[i] {
            reverse_in_degree[i] += 1;
        }
    }

    let mut layers = Vec::new();
    let mut queue: VecDeque<usize> = (0..n)
        .filter(|&i| reverse_in_degree[i] == 0)
        .collect();

    while !queue.is_empty() {
        let mut layer = Vec::new();
        let current_size = queue.len();
        for _ in 0..current_size {
            if let Some(scc_id) = queue.pop_front() {
                layer.push(scc_id);
            }
        }

        for &scc_id in &layer {
            // Find all SCCs that depend on this one
            for i in 0..n {
                if deps[i].contains(&scc_id) {
                    reverse_in_degree[i] -= 1;
                    if reverse_in_degree[i] == 0 {
                        queue.push_back(i);
                    }
                }
            }
        }

        if !layer.is_empty() {
            layers.push(layer);
        }
    }

    layers
}

/// Assign layers to partitions (combine small layers).
fn assign_partitions(
    layers: &[Vec<usize>],
    sccs: &[Vec<usize>],
    modules: &[CompileModule],
    max_partitions: usize,
) -> Vec<CompilePartition> {
    let mut partitions = Vec::new();
    let mut partition_modules: Vec<Vec<String>> = Vec::new();
    let mut partition_depends: Vec<Vec<usize>> = Vec::new();

    // Build SCC → module name mapping
    let mut scc_to_names: Vec<Vec<String>> = Vec::new();
    for scc in sccs {
        scc_to_names.push(scc.iter().map(|&idx| modules[idx].name.clone()).collect());
    }

    for layer in layers {
        for &scc_id in layer {
            let part_id = partitions.len();
            let module_names = scc_to_names[scc_id].clone();
            let size: usize = module_names
                .iter()
                .filter_map(|name| modules.iter().find(|m| m.name == *name))
                .map(|m| m.estimated_size)
                .sum();

            // Dependencies: this partition depends on partitions containing
            // the SCCs that this SCC depends on
            partitions.push(CompilePartition {
                id: part_id,
                modules: module_names,
                depends_on: Vec::new(), // filled below
                estimated_size: size,
            });
        }
    }

    // Now fill depends_on
    // We need to know which partition contains which SCC
    let mut scc_to_partition = HashMap::new();
    for (part_idx, part) in partitions.iter().enumerate() {
        // Find which SCC this partition represents by matching module names
        for (scc_idx, scc) in sccs.iter().enumerate() {
            let scc_names: Vec<String> = scc.iter().map(|&idx| modules[idx].name.clone()).collect();
            if scc_names == part.modules {
                scc_to_partition.insert(scc_idx, part_idx);
                break;
            }
        }
    }

    // Rebuild layer assignment
    let mut scc_layer: Vec<usize> = vec![0; sccs.len()];
    for (layer_idx, layer) in layers.iter().enumerate() {
        for &scc_id in layer {
            scc_layer[scc_id] = layer_idx;
        }
    }

    // For each partition, find dependencies from SCC deps
    // We need original SCC deps — recompute from layers
    // A partition in layer L depends on partitions in layers < L
    // that have modules it references
    // Build name→partition mapping, then compute depends_on separately
    {
        let name_to_part: HashMap<String, usize> = partitions
            .iter()
            .enumerate()
            .flat_map(|(i, p)| p.modules.iter().map(move |m| (m.clone(), i)))
            .collect();

        for i in 0..partitions.len() {
            let mut deps = HashSet::new();
            for mod_name in &partitions[i].modules {
                if let Some(m) = modules.iter().find(|m| m.name == *mod_name) {
                    for dep_name in &m.dependencies {
                        if let Some(&dep_part) = name_to_part.get(dep_name.as_str()) {
                            if dep_part != i {
                                deps.insert(dep_part);
                            }
                        }
                    }
                }
            }
            partitions[i].depends_on = deps.into_iter().collect();
        }
    }

    partitions
}

/// Compute cross-partition dependencies.
fn compute_cross_partition_deps(
    modules: &[CompileModule],
    name_to_idx: &HashMap<&str, usize>,
    partitions: &[CompilePartition],
    _scc_map: &HashMap<usize, usize>,
) -> Vec<(String, String)> {
    let name_to_part: HashMap<&str, usize> = partitions
        .iter()
        .enumerate()
        .flat_map(|(i, p)| p.modules.iter().map(move |m| (m.as_str(), i)))
        .collect();

    let mut deps = Vec::new();
    for part in partitions {
        for mod_name in &part.modules {
            if let Some(m) = modules.iter().find(|m| m.name == *mod_name) {
                for dep_name in &m.dependencies {
                    if let Some(&dep_part) = name_to_part.get(dep_name.as_str()) {
                        if dep_part != part.id {
                            deps.push((mod_name.clone(), dep_name.clone()));
                        }
                    }
                }
            }
        }
    }
    deps.sort();
    deps.dedup();
    deps
}

/// Topological sort partitions.
fn topological_sort_partitions(partitions: &[CompilePartition]) -> Vec<usize> {
    let n = partitions.len();
    let mut in_degree = vec![0usize; n];

    for part in partitions {
        for &dep in &part.depends_on {
            in_degree[part.id] += 1; // part depends on dep → part has incoming edge
        }
    }

    let mut queue: VecDeque<usize> = (0..n)
        .filter(|&i| in_degree[i] == 0)
        .collect();

    let mut order = Vec::new();
    while let Some(id) = queue.pop_front() {
        order.push(id);
        // Find all partitions that depend on `id`
        for part in partitions {
            if part.depends_on.contains(&id) {
                in_degree[part.id] -= 1;
                if in_degree[part.id] == 0 {
                    queue.push_back(part.id);
                }
            }
        }
    }

    order
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module(name: &str, deps: &[&str], size: usize) -> CompileModule {
        CompileModule {
            name: name.into(),
            file_path: None,
            dependencies: deps.iter().map(|s| s.to_string()).collect(),
            estimated_size: size,
        }
    }

    #[test]
    fn test_empty_design() {
        let result = CompilePartitioner::partition(&[], 4);
        assert!(result.partitions.is_empty());
    }

    #[test]
    fn test_single_module() {
        let modules = vec![module("top", &[], 100)];
        let result = CompilePartitioner::partition(&modules, 4);
        assert_eq!(result.partitions.len(), 1);
        assert_eq!(result.partitions[0].modules, vec!["top"]);
        assert!(result.compile_order.contains(&0));
    }

    #[test]
    fn test_independent_modules() {
        let modules = vec![
            module("a", &[], 100),
            module("b", &[], 200),
            module("c", &[], 150),
        ];
        let result = CompilePartitioner::partition(&modules, 4);
        assert_eq!(result.partitions.len(), 3);
        // All should be in same layer (no deps)
        assert!(result.cross_partition_deps.is_empty());
    }

    #[test]
    fn test_linear_chain() {
        let modules = vec![
            module("c", &["b"], 100),
            module("b", &["a"], 100),
            module("a", &[], 100),
        ];
        let result = CompilePartitioner::partition(&modules, 4);
        // Should have 3 partitions in 3 layers
        assert_eq!(result.partitions.len(), 3);
        // Compile order: a → b → c
        let a_idx = result.partitions.iter().position(|p| p.modules == vec!["a"]).unwrap();
        let b_idx = result.partitions.iter().position(|p| p.modules == vec!["b"]).unwrap();
        let c_idx = result.partitions.iter().position(|p| p.modules == vec!["c"]).unwrap();
        let order_a = result.compile_order.iter().position(|&i| i == a_idx).unwrap();
        let order_b = result.compile_order.iter().position(|&i| i == b_idx).unwrap();
        let order_c = result.compile_order.iter().position(|&i| i == c_idx).unwrap();
        assert!(order_a < order_b);
        assert!(order_b < order_c);
    }

    #[test]
    fn test_diamond_dependency() {
        // a → b, a → c, b → d, c → d
        let modules = vec![
            module("d", &[], 100),
            module("b", &["d"], 100),
            module("c", &["d"], 100),
            module("a", &["b", "c"], 100),
        ];
        let result = CompilePartitioner::partition(&modules, 4);
        assert!(result.partitions.len() >= 3);
        // d must compile before b and c
        let d_part = result.partitions.iter().position(|p| p.modules == vec!["d"]).unwrap();
        let order_d = result.compile_order.iter().position(|&i| i == d_part).unwrap();
        for name in &["b", "c"] {
            let part = result.partitions.iter().position(|p| p.modules == vec![*name]).unwrap();
            let order = result.compile_order.iter().position(|&i| i == part).unwrap();
            assert!(order_d < order, "{} should compile before {}", name, "d");
        }
    }

    #[test]
    fn test_circular_dependency() {
        // a → b → a (SCC)
        let modules = vec![
            module("a", &["b"], 100),
            module("b", &["a"], 100),
        ];
        let result = CompilePartitioner::partition(&modules, 4);
        // Both in same SCC → same partition or both in compile_order
        assert!(result.partitions.len() >= 1);
        assert!(!result.compile_order.is_empty());
    }

    #[test]
    fn test_cross_partition_deps() {
        let modules = vec![
            module("base", &[], 100),
            module("top", &["base"], 200),
        ];
        let result = CompilePartitioner::partition(&modules, 4);
        assert_eq!(result.cross_partition_deps.len(), 1);
        assert_eq!(result.cross_partition_deps[0].0, "top");
        assert_eq!(result.cross_partition_deps[0].1, "base");
    }

    #[test]
    fn test_stats() {
        let modules = vec![
            module("a", &[], 100),
            module("b", &["a"], 200),
        ];
        let result = CompilePartitioner::partition(&modules, 4);
        let stats = CompilePartitioner::stats(&result);
        assert!(stats.contains("partitions"));
        assert!(stats.contains("modules"));
    }
}
