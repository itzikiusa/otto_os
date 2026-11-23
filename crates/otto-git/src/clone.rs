//! Hardened `git clone` runner split out of `local.rs` (LOC ratchet);
//! re-exported from `crate::local` and crate root so existing paths keep working.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use otto_core::{Error, Result};
use otto_git_cmd::HARDENED_GIT_CONFIG;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

use crate::local::{env_secs, kill_group, AskPass, GIT_STDERR_CAP};
use crate::remote_url::validate_remote_url;
use crate::transport::TransportCredential;

/// Maximum bytes buffered for a single unterminated stderr progress line.
const CLONE_LINE_CAP: usize = 8 * 1024;

#[derive(Debug)]
enum CloneReadErr {
    Io(std::io::Error),
    Stalled(String),
}

fn build_clone_cmd(url: &str, dest: &Path, askpass: Option<&AskPass>) -> Command {
    let mut cmd = Command::new("git");
    cmd.arg("clone")
        .arg("--progress")
        // `--` so a URL can never be read as an option, and `LC_ALL=C` so the
        // progress/error lines parsed below stay English.
        .arg("--")
        .arg(url)
        .arg(dest)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_PARAMETERS", HARDENED_GIT_CONFIG)
        .env("GIT_PAGER", "cat")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        // Own process group (git + its `git-remote-https`/`ssh` helpers) so a
        // stall can be killed as a whole; never outlive the caller either.
        .process_group(0)
        .kill_on_drop(true);
    if let Some(a) = askpass {
        for (k, v) in a.envs() {
            cmd.env(k, v);
        }
    }
    cmd
}

async fn read_clone_stderr_capped<R: AsyncRead + Unpin>(
    mut stderr: R,
    idle: Duration,
    line_cap: usize,
    total_cap: usize,
    progress: &mut (impl FnMut(String) + Send),
) -> std::result::Result<String, CloneReadErr> {
    // git progress lines are \r-terminated; split on both \r and \n.
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let mut last_line = String::new();
    let mut total_seen: usize = 0;
    let mut skipping_line = false;

    loop {
        let n = match tokio::time::timeout(idle, stderr.read(&mut chunk)).await {
            Ok(Ok(n)) => n,
            Ok(Err(e)) => return Err(CloneReadErr::Io(e)),
            Err(_) => return Err(CloneReadErr::Stalled(last_line)),
        };
        if n == 0 {
            break;
        }
        // Keep draining past `total_cap` so git never blocks on a full stderr
        // pipe, without buffering or emitting unbounded output.
        if total_seen >= total_cap {
            continue;
        }
        let take = n.min(total_cap - total_seen);
        total_seen += take;

        let mut slice = &chunk[..take];
        while !slice.is_empty() {
            if let Some(pos) = slice.iter().position(|&b| b == b'\n' || b == b'\r') {
                if !skipping_line {
                    let room = line_cap.saturating_sub(buf.len());
                    buf.extend_from_slice(&slice[..pos.min(room)]);
                    let text = String::from_utf8_lossy(&buf).trim().to_string();
                    if !text.is_empty() {
                        last_line = text.clone();
                        progress(text);
                    }
                    buf.clear();
                } else {
                    skipping_line = false;
                }
                slice = &slice[pos + 1..];
            } else {
                if !skipping_line {
                    let room = line_cap.saturating_sub(buf.len());
                    let copy = slice.len().min(room);
                    buf.extend_from_slice(&slice[..copy]);
                    if slice.len() > room {
                        // Flush the capped prefix and discard the rest of this
                        // overly long line until the next delimiter.
                        let text = String::from_utf8_lossy(&buf).trim().to_string();
                        if !text.is_empty() {
                            last_line = text.clone();
                            progress(text);
                        }
                        buf.clear();
                        skipping_line = true;
                    }
                }
                break;
            }
        }
    }
    if !buf.is_empty() && !skipping_line {
        let text = String::from_utf8_lossy(&buf).trim().to_string();
        if !text.is_empty() {
            last_line = text.clone();
            progress(text);
        }
    }
    Ok(last_line)
}

