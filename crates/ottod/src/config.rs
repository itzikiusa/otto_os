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

    /// Log directory, first match wins:
    /// 1. `$OTTO_LOG_DIR`;
    /// 2. `<data_dir>/logs` when `$OTTO_DATA_DIR` is set — test / e2e / dev
    ///    daemons on a throwaway data dir used to write into the user's real
    ///    `~/Library/Logs/Otto`, burying real signals under their noise;
    /// 3. `~/Library/Logs/Otto` (the installed daemon; `<data_dir>/logs` when
    ///    there is no home directory).
    ///
    /// `otto-server`'s `GET /logs/daemon` resolves the same order.
    pub fn log_dir(&self) -> PathBuf {
        if let Some(dir) = std::env::var_os("OTTO_LOG_DIR") {
            return PathBuf::from(dir);
        }
        if std::env::var_os("OTTO_DATA_DIR").is_some() {
            return self.data_dir.join("logs");
        }
        dirs::home_dir()
            .map(|h| h.join("Library/Logs/Otto"))
            .unwrap_or_else(|| self.data_dir.join("logs"))
    }
}
