//! Scratch debug helper: read preprocessed combined source for a file from an
//! MICD preprocess cache store. Debug aid, not production code.
use std::path::Path;

fn main() {
    let pid = std::env::var("MP_PID").expect("MP_PID=<project id>");
    let path = std::env::var("MP_FILE").expect("MP_FILE=<relative file path>");
    let db_root = std::env::var("MP_ROOT").unwrap_or_else(|_| "/home/whale-d/maria/.maria/database".into());
    let store_root = Path::new(&db_root).join("cache").join(pid).join("preprocess");
    let mut store = maria_compiler::micd::cache::store::CategoryStore::open(
        &store_root,
        maria_compiler::micd::cache::category::CacheCategory::Preprocess,
        0,
    );
    match store.get(&path) {
        Some(bytes) => {
            print!("{}", String::from_utf8_lossy(&bytes));
        }
        None => {
            if std::env::var("MP_KEYS").is_ok() {
                eprintln!("keys: {:?}", store.keys());
            } else {
                eprintln!("NO CACHE ENTRY for {}", path);
            }
        }
    }
}