//! Process ownership of a profile is independent of its listening port.
use rustix::fd::OwnedFd;
use rustix::fs::{flock, open, FlockOperation, Mode, OFlags};
use std::path::Path;

/// Keep this descriptor alive through shutdown. Never unlink the lock file:
/// another process may already hold its inode open while waiting to acquire it.
pub fn acquire(data_dir: &Path) -> Result<OwnedFd, String> {
    std::fs::create_dir_all(data_dir).map_err(|e| format!("create profile directory: {e}"))?;
    let directory = data_dir
        .canonicalize()
        .map_err(|e| format!("resolve profile directory: {e}"))?;
    let lock = open(
        directory.join("ottod.lock"),
        OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
    )
    .map_err(|e| format!("open profile lock {}: {e}", directory.display()))?;
    flock(&lock, FlockOperation::NonBlockingLockExclusive)
        .map_err(|e| format!("profile {} is already in use or cannot be locked: {e}; choose a separate OTTO_DATA_DIR for another daemon", directory.display()))?;
    Ok(lock)
}
