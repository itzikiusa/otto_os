//! Shared LSP stdio framing, language-server registry (with Otto toolchain and
//! runtime IDE plugin discovery), and multiplexed Local / Remote-SSH server pool.

pub mod framing;
pub mod pool;
pub mod servers;

pub use pool::evict_detached_other_roots;
