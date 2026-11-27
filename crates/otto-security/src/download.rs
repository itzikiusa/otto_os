use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use flate2::read::GzDecoder;
use otto_core::{Error, Result};
use reqwest::{Client, Response};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

use crate::governor::cancelled;

const UPSTREAM_HOSTS: &[&str] = &[
    "api.github.com",
    "github.com",
    "release-assets.githubusercontent.com",
    "ghcr.io",
    "pkg-containers.githubusercontent.com",
    "semgrep.dev",
];

pub(crate) fn client() -> Result<Client> {
    Client::builder()
        .user_agent("Otto-security-scans")
        .no_proxy()
        .https_only(true)
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() < 4
                && attempt
                    .url()
                    .host_str()
                    .is_some_and(|h| UPSTREAM_HOSTS.contains(&h))
                && attempt.url().port_or_known_default() == Some(443)
            {
                attempt.follow()
            } else {
                attempt.error("unexpected scanner download redirect")
            }
        }))
        .build()
        .map_err(|_| Error::Internal("could not construct scanner update client".into()))
}

pub(crate) fn upstream(message: &str) -> Error {
    Error::Upstream(message.into())
}

pub(crate) async fn json<T: serde::de::DeserializeOwned>(response: Response) -> Result<T> {
    const MAX_BYTES: usize = 2 * 1024 * 1024;
    let mut response = response
        .error_for_status()
        .map_err(|_| upstream("The scanner update service rejected the request."))?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| upstream("Scanner update read failed."))?
    {
        if bytes.len() + chunk.len() > MAX_BYTES {
            return Err(upstream(
                "The scanner update metadata exceeded its size cap.",
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| upstream("The scanner update metadata was unreadable."))
}

pub(crate) fn sha256(value: &str) -> Result<&str> {
    value
        .strip_prefix("sha256:")
        .filter(|v| v.len() == 64 && v.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(|| upstream("The upstream scanner asset has no valid SHA-256 digest."))
}

pub(crate) async fn verified_file(
    response: Response,
    path: &Path,
    expected: &str,
    limit: u64,
) -> Result<()> {
    let mut response = response
        .error_for_status()
        .map_err(|_| upstream("The scanner download service rejected the request."))?;
    if response.content_length().is_some_and(|n| n > limit) {
        return Err(upstream(
            "The scanner update exceeded its download size cap.",
        ));
    }
    let mut file = tokio::fs::File::create(path).await.map_err(io_error)?;
    let mut hash = Sha256::new();
    let mut total = 0u64;
    let start = Instant::now();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| upstream("Scanner download failed."))?
    {
        total += chunk.len() as u64;
        if total > limit {
            return Err(upstream(
                "The scanner update exceeded its download size cap.",
            ));
        }
        hash.update(&chunk);
        file.write_all(&chunk).await.map_err(io_error)?;
        // Leave room for interactive traffic even on a fast connection.
        let paced = Duration::from_secs_f64(total as f64 / (12.0 * 1024.0 * 1024.0));
        if let Some(delay) = paced.checked_sub(start.elapsed()) {
            tokio::time::sleep(delay).await;
        }
    }
    file.flush().await.map_err(io_error)?;
    if !hex::encode(hash.finalize()).eq_ignore_ascii_case(expected) {
        return Err(upstream("Scanner download SHA-256 verification failed."));
    }
    Ok(())
}

pub(crate) fn extract(
    archive: &Path,
    dir: &Path,
    names: &[&str],
    limit: u64,
    cancel: &CancellationToken,
) -> Result<()> {
    #[cfg(target_os = "macos")]
    let _background = BackgroundThread::enter();
    let decoder = GzDecoder::new(File::open(archive).map_err(io_error)?);
    let mut archive = tar::Archive::new(decoder.take(limit));
    let mut found = 0;
    for entry in archive.entries().map_err(io_error)?.raw(true) {
        if cancel.is_cancelled() {
            return Err(cancelled());
        }
        let mut entry = entry.map_err(io_error)?;
        let path = entry.path().map_err(io_error)?;
        let Some(name) = names.iter().find(|name| {
            path == Path::new(name) || path.strip_prefix(".").ok() == Some(Path::new(name))
        }) else {
            continue;
        };
        if !entry.header().entry_type().is_file() || entry.size() > limit {
            return Err(upstream("The scanner archive contains an invalid asset."));
        }
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dir.join(name))
            .map_err(io_error)?;
        let mut buffer = [0; 64 * 1024];
        loop {
            if cancel.is_cancelled() {
                return Err(cancelled());
            }
            let len = entry.read(&mut buffer).map_err(io_error)?;
            if len == 0 {
                break;
            }
            output.write_all(&buffer[..len]).map_err(io_error)?;
        }
        if output.metadata().map_err(io_error)?.len() != entry.size() {
            return Err(upstream("The scanner archive contains a truncated asset."));
        }
        found += 1;
    }
    if found != names.len() {
        return Err(upstream("The scanner archive is missing required assets."));
    }
    Ok(())
}

pub(crate) fn io_error(error: io::Error) -> Error {
    tracing::warn!(%error, "scanner file operation failed");
    Error::internal_public("Otto could not read or write the scanner's local files.")
}

#[cfg(target_os = "macos")]
pub(crate) struct BackgroundThread;

#[cfg(target_os = "macos")]
impl BackgroundThread {
    pub(crate) fn enter() -> Self {
        // This runs on the blocking pool. Restore its normal priority before returning it.
        unsafe { libc::setpriority(libc::PRIO_DARWIN_THREAD, 0, libc::PRIO_DARWIN_BG) };
        Self
    }
}

#[cfg(target_os = "macos")]
impl Drop for BackgroundThread {
    fn drop(&mut self) {
        unsafe { libc::setpriority(libc::PRIO_DARWIN_THREAD, 0, 0) };
    }
}
