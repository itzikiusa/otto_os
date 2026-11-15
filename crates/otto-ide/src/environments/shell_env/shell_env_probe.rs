//! Subprocess probe runner and process-group guard for [`super::ShellEnvResolver`].

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::Ordering;
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::process::Command;

use super::{
    find_subslice, parse_sentinel_env_zero, LOGIN_FALLBACK_TIMEOUT, MAX_SHELL_STDOUT_BYTES,
    PROBE_TOKEN_SEQ,
};

/// Capture environment variables from `shell_bin` using `-lic` (login + interactive)
/// with fallback to `-lc` (login only) if interactive mode times out or errors.
pub(crate) async fn capture_shell_env_with_shell(
    shell_bin: &str,
    interactive_timeout: Duration,
) -> std::result::Result<BTreeMap<String, String>, String> {
    match run_shell_env_probe(shell_bin, "-lic", interactive_timeout).await {
        Ok(vars) if !vars.is_empty() => Ok(vars),
        Ok(_) | Err(_) => run_shell_env_probe(shell_bin, "-lc", LOGIN_FALLBACK_TIMEOUT).await,
    }
}

pub(crate) async fn run_shell_env_probe(
    shell_bin: &str,
    mode_flag: &str,
    timeout_dur: Duration,
) -> std::result::Result<BTreeMap<String, String>, String> {
    let token = PROBE_TOKEN_SEQ.fetch_add(1, Ordering::Relaxed);
    let start_marker = format!("__OTTO_SHELL_ENV_BEGIN_{}_{}__", std::process::id(), token);
    let end_marker = format!("__OTTO_SHELL_ENV_END_{}_{}__", std::process::id(), token);
    let env_bin = if Path::new("/usr/bin/env").is_file() {
        "/usr/bin/env"
    } else {
        "env"
    };
    let script = format!("printf '%s' '{start_marker}'; {env_bin} -0; printf '%s' '{end_marker}'");

    let mut cmd = Command::new(shell_bin);
    cmd.args([mode_flag, &script])
        .env("TERM", "dumb")
        .env("OTTO_SHELL_ENV_PROBE", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);

    if let Some(home) = dirs::home_dir() {
        if home.is_dir() {
            cmd.current_dir(&home);
        }
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("spawn '{shell_bin} {mode_flag}': {e}"))?;
    let child_pid = child.id();
    let mut guard = ShellProbeGuard {
        pid: child_pid,
        disarmed: false,
    };

    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| "missing stdout pipe from shell probe".to_string())?;

    let read_fut = async {
        let mut buf = Vec::with_capacity(8192);
        let mut chunk = [0u8; 4096];
        loop {
            let n = stdout
                .read(&mut chunk)
                .await
                .map_err(|e| format!("read shell probe stdout: {e}"))?;
            if n == 0 {
                break;
            }
            let remaining = MAX_SHELL_STDOUT_BYTES.saturating_sub(buf.len());
            let take = n.min(remaining);
            buf.extend_from_slice(&chunk[..take]);
            if buf.len() >= MAX_SHELL_STDOUT_BYTES {
                break;
            }
            if find_subslice(&buf, end_marker.as_bytes()).is_some() {
                break;
            }
        }
        Ok::<Vec<u8>, String>(buf)
    };

    let raw_bytes = match tokio::time::timeout(timeout_dur, read_fut).await {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(err)) => {
            terminate_shell_process_group(&mut child, child_pid).await;
            guard.disarmed = true;
            return Err(err);
        }
        Err(_) => {
            terminate_shell_process_group(&mut child, child_pid).await;
            guard.disarmed = true;
            return Err(format!(
                "shell environment probe '{shell_bin} {mode_flag}' timed out after {}ms",
                timeout_dur.as_millis()
            ));
        }
    };

    terminate_shell_process_group(&mut child, child_pid).await;
    guard.disarmed = true;

    parse_sentinel_env_zero(&raw_bytes, &start_marker, &end_marker)
}

struct ShellProbeGuard {
    pid: Option<u32>,
    disarmed: bool,
}

impl Drop for ShellProbeGuard {
    #[allow(clippy::disallowed_methods)] // synchronous Drop safeguard when task is cancelled
    fn drop(&mut self) {
        if !self.disarmed {
            #[cfg(unix)]
            if let Some(pid) = self.pid {
                let _ = std::process::Command::new("/bin/kill")
                    .args(["-KILL", &format!("-{pid}")])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
        }
    }
}

async fn terminate_shell_process_group(child: &mut tokio::process::Child, pid: Option<u32>) {
    if tokio::time::timeout(Duration::from_millis(40), child.wait())
        .await
        .is_ok()
    {
        return;
    }
    #[cfg(unix)]
    if let Some(p) = pid {
        let _ = Command::new("/bin/kill")
            .args(["-KILL", &format!("-{p}")])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
    }
    let _ = child.kill().await;
    let _ = tokio::time::timeout(Duration::from_millis(100), child.wait()).await;
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::environments::shell_env::collect_safe_non_root_base_env;

    #[test]
    fn sentinel_parser_ignores_zshrc_motd_banners_ansi_and_multiline_values() {
        let start = "__OTTO_START_1__";
        let end = "__OTTO_END_1__";
        let mut raw = Vec::new();
        raw.extend_from_slice(b"\x1b[32mWelcome to Oh My Zsh!\x1b[0m\nFAKE_VAR=corrupted\n");
        raw.extend_from_slice(start.as_bytes());
        raw.extend_from_slice(b"GO_DEBUGGING_PRODAMS=host=127.0.0.1;env=qa\0");
        raw.extend_from_slice(b"GOPRIVATE=bitbucket.org/gamescale-rnd/*\0");
        raw.extend_from_slice(b"MULTILINE_CERT=-----BEGIN-----\nline2\n-----END-----\0");
        raw.extend_from_slice(b"PWD=/should/be/filtered\0");
        raw.extend_from_slice(b"SHLVL=3\0");
        raw.extend_from_slice(b"PATH=/Users/dev/go/bin:/opt/homebrew/bin:/usr/bin\0");
        raw.extend_from_slice(end.as_bytes());
        raw.extend_from_slice(b"\nGoodbye from .zlogout!\nTRAILING=ignored\0");

        let parsed = parse_sentinel_env_zero(&raw, start, end).unwrap();
        assert_eq!(
            parsed.get("GO_DEBUGGING_PRODAMS").map(String::as_str),
            Some("host=127.0.0.1;env=qa")
        );
        assert_eq!(
            parsed.get("GOPRIVATE").map(String::as_str),
            Some("bitbucket.org/gamescale-rnd/*")
        );
        assert_eq!(
            parsed.get("MULTILINE_CERT").map(String::as_str),
            Some("-----BEGIN-----\nline2\n-----END-----")
        );
        assert_eq!(
            parsed.get("PATH").map(String::as_str),
            Some("/Users/dev/go/bin:/opt/homebrew/bin:/usr/bin")
        );
        assert!(!parsed.contains_key("FAKE_VAR"));
        assert!(!parsed.contains_key("TRAILING"));
        assert!(!parsed.contains_key("PWD"));
        assert!(!parsed.contains_key("SHLVL"));
    }

    #[test]
    fn safe_non_root_base_env_only_retains_allowlisted_keys() {
        let base = collect_safe_non_root_base_env();
        for key in base.keys() {
            assert!(matches!(
                key.as_str(),
                "PATH"
                    | "HOME"
                    | "USER"
                    | "LOGNAME"
                    | "TMPDIR"
                    | "LANG"
                    | "LC_ALL"
                    | "LC_CTYPE"
                    | "TERM"
            ));
        }
    }

    #[tokio::test]
    async fn shell_probe_captures_exported_vars_and_kills_hanging_shell_within_timeout() {
        let tmp = tempfile::tempdir().unwrap();
        let noisy_shell = tmp.path().join("mock_zsh.sh");
        std::fs::write(
            &noisy_shell,
            "#!/bin/sh\n\
             echo 'MOTD: loading custom zsh profile...'\n\
             export GO_DEBUGGING_PRODAMS='enabled-from-zshrc'\n\
             export GOPRIVATE='github.com/acme/*'\n\
             exec /bin/sh -c \"$2\"\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&noisy_shell, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let vars = run_shell_env_probe(
            noisy_shell.to_str().unwrap(),
            "-lic",
            Duration::from_secs(2),
        )
        .await
        .unwrap();
        assert_eq!(
            vars.get("GO_DEBUGGING_PRODAMS").map(String::as_str),
            Some("enabled-from-zshrc")
        );
        assert_eq!(
            vars.get("GOPRIVATE").map(String::as_str),
            Some("github.com/acme/*")
        );

        let hanging_shell = tmp.path().join("hanging_zsh.sh");
        std::fs::write(&hanging_shell, "#!/bin/sh\nsleep 30 &\nwait\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&hanging_shell, std::fs::Permissions::from_mode(0o755))
                .unwrap();
        }

        let started = Instant::now();
        let res = run_shell_env_probe(
            hanging_shell.to_str().unwrap(),
            "-lic",
            Duration::from_millis(200),
        )
        .await;
        assert!(res.is_err());
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
