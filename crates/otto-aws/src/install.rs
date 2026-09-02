//! `aws` binary discovery + the on-demand installer job.
//!
//! `locate()` is the ladder from the contract (§1): `OTTO_AWS_BIN` override
//! (tests / power users) → `PATH` → `/opt/homebrew/bin`, `/usr/local/bin`,
//! `~/.local/bin`, `<data_dir>/bin`, `~/aws-cli` (where the pkg installer
//! lands). The installer is a background task whose state lives in a
//! process-wide `OnceLock` (one job per tool; the router is stateless and the
//! `AwsCtx` trait carries no service handle). It prefers `brew install awscli`
//! and falls back to the official `.pkg` installed into the user's home —
//! never `sudo`, never a write outside `$HOME` / `<data_dir>`.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use chrono::{DateTime, Utc};
use otto_core::event::Event;
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tokio::sync::broadcast;

/// The tool this crate installs (the K8s crate has its own `kubectl` / `k9s`).
pub const TOOL: &str = "aws";
/// Env override honoured by [`locate`] — absolute path to an `aws` binary.
pub const BIN_ENV: &str = "OTTO_AWS_BIN";
const PKG_URL: &str = "https://awscli.amazonaws.com/AWSCLIV2.pkg";
/// Keep the last N bytes of installer output for `/status`.
const LOG_TAIL_CAP: usize = 4096;
const STEP_TIMEOUT: Duration = Duration::from_secs(20 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InstallState {
    Idle,
    Running,
    Done,
    Failed,
}

impl InstallState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }
}

/// `InstallJob` DTO from the contract (§2.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallJob {
    pub tool: String,
    pub state: InstallState,
    pub log_tail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl InstallJob {
    fn idle() -> Self {
        Self {
            tool: TOOL.into(),
            state: InstallState::Idle,
            log_tail: String::new(),
            started_at: None,
            finished_at: None,
            error: None,
        }
    }
}

/// `GET /aws/status` response.
#[derive(Debug, Clone, Serialize)]
pub struct AwsStatus {
    pub installed: bool,
    pub version: Option<String>,
    pub path: Option<String>,
    pub install: InstallJob,
}

/// Find the `aws` binary. Returns `None` when nothing runnable is found.
pub fn locate(data_dir: &Path) -> Option<PathBuf> {
    if let Ok(p) = std::env::var(BIN_ENV) {
        let p = p.trim();
        if !p.is_empty() {
            let pb = PathBuf::from(p);
            // An explicit override is authoritative: never fall through.
            return pb.is_file().then_some(pb);
        }
    }
    locate_in_path("aws").or_else(|| {
        let mut candidates = vec![
            PathBuf::from("/opt/homebrew/bin/aws"),
            PathBuf::from("/usr/local/bin/aws"),
        ];
        if let Some(home) = dirs::home_dir() {
            candidates.push(home.join(".local/bin/aws"));
        }
        candidates.push(data_dir.join("bin/aws"));
        if let Some(home) = dirs::home_dir() {
            candidates.push(home.join("aws-cli/aws"));
        }
        candidates.into_iter().find(|p| p.is_file())
    })
}

/// Scan `$PATH` for an executable named `name` (what `which` does, without a
/// subprocess).
pub fn locate_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

/// `aws --version` → `"2.17.12"` (v2 prints `aws-cli/2.17.12 Python/… ` on
/// stdout; v1 on stderr — we read both).
pub async fn version(bin: &Path) -> Option<String> {
    let out = tokio::time::timeout(
        Duration::from_secs(10),
        Command::new(bin)
            .arg("--version")
            .env("AWS_PAGER", "")
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    parse_version(&text)
}

pub fn parse_version(text: &str) -> Option<String> {
    text.split_whitespace()
        .find_map(|tok| tok.strip_prefix("aws-cli/"))
        .map(|v| v.to_string())
}

/// Presence + version + install-job snapshot.
pub async fn status(data_dir: &Path) -> AwsStatus {
    let path = locate(data_dir);
    let version = match &path {
        Some(p) => version(p).await,
        None => None,
    };
    AwsStatus {
        installed: path.is_some(),
        version,
        path: path.map(|p| p.to_string_lossy().into_owned()),
        install: installer().snapshot(),
    }
}

/// Process-wide installer state for the `aws` tool.
pub struct Installer {
    job: Mutex<InstallJob>,
}

static INSTALLER: OnceLock<Installer> = OnceLock::new();

pub fn installer() -> &'static Installer {
    INSTALLER.get_or_init(|| Installer {
        job: Mutex::new(InstallJob::idle()),
    })
}

