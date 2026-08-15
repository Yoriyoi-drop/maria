//! PLI — Programming Language Interface (IEEE 1364 Verilog PLI 1.0/2.0).
//!
//! Adapter ABI-compatible di atas kernel Maria (arsitektur masukan user
//! poin 2). Dua mekanisme:
//!
//! - **tf** (task/function): akses argumen system task/function dari C
//!   (`tf_getp`, `tf_putp`, `tf_strgetp`, `tf_gettime`, ...).
//! - **acc** (access): navigasi object desain (`acc_handle_signal`,
//!   `acc_fetch_value`, `acc_next`, ...).
//!
//! Library eksternal tetap C ABI — tidak diterjemahkan ke Rust (poin 3).

pub mod acc;
pub mod loader;
pub mod tf;

pub use tf::{plio_error, plio_warning};
