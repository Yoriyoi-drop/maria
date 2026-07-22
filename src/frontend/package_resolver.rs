//! Package Resolver — maps package names to file paths.
//!
//! Supports `import pkg::*` and `import pkg::item` resolution.

use std::path::PathBuf;

use dashmap::DashMap;

use crate::intern::Symbol;

/// Package entry — metadata about a package.
#[derive(Debug, Clone)]
pub struct PackageEntry {
    pub name: Symbol,
    pub file: PathBuf,
    pub checksum: u64,
    /// Items exported by this package
    pub exports: Vec<Symbol>,
}

/// Thread-safe package resolver.
pub struct PackageResolver {
    packages: DashMap<Symbol, PackageEntry>,
    /// File → packages it defines
    file_packages: DashMap<PathBuf, Vec<Symbol>>,
}

impl PackageResolver {
    pub fn new() -> Self {
        PackageResolver {
            packages: DashMap::new(),
            file_packages: DashMap::new(),
        }
    }

    /// Register a package definition.
    pub fn register(&self, name: Symbol, file: PathBuf, checksum: u64, exports: Vec<Symbol>) {
        let entry = PackageEntry {
            name,
            file: file.clone(),
            checksum,
            exports,
        };
        self.packages.insert(name, entry);
        self.file_packages.entry(file).or_default().push(name);
    }

    /// Look up a package by name.
    pub fn resolve(&self, name: Symbol) -> Option<PackageEntry> {
        self.packages.get(&name).map(|e| e.clone())
    }

    /// Check if a package exists.
    pub fn has_package(&self, name: Symbol) -> bool {
        self.packages.contains_key(&name)
    }

    /// Get all packages defined in a file.
    pub fn packages_in_file(&self, file: &std::path::Path) -> Vec<Symbol> {
        self.file_packages
            .get(file)
            .map(|e| e.clone())
            .unwrap_or_default()
    }

    /// Get all registered package names.
    pub fn all_packages(&self) -> Vec<Symbol> {
        self.packages.iter().map(|e| *e.key()).collect()
    }

    /// Number of registered packages.
    pub fn len(&self) -> usize {
        self.packages.len()
    }

    /// Is empty.
    pub fn is_empty(&self) -> bool {
        self.packages.is_empty()
    }

    /// Clear all entries.
    pub fn clear(&self) {
        self.packages.clear();
        self.file_packages.clear();
    }
}

impl Default for PackageResolver {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_package_resolver_register() {
        let resolver = PackageResolver::new();
        let name = Symbol::intern("my_pkg");
        let file = PathBuf::from("/tmp/my_pkg.sv");

        resolver.register(name, file.clone(), 12345, vec![Symbol::intern("MyType")]);

        assert!(resolver.has_package(name));
        let entry = resolver.resolve(name).unwrap();
        assert_eq!(entry.name, name);
        assert_eq!(entry.file, file);
    }

    #[test]
    fn test_package_resolver_file_lookup() {
        let resolver = PackageResolver::new();
        let file = PathBuf::from("/tmp/pkg.sv");

        resolver.register(Symbol::intern("pkg1"), file.clone(), 1, vec![]);
        resolver.register(Symbol::intern("pkg2"), file.clone(), 2, vec![]);

        let pkgs = resolver.packages_in_file(&file);
        assert_eq!(pkgs.len(), 2);
    }
}
