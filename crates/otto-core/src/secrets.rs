//! Secret storage abstraction. Implemented by otto-keychain (macOS Keychain,
//! file fallback); consumed by connections, git, and the server.

use crate::Result;

/// Simple synchronous secret store keyed by item name.
///
/// Implementations must be cheap to call from async contexts (Keychain and
/// file I/O are fast); callers may wrap calls in `spawn_blocking` when needed.
pub trait SecretStore: Send + Sync {
    fn put(&self, key: &str, value: &str) -> Result<()>;
    fn get(&self, key: &str) -> Result<Option<String>>;
    fn delete(&self, key: &str) -> Result<()>;
}
