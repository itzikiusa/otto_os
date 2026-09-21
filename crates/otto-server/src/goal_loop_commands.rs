//! Bounded verification commands with cancellation and process-tree cleanup.
use crate::proof::CmdRun;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[cfg(unix)]
struct ProcessGroup(rustix::process::Pid);
#[cfg(unix)]
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        let _ = rustix::process::kill_process_group(self.0, rustix::process::Signal::KILL);
    }
}

pub async fn run(cwd: &str, command: &str, timeout_secs: u64, cancelled: &AtomicBool) -> CmdRun {
    let start = Instant::now();
    let failure = |message: String| CmdRun {
        success: false,
        exit_code: -1,
        output: message,
        duration_ms: start.elapsed().as_millis() as u64,
    };
    if cancelled.load(Ordering::Relaxed) {
        return failure("verification cancelled".into());
    }
    let mut command_spec = tokio::process::Command::new("sh");
    command_spec
        .arg("-c")
        .arg(command)
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    command_spec.process_group(0);
    let child = match command_spec.spawn() {
        Ok(child) => child,
        Err(error) => return failure(format!("failed to spawn verification: {error}")),
    };
    // Own a distinct group before waiting: dropping the output future alone
    // kills only the shell and would leave its child tests running.
    #[cfg(unix)]
    let _group = child
        .id()
        .and_then(|id| rustix::process::Pid::from_raw(id as i32))
        .map(ProcessGroup);
    let output = tokio::select! {
        biased;
        _ = async {
            loop {
                if cancelled.load(Ordering::Relaxed) { break; }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        } => return failure("verification cancelled".into()),
        result = tokio::time::timeout(Duration::from_secs(timeout_secs), child.wait_with_output()) => result,
    };
    match output {
        Ok(Ok(output)) => {
            let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
            if !output.stderr.is_empty() {
                text.push_str("\n--- stderr ---\n");
                text.push_str(&String::from_utf8_lossy(&output.stderr));
            }
            CmdRun {
                success: output.status.success(),
                exit_code: output.status.code().unwrap_or(-1),
                output: text,
                duration_ms: start.elapsed().as_millis() as u64,
            }
        }
        Ok(Err(error)) => failure(format!("verification failed: {error}")),
        Err(_) => failure(format!("verification timed out after {timeout_secs}s")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn cancellation_before_spawn_does_not_execute_verification() {
        let dir = tempfile::tempdir().unwrap();
        let cancelled = AtomicBool::new(true);
        let run = run(
            dir.path().to_str().unwrap(),
            "touch should-not-exist",
            5,
            &cancelled,
        )
        .await;
        assert!(!run.success);
        assert!(!dir.path().join("should-not-exist").exists());
    }
    #[tokio::test]
    async fn cancellation_kills_verification_descendants() {
        let dir = tempfile::tempdir().unwrap();
        let cancelled = AtomicBool::new(false);
        let work = run(
            dir.path().to_str().unwrap(),
            "(sleep 1; touch should-not-exist) & wait",
            10,
            &cancelled,
        );
        let stop = async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            cancelled.store(true, Ordering::Relaxed);
        };
        let start = Instant::now();
        let (result, ()) = tokio::join!(work, stop);
        assert!(!result.success);
        assert!(start.elapsed() < Duration::from_millis(750));
        tokio::time::sleep(Duration::from_millis(1100)).await;
        assert!(!dir.path().join("should-not-exist").exists());
    }
}
