//! A new worktree that is ready to work in (T3 Code / Conductor's
//! `.worktreeinclude`): the untracked or ignored files the main checkout's
//! `.worktreeinclude` names (gitignore patterns, matched by git itself) are
//! COPIED into the new worktree (`.env`, local config; at most
//! [`COPY_CAP`] bytes in all), and the ones listed after a `[clone]` line
//! (dependency folders: `node_modules/`, `target/`) are CLONED copy-on-write
//! (`cp -c`, APFS clonefile: instant, no extra disk until changed) — or
//! symlinked when the volume cannot clone, which the result says. Nothing
//! outside the repo is read or written. Then the repo's optional setup
//! command (setting [`SETUP_KEY`], set from Threads) runs there on request.

use std::path::{Component, Path};
use std::time::Duration;

use otto_core::{Error, Result};
use serde::Serialize;

use crate::LocalGit;

/// The file in the main checkout's root that names what a worktree gets.
pub const INCLUDE_FILE: &str = ".worktreeinclude";
/// The most a worktree's copied (not cloned) files may add up to.
pub const COPY_CAP: u64 = 100 * 1024 * 1024;
/// Settings key: `{ "<main checkout path>": { "name": "...", "command": "..." } }`
/// (every worktree of a repository shares its main checkout's setup).
pub const SETUP_KEY: &str = "worktree_setup";
/// How long a setup command may run.
pub const SETUP_TIMEOUT: Duration = Duration::from_secs(600);
/// How often a running setup's latest output line is sent at most.
pub const PROGRESS_EVERY: Duration = Duration::from_secs(1);
/// How much of a setup command's output is kept (its end).
const OUTPUT_TAIL: usize = 4000;

/// What a new worktree got (`POST /repos/{id}/worktrees` `prepared`).
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct Prepared {
    /// Files copied.
    pub copied: u32,
    /// Paths cloned copy-on-write.
    pub cloned: Vec<String>,
    /// Paths symlinked to the main checkout (cloning was impossible).
    pub linked: Vec<String>,
    /// Paths left out, with why (`"big.bin: over the 100 MB copy limit"`).
    pub skipped: Vec<String>,
    /// The repo's setup command's name, when one is set (run it next).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub setup: Option<String>,
}

/// The copy and clone pattern lists of a `.worktreeinclude`: blank lines and
/// `#` comments skipped, `[clone]` / `[copy]` switch the list.
pub fn parse_include(text: &str) -> (Vec<String>, Vec<String>) {
    let (mut copy, mut clone) = (Vec::new(), Vec::new());
    let mut cloning = false;
    for line in text.lines().map(str::trim) {
        match line {
            "" => {}
            "[clone]" => cloning = true,
            "[copy]" => cloning = false,
            l if l.starts_with('#') => {}
            l => (if cloning { &mut clone } else { &mut copy }).push(l.to_string()),
        }
    }
    (copy, clone)
}

/// A path git listed, kept only when it stays inside the checkout.
fn inside(rel: &str) -> bool {
    let p = Path::new(rel.trim_end_matches('/'));
    !rel.is_empty() && p.components().all(|c| matches!(c, Component::Normal(_)))
}

