use std::path::Path;

use otto_core::{Error, Result};
use tokio_util::sync::CancellationToken;

use crate::{ScanMetrics, ScanSettings};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NetworkScope {
    Offline,
    LoopbackPort(u16),
    TemplateUpdate,
}

#[cfg(target_os = "macos")]
#[path = "governor/macos.rs"]
mod platform;

#[cfg(target_os = "macos")]
pub(crate) use platform::{run, run_with_network, run_with_network_lines};

#[cfg(not(target_os = "macos"))]
pub(crate) async fn run(
    program: &Path,
    args: &[String],
    project: &Path,
    cache: &Path,
    output: &Path,
    settings: &ScanSettings,
    cancel: &CancellationToken,
) -> Result<ScanMetrics> {
    run_with_network(
        program,
        args,
        project,
        cache,
        output,
        settings,
        NetworkScope::Offline,
        cancel,
    )
    .await
}

#[cfg(not(target_os = "macos"))]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_with_network(
    _program: &Path,
    _args: &[String],
    _project: &Path,
    _cache: &Path,
    _output: &Path,
    _settings: &ScanSettings,
    _network: NetworkScope,
    _cancel: &CancellationToken,
) -> Result<ScanMetrics> {
    Err(Error::Invalid(
        "Security scans require macOS Seatbelt.".into(),
    ))
}

#[cfg(not(target_os = "macos"))]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_with_network_lines<F>(
    _program: &Path,
    _args: &[String],
    _project: &Path,
    _cache: &Path,
    _output: &Path,
    _settings: &ScanSettings,
    _network: NetworkScope,
    _cancel: &CancellationToken,
    _on_line: F,
) -> Result<ScanMetrics>
where
    F: FnMut(&str) -> Result<()> + Send + 'static,
{
    Err(Error::Invalid(
        "Security scans require macOS Seatbelt.".into(),
    ))
}

