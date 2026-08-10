//! Arena allocator — bump allocation untuk AST nodes dan struktur data compiler.
//!
//! Menyediakan alokasi O(1) dengan dealokasi bulk O(1).
//! Semua alokasi thread-local dengan bump pointer — tidak ada free list traversal.

mod bump;
pub mod pool;
mod slab;
mod typed;

pub use bump::BumpArena;
pub use pool::ObjectPool;
pub use slab::Slab;
pub use typed::TypedArena;
