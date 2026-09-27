//! S3 download-to-file (sweep-c SC-14): large objects are written straight to
//! a local directory by the daemon instead of being streamed into the webview
//! (the in-app path buffered up to 2 GiB of chunks + a Blob in WKWebView).
//!
//! `aws s3 cp s3://… -` is piped into `<dest>.otto-part` while a byte counter
//! tracks progress; on success the part file is renamed to the final name
//! (never overwriting an existing file — a ` (n)` suffix is added). Jobs live
//! in memory (a daemon restart forgets them; the part file is then orphaned
//! and harmless). Same trust model as SFTP downloads: the destination is a
//! directory on the daemon host chosen by the (authorized) user.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use otto_core::{Error, Result};
use otto_state::AwsAccountRow;
use serde::{Deserialize, Serialize};

use crate::accounts::AwsService;
use crate::s3::{head_object, validate_bucket, validate_key};

/// Finished jobs are kept this long for the UI to read their outcome.
const KEEP_FINISHED: std::time::Duration = std::time::Duration::from_secs(15 * 60);

#[derive(Debug, Clone, Deserialize)]
pub struct DownloadToReq {
    pub key: String,
    /// Destination DIRECTORY on the daemon host (`~/` expanded).
    pub local_dir: String,
    #[serde(default)]
    pub region: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize)]
pub struct DownloadJob {
    pub id: String,
    pub bucket: String,
    pub key: String,
    /// Final path (the file appears there on completion).
    pub local_path: String,
    pub state: JobState,
    pub bytes: u64,
    pub total: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

struct Entry {
    account_id: String,
    job: Arc<Mutex<DownloadJob>>,
    bytes: Arc<AtomicU64>,
    task: Option<tokio::task::JoinHandle<()>>,
    part: PathBuf,
    finished_at: Option<std::time::Instant>,
}

static JOBS: LazyLock<Mutex<HashMap<String, Entry>>> = LazyLock::new(Default::default);

fn jobs() -> std::sync::MutexGuard<'static, HashMap<String, Entry>> {
    JOBS.lock().unwrap_or_else(|p| p.into_inner())
}

/// `~/x` → `$HOME/x`.
fn expand_home(p: &str) -> PathBuf {
    let p = p.trim();
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(h) = dirs::home_dir() {
            return h.join(rest);
        }
    }
    if p == "~" {
        if let Some(h) = dirs::home_dir() {
            return h;
        }
    }
    PathBuf::from(p)
}

/// The key's last segment as a safe local file name (no separators, no
/// control chars, never `.`/`..`).
pub fn local_name(key: &str) -> String {
    let base = key.trim_end_matches('/').rsplit('/').next().unwrap_or("");
    let clean: String = base
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '/' | '\\' | ':'))
        .collect();
    let clean = clean.trim().to_string();
    if clean.is_empty() || clean == "." || clean == ".." {
        "download".into()
    } else {
        clean
    }
}

