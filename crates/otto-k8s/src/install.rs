//! Binary discovery + on-demand installers for `kubectl` and `k9s`.
//!
//! Contract §0/§1: on first enable the daemon checks for the binary and installs
//! it when missing — `brew` when present, else a direct download into
//! `<data_dir>/bin` (already on the daemon PATH via `augment_path`). Never
//! `sudo`; downloads use the system `curl -fsSL` so no HTTP client dependency
//! is added; the binary must run before a job reports `done`.
//!
//! Job state lives in a process-wide registry (one slot per tool) because the
//! [`crate::K8sCtx`] trait is fixed by `otto-server` and carries no service
//! object — the design doc's "Arc<Mutex<..>> inside the service" is this
//! `OnceLock`. Progress is polled through `GET /k8s/status` and pushed as
//! `Event::K8sInstallUpdated { tool, state }` on every transition.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use chrono::{DateTime, Utc};
use otto_core::event::Event;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::cli;

/// The two binaries this module manages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tool {
    Kubectl,
    K9s,
}

impl Tool {
    pub fn as_str(&self) -> &'static str {
        match self {
            Tool::Kubectl => "kubectl",
            Tool::K9s => "k9s",
        }
    }

    /// Homebrew formula name.
    fn brew_formula(&self) -> &'static str {
        self.as_str()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallState {
    Idle,
    Running,
    Done,
    Failed,
}

impl InstallState {
    pub fn as_str(&self) -> &'static str {
        match self {
            InstallState::Idle => "idle",
            InstallState::Running => "running",
            InstallState::Done => "done",
            InstallState::Failed => "failed",
        }
    }
}

/// Wire shape of one installer job (contract §2.1 `InstallJob`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallJob {
    pub tool: Tool,
    pub state: InstallState,
    pub log_tail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl InstallJob {
    fn idle(tool: Tool) -> Self {
        Self {
            tool,
            state: InstallState::Idle,
            log_tail: String::new(),
            started_at: None,
            finished_at: None,
            error: None,
        }
    }
}

/// `ToolStatus { installed, version?, path? }` for `GET /k8s/status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolStatus {
    pub installed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// Max bytes of installer log kept per job (the UI shows a collapsible tail).
const LOG_TAIL_CAP: usize = 8 * 1024;

/// Process-wide installer registry (see module doc).
#[derive(Default)]
pub struct Installer {
    jobs: Mutex<HashMap<Tool, InstallJob>>,
}

static INSTALLER: OnceLock<Arc<Installer>> = OnceLock::new();

/// The shared installer.
pub fn installer() -> Arc<Installer> {
    Arc::clone(INSTALLER.get_or_init(|| Arc::new(Installer::default())))
}

impl Installer {
    /// Current job snapshot for `tool` (`idle` when never started).
    pub fn job(&self, tool: Tool) -> InstallJob {
        self.jobs
            .lock()
            .expect("installer lock")
            .get(&tool)
            .cloned()
            .unwrap_or_else(|| InstallJob::idle(tool))
    }

    fn update(&self, tool: Tool, f: impl FnOnce(&mut InstallJob)) -> InstallJob {
        let mut jobs = self.jobs.lock().expect("installer lock");
        let job = jobs.entry(tool).or_insert_with(|| InstallJob::idle(tool));
        f(job);
        job.clone()
    }

    fn append_log(&self, tool: Tool, line: &str) {
        self.update(tool, |j| {
            if !j.log_tail.is_empty() && !j.log_tail.ends_with('\n') {
                j.log_tail.push('\n');
            }
            j.log_tail.push_str(line.trim_end());
            if j.log_tail.len() > LOG_TAIL_CAP {
                let cut = j.log_tail.len() - LOG_TAIL_CAP;
                // Keep whole lines: drop up to the first newline after the cut.
                let cut = j.log_tail[cut..]
                    .find('\n')
                    .map(|i| cut + i + 1)
                    .unwrap_or(cut);
                j.log_tail.drain(..cut);
            }
        });
    }

    /// Start installing `tool` in the background. Idempotent while a job for
    /// that tool is `running` (returns the live snapshot). A finished job may
    /// be re-run (e.g. retry after `failed`).
    pub fn start(
        self: &Arc<Self>,
        tool: Tool,
        data_dir: PathBuf,
        events: broadcast::Sender<Event>,
    ) -> InstallJob {
        let me = Arc::clone(self);
        self.start_job(
            tool,
            events,
            async move { me.install(tool, &data_dir).await },
        )
    }

    /// Job bookkeeping around an arbitrary install future (split out so tests
    /// can drive the state machine without touching brew / the network).
    fn start_job<F>(
        self: &Arc<Self>,
        tool: Tool,
        events: broadcast::Sender<Event>,
        work: F,
    ) -> InstallJob
    where
        F: std::future::Future<Output = std::result::Result<PathBuf, String>> + Send + 'static,
    {
        {
            let cur = self.job(tool);
            if cur.state == InstallState::Running {
                return cur;
            }
        }
        let snapshot = self.update(tool, |j| {
            *j = InstallJob {
                tool,
                state: InstallState::Running,
                log_tail: String::new(),
                started_at: Some(Utc::now()),
                finished_at: None,
                error: None,
            };
        });
        broadcast(&events, tool, InstallState::Running);
        let me = Arc::clone(self);
        tokio::spawn(async move {
            let result = work.await;
            let state = match &result {
                Ok(path) => {
                    me.append_log(tool, &format!("installed at {}", path.display()));
                    me.update(tool, |j| {
                        j.state = InstallState::Done;
                        j.finished_at = Some(Utc::now());
                    });
                    InstallState::Done
                }
                Err(msg) => {
                    me.append_log(tool, &format!("failed: {msg}"));
                    me.update(tool, |j| {
                        j.state = InstallState::Failed;
                        j.finished_at = Some(Utc::now());
                        j.error = Some(msg.clone());
                    });
                    InstallState::Failed
                }
            };
            broadcast(&events, tool, state);
        });
        snapshot
    }

    /// The install ladder: brew when available, else direct download. Returns
    /// the path of a binary that has been verified to run.
    async fn install(&self, tool: Tool, data_dir: &Path) -> std::result::Result<PathBuf, String> {
        if let Some(p) = locate(tool, data_dir) {
            if verify(tool, &p).await.is_some() {
                self.append_log(
                    tool,
                    &format!("{} already present at {}", tool.as_str(), p.display()),
                );
                return Ok(p);
            }
        }
        if which("brew").is_some() {
            self.append_log(tool, &format!("$ brew install {}", tool.brew_formula()));
            match self
                .run_logged(
                    tool,
                    "brew",
                    &["install", tool.brew_formula()],
                    Duration::from_secs(900),
                )
                .await
            {
                Ok(()) => {
                    if let Some(p) = locate(tool, data_dir) {
                        if verify(tool, &p).await.is_some() {
                            return Ok(p);
                        }
                    }
                    self.append_log(tool, "brew finished but the binary is not runnable; falling back to direct download");
                }
                Err(e) => {
                    self.append_log(
                        tool,
                        &format!("brew failed ({e}); falling back to direct download"),
                    );
                }
            }
        }
        self.direct_download(tool, data_dir).await
    }

    async fn direct_download(
        &self,
        tool: Tool,
        data_dir: &Path,
    ) -> std::result::Result<PathBuf, String> {
        let bin_dir = data_dir.join("bin");
        std::fs::create_dir_all(&bin_dir)
            .map_err(|e| format!("create {}: {e}", bin_dir.display()))?;
        let arch = download_arch();
        let dest = bin_dir.join(tool.as_str());
        let tmp_dir = tempdir_in(&bin_dir)?;
        let result = match tool {
            Tool::Kubectl => {
                self.append_log(tool, "$ curl -fsSL https://dl.k8s.io/release/stable.txt");
                let stable = cli::run(
                    "curl",
                    &[
                        "-fsSL".into(),
                        "https://dl.k8s.io/release/stable.txt".into(),
                    ],
                    &[],
                    Duration::from_secs(60),
                    None,
                )
                .await
                .map_err(|e| e.to_string())?
                .stdout
                .trim()
                .to_string();
                if stable.is_empty() || !stable.starts_with('v') {
                    return Err(format!("unexpected stable version marker {stable:?}"));
                }
                let url = format!("https://dl.k8s.io/release/{stable}/bin/darwin/{arch}/kubectl");
                let tmp = tmp_dir.join("kubectl");
                self.append_log(tool, &format!("$ curl -fsSL -o {} {url}", tmp.display()));
                self.run_logged(
                    tool,
                    "curl",
                    &["-fsSL", "-o", &tmp.to_string_lossy(), &url],
                    Duration::from_secs(600),
                )
                .await?;
                install_file(&tmp, &dest)
            }
            Tool::K9s => {
                let url = format!(
                    "https://github.com/derailed/k9s/releases/latest/download/k9s_Darwin_{arch}.tar.gz"
                );
                let tarball = tmp_dir.join("k9s.tar.gz");
                self.append_log(
                    tool,
                    &format!("$ curl -fsSL -o {} {url}", tarball.display()),
                );
                self.run_logged(
                    tool,
                    "curl",
                    &["-fsSL", "-o", &tarball.to_string_lossy(), &url],
                    Duration::from_secs(600),
                )
                .await?;
                self.append_log(tool, "$ tar -xzf k9s.tar.gz k9s");
                self.run_logged(
                    tool,
                    "tar",
                    &[
                        "-xzf",
                        &tarball.to_string_lossy(),
                        "-C",
                        &tmp_dir.to_string_lossy(),
                        "k9s",
                    ],
                    Duration::from_secs(120),
                )
                .await?;
                install_file(&tmp_dir.join("k9s"), &dest)
            }
        };
        let _ = std::fs::remove_dir_all(&tmp_dir);
        result?;
        match verify(tool, &dest).await {
            Some(v) => {
                self.append_log(tool, &format!("{} {v}", tool.as_str()));
                Ok(dest)
            }
            None => Err(format!(
                "{} was downloaded but does not run",
                dest.display()
            )),
        }
    }

    /// Run a step, appending stdout+stderr to the job log.
    async fn run_logged(
        &self,
        tool: Tool,
        program: &str,
        args: &[&str],
        timeout: Duration,
    ) -> std::result::Result<(), String> {
        let argv: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        // Homebrew must not prompt or auto-update mid-install.
        let env = vec![
            ("HOMEBREW_NO_AUTO_UPDATE".to_string(), "1".to_string()),
            ("HOMEBREW_NO_INSTALL_CLEANUP".to_string(), "1".to_string()),
            ("NONINTERACTIVE".to_string(), "1".to_string()),
        ];
        let out = cli::run_raw(program, &argv, &env, timeout, None)
            .await
            .map_err(|e| e.to_string())?;
        for line in out.stdout.lines().chain(out.stderr.lines()) {
            if !line.trim().is_empty() {
                self.append_log(tool, line);
            }
        }
        if out.status != 0 {
            return Err(format!("{program} exited with status {}", out.status));
        }
        Ok(())
    }
}

fn broadcast(events: &broadcast::Sender<Event>, tool: Tool, state: InstallState) {
    let _ = events.send(Event::K8sInstallUpdated {
        tool: tool.as_str().to_string(),
        state: state.as_str().to_string(),
    });
}

/// `arm64` / `amd64` as the download URLs spell it.
pub fn download_arch() -> &'static str {
    arch_label(std::env::consts::ARCH)
}

fn arch_label(rust_arch: &str) -> &'static str {
    match rust_arch {
        "aarch64" | "arm64" => "arm64",
        _ => "amd64",
    }
}

fn tempdir_in(dir: &Path) -> std::result::Result<PathBuf, String> {
    let p = dir.join(format!(".dl-{}", otto_core::new_id()));
    std::fs::create_dir_all(&p).map_err(|e| format!("create {}: {e}", p.display()))?;
    Ok(p)
}

/// Move a downloaded file into place with mode 0755.
fn install_file(src: &Path, dest: &Path) -> std::result::Result<(), String> {
    if !src.is_file() {
        return Err(format!("download did not produce {}", src.display()));
    }
    std::fs::rename(src, dest)
        .or_else(|_| std::fs::copy(src, dest).map(|_| ()))
        .map_err(|e| format!("install {}: {e}", dest.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dest, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("chmod {}: {e}", dest.display()))?;
    }
    Ok(())
}

/// `which <name>` (first PATH hit that is a regular file).
pub fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

/// Locate ladder (contract §1): `which` → `/opt/homebrew/bin`, `/usr/local/bin`,
/// `~/.local/bin`, `<data_dir>/bin`.
pub fn locate(tool: Tool, data_dir: &Path) -> Option<PathBuf> {
    locate_in(tool.as_str(), data_dir, dirs::home_dir().as_deref())
}

fn locate_in(name: &str, data_dir: &Path, home: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = which(name) {
        return Some(p);
    }
    let mut candidates = vec![
        PathBuf::from("/opt/homebrew/bin").join(name),
        PathBuf::from("/usr/local/bin").join(name),
    ];
    if let Some(h) = home {
        candidates.push(h.join(".local/bin").join(name));
    }
    candidates.push(data_dir.join("bin").join(name));
    candidates.into_iter().find(|p| p.is_file())
}

/// Run the binary's version command; `Some(version)` when it executes.
pub async fn verify(tool: Tool, path: &Path) -> Option<String> {
    let program = path.to_string_lossy().to_string();
    let args: Vec<String> = match tool {
        Tool::Kubectl => vec![
            "version".into(),
            "--client".into(),
            "-o".into(),
            "json".into(),
        ],
        Tool::K9s => vec!["version".into(), "-s".into()],
    };
    let out = cli::run_raw(&program, &args, &[], Duration::from_secs(15), None)
        .await
        .ok()?;
    if out.status != 0 {
        return None;
    }
    Some(match tool {
        Tool::Kubectl => {
            parse_kubectl_client_version(&out.stdout).unwrap_or_else(|| "unknown".into())
        }
        Tool::K9s => parse_k9s_version(&out.stdout).unwrap_or_else(|| "unknown".into()),
    })
}

/// `.clientVersion.gitVersion` from `kubectl version --client -o json`.
pub fn parse_kubectl_client_version(stdout: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).ok()?;
    v.pointer("/clientVersion/gitVersion")
        .and_then(|s| s.as_str())
        .map(str::to_string)
}

/// `k9s version -s` prints `Version  v0.32.5 / Commit … / Date …` lines.
pub fn parse_k9s_version(stdout: &str) -> Option<String> {
    for line in stdout.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("Version") {
            let v = rest.trim_start_matches([':', ' ', '\t']).trim();
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    let first = stdout.lines().find(|l| !l.trim().is_empty())?.trim();
    (first.starts_with('v') && first.len() < 32).then(|| first.to_string())
}

/// `ToolStatus` for `tool`: located path + version (or `installed: false`).
pub async fn tool_status(tool: Tool, data_dir: &Path) -> ToolStatus {
    match locate(tool, data_dir) {
        Some(p) => {
            let version = verify(tool, &p).await;
            ToolStatus {
                installed: version.is_some(),
                version,
                path: Some(p.to_string_lossy().to_string()),
            }
        }
        None => ToolStatus {
            installed: false,
            version: None,
            path: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arch_labels() {
        assert_eq!(arch_label("aarch64"), "arm64");
        assert_eq!(arch_label("x86_64"), "amd64");
    }

    #[test]
    fn locate_ladder_falls_back_to_data_dir_bin() {
        let tmp = tempfile::tempdir().unwrap();
        let name = "otto-k8s-test-binary-that-does-not-exist-on-path";
        assert_eq!(locate_in(name, tmp.path(), None), None);
        let bin = tmp.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join(name), b"#!/bin/sh\n").unwrap();
        assert_eq!(locate_in(name, tmp.path(), None), Some(bin.join(name)));
        // ~/.local/bin wins over <data_dir>/bin.
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".local/bin")).unwrap();
        std::fs::write(home.path().join(".local/bin").join(name), b"").unwrap();
        assert_eq!(
            locate_in(name, tmp.path(), Some(home.path())),
            Some(home.path().join(".local/bin").join(name))
        );
    }

    #[test]
    fn version_parsers() {
        let j = r#"{"clientVersion":{"major":"1","minor":"30","gitVersion":"v1.30.2","platform":"darwin/arm64"},"kustomizeVersion":"v5.0.4"}"#;
        assert_eq!(parse_kubectl_client_version(j).as_deref(), Some("v1.30.2"));
        assert_eq!(parse_kubectl_client_version("garbage"), None);
        assert_eq!(
            parse_k9s_version("Version    v0.32.5\nCommit     abc\nDate       2024\n").as_deref(),
            Some("v0.32.5")
        );
        assert_eq!(parse_k9s_version("v0.31.0\n").as_deref(), Some("v0.31.0"));
    }

    #[test]
    fn log_tail_is_capped_on_line_boundaries() {
        let inst = Installer::default();
        for i in 0..2000 {
            inst.append_log(
                Tool::K9s,
                &format!("line number {i} with some padding text"),
            );
        }
        let job = inst.job(Tool::K9s);
        assert!(job.log_tail.len() <= LOG_TAIL_CAP);
        assert!(
            job.log_tail.starts_with("line number"),
            "{}",
            &job.log_tail[..40]
        );
        assert!(job
            .log_tail
            .ends_with("line number 1999 with some padding text"));
    }

    #[tokio::test]
    async fn start_is_idempotent_while_running_and_broadcasts() {
        let inst = Arc::new(Installer::default());
        let (tx, mut rx) = broadcast::channel(8);
        assert_eq!(inst.job(Tool::Kubectl).state, InstallState::Idle);
        let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
        let first = inst.start_job(Tool::Kubectl, tx.clone(), async move {
            let _ = release_rx.await;
            Err("boom".to_string())
        });
        assert_eq!(first.state, InstallState::Running);
        let second = inst.start_job(Tool::Kubectl, tx.clone(), async { Ok(PathBuf::from("/x")) });
        assert_eq!(
            second.started_at, first.started_at,
            "second start must not restart the job"
        );
        assert_eq!(inst.job(Tool::Kubectl).state, InstallState::Running);
        let ev = rx.recv().await.unwrap();
        assert!(
            matches!(ev, Event::K8sInstallUpdated { tool, state } if tool == "kubectl" && state == "running")
        );

        let _ = release_tx.send(());
        let ev = rx.recv().await.unwrap();
        assert!(matches!(ev, Event::K8sInstallUpdated { state, .. } if state == "failed"));
        let job = inst.job(Tool::Kubectl);
        assert_eq!(job.state, InstallState::Failed);
        assert_eq!(job.error.as_deref(), Some("boom"));
        assert!(job.finished_at.is_some());
        assert!(job.log_tail.contains("failed: boom"));
        // A finished job can be retried.
        let third = inst.start_job(Tool::Kubectl, tx.clone(), async { Ok(PathBuf::from("/x")) });
        assert_eq!(third.state, InstallState::Running);
        assert_ne!(third.started_at, first.started_at);
        let _ = rx.recv().await;
        let ev = rx.recv().await.unwrap();
        assert!(matches!(ev, Event::K8sInstallUpdated { state, .. } if state == "done"));
        assert_eq!(inst.job(Tool::Kubectl).state, InstallState::Done);
        assert_eq!(serde_json::to_value(&first).unwrap()["tool"], "kubectl");
    }
}