impl Installer {
    pub fn snapshot(&self) -> InstallJob {
        self.job.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    fn append_log(&self, line: &str) {
        let mut j = self.job.lock().unwrap_or_else(|p| p.into_inner());
        j.log_tail.push_str(line);
        if !line.ends_with('\n') {
            j.log_tail.push('\n');
        }
        if j.log_tail.len() > LOG_TAIL_CAP {
            let cut = j.log_tail.len() - LOG_TAIL_CAP;
            // Cut on a char boundary.
            let mut idx = cut;
            while !j.log_tail.is_char_boundary(idx) {
                idx += 1;
            }
            j.log_tail.drain(..idx);
        }
    }

    fn set_state(
        &self,
        state: InstallState,
        error: Option<String>,
        events: &broadcast::Sender<Event>,
    ) {
        {
            let mut j = self.job.lock().unwrap_or_else(|p| p.into_inner());
            j.state = state;
            match state {
                InstallState::Running => {
                    j.started_at = Some(Utc::now());
                    j.finished_at = None;
                    j.error = None;
                    j.log_tail.clear();
                }
                InstallState::Done | InstallState::Failed => {
                    j.finished_at = Some(Utc::now());
                    j.error = error;
                }
                InstallState::Idle => {}
            }
        }
        let _ = events.send(Event::AwsInstallUpdated {
            tool: TOOL.into(),
            state: state.as_str().into(),
        });
    }

    /// Kick off the install unless one is already running (idempotent: the
    /// running job's snapshot is returned as-is). If the binary is already
    /// present the job completes immediately as `done`.
    pub fn start(&'static self, data_dir: PathBuf, events: broadcast::Sender<Event>) -> InstallJob {
        {
            let j = self.job.lock().unwrap_or_else(|p| p.into_inner());
            if j.state == InstallState::Running {
                return j.clone();
            }
        }
        self.set_state(InstallState::Running, None, &events);
        tokio::spawn(async move {
            let res = self.install(&data_dir).await;
            match res {
                Ok(path) => {
                    self.append_log(&format!("installed: {}", path.display()));
                    self.set_state(InstallState::Done, None, &events);
                }
                Err(e) => {
                    self.append_log(&format!("error: {e}"));
                    self.set_state(InstallState::Failed, Some(e), &events);
                }
            }
        });
        self.snapshot()
    }

    async fn install(&'static self, data_dir: &Path) -> std::result::Result<PathBuf, String> {
        if let Some(p) = locate(data_dir) {
            self.append_log(&format!("aws already present at {}", p.display()));
            return Ok(p);
        }
        if let Some(brew) = locate_in_path("brew")
            .or_else(|| Some(PathBuf::from("/opt/homebrew/bin/brew")).filter(|p| p.is_file()))
            .or_else(|| Some(PathBuf::from("/usr/local/bin/brew")).filter(|p| p.is_file()))
        {
            self.append_log(&format!("$ {} install awscli", brew.display()));
            self.step(&brew, &["install", "awscli"], None).await?;
        } else {
            self.install_pkg(data_dir).await?;
        }
        let bin = locate(data_dir).ok_or_else(|| {
            "install finished but no `aws` binary was found on the ladder".to_string()
        })?;
        match version(&bin).await {
            Some(v) => {
                self.append_log(&format!("aws-cli/{v} at {}", bin.display()));
                Ok(bin)
            }
            None => Err(format!(
                "{} does not run (`aws --version` failed)",
                bin.display()
            )),
        }
    }

    /// Fallback: official pkg into the user's home (`~/aws-cli`), then symlink
    /// into `<data_dir>/bin` which is on the daemon PATH.
    async fn install_pkg(&'static self, data_dir: &Path) -> std::result::Result<(), String> {
        let home = dirs::home_dir().ok_or("no home directory")?;
        let tmp = data_dir.join("tmp").join("aws-install");
        std::fs::create_dir_all(&tmp).map_err(|e| format!("create {}: {e}", tmp.display()))?;
        let pkg = tmp.join("AWSCLIV2.pkg");
        self.append_log(&format!("$ curl -fsSL -o {} {PKG_URL}", pkg.display()));
        self.step(
            Path::new("curl"),
            &["-fsSL", "-o", &pkg.to_string_lossy(), PKG_URL],
            None,
        )
        .await?;
        self.append_log("$ installer -pkg AWSCLIV2.pkg -target CurrentUserHomeDirectory");
        self.step(
            Path::new("installer"),
            &[
                "-pkg",
                &pkg.to_string_lossy(),
                "-target",
                "CurrentUserHomeDirectory",
            ],
            None,
        )
        .await?;
        let _ = std::fs::remove_file(&pkg);
        let bin_dir = data_dir.join("bin");
        std::fs::create_dir_all(&bin_dir)
            .map_err(|e| format!("create {}: {e}", bin_dir.display()))?;
        for name in ["aws", "aws_completer"] {
            let target = home.join("aws-cli").join(name);
            if !target.is_file() {
                return Err(format!(
                    "pkg installer did not produce {}",
                    target.display()
                ));
            }
            let link = bin_dir.join(name);
            let _ = std::fs::remove_file(&link);
            std::os::unix::fs::symlink(&target, &link)
                .map_err(|e| format!("symlink {}: {e}", link.display()))?;
            self.append_log(&format!("{} -> {}", link.display(), target.display()));
        }
        Ok(())
    }

    /// Run one installer step, streaming its combined output into the log.
    async fn step(
        &'static self,
        program: &Path,
        args: &[&str],
        cwd: Option<&Path>,
    ) -> std::result::Result<(), String> {
        let mut cmd = Command::new(program);
        cmd.args(args)
            .env("HOMEBREW_NO_AUTO_UPDATE", "1")
            .env("NONINTERACTIVE", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(c) = cwd {
            cmd.current_dir(c);
        }
        let out = tokio::time::timeout(STEP_TIMEOUT, cmd.output())
            .await
            .map_err(|_| format!("{} timed out", program.display()))?
            .map_err(|e| format!("run {}: {e}", program.display()))?;
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        if !text.trim().is_empty() {
            self.append_log(text.trim_end());
        }
        if out.status.success() {
            Ok(())
        } else {
            Err(format!(
                "{} exited with {}",
                program.display(),
                out.status.code().unwrap_or(-1)
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_v2_and_v1_version_strings() {
        assert_eq!(
            parse_version("aws-cli/2.17.12 Python/3.11.9 Darwin/24.0.0 exe/x86_64\n"),
            Some("2.17.12".into())
        );
        assert_eq!(
            parse_version("aws-cli/1.32.0 Python/3.9.6 Darwin/23.0.0 botocore/1.34.0"),
            Some("1.32.0".into())
        );
        assert_eq!(parse_version("command not found"), None);
    }

    #[test]
    fn install_job_serializes_per_contract() {
        let j = InstallJob::idle();
        let v = serde_json::to_value(&j).unwrap();
        assert_eq!(v["tool"], "aws");
        assert_eq!(v["state"], "idle");
        assert_eq!(v["log_tail"], "");
        assert!(v.get("started_at").is_none());
        assert!(v.get("error").is_none());
    }

    #[test]
    fn log_tail_is_capped() {
        let inst = Installer {
            job: Mutex::new(InstallJob::idle()),
        };
        for _ in 0..200 {
            inst.append_log(&"x".repeat(100));
        }
        assert!(inst.snapshot().log_tail.len() <= LOG_TAIL_CAP);
    }
}
