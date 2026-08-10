//! Global Module Index — DashMap-based registry untuk module lookup O(1).

use maria_core::intern::Symbol;
use dashmap::DashMap;
use std::path::PathBuf;

/// Metadata sebuah module dalam index.
#[derive(Debug, Clone)]
pub struct ModuleMeta {
    pub name: Symbol,
    pub file: PathBuf,
    pub file_checksum: u64,
    pub ports: Vec<Symbol>,
    pub params: Vec<ParamMeta>,
    pub instances: Vec<Symbol>,
    pub imports: Vec<(Symbol, Symbol)>,
}

/// Metadata parameter.
#[derive(Debug, Clone)]
pub struct ParamMeta {
    pub name: Symbol,
    pub has_default: bool,
    pub is_type: bool,
    pub is_local: bool,
}

/// Jenis entry dalam index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntryKind {
    Module,
    Package,
    Interface,
    Program,
    Class,
    Primitive,
    Config,
}

/// Global Module Index — DashMap-based concurrent registry.
pub struct ModuleIndex {
    modules: DashMap<Symbol, Vec<(EntryKind, ModuleMeta)>>,
    file_map: DashMap<PathBuf, Vec<Symbol>>,
    count: std::sync::atomic::AtomicUsize,
}

impl ModuleIndex {
    pub fn new() -> Self {
        ModuleIndex {
            modules: DashMap::new(),
            file_map: DashMap::new(),
            count: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    pub fn insert(&self, name: Symbol, kind: EntryKind, meta: ModuleMeta) {
        let file = meta.file.clone();
        self.modules
            .entry(name)
            .or_insert_with(Vec::new)
            .push((kind, meta));
        self.file_map
            .entry(file)
            .or_insert_with(Vec::new)
            .push(name);
        self.count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn lookup(&self, name: Symbol, kind: EntryKind) -> Option<ModuleMeta> {
        self.modules
            .get(&name)?
            .iter()
            .find(|(k, _)| *k == kind)
            .map(|(_, m)| m.clone())
    }

    pub fn contains(&self, name: Symbol, kind: EntryKind) -> bool {
        self.modules
            .get(&name)
            .map(|entries| entries.iter().any(|(k, _)| *k == kind))
            .unwrap_or(false)
    }

    pub fn get_all(&self, name: Symbol) -> Option<Vec<(EntryKind, ModuleMeta)>> {
        self.modules.get(&name).map(|r| r.clone())
    }

    pub fn remove_file(&self, file: &PathBuf) {
        if let Some((_, entries)) = self.file_map.remove(file) {
            for name in entries {
                self.modules.remove(&name);
                self.count
                    .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }

    pub fn len(&self) -> usize {
        self.count.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn clear(&self) {
        self.modules.clear();
        self.file_map.clear();
        self.count.store(0, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn iter(&self) -> impl Iterator<Item = (Symbol, EntryKind, ModuleMeta)> + '_ {
        self.modules.iter().flat_map(|entry| {
            let name = *entry.key();
            let items: Vec<(Symbol, EntryKind, ModuleMeta)> = entry
                .value()
                .iter()
                .map(|(kind, meta)| (name, *kind, meta.clone()))
                .collect();
            items
        })
    }

    /// Get all module symbols that belong to a specific file.
    pub fn file_modules(&self, file: &PathBuf) -> Vec<Symbol> {
        self.file_map
            .get(file)
            .map(|r| r.clone())
            .unwrap_or_default()
    }

    /// Count modules of a specific kind.
    pub fn count_by_kind(&self, kind: EntryKind) -> usize {
        self.modules
            .iter()
            .filter(|entry| entry.value().iter().any(|(k, _)| *k == kind))
            .count()
    }

    /// Get all module names as Symbol vec.
    pub fn module_names(&self) -> Vec<Symbol> {
        self.modules.iter().map(|entry| *entry.key()).collect()
    }

    /// Get all module names as String vec (for legacy interop).
    pub fn module_names_string(&self) -> Vec<String> {
        self.modules
            .iter()
            .map(|entry| entry.key().as_str().to_string())
            .collect()
    }

    /// Detect cycles in module instantiation graph via DFS coloring.
    /// Returns a cycle path if found, None if graph is acyclic.
    pub fn detect_cycles(&self) -> Option<Vec<String>> {
        use std::collections::HashMap;

        #[derive(Clone, Copy, PartialEq)]
        enum Color { White, Gray, Black }

        let all: Vec<_> = self.module_names();
        let mut adj: HashMap<Symbol, Vec<Symbol>> = HashMap::new();
        let mut name_map: HashMap<Symbol, String> = HashMap::new();

        for name in &all {
            name_map.insert(*name, name.as_str().to_string());
            if let Some(entries) = self.modules.get(name) {
                for (_, meta) in entries.iter() {
                    adj.entry(*name).or_default().extend(meta.instances.iter().copied());
                }
            }
        }

        let mut color: HashMap<Symbol, Color> = HashMap::new();
        for name in &all {
            color.insert(*name, Color::White);
        }

        fn dfs(
            u: Symbol,
            adj: &HashMap<Symbol, Vec<Symbol>>,
            color: &mut HashMap<Symbol, Color>,
            parent: &mut Vec<Symbol>,
            name_map: &HashMap<Symbol, String>,
        ) -> Option<Vec<String>> {
            color.insert(u, Color::Gray);
            parent.push(u);
            if let Some(neighbors) = adj.get(&u) {
                for v in neighbors {
                    match color.get(v).copied().unwrap_or(Color::White) {
                        Color::Gray => {
                            // Found cycle — extract path from first occurrence of v
                            let start = parent.iter().position(|x| x == v).unwrap_or(0);
                            let cycle: Vec<String> = parent[start..]
                                .iter()
                                .chain(std::iter::once(v))
                                .map(|s| name_map.get(s).cloned().unwrap_or_default())
                                .collect();
                            return Some(cycle);
                        }
                        Color::White => {
                            if let Some(cycle) = dfs(*v, adj, color, parent, name_map) {
                                return Some(cycle);
                            }
                        }
                        Color::Black => {}
                    }
                }
            }
            parent.pop();
            color.insert(u, Color::Black);
            None
        }

        let mut parent = Vec::new();
        for name in &all {
            if color.get(name).copied().unwrap_or(Color::White) == Color::White {
                if let Some(cycle) = dfs(*name, &adj, &mut color, &mut parent, &name_map) {
                    return Some(cycle);
                }
            }
        }
        None
    }
}

impl Default for ModuleIndex {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn test_insert_lookup() {
        let index = ModuleIndex::new();
        let meta = ModuleMeta {
            name: Symbol::intern("adder"),
            file: PathBuf::from("adder.sv"),
            file_checksum: 42,
            ports: vec![Symbol::intern("a"), Symbol::intern("b")],
            params: vec![],
            instances: vec![],
            imports: vec![],
        };
        index.insert(Symbol::intern("adder"), EntryKind::Module, meta);
        let found = index.lookup(Symbol::intern("adder"), EntryKind::Module);
        assert!(found.is_some());
        assert_eq!(found.unwrap().file_checksum, 42);
    }

    #[test]
    fn test_contains() {
        let index = ModuleIndex::new();
        let meta = ModuleMeta {
            name: Symbol::intern("top"),
            file: "top.sv".into(),
            file_checksum: 0,
            ports: vec![],
            params: vec![],
            instances: vec![],
            imports: vec![],
        };
        index.insert(Symbol::intern("top"), EntryKind::Module, meta);
        assert!(index.contains(Symbol::intern("top"), EntryKind::Module));
        assert!(!index.contains(Symbol::intern("top"), EntryKind::Package));
    }

    #[test]
    fn test_remove_file() {
        let index = ModuleIndex::new();
        let file = PathBuf::from("test.sv");
        for name in &["mod_a", "mod_b"] {
            index.insert(
                Symbol::intern(name),
                EntryKind::Module,
                ModuleMeta {
                    name: Symbol::intern(name),
                    file: file.clone(),
                    file_checksum: 0,
                    ports: vec![],
                    params: vec![],
                    instances: vec![],
                    imports: vec![],
                },
            );
        }
        assert_eq!(index.len(), 2);
        index.remove_file(&file);
        assert_eq!(index.len(), 0);
    }

    #[test]
    fn test_thread_safety() {
        let index = Arc::new(ModuleIndex::new());
        let mut handles = Vec::new();
        for t in 0..8 {
            let idx = index.clone();
            handles.push(std::thread::spawn(move || {
                for i in 0..100 {
                    let name = format!("module_{}_{}", t, i);
                    idx.insert(
                        Symbol::intern(&name),
                        EntryKind::Module,
                        ModuleMeta {
                            name: Symbol::intern(&name),
                            file: PathBuf::from(format!("{}.sv", name)),
                            file_checksum: (t * 100 + i) as u64,
                            ports: vec![],
                            params: vec![],
                            instances: vec![],
                            imports: vec![],
                        },
                    );
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert!(
            index.len() >= 700,
            "expected ~800 entries, got {}",
            index.len()
        );
    }

    #[test]
    fn test_iter() {
        let index = ModuleIndex::new();
        index.insert(
            Symbol::intern("a"),
            EntryKind::Module,
            ModuleMeta {
                name: Symbol::intern("a"),
                file: "a.sv".into(),
                file_checksum: 0,
                ports: vec![],
                params: vec![],
                instances: vec![],
                imports: vec![],
            },
        );
        assert_eq!(index.iter().count(), 1);
    }

    #[test]
    fn test_cycle_detection_no_cycle() {
        let index = ModuleIndex::new();
        let meta_a = ModuleMeta {
            name: Symbol::intern("a"),
            file: "a.sv".into(),
            file_checksum: 0,
            ports: vec![],
            params: vec![],
            instances: vec![Symbol::intern("b")],
            imports: vec![],
        };
        let meta_b = ModuleMeta {
            name: Symbol::intern("b"),
            file: "b.sv".into(),
            file_checksum: 0,
            ports: vec![],
            params: vec![],
            instances: vec![],
            imports: vec![],
        };
        index.insert(Symbol::intern("a"), EntryKind::Module, meta_a);
        index.insert(Symbol::intern("b"), EntryKind::Module, meta_b);
        assert!(index.detect_cycles().is_none());
    }

    #[test]
    fn test_cycle_detection_self_loop() {
        let index = ModuleIndex::new();
        let meta = ModuleMeta {
            name: Symbol::intern("a"),
            file: "a.sv".into(),
            file_checksum: 0,
            ports: vec![],
            params: vec![],
            instances: vec![Symbol::intern("a")],
            imports: vec![],
        };
        index.insert(Symbol::intern("a"), EntryKind::Module, meta);
        assert!(index.detect_cycles().is_some());
    }

    #[test]
    fn test_cycle_detection_multi() {
        let index = ModuleIndex::new();
        let meta_a = ModuleMeta {
            name: Symbol::intern("a"),
            file: "a.sv".into(),
            file_checksum: 0,
            ports: vec![],
            params: vec![],
            instances: vec![Symbol::intern("b")],
            imports: vec![],
        };
        let meta_b = ModuleMeta {
            name: Symbol::intern("b"),
            file: "b.sv".into(),
            file_checksum: 0,
            ports: vec![],
            params: vec![],
            instances: vec![Symbol::intern("c")],
            imports: vec![],
        };
        let meta_c = ModuleMeta {
            name: Symbol::intern("c"),
            file: "c.sv".into(),
            file_checksum: 0,
            ports: vec![],
            params: vec![],
            instances: vec![Symbol::intern("a")],
            imports: vec![],
        };
        index.insert(Symbol::intern("a"), EntryKind::Module, meta_a);
        index.insert(Symbol::intern("b"), EntryKind::Module, meta_b);
        index.insert(Symbol::intern("c"), EntryKind::Module, meta_c);
        assert!(index.detect_cycles().is_some());
    }
}
