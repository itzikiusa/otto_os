//! Daemon configuration from environment with macOS defaults.

use std::path::PathBuf;

/// Default loopback port.
pub const DEFAULT_PORT: u16 = 7700;

#[derive(Debug, Clone)]
pub struct Config {
    /// Data directory: `$OTTO_DATA_DIR` or `~/Library/Application Support/Otto`.
    pub data_dir: PathBuf,
    /// Loopback port: `$OTTO_PORT` or 7700.
    pub port: u16,
}

impl Config {
    pub fn load() -> Self {
        let data_dir = std::env::var_os("OTTO_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join("Library/Application Support/Otto")
            });

        let port = std::env::var("OTTO_PORT")
            .ok()
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(DEFAULT_PORT);

        Self { data_dir, port }
    }

    /// SQLite database path inside the data dir.
    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("otto.db")
    }

    /// Log directory: `~/Library/Logs/Otto` (falls back to `<data_dir>/logs`
    /// when no home directory is available).
    pub fn log_dir(&self) -> PathBuf {
        dirs::home_dir()
            .map(|h| h.join("Library/Logs/Otto"))
            .unwrap_or_else(|| self.data_dir.join("logs"))
    }
}
