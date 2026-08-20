//! VHPI Library Loader (arsitektur masukan user poin 2, 9, 10).
//!
//! Load library VHPI → cari symbol entry (`vhpi_startup` /
//! `vhpi_init` / `vhpi_register_cb` dll) → ABI validation → panggil init.
//! Murni adapter: library eksternal tetap C ABI, tidak diterjemahkan ke Rust.

use super::object;
use crate::foreign::loader::AbiInfo;
use std::path::PathBuf;

/// Deskripsi library VHPI yang sudah dimuat.
pub struct LoadedVhpi {
    pub path: PathBuf,
    pub abi: AbiInfo,
    #[cfg(feature = "dpi")]
    pub library: std::sync::Arc<libloading::Library>,
}

impl std::fmt::Debug for LoadedVhpi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LoadedVhpi {{ path: {:?}, abi: {:?} }}", self.path, self.abi)
    }
}

/// Cari + muat library VHPI. `name` bisa path penuh atau nama (`libfoo.so`).
#[cfg(feature = "dpi")]
pub fn load_vhpi_library(name: &str) -> Result<LoadedVhpi, String> {
    use crate::foreign::loader::{load_library, find_library, current_abi};
    let search = crate::foreign::loader::default_search_paths();
    let path = find_library(name, &search)
        .ok_or_else(|| format!("VHPI library '{}' not found in search paths", name))?;
    let library = load_library(&path)?;
    Ok(LoadedVhpi { path, abi: current_abi(), library })
}

#[cfg(not(feature = "dpi"))]
pub fn load_vhpi_library(name: &str) -> Result<LoadedVhpi, String> {
    Err(format!("VHPI library '{}' loading requires feature \"dpi\"", name))
}

/// Panggil entry point init library (jika ada): `vhpi_startup` biasa
/// dipanggil simulator setelah load; di Maria engine hook `set_vhpi_engine`
/// dipanggil sebelum sim agar library bisa akses object.
#[cfg(feature = "dpi")]
pub fn call_vhpi_startup(vhpi: &LoadedVhpi) -> Result<(), String> {
    type InitFn = unsafe extern "C" fn() -> i32;
    if let Some(init) = crate::foreign::loader::resolve_symbol::<InitFn>(&vhpi.library, "vhpi_startup") {
        let rc = unsafe { init() };
        if rc != 0 {
            return Err(format!("vhpi_startup returned {}", rc));
        }
    }
    Ok(())
}

#[cfg(not(feature = "dpi"))]
pub fn call_vhpi_startup(_vhpi: &LoadedVhpi) -> Result<(), String> {
    Ok(())
}

/// Bersihkan state VHPI di end-of-simulation (engine cleanup).
pub fn vhpi_cleanup() {
    object::clear_vhpi_engine();
    object::clear_cstring_cache();
    super::handle::vhpi_clear_all_objects();
    super::callback::clear_all_callbacks();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foreign::loader::current_abi;

    #[test]
    fn test_abi_info_reports_current() {
        let abi = current_abi();
        assert_eq!(abi.arch, std::env::consts::ARCH);
        assert_eq!(abi.os, std::env::consts::OS);
        assert_eq!(abi.pointer_width, std::mem::size_of::<usize>() * 8);
    }

    #[test]
    fn test_load_missing_library_errors() {
        let res = load_vhpi_library("definitely_missing_vhpi_lib_98765");
        assert!(res.is_err(), "library imajiner harus error");
    }
}
