/// Nilai default Maria — dipakai loader/validator/context bila field tidak
/// di-set di file config (semua field `MariaConfig` bertipe `Option`).
pub const DEFAULT_EDITION: &str = "2012";
pub const DEFAULT_TARGET: &str = "native";
pub const DEFAULT_OPT_LEVEL: u8 = 1;
pub const MAX_OPT_LEVEL: u8 = 3;
pub const DEFAULT_SNAP_INTERVAL: u64 = 1000;

/// Default jumlah thread parallel (0 = auto di config berarti ikut core).
pub fn default_jobs() -> usize {
    num_cpus::get()
}

pub fn default_incremental() -> bool {
    true
}

pub fn default_cache() -> bool {
    true
}
