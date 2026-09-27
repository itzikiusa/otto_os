//! Bounded child I/O; no shell interpolation or inherited Otto/API credentials.
use otto_core::{Error, Result};
use std::{path::Path, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::Command,
};

pub(super) const OUTPUT_LIMIT: usize = 256 * 1024;
async fn read_bounded(reader: impl AsyncRead + Unpin) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take((OUTPUT_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .await
        .map_err(|e| Error::Upstream(format!("Recognition process output: {e}")))?;
    if bytes.len() > OUTPUT_LIMIT {
        return Err(Error::Upstream(
            "Recognition process output exceeded its limit".into(),
        ));
    }
    Ok(bytes)
}
pub(super) async fn run_capture(
    program: &str,
    args: &[String],
    input: &[u8],
    cwd: &Path,
    timeout: Duration,
) -> Result<(Vec<u8>, Vec<u8>)> {
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(cwd)
        .kill_on_drop(true)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Auth remains under Codex's home/keychain. Arbitrary provider/API tokens,
    // Otto capability tokens and repository instructions are never inherited.
    for key in ["HOME", "PATH", "TMPDIR", "CODEX_HOME", "USER", "LANG"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    let mut child = command.spawn().map_err(|e| {
        Error::Upstream(format!(
            "Could not start local recognition/summary program: {e}"
        ))
    })?;
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let mut stdin = child.stdin.take().expect("piped stdin");
    let result = tokio::time::timeout(timeout, async {
        let write = async {
            stdin.write_all(input).await.map_err(|e| Error::Upstream(format!("Could not provide summary input: {e}")))?;
            drop(stdin); Ok::<_, Error>(())
        };
        let wait = async { child.wait().await.map_err(|e| Error::Upstream(format!("Local process wait failed: {e}"))) };
        let (out, err, (), status) = tokio::try_join!(read_bounded(stdout),read_bounded(stderr),write,wait)?;
        // Do not reflect stderr: it may quote private transcript or auth details.
        if !status.success() { return Err(Error::Upstream(format!("Local recognition/summary process failed ({status}); check installation, model and Codex sign-in"))); }
        Ok((out,err))
    }).await;
    match result {
        Ok(result) => result,
        Err(_) => Err(Error::Upstream(
            "Local recognition/summary timed out; the transcript remains saved".into(),
        )),
    }
}
pub(super) async fn run(
    program: &str,
    args: &[String],
    input: &[u8],
    cwd: &Path,
    timeout: Duration,
) -> Result<Vec<u8>> {
    run_capture(program, args, input, cwd, timeout)
        .await
        .map(|v| v.0)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn bounded_reader_rejects_overflow() {
        assert!(read_bounded(&vec![0u8; OUTPUT_LIMIT + 1][..])
            .await
            .is_err());
        assert_eq!(read_bounded(&b"ok"[..]).await.unwrap(), b"ok");
    }
    #[tokio::test]
    async fn child_timeout_and_failure_are_errors() {
        let cwd = tempfile::tempdir().unwrap();
        assert!(run(
            "/bin/sleep",
            &["2".into()],
            b"",
            cwd.path(),
            Duration::from_millis(10)
        )
        .await
        .is_err());
        assert!(run(
            "/usr/bin/false",
            &[],
            b"",
            cwd.path(),
            Duration::from_secs(2)
        )
        .await
        .is_err());
        assert_eq!(
            run(
                "/bin/cat",
                &[],
                b"a literal $(command)",
                cwd.path(),
                Duration::from_secs(2)
            )
            .await
            .unwrap(),
            b"a literal $(command)"
        );
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn cancelling_recognition_kills_its_process() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("wait.sh");
        let pid_file = dir.path().join("pid");
        std::fs::write(
            &script,
            b"#!/bin/sh\nprintf '%s' \"$$\" > \"$1\"\nexec /bin/sleep 30\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let cwd = dir.path().to_owned();
        let pid_arg = pid_file.display().to_string();
        let task = tokio::spawn(async move {
            run(
                script.to_str().unwrap(),
                &[pid_arg],
                b"",
                &cwd,
                Duration::from_secs(60),
            )
            .await
        });
        for _ in 0..100 {
            if pid_file.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let pid = std::fs::read_to_string(&pid_file).expect("test child started");
        task.abort();
        let _ = task.await;
        let alive = || {
            std::process::Command::new("/bin/kill")
                .args(["-0", &pid])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success()
        };
        for _ in 0..100 {
            if !alive() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let _ = std::process::Command::new("/bin/kill")
            .args(["-KILL", &pid])
            .status();
        panic!("cancelled recognition child remained alive");
    }
}
