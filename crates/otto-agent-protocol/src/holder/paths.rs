//! Launcher discovery and Unix domain socket path management for `runtime-holder`.

use std::ffi::OsStr;
use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// Safe margin below macOS `sockaddr_un.sun_path` (104 bytes incl. trailing NUL).
const MAX_SOCKET_PATH_BYTES: usize = 96;

/// Binary + prefix args used to invoke `runtime-holder`.
#[derive(Debug, Clone)]
pub struct HolderLauncher {
    pub(super) program: PathBuf,
    pub(super) prefix_args: Vec<String>,
}

impl HolderLauncher {
    /// Launch via `ottod runtime-holder --socket <path>`.
    pub fn for_ottod(ottod_bin: impl Into<PathBuf>) -> Self {
        Self {
            program: ottod_bin.into(),
            prefix_args: vec!["runtime-holder".into()],
        }
    }

    /// Launch via standalone `otto-runtime-holder --socket <path>`.
    pub fn for_bin(holder_bin: impl Into<PathBuf>) -> Self {
        Self {
            program: holder_bin.into(),
            prefix_args: Vec::new(),
        }
    }

    /// Auto-detect the holder launcher from `OTTO_RUNTIME_HOLDER_BIN` or when
    /// running as the `ottod` daemon binary (`ottod runtime-holder`). Returns
    /// `None` inside `deps/*` test harnesses unless explicitly configured via
    /// `RuntimeConfig::holder_launcher` or `OTTO_RUNTIME_HOLDER_BIN`, so
    /// in-process tests never leak detached processes on drop.
    #[allow(clippy::disallowed_methods)]
    pub fn auto_detect() -> Option<Self> {
        if let Some(raw) = std::env::var_os("OTTO_RUNTIME_HOLDER_BIN") {
            let path = PathBuf::from(raw);
            if !path.as_os_str().is_empty() {
                let is_ottod = path
                    .file_stem()
                    .and_then(OsStr::to_str)
                    .is_some_and(|s| s == "ottod");
                return Some(if is_ottod {
                    Self::for_ottod(path)
                } else {
                    Self::for_bin(path)
                });
            }
        }
        let exe = std::env::current_exe().ok()?;
        let in_deps = exe
            .parent()
            .and_then(|p| p.file_name())
            .is_some_and(|n| n == OsStr::new("deps"));
        if !in_deps {
            match exe.file_stem().and_then(OsStr::to_str) {
                Some("ottod") => return Some(Self::for_ottod(exe)),
                Some("otto-runtime-holder") => return Some(Self::for_bin(exe)),
                _ => {}
            }
        }
        None
    }
}

/// Resolve the Unix socket path for `session_id`, falling back to a private
/// `0700` `/tmp/otto-rth-<hash>` directory when `holders_dir` exceeds `SUN_LEN`.
#[allow(clippy::disallowed_methods)]
pub fn socket_path_for(holders_dir: &Path, session_id: &str) -> io::Result<PathBuf> {
    let file_name = format!("{}.sock", sanitize_session_id(session_id));
    let primary = holders_dir.join(&file_name);
    if primary.as_os_str().as_encoded_bytes().len() <= MAX_SOCKET_PATH_BYTES {
        ensure_private_dir(holders_dir)?;
        return Ok(primary);
    }
    let fallback_dir = fallback_socket_dir(holders_dir);
    ensure_private_dir(&fallback_dir)?;
    let fallback = fallback_dir.join(&file_name);
    if fallback.as_os_str().as_encoded_bytes().len() > MAX_SOCKET_PATH_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "runtime holder socket path too long: {}",
                fallback.display()
            ),
        ));
    }
    Ok(fallback)
}

/// Discover all `*.sock` holder endpoints in both `holders_dir` and its
/// length-fallback `/tmp/otto-rth-<hash>` directory.
#[allow(clippy::disallowed_methods)]
pub fn list_holder_sockets(holders_dir: &Path) -> Vec<PathBuf> {
    let uid = current_euid();
    let mut out = Vec::new();
    for dir in [holders_dir.to_path_buf(), fallback_socket_dir(holders_dir)] {
        let Ok(meta) = std::fs::symlink_metadata(&dir) else {
            continue;
        };
        if !meta.file_type().is_dir() || meta.uid() != uid {
            continue;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in rd.flatten() {
            let path = entry.path();
            if path.extension() == Some(OsStr::new("sock")) && !out.contains(&path) {
                out.push(path);
            }
        }
    }
    out
}

#[allow(clippy::disallowed_methods)]
fn ensure_private_dir(dir: &Path) -> io::Result<()> {
    if std::fs::symlink_metadata(dir).is_err() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
    }
    let meta = std::fs::symlink_metadata(dir)?;
    if !meta.file_type().is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "holder directory is not a real directory: {}",
                dir.display()
            ),
        ));
    }
    if meta.uid() != current_euid() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "holder directory owned by unexpected uid: {}",
                dir.display()
            ),
        ));
    }
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
}

fn fallback_socket_dir(holders_dir: &Path) -> PathBuf {
    let hash = fnv1a64(holders_dir.as_os_str().as_encoded_bytes());
    PathBuf::from(format!("/tmp/otto-rth-{hash:016x}"))
}

fn sanitize_session_id(session_id: &str) -> String {
    let clean: String = session_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(40)
        .collect();
    if clean == session_id && !clean.is_empty() {
        clean
    } else {
        format!("{clean}-{:08x}", fnv1a64(session_id.as_bytes()) as u32)
    }
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn current_euid() -> u32 {
    // SAFETY: `geteuid` has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}
