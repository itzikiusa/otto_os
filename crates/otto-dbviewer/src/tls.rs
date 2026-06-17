//! TLS material helpers shared by drivers.
//!
//! Engines consume TLS differently: sqlx (MySQL) and the mongodb crate want
//! *file paths*, while others can take in-memory PEM. We persist any inline PEM
//! to stable temp files (named by a content hash, so repeated calls reuse the
//! same file) and hand back paths.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use otto_core::{Error, Result};

use crate::types::TlsConfig;

fn temp_pem(prefix: &str, pem: &str) -> Result<PathBuf> {
    let mut h = DefaultHasher::new();
    pem.hash(&mut h);
    let name = format!("otto-dbv-{prefix}-{:016x}.pem", h.finish());
    let path = std::env::temp_dir().join(name);
    if !path.exists() {
        std::fs::write(&path, pem.as_bytes())
            .map_err(|e| Error::Internal(format!("write tls pem: {e}")))?;
    }
    Ok(path)
}

/// On-disk paths for the TLS material in a [`TlsConfig`]. Absent fields are
/// `None`. `client_pair` is a combined cert+key file (some clients want one).
#[derive(Debug, Clone, Default)]
pub struct TlsFiles {
    pub ca: Option<PathBuf>,
    pub client_cert: Option<PathBuf>,
    pub client_key: Option<PathBuf>,
    pub client_pair: Option<PathBuf>,
}

impl TlsFiles {
    /// Materialize the inline PEM in `cfg` to temp files.
    pub fn materialize(cfg: &TlsConfig) -> Result<TlsFiles> {
        let mut files = TlsFiles::default();
        if let Some(ca) = cfg.ca_cert.as_deref().filter(|s| !s.is_empty()) {
            files.ca = Some(temp_pem("ca", ca)?);
        }
        if let Some(cert) = cfg.client_cert.as_deref().filter(|s| !s.is_empty()) {
            files.client_cert = Some(temp_pem("cert", cert)?);
        }
        if let Some(key) = cfg.client_key.as_deref().filter(|s| !s.is_empty()) {
            files.client_key = Some(temp_pem("key", key)?);
        }
        if let (Some(cert), Some(key)) = (
            cfg.client_cert.as_deref().filter(|s| !s.is_empty()),
            cfg.client_key.as_deref().filter(|s| !s.is_empty()),
        ) {
            let combined = format!("{}\n{}\n", cert.trim_end(), key.trim_end());
            files.client_pair = Some(temp_pem("pair", &combined)?);
        }
        Ok(files)
    }
}
