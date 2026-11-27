use std::path::Path;

use otto_core::{Error, Result};
use tokio_util::sync::CancellationToken;

use crate::{ScanMetrics, ScanSettings};

#[cfg(target_os = "macos")]
#[path = "governor/macos.rs"]
mod platform;

#[cfg(target_os = "macos")]
pub(crate) use platform::run;

#[cfg(not(target_os = "macos"))]
pub(crate) async fn run(
    _program: &Path,
    _args: &[String],
    _project: &Path,
    _cache: &Path,
    _output: &Path,
    _settings: &ScanSettings,
    _cancel: &CancellationToken,
) -> Result<ScanMetrics> {
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
}