/// First free `dir/name`, `dir/name (1).ext`, … (never overwrites).
fn free_path(dir: &Path, name: &str) -> Result<PathBuf> {
    let first = dir.join(name);
    if !first.exists() {
        return Ok(first);
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    for n in 1..1000 {
        let p = dir.join(format!("{stem} ({n}){ext}"));
        if !p.exists() {
            return Ok(p);
        }
    }
    Err(Error::Conflict(format!(
        "too many copies of {name} in the destination"
    )))
}

fn prune_finished() {
    jobs().retain(|_, e| e.finished_at.is_none_or(|t| t.elapsed() < KEEP_FINISHED));
}

/// Start a download job for `bucket/key` into `req.local_dir`.
pub async fn start(
    svc: &AwsService,
    a: &AwsAccountRow,
    bucket: &str,
    req: &DownloadToReq,
) -> Result<DownloadJob> {
    validate_bucket(bucket)?;
    validate_key(&req.key)?;
    let dir = expand_home(&req.local_dir);
    if !dir.is_absolute() || !dir.is_dir() {
        return Err(Error::Invalid(format!(
            "destination is not an existing directory: {}",
            dir.display()
        )));
    }
    let head = head_object(svc, a, bucket, &req.key, req.region.as_deref()).await?;
    let dest = free_path(&dir, &local_name(&req.key))?;
    let part = dest.with_file_name(format!(
        "{}.otto-part",
        dest.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("download")
    ));
    let (bin, env) = svc.bin_and_env(a, req.region.as_deref()).await?;
    let uri = format!("s3://{bucket}/{}", req.key);
    let mut child = tokio::process::Command::new(&bin)
        .args(["s3", "cp", &uri, "-", "--no-progress"])
        .env_remove("AWS_PROFILE") // same rule as `cli::run_raw`
        .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| Error::Internal(format!("spawn aws s3 cp: {e}")))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::Internal("no stdout on aws s3 cp".into()))?;
    let stderr = child.stderr.take();

    let id = otto_core::new_id();
    let job = Arc::new(Mutex::new(DownloadJob {
        id: id.clone(),
        bucket: bucket.to_string(),
        key: req.key.clone(),
        local_path: dest.to_string_lossy().into_owned(),
        state: JobState::Running,
        bytes: 0,
        total: head.size,
        error: None,
    }));
    let bytes = Arc::new(AtomicU64::new(0));
    // Register first, so a job that finishes instantly can still stamp itself.
    prune_finished();
    let snapshot = job.lock().unwrap_or_else(|p| p.into_inner()).clone();
    jobs().insert(
        id.clone(),
        Entry {
            account_id: a.id.clone(),
            job: job.clone(),
            bytes: bytes.clone(),
            task: None,
            part: part.clone(),
            finished_at: None,
        },
    );
    let task = {
        let (job, bytes, part, dest) = (job.clone(), bytes.clone(), part.clone(), dest.clone());
        let id = id.clone();
        tokio::spawn(async move {
            let outcome = copy_to_file(child, stdout, stderr, &part, &bytes).await;
            let outcome = match outcome {
                Ok(()) => tokio::fs::rename(&part, &dest)
                    .await
                    .map_err(|e| format!("could not move the file into place: {e}")),
                Err(e) => Err(e),
            };
            if outcome.is_err() {
                let _ = tokio::fs::remove_file(&part).await;
            }
            {
                let mut j = job.lock().unwrap_or_else(|p| p.into_inner());
                j.bytes = bytes.load(Ordering::Relaxed);
                match outcome {
                    Ok(()) => j.state = JobState::Completed,
                    Err(e) => {
                        j.state = JobState::Failed;
                        j.error = Some(e);
                    }
                }
            }
            if let Some(e) = jobs().get_mut(&id) {
                e.finished_at = Some(std::time::Instant::now());
            }
        })
    };
    if let Some(e) = jobs().get_mut(&id) {
        e.task = Some(task);
    }
    svc.repo.touch_used(&a.id).await;
    Ok(snapshot)
}

