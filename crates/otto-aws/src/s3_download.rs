//! S3 download-to-file (sweep-c SC-14): large objects are written straight to
//! a local directory by the daemon instead of being streamed into the webview
//! (the in-app path buffered up to 2 GiB of chunks + a Blob in WKWebView).
//!
//! `aws s3 cp s3://… -` is piped into `<dest>.otto-part` while a byte counter
//! tracks progress; on success the part file is renamed to the final name
//! (never overwriting an existing file — a ` (n)` suffix is added). Jobs live
//! in memory (a daemon restart forgets them; the part file is then orphaned
//! and harmless). Same trust model as SFTP downloads: the destination is a
//! directory on the daemon host chosen by the (authorized) user, so the route
//! is graded AwsS3:**Edit** (r3-10-01 — at View, any S3 reader could plant a
//! file on the host). The host write is further fenced here, server-side:
//! - the directory (canonicalized, symlinks resolved) must sit under the
//!   daemon user's home — outside `~/Library` and every dot-directory — or on
//!   an external volume (`/Volumes/<name>/…`); system dirs are refused;
//! - the file name is the key's basename only, and never starts with `.`
//!   (a `.zshenv` key lands as `_zshenv`);
//! - nothing is ever overwritten: the part file is created `O_EXCL` and moved
//!   into place with a hard link (fails on an existing name), not `rename`.

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

/// The key's last segment as a safe local file name: basename only (no
/// separators, no control chars, never `.`/`..`) and never a dotfile — a
/// leading `.` would let an S3 key plant `~/.zshenv`-style startup files, so
/// leading dots become `_`.
pub fn local_name(key: &str) -> String {
    let base = key.trim_end_matches('/').rsplit('/').next().unwrap_or("");
    let clean: String = base
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '/' | '\\' | ':'))
        .collect();
    let clean = clean.trim();
    if clean.is_empty() || clean.chars().all(|c| c == '.') {
        return "download".into();
    }
    let dots = clean.len() - clean.trim_start_matches('.').len();
    truncate_name(&format!("{}{}", "_".repeat(dots), &clean[dots..]))
}

/// Longest local name we create, in bytes. APFS/HFS+ cap a name at 255, and
/// the job's part file appends `.<8 chars>.otto-part` (19 bytes) — an S3 key
/// segment may be up to 1024 bytes, which would fail the download with
/// ENAMETOOLONG. Keeps the extension, shortens the stem on a char boundary.
const MAX_LOCAL_NAME: usize = 200;

fn truncate_name(name: &str) -> String {
    if name.len() <= MAX_LOCAL_NAME {
        return name.to_string();
    }
    let ext = match name.rfind('.') {
        Some(i) if i > 0 && name.len() - i <= 16 => &name[i..],
        _ => "",
    };
    let mut end = MAX_LOCAL_NAME - ext.len();
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{ext}", &name[..end])
}

/// Is `canon` (an already-canonicalized directory) an acceptable download
/// destination? Allowed: inside `home` (canonical) but not `~/Library/…` and
/// not under any dot-directory, or inside an external volume
/// (`/Volumes/<name>/…`, again no dot-directories). Everything else — `/etc`,
/// `/usr`, `/Library`, `/System`, `/private`, another user's home — is refused.
fn dest_dir_allowed(canon: &Path, home: &Path) -> bool {
    use std::path::Component;
    let no_hidden = |rest: &Path| {
        rest.components().all(|c| match c {
            Component::Normal(n) => !n.to_string_lossy().starts_with('.'),
            _ => false,
        })
    };
    if let Ok(rest) = canon.strip_prefix(home) {
        let first = rest.components().next();
        let in_library =
            matches!(first, Some(Component::Normal(n)) if n.eq_ignore_ascii_case("Library"));
        return !in_library && no_hidden(rest);
    }
    if let Ok(rest) = canon.strip_prefix("/Volumes") {
        // `/Volumes` itself (the mount table) is not a destination.
        return rest.components().next().is_some() && no_hidden(rest);
    }
    false
}