pub(crate) fn cancelled() -> Error {
    Error::Conflict("The scan was cancelled.".into())
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn confines_engine_writes_and_kills_the_whole_group_on_cancel() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let project = root.join("project");
        let cache = root.join("cache");
        let output = root.join("output");
        for path in [&project, &cache, &output] {
            std::fs::create_dir(path).unwrap();
        }
        let outside = root.join("outside.txt");
        std::fs::write(&outside, "outside project data").unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let marker = output.join("pid");
        let forbidden = project.join("forbidden");
        let args = vec![
            "-c".into(),
            format!(
                "if touch '{}' || cat '{}' || nc -z 127.0.0.1 {port}; then exit 10; fi; \
                 sleep 60 & echo $! > '{}'; wait",
                forbidden.display(),
                outside.display(),
                marker.display(),
            ),
        ];
        let token = CancellationToken::new();
        let settings = ScanSettings::default();
        let scan = run(
            Path::new("/bin/sh"),
            &args,
            &project,
            &cache,
            &output,
            &settings,
            &token,
        );
        let stop = async {
            let pid = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    if let Ok(text) = tokio::fs::read_to_string(&marker).await {
                        if let Ok(pid) = text.trim().parse::<libc::pid_t>() {
                            break pid;
                        }
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                }
            })
            .await
            .expect("sandboxed child never reached the marker");
            token.cancel();
            pid
        };
        let (result, pid) = tokio::join!(scan, stop);
        assert_eq!(result.unwrap_err().to_string(), cancelled().to_string());
        assert!(!forbidden.exists());
        for _ in 0..100 {
            if unsafe { libc::kill(pid, 0) } == -1 {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        panic!("engine descendant survived cancellation");
    }

    #[tokio::test]
    async fn dast_sandbox_allows_only_proxy_port_blocks_writes_and_kills_group_on_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let project = root.join("project");
        let templates = root.join("templates");
        let output = root.join("output");
        for path in [&project, &templates, &output] {
            std::fs::create_dir(path).unwrap();
        }
        let allowed_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let allowed_port = allowed_listener.local_addr().unwrap().port();
        let blocked_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let blocked_port = blocked_listener.local_addr().unwrap().port();
        let accept_allowed = tokio::spawn(async move { allowed_listener.accept().await.is_ok() });

        let marker = output.join("pid");
        let args = vec![
            "-c".into(),
            format!(
                "if touch '{}/bad' || touch '{}/bad' || nc -z 127.0.0.1 {blocked_port}; then exit 10; fi; \
                 nc -z 127.0.0.1 {allowed_port} || exit 11; \
                 sleep 60 & echo $! > '{}'; wait",
                project.display(),
                templates.display(),
                marker.display(),
            ),
        ];
        let token = CancellationToken::new();
        let settings = ScanSettings {
            timeout_secs: 2,
            ..Default::default()
        };
        let result = run_with_network(
            Path::new("/bin/sh"),
            &args,
            &project,
            &templates,
            &output,
            &settings,
            NetworkScope::LoopbackPort(allowed_port),
            &token,
        )
        .await;
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("reached the scan time limit"),
            "expected timeout kill"
        );
        assert!(accept_allowed.await.unwrap());
        assert!(!project.join("bad").exists());
        assert!(!templates.join("bad").exists());
        let pid: libc::pid_t = tokio::fs::read_to_string(&marker)
            .await
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        for _ in 0..100 {
            if unsafe { libc::kill(pid, 0) } == -1 {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        panic!("DAST engine descendant survived timeout");
    }

    #[tokio::test]
    async fn piped_stdout_streams_lines_and_kills_group_when_output_exceeds_limit() {
        use std::sync::{Arc, Mutex};

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let project = root.join("project");
        let templates = root.join("templates");
        let output = root.join("output");
        for path in [&project, &templates, &output] {
            std::fs::create_dir(path).unwrap();
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let token = CancellationToken::new();
        let settings = ScanSettings {
            timeout_secs: 10,
            ..Default::default()
        };

        // Streams lines over the anonymous stdout pipe without creating any report files.
        let collected = Arc::new(Mutex::new(Vec::<String>::new()));
        let sink = collected.clone();
        let args = vec!["-c".into(), "printf 'line-one\\nline-two\\n'".into()];
        run_with_network_lines(
            Path::new("/bin/sh"),
            &args,
            &project,
            &templates,
            &output,
            &settings,
            NetworkScope::LoopbackPort(port),
            &token,
            move |line| {
                sink.lock().unwrap().push(line.to_string());
                Ok(())
            },
        )
        .await
        .unwrap();
        assert_eq!(*collected.lock().unwrap(), vec!["line-one", "line-two"]);
        assert!(tokio::fs::read_dir(&output)
            .await
            .unwrap()
            .next_entry()
            .await
            .unwrap()
            .is_none());

        // A single line exceeding MAX_STDOUT_LINE_BYTES fails closed and kills the group.
        let marker = output.join("pid");
        let oversize_args = vec![
            "-c".into(),
            format!(
                "sleep 60 & echo $! > '{}'; \
                 s='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'; \
                 s=\"$s$s$s$s$s$s$s$s$s$s$s$s$s$s$s$s\"; \
                 s=\"$s$s$s$s$s$s$s$s$s$s$s$s$s$s$s$s\"; \
                 i=0; while [ $i -lt 70 ]; do printf '%s' \"$s\"; i=$((i+1)); done; \
                 wait",
                marker.display()
            ),
        ];
        let err = run_with_network_lines(
            Path::new("/bin/sh"),
            &oversize_args,
            &project,
            &templates,
            &output,
            &settings,
            NetworkScope::LoopbackPort(port),
            &token,
            |_| Ok(()),
        )
        .await
        .unwrap_err();
        assert!(
            err.to_string().contains("maximum allowed size")
                || err.to_string().contains("report limit"),
            "unexpected error: {err}"
        );
        if let Ok(text) = tokio::fs::read_to_string(&marker).await {
            if let Ok(pid) = text.trim().parse::<libc::pid_t>() {
                for _ in 0..100 {
                    if unsafe { libc::kill(pid, 0) } == -1 {
                        return;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                }
                panic!("child descendant survived stdout limit breach");
            }
        }
    }
}