impl LocalGit {
    /// The untracked / ignored paths of this checkout that `patterns` match
    /// (a whole matching folder as one `dir/`).
    async fn included(&self, patterns: &[String]) -> Vec<String> {
        if patterns.is_empty() {
            return Vec::new();
        }
        let mut args = vec![
            "ls-files",
            "--others",
            "--ignored",
            "--directory",
            "--no-empty-directory",
        ];
        let ex: Vec<String> = patterns.iter().map(|p| format!("--exclude={p}")).collect();
        args.extend(ex.iter().map(String::as_str));
        match self.run(&args).await {
            Ok(out) => out
                .lines()
                .filter(|l| inside(l))
                .map(str::to_string)
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Copy / clone what `.worktreeinclude` names into the worktree at `dest`.
    /// Best-effort: a path that fails is listed in `skipped`, never an error.
    pub async fn prepare_worktree(&self, dest: &Path) -> Prepared {
        let Ok(text) = tokio::fs::read_to_string(self.path().join(INCLUDE_FILE)).await else {
            return Prepared::default();
        };
        let (copy, clone) = parse_include(&text);
        let (copy, clone) = (self.included(&copy).await, self.included(&clone).await);
        let (src, dest) = (self.path().to_path_buf(), dest.to_path_buf());
        tokio::task::spawn_blocking(move || place(&src, &dest, &copy, &clone))
            .await
            .unwrap_or_default()
    }
}

fn place(src: &Path, dest: &Path, copy: &[String], clone: &[String]) -> Prepared {
    let mut out = Prepared::default();
    let mut budget = COPY_CAP;
    for rel in copy {
        let key = rel.trim_end_matches('/');
        if let Err(why) = copy_tree(
            &src.join(key),
            &dest.join(key),
            &mut budget,
            &mut out.copied,
        ) {
            out.skipped.push(format!("{key}: {why}"));
        }
    }
    for rel in clone {
        let key = rel.trim_end_matches('/');
        let (from, to) = (src.join(key), dest.join(key));
        if to.symlink_metadata().is_ok() || from.symlink_metadata().map_or(true, |m| m.is_symlink())
        {
            continue; // already there (tracked), or a link we do not follow
        }
        if let Some(parent) = to.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let cloned = std::process::Command::new("cp")
            .args(["-c", "-R"])
            .arg(&from)
            .arg(&to)
            .status()
            .is_ok_and(|s| s.success());
        if cloned {
            out.cloned.push(key.to_string());
        } else {
            let _ = std::fs::remove_dir_all(&to);
            match std::os::unix::fs::symlink(&from, &to) {
                Ok(()) => out.linked.push(key.to_string()),
                Err(e) => out.skipped.push(format!("{key}: {e}")),
            }
        }
    }
    out
}

/// Copy `from` (a file or a folder) to `to`, within `budget` bytes; links
/// are skipped. Err: why nothing more was copied.
fn copy_tree(
    from: &Path,
    to: &Path,
    budget: &mut u64,
    n: &mut u32,
) -> std::result::Result<(), String> {
    let meta = from.symlink_metadata().map_err(|e| e.to_string())?;
    if meta.is_symlink() || to.symlink_metadata().is_ok() {
        return Ok(());
    }
    if meta.is_dir() {
        std::fs::create_dir_all(to).map_err(|e| e.to_string())?;
        for entry in std::fs::read_dir(from)
            .map_err(|e| e.to_string())?
            .flatten()
        {
            copy_tree(&entry.path(), &to.join(entry.file_name()), budget, n)?;
        }
        return Ok(());
    }
    if meta.len() > *budget {
        return Err(format!(
            "over the {} MB copy limit",
            COPY_CAP / (1024 * 1024)
        ));
    }
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::copy(from, to).map_err(|e| e.to_string())?;
    *budget -= meta.len();
    *n += 1;
    Ok(())
}

/// How a setup command ended (`POST /repos/{id}/worktrees/setup`).
#[derive(Debug, Clone, Serialize)]
pub struct SetupResult {
    pub name: String,
    pub ok: bool,
    pub exit_code: Option<i32>,
    /// The end of what it printed (stdout and stderr).
    pub output: String,
}

/// Run `command` in `cwd` through the person's login shell, bounded by
/// [`SETUP_TIMEOUT`]; a timeout is a failure, never an error. `progress`
/// hears its latest output line as it runs: at once, then at most every
/// [`PROGRESS_EVERY`] (the newest line of a burst is sent when it settles).
pub async fn run_setup(
    name: &str,
    command: &str,
    cwd: &Path,
    progress: Option<tokio::sync::mpsc::UnboundedSender<String>>,
) -> Result<SetupResult> {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    let mut child = tokio::process::Command::new(shell)
        .arg("-lc")
        .arg(command)
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| Error::Internal(format!("couldn't start the setup: {e}")))?;
    // Both pipes, line by line, into one channel (closed when both end).
    let (tx, mut lines) = tokio::sync::mpsc::unbounded_channel::<String>();
    fn pump(
        r: impl tokio::io::AsyncRead + Unpin + Send + 'static,
        tx: tokio::sync::mpsc::UnboundedSender<String>,
    ) {
        tokio::spawn(async move {
            let mut r = BufReader::new(r).lines();
            while let Ok(Some(l)) = r.next_line().await {
                if tx.send(l).is_err() {
                    break;
                }
            }
        });
    }
    pump(child.stdout.take().expect("piped"), tx.clone());
    pump(child.stderr.take().expect("piped"), tx);
    let mut tail = String::new();
    let (mut pending, mut sent_at) = (None::<String>, None::<std::time::Instant>);
    let mut tick = tokio::time::interval(PROGRESS_EVERY);
    let deadline = tokio::time::sleep(SETUP_TIMEOUT);
    tokio::pin!(deadline);
    let mut timed_out = false;
    loop {
        tokio::select! {
            l = lines.recv() => {
                let Some(l) = l else { break };
                tail.push_str(&l);
                tail.push('\n');
                if tail.len() > 2 * OUTPUT_TAIL {
                    let cut = tail.len() - OUTPUT_TAIL;
                    let cut = (cut..tail.len()).find(|i| tail.is_char_boundary(*i)).unwrap_or(0);
                    tail.drain(..cut);
                }
                if !l.trim().is_empty() {
                    pending = Some(l.trim().to_string());
                }
                if sent_at.is_none_or(|t| t.elapsed() >= PROGRESS_EVERY) {
                    if let (Some(p), Some(tx)) = (pending.take(), &progress) {
                        let _ = tx.send(p);
                        sent_at = Some(std::time::Instant::now());
                    }
                }
            }
            _ = tick.tick() => {
                if let (Some(p), Some(tx)) = (pending.take(), &progress) {
                    let _ = tx.send(p);
                    sent_at = Some(std::time::Instant::now());
                }
            }
            _ = &mut deadline => {
                timed_out = true;
                let _ = child.kill().await;
                break;
            }
        }
    }
    let status = if timed_out {
        None
    } else {
        child.wait().await.ok()
    };
    if timed_out {
        tail.push_str(&format!(
            "timed out after {} min",
            SETUP_TIMEOUT.as_secs() / 60
        ));
    }
    let start = tail.len().saturating_sub(OUTPUT_TAIL);
    let start = (start..tail.len())
        .find(|i| tail.is_char_boundary(*i))
        .unwrap_or(tail.len());
    Ok(SetupResult {
        name: name.to_string(),
        ok: status.is_some_and(|s| s.success()),
        exit_code: status.and_then(|s| s.code()),
        output: tail[start..].trim().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_copy_and_clone_sections() {
        let (copy, clone) = parse_include(
            "# local config\n.env\n.env.*\n\n[clone]\nnode_modules/\n[copy]\nconfig/local.json\n",
        );
        assert_eq!(copy, [".env", ".env.*", "config/local.json"]);
        assert_eq!(clone, ["node_modules/"]);
    }

    #[test]
    fn never_leaves_the_checkout() {
        assert!(inside(".env"));
        assert!(inside("node_modules/"));
        assert!(!inside("../secret"));
        assert!(!inside("/etc/passwd"));
        assert!(!inside("a/../../b"));
        assert!(!inside(""));
    }

    #[test]
    fn copies_within_the_cap_and_skips_links() {
        let (src, dest) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        std::fs::write(src.path().join(".env"), "A=1").unwrap();
        std::fs::create_dir_all(src.path().join("conf")).unwrap();
        std::fs::write(src.path().join("conf/x.json"), "{}").unwrap();
        std::os::unix::fs::symlink("/etc/hosts", src.path().join("conf/link")).unwrap();
        std::fs::create_dir_all(src.path().join("node_modules/pkg")).unwrap();
        std::fs::write(src.path().join("node_modules/pkg/index.js"), "x").unwrap();
        let got = place(
            src.path(),
            dest.path(),
            &[".env".into(), "conf/".into()],
            &["node_modules/".into()],
        );
        assert_eq!(got.copied, 2);
        assert_eq!(
            std::fs::read_to_string(dest.path().join(".env")).unwrap(),
            "A=1"
        );
        assert!(
            dest.path().join("conf/link").symlink_metadata().is_err(),
            "a link is not followed"
        );
        assert_eq!(got.cloned.len() + got.linked.len(), 1);
        assert_eq!(
            std::fs::read_to_string(dest.path().join("node_modules/pkg/index.js")).unwrap(),
            "x"
        );
        let mut budget = 1;
        let mut n = 0;
        let err = copy_tree(
            &src.path().join(".env"),
            &dest.path().join("again.env"),
            &mut budget,
            &mut n,
        )
        .unwrap_err();
        assert!(err.contains("copy limit"), "{err}");
    }

    #[tokio::test]
    async fn setup_reports_failure_and_output() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let ok = run_setup("hi", "echo ready", dir.path(), Some(tx))
            .await
            .unwrap();
        assert_eq!(
            rx.recv().await.as_deref(),
            Some("ready"),
            "the latest line streams"
        );
        assert!(ok.ok);
        assert!(ok.output.contains("ready"));
        let bad = run_setup("bad", "echo nope >&2; exit 3", dir.path(), None)
            .await
            .unwrap();
        assert!(!bad.ok);
        assert_eq!(bad.exit_code, Some(3));
        assert!(bad.output.contains("nope"));
    }
}