/// Clone `url` into `dest`, streaming progress lines (from git's stderr) into
/// `progress`. Token is used via askpass for https remotes.
pub async fn clone_repo(
    url: &str,
    dest: &Path,
    token: Option<&TransportCredential>,
    mut progress: impl FnMut(String) + Send,
) -> Result<()> {
    validate_remote_url(url)?;
    let askpass = match token {
        Some(t) => Some(AskPass::new(t)?),
        None => None,
    };
    let mut cmd = build_clone_cmd(url, dest, askpass.as_ref());
    let mut child = cmd
        .spawn()
        .map_err(|e| Error::Internal(format!("spawn git clone: {e}")))?;

    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| Error::Internal("clone stderr unavailable".into()))?;

    // A clone may legitimately take long, but not SILENTLY: `--progress`
    // reports every second or so. No output for this long (a dead VPN, a
    // server that stopped sending) ends the clone instead of hanging the
    // clone task forever — the one git spawn here that had no bound.
    let idle = Duration::from_secs(env_secs("OTTO_GIT_CLONE_IDLE_SECS", 300));
    let last_line =
        match read_clone_stderr_capped(stderr, idle, CLONE_LINE_CAP, GIT_STDERR_CAP, &mut progress)
            .await
        {
            Ok(line) => line,
            Err(CloneReadErr::Io(e)) => return Err(Error::Internal(format!("clone read: {e}"))),
            Err(CloneReadErr::Stalled(last)) => {
                if let Some(pid) = child.id() {
                    kill_group(pid as libc::pid_t).await;
                }
                let _ = child.wait().await;
                return Err(Error::Upstream(format!(
                    "git clone stalled: no progress for {}s (last: {last})",
                    idle.as_secs()
                )));
            }
        };

    let status = child
        .wait()
        .await
        .map_err(|e| Error::Internal(format!("clone wait: {e}")))?;
    if !status.success() {
        return Err(Error::Upstream(format!(
            "git clone exited {}: {}",
            status.code().map_or_else(|| "?".into(), |c| c.to_string()),
            last_line
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clone_command_sets_hardened_git_config() {
        let cmd = build_clone_cmd(
            "https://example.com/org/repo.git",
            Path::new("/tmp/r"),
            None,
        );
        let env_of = |key: &str| {
            cmd.as_std()
                .get_envs()
                .find(|(k, _)| *k == key)
                .and_then(|(_, v)| v.map(|v| v.to_string_lossy().into_owned()))
        };
        assert_eq!(
            env_of("GIT_CONFIG_PARAMETERS").as_deref(),
            Some(HARDENED_GIT_CONFIG)
        );
        assert_eq!(env_of("GIT_TERMINAL_PROMPT").as_deref(), Some("0"));
        assert_eq!(env_of("GIT_PAGER").as_deref(), Some("cat"));
    }

    #[tokio::test]
    async fn clone_stderr_reader_caps_unterminated_lines_and_total_bytes() {
        // 200-byte single line + another line, with line_cap=32 and total_cap=256.
        let mut input = vec![b'a'; 200];
        input.push(b'\n');
        input.extend_from_slice(b"final-line\n");
        input.extend_from_slice(&vec![b'z'; 1024]);

        let mut lines = Vec::new();
        let last = read_clone_stderr_capped(
            input.as_slice(),
            Duration::from_secs(5),
            32,
            256,
            &mut |l| lines.push(l),
        )
        .await
        .unwrap();

        assert!(
            lines.iter().all(|l| l.len() <= 32),
            "every line must be capped at 32 bytes: {lines:?}"
        );
        assert_eq!(lines[0], "a".repeat(32));
        assert_eq!(lines[1], "final-line");
        // Trailing bytes past total_cap=256 are drained without unbounded emission.
        assert!(lines.len() <= 3, "unexpected extra lines: {lines:?}");
        assert!(!last.is_empty());
    }
}