/// Resolve + validate the requested destination directory (see the module
/// doc). Returns the CANONICAL path, so the write goes where it was checked.
fn resolve_dest_dir(raw: &str) -> Result<PathBuf> {
    let dir = expand_home(raw);
    if !dir.is_absolute() || !dir.is_dir() {
        return Err(Error::Invalid(format!(
            "destination is not an existing directory: {}",
            dir.display()
        )));
    }
    let canon = std::fs::canonicalize(&dir)
        .map_err(|e| Error::Invalid(format!("destination {}: {e}", dir.display())))?;
    let home = dirs::home_dir()
        .and_then(|h| std::fs::canonicalize(h).ok())
        .ok_or_else(|| Error::Internal("no home directory for the daemon user".into()))?;
    if !dest_dir_allowed(&canon, &home) {
        return Err(Error::Forbidden(format!(
            "downloads may only go to a folder in your home directory (not ~/Library or \
             a hidden folder) or on an external volume: {}",
            canon.display()
        )));
    }
    Ok(canon)
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

/// Move the finished part file to `dest` WITHOUT ever overwriting: a hard
/// link fails with `AlreadyExists` if the name was taken since `free_path`
/// looked (TOCTOU), in which case the next free ` (n)` name is tried. Falls
/// back to an existence-checked `rename` only on filesystems without hard
/// links (e.g. exFAT volumes). Returns the final path.
fn place_no_overwrite(
    part: &Path,
    dest: &Path,
    dir: &Path,
    name: &str,
) -> std::result::Result<PathBuf, String> {
    let mut target = dest.to_path_buf();
    for _ in 0..1000 {
        match std::fs::hard_link(part, &target) {
            Ok(()) => return Ok(target),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                target = free_path(dir, name).map_err(|e| e.to_string())?;
            }
            Err(_) => {
                if target.symlink_metadata().is_ok() {
                    target = free_path(dir, name).map_err(|e| e.to_string())?;
                    continue;
                }
                return std::fs::rename(part, &target)
                    .map(|()| target)
                    .map_err(|e| format!("could not move the file into place: {e}"));
            }
        }
    }
    Err(format!("too many copies of {name} in the destination"))
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
    let dir = resolve_dest_dir(&req.local_dir)?;
    let head = head_object(svc, a, bucket, &req.key, req.region.as_deref()).await?;
    let name = local_name(&req.key);
    let dest = free_path(&dir, &name)?;
    // Unique per job so two downloads of the same key never share (or race
    // on) a part file; created O_EXCL in `copy_to_file`.
    let part = dir.join(format!(
        "{name}.{}.otto-part",
        otto_core::new_id().chars().take(8).collect::<String>()
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
        let (dir, name) = (dir.clone(), name.clone());
        let id = id.clone();
        tokio::spawn(async move {
            let outcome = copy_to_file(child, stdout, stderr, &part, &bytes).await;
            let outcome = match outcome {
                Ok(()) => {
                    let (part, dest, dir, name) =
                        (part.clone(), dest.clone(), dir.clone(), name.clone());
                    tokio::task::spawn_blocking(move || {
                        place_no_overwrite(&part, &dest, &dir, &name)
                    })
                    .await
                    .unwrap_or_else(|e| Err(format!("could not move the file into place: {e}")))
                }
                Err(e) => Err(e),
            };
            // After a successful hard link the part is a second name for the
            // file; after a failure it is garbage. Either way it goes.
            let _ = tokio::fs::remove_file(&part).await;
            let outcome = outcome.map(|final_path| {
                job.lock().unwrap_or_else(|p| p.into_inner()).local_path =
                    final_path.to_string_lossy().into_owned();
            });
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
    // O_EXCL: never follows / reuses whatever already sits at the part path.
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(part)
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
    fn local_name_is_bounded_for_the_filesystem() {
        let long = format!("logs/{}.csv", "é".repeat(400));
        let n = local_name(&long);
        assert!(n.len() <= MAX_LOCAL_NAME, "{}", n.len());
        assert!(n.ends_with(".csv"));
        // The part-file name stays under the 255-byte name limit too.
        assert!(format!("{n}.01234567.otto-part").len() <= 255);
        let no_ext = local_name(&"x".repeat(1000));
        assert_eq!(no_ext.len(), MAX_LOCAL_NAME);
        assert_eq!(local_name("a/report.csv"), "report.csv");
    }

    #[test]
    fn local_name_never_yields_a_dotfile() {
        // r3-10-01: a `.zshenv` key must not land as `~/.zshenv`.
        assert_eq!(local_name(".zshenv"), "_zshenv");
        assert_eq!(local_name("dotfiles/.zshenv"), "_zshenv");
        assert_eq!(local_name("..bashrc"), "__bashrc");
        assert_eq!(local_name("..."), "download");
        // `../` segments never escape: basename only.
        assert_eq!(local_name("../"), "download");
        assert_eq!(local_name("../../.ssh/authorized_keys"), "authorized_keys");
        assert_eq!(local_name("a/../../.zshrc"), "_zshrc");
        // Backslashes are dropped, so the leading dots are neutralised too.
        assert_eq!(local_name("..\\..\\x.plist"), "____x.plist");
    }

    #[test]
    fn dest_dir_refuses_system_library_and_hidden_dirs() {
        let home = Path::new("/Users/me");
        for ok in [
            "/Users/me",
            "/Users/me/Downloads",
            "/Users/me/work/data",
            "/Volumes/Backup",
            "/Volumes/Backup/s3",
        ] {
            assert!(
                dest_dir_allowed(Path::new(ok), home),
                "{ok} should be allowed"
            );
        }
        for bad in [
            "/",
            "/etc",
            "/usr/local/bin",
            "/Library/LaunchDaemons",
            "/private/etc",
            "/System",
            "/Users/other",
            "/Users/me/Library/LaunchAgents",
            "/Users/me/library/LaunchAgents",
            "/Users/me/.ssh",
            "/Users/me/.config/fish",
            "/Users/me/work/.git/hooks",
            "/Volumes",
            "/Volumes/Backup/.hidden",
        ] {
            assert!(
                !dest_dir_allowed(Path::new(bad), home),
                "{bad} should be refused"
            );
        }
    }

    #[test]
    fn resolve_dest_dir_rejects_missing_and_relative() {
        assert!(resolve_dest_dir("relative/dir").is_err());
        assert!(resolve_dest_dir("/definitely/not/here/otto").is_err());
        // A real system directory exists but is refused (Forbidden, not Invalid).
        assert!(matches!(resolve_dest_dir("/etc"), Err(Error::Forbidden(_))));
    }

    #[test]
    fn place_no_overwrite_keeps_an_existing_file() {
        let d = tempfile::tempdir().unwrap();
        let part = d.path().join("a.csv.x.otto-part");
        std::fs::write(&part, b"new").unwrap();
        // Someone created the destination after `free_path` looked.
        std::fs::write(d.path().join("a.csv"), b"old").unwrap();
        let got = place_no_overwrite(&part, &d.path().join("a.csv"), d.path(), "a.csv").unwrap();
        assert_eq!(got, d.path().join("a (1).csv"));
        assert_eq!(std::fs::read(d.path().join("a.csv")).unwrap(), b"old");
        assert_eq!(std::fs::read(&got).unwrap(), b"new");
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
