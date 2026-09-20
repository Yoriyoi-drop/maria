//! LINUX-13: Thread Affinity / CPU Pinning Configuration.
//!
//! Menyediakan API untuk mengatur thread affinity pada platform Linux.
//! Berguna untuk mengurangi context switching dan cache thrashing
//! pada NUMA system / large core count machines.
//!
//! Catatan: Hanya diimplementasikan di Linux (cfg(target_os = "linux")).
//! Pada platform lain, API tersedia tapi no-op.

/// Set CPU affinity untuk thread saat ini.
/// `cores` adalah daftar core ID (0-based) yang diizinkan.
///
/// Contoh: `pin_to_cores(&[0, 1, 2, 3])` hanya mengizinkan
/// thread berjalan di core 0-3.
///
/// Returns `Ok(())` bila berhasil, `Err(String)` bila gagal.
pub fn pin_to_cores(cores: &[usize]) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        pin_to_cores_linux(cores)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = cores;
        Ok(()) // no-op on non-Linux
    }
}

/// Pin thread ke range连续 core IDs.
/// `start` adalah core pertama, `count` jumlah core.
pub fn pin_to_range(start: usize, count: usize) -> Result<(), String> {
    let cores: Vec<usize> = (start..start + count).collect();
    pin_to_cores(&cores)
}

/// Set affinity untuk thread saat ini ke core tertentu saja.
pub fn pin_to_core(core_id: usize) -> Result<(), String> {
    pin_to_cores(&[core_id])
}

/// Set affinity ke semua core (reset pinning).
pub fn pin_to_all_cores() -> Result<(), String> {
    let n = num_cpus::get();
    let cores: Vec<usize> = (0..n).collect();
    pin_to_cores(&cores)
}

/// Dapatkan jumlah CPU logical cores.
pub fn logical_cores() -> usize {
    num_cpus::get()
}

/// Dapatkan jumlah CPU physical cores.
pub fn physical_cores() -> usize {
    num_cpus::get_physical()
}

/// Dapatkan core ID yang tersedia untuk thread saat ini.
/// Returns daftar core IDs dari CPU set mask.
pub fn current_affinity() -> Result<Vec<usize>, String> {
    #[cfg(target_os = "linux")]
    {
        current_affinity_linux()
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok((0..num_cpus::get()).collect())
    }
}

// ═══ Linux Implementation ═══

#[cfg(target_os = "linux")]
fn pin_to_cores_linux(cores: &[usize]) -> Result<(), String> {
    use std::mem;

    if cores.is_empty() {
        return Err("cores list kosong".into());
    }

    let ncpu = num_cpus::get();

    // Build cpu_set_t bitmask
    // cpu_set_t is typically 1024 bytes (128 * 8 bits)
    const CPU_SETSIZE: usize = 1024;
    let mut cpu_set: [u64; CPU_SETSIZE / 64] = [0u64; CPU_SETSIZE / 64];

    for &core in cores {
        if core >= ncpu {
            return Err(format!("core {} >= jumlah CPU {}", core, ncpu));
        }
        let word = core / 64;
        let bit = core % 64;
        cpu_set[word] |= 1u64 << bit;
    }

    // sched_setaffinity syscall
    // pid=0 means current thread
    let ret = unsafe {
        libc::sched_setaffinity(
            0, // pid = current thread
            mem::size_of::<[u64; CPU_SETSIZE / 64]>(),
            cpu_set.as_ptr() as *const libc::cpu_set_t,
        )
    };

    if ret == 0 {
        Ok(())
    } else {
        let errno = std::io::Error::last_os_error();
        Err(format!("sched_setaffinity gagal: {}", errno))
    }
}

#[cfg(target_os = "linux")]
fn current_affinity_linux() -> Result<Vec<usize>, String> {
    use std::mem;

    const CPU_SETSIZE: usize = 1024;
    let mut cpu_set: [u64; CPU_SETSIZE / 64] = [0u64; CPU_SETSIZE / 64];

    let ret = unsafe {
        libc::sched_getaffinity(
            0,
            mem::size_of::<[u64; CPU_SETSIZE / 64]>(),
            cpu_set.as_mut_ptr() as *mut libc::cpu_set_t,
        )
    };

    if ret == 0 {
        let mut cores = Vec::new();
        for (word_idx, &word) in cpu_set.iter().enumerate() {
            for bit in 0..64 {
                if word & (1u64 << bit) != 0 {
                    cores.push(word_idx * 64 + bit);
                }
            }
        }
        Ok(cores)
    } else {
        let errno = std::io::Error::last_os_error();
        Err(format!("sched_getaffinity gagal: {}", errno))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_logical_cores() {
        let n = logical_cores();
        assert!(n >= 1, "logical cores harus >= 1, got {}", n);
    }

    #[test]
    fn test_physical_cores() {
        let n = physical_cores();
        assert!(n >= 1, "physical cores harus >= 1, got {}", n);
    }

    #[test]
    fn test_current_affinity_default() {
        let affinity = current_affinity().unwrap();
        assert!(
            !affinity.is_empty(),
            "default affinity harus ada minimal 1 core"
        );
    }

    #[test]
    fn test_pin_to_all_cores() {
        // Pin ke semua core — harus selalu berhasil
        let result = pin_to_all_cores();
        assert!(
            result.is_ok(),
            "pin_to_all_cores harus berhasil: {:?}",
            result
        );
    }

    #[test]
    fn test_pin_to_core_out_of_range() {
        let n = logical_cores();
        let result = pin_to_core(n + 100);
        assert!(result.is_err(), "pin ke core yang tidak ada harus error");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_pin_to_cores_roundtrip() {
        // Pin ke core 0 lalu check affinity
        let _ = pin_to_core(0);
        let affinity = current_affinity().unwrap();
        assert!(affinity.contains(&0), "affinity harus mengandung core 0");

        // Reset ke semua core
        pin_to_all_cores().unwrap();
        let affinity = current_affinity().unwrap();
        assert_eq!(
            affinity.len(),
            logical_cores(),
            "reset harus mengembalikan semua core"
        );
    }
}
