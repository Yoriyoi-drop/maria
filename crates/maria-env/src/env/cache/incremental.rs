use maria_compiler::scheduler::incremental::IncrementalTracker;
use maria_compiler::scheduler::NodeId;
use std::path::Path;

/// Handle ke incremental tracker — deteksi file berubah antar build.
#[derive(Default)]
pub struct IncrementalHandle {
    inner: IncrementalTracker,
}

impl std::fmt::Debug for IncrementalHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IncrementalHandle").finish()
    }
}

impl IncrementalHandle {
    pub fn new() -> Self {
        IncrementalHandle {
            inner: IncrementalTracker::new(),
        }
    }

    pub fn register_file(&self, path: &Path, modules: Vec<NodeId>, checksum: u64) {
        self.inner.register_file(path, modules, checksum);
    }

    pub fn mark_changed(&self, path: &Path) {
        self.inner.mark_changed(path);
    }

    pub fn inner(&self) -> &IncrementalTracker {
        &self.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_incremental_handle_ops() {
        let h = IncrementalHandle::new();
        let p = Path::new("a.sv");
        h.register_file(p, vec![], 1234);
        h.mark_changed(p); // tidak panic
    }
}
