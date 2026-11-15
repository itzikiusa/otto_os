//! Shared LSP stdio framing, language-server registry (with Otto toolchain and
//! runtime IDE plugin discovery), module-root scoping, and bounded Local /
//! Remote-SSH server pool with total RSS enforcement.

pub mod framing;
pub mod pool;
mod pool_evict;
mod pool_mux;
mod pool_spawn;
#[cfg(test)]
mod pool_tests;
pub mod servers;
mod servers_root;