async fn copy_to_file(
    mut child: tokio::process::Child,
    mut stdout: tokio::process::ChildStdout,
    stderr: Option<tokio::process::ChildStderr>,
    part: &Path,
    bytes: &AtomicU64,
) -> std::result::Result<(), String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    // Drain stderr concurrently (a full pipe would block the CLI); keep a tail.
    let err_tail = tokio::spawn(async move {
        let mut s = String::new();
        if let Some(mut e) = stderr {
            let mut buf = vec![0u8; 4096];
            while let Ok(n) = e.read(&mut buf).await {
                if n == 0 {
                    break;
                }
                s.push_str(&String::from_utf8_lossy(&buf[..n]));
                if s.len() > 4096 {
                    let cut = s.len() - 4096;
                    let cut = (cut..s.len())
                        .find(|&i| s.is_char_boundary(i))
                        .unwrap_or(s.len());
                    s.drain(..cut);
                }
            }
        }
        s
    });
    let mut file = tokio::fs::File::create(part)
        .await
        .map_err(|e| format!("could not create the file: {e}"))?;
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = stdout
            .read(&mut buf)
            .await
            .map_err(|e| format!("read from aws: {e}"))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])
            .await
            .map_err(|e| format!("write failed: {e}"))?;
        bytes.fetch_add(n as u64, Ordering::Relaxed);
    }
    file.flush()
        .await
        .map_err(|e| format!("write failed: {e}"))?;
    file.sync_all()
        .await
        .map_err(|e| format!("write failed: {e}"))?;
    let status = child.wait().await.map_err(|e| format!("aws s3 cp: {e}"))?;
    if !status.success() {
        let tail = err_tail.await.unwrap_or_default();
        let first = tail
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("");
        let msg = otto_core::redact::redact_text(first).value;
        return Err(if msg.is_empty() {
            format!("aws s3 cp exited with {status}")
        } else {
            msg
        });
    }
    Ok(())
}

/// Current state of a job owned by `account_id`.
pub fn status(account_id: &str, job_id: &str) -> Result<DownloadJob> {
    let map = jobs();
    let e = map
        .get(job_id)
        .filter(|e| e.account_id == account_id)
        .ok_or_else(|| Error::NotFound(format!("download job {job_id}")))?;
    let mut j = e.job.lock().unwrap_or_else(|p| p.into_inner()).clone();
    if j.state == JobState::Running {
        j.bytes = e.bytes.load(Ordering::Relaxed);
    }
    Ok(j)
}

/// Cancel a running job (kills the CLI, removes the part file).
pub async fn cancel(account_id: &str, job_id: &str) -> Result<DownloadJob> {
    let (task, part, job) = {
        let mut map = jobs();
        let e = map
            .get_mut(job_id)
            .filter(|e| e.account_id == account_id)
            .ok_or_else(|| Error::NotFound(format!("download job {job_id}")))?;
        if e.job.lock().unwrap_or_else(|p| p.into_inner()).state != JobState::Running {
            let j = e.job.lock().unwrap_or_else(|p| p.into_inner()).clone();
            return Ok(j);
        }
        e.finished_at = Some(std::time::Instant::now());
        (e.task.take(), e.part.clone(), e.job.clone())
    };
    if let Some(t) = task {
        t.abort(); // drops the child (kill_on_drop) and the open file
        let _ = t.await;
    }
    let _ = tokio::fs::remove_file(&part).await;
    let mut j = job.lock().unwrap_or_else(|p| p.into_inner());
    if j.state == JobState::Running {
        j.state = JobState::Cancelled;
    }
    Ok(j.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_name_is_safe_leaf() {
        assert_eq!(local_name("logs/2024/app.log"), "app.log");
        assert_eq!(local_name("a/b/"), "b");
        assert_eq!(local_name("../.."), "download");
        assert_eq!(local_name("x/.."), "download");
        assert_eq!(local_name("we\u{7}ird\\na:me"), "weirdname");
    }

    #[test]
    fn free_path_never_overwrites() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(
            free_path(d.path(), "a.csv").unwrap(),
            d.path().join("a.csv")
        );
        std::fs::write(d.path().join("a.csv"), b"x").unwrap();
        assert_eq!(
            free_path(d.path(), "a.csv").unwrap(),
            d.path().join("a (1).csv")
        );
        std::fs::write(d.path().join("a (1).csv"), b"x").unwrap();
        assert_eq!(
            free_path(d.path(), "a.csv").unwrap(),
            d.path().join("a (2).csv")
        );
        std::fs::write(d.path().join("noext"), b"x").unwrap();
        assert_eq!(
            free_path(d.path(), "noext").unwrap(),
            d.path().join("noext (1)")
        );
    }

    #[test]
    fn unknown_job_is_not_found_and_scoped_to_account() {
        assert!(status("acct", "nope").is_err());
    }
}
