//! Ownership of the helper processes the IDE starts on a person's behalf —
//! build steps (`go build`, `cargo build`, `javac`), `before_launch` hooks and
//! short probes.
//!
//! Every helper runs in its own process group (`process_group(0)`). A
//! [`HelperGroup`] is the one handle on that group: it registers the group while
//! the helper is live, kills the WHOLE group when dropped (so a cancelled or
//! timed-out `cargo` takes its `rustc` children with it, and `sh -lc "javac …"`
//! takes `javac`), and [`kill_all_helper_groups`] reaps whatever is still
//! registered when the daemon shuts down.

use std::collections::BTreeSet;
use std::io;
use std::process::Output;
use std::sync::Mutex;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt};

/// Backstop for build steps. Cold builds of large workspaces take many minutes,
/// so this only bounds a genuine hang; Stop is the person's escape hatch.
pub const BUILD_STEP_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// Bytes kept from each of a helper's stdout/stderr (the tail; the rest is
/// read and dropped so the helper never blocks on a full pipe).
pub const HELPER_OUTPUT_CAP: usize = 8 * 1024 * 1024;

static LIVE_GROUPS: Mutex<BTreeSet<u32>> = Mutex::new(BTreeSet::new());

fn live_groups() -> std::sync::MutexGuard<'static, BTreeSet<u32>> {
    LIVE_GROUPS.lock().unwrap_or_else(|e| e.into_inner())
}

/// A live helper process group, killed on drop.
pub struct HelperGroup {
    pgid: Option<u32>,
}

impl HelperGroup {
    /// Track the group led by `pid` (spawned with `process_group(0)`).
    pub fn track(pid: Option<u32>) -> Self {
        let pgid = pid.filter(|p| *p > 1);
        if let Some(p) = pgid {
            live_groups().insert(p);
        }
        Self { pgid }
    }

    pub fn pgid(&self) -> Option<u32> {
        self.pgid
    }

    /// SIGKILL every process in the group now and stop tracking it.
    pub fn kill(&mut self) {
        if let Some(p) = self.pgid.take() {
            live_groups().remove(&p);
            kill_group(p);
        }
    }
}

impl Drop for HelperGroup {
    fn drop(&mut self) {
        self.kill();
    }
}

fn kill_group(pgid: u32) {
    #[cfg(unix)]
    crate::lsp_host::pool_spawn::kill_process_group_sync(pgid);
    #[cfg(not(unix))]
    let _ = pgid;
}

/// Sets its flag when dropped: an async caller holds one across a blocking build
/// so that Stop (which drops the caller) reaches the build thread.
pub struct CancelOnDrop(pub std::sync::Arc<std::sync::atomic::AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::Release);
    }
}

/// Kill every helper group still registered (daemon shutdown). Returns how many.
pub fn kill_all_helper_groups() -> usize {
    let groups: Vec<u32> = std::mem::take(&mut *live_groups()).into_iter().collect();
    for p in &groups {
        kill_group(*p);
    }
    groups.len()
}

#[cfg(test)]
pub(crate) fn is_tracked(pgid: u32) -> bool {
    live_groups().contains(&pgid)
}

/// Read `reader` to EOF keeping only its last `cap` bytes.
pub async fn read_tail<R: AsyncRead + Unpin>(mut reader: R, cap: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut chunk = vec![0u8; 16 * 1024];
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) => break,
            Ok(n) => {
                out.extend_from_slice(&chunk[..n]);
                if out.len() > cap.saturating_mul(2).max(64 * 1024) {
                    let cut = out.len() - cap;
                    out.drain(..cut);
                }
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    if out.len() > cap {
        let cut = out.len() - cap;
        out.drain(..cut);
    }
    out
}

/// Wait for a helper spawned with `process_group(0)` and piped stdout/stderr,
/// keeping at most `cap` bytes of each stream. On timeout the whole group is
/// killed and an `ErrorKind::TimedOut` error returned; when this future is
/// dropped (Stop, a cancelled debug start) the group is killed too. After the
/// leader exits, stragglers left in its group are killed so the pipes close.
pub async fn wait_bounded(
    mut child: tokio::process::Child,
    limit: Duration,
    cap: usize,
) -> io::Result<Output> {
    let mut group = HelperGroup::track(child.id());
    let out_task = child.stdout.take().map(|r| tokio::spawn(read_tail(r, cap)));
    let err_task = child.stderr.take().map(|r| tokio::spawn(read_tail(r, cap)));
    let status = match tokio::time::timeout(limit, child.wait()).await {
        Ok(Ok(status)) => status,
        Ok(Err(e)) => {
            group.kill();
            return Err(e);
        }
        Err(_) => {
            group.kill();
            let _ = child.start_kill();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("timed out after {}s", limit.as_secs()),
            ));
        }
    };
    group.kill();
    let join = |t: Option<tokio::task::JoinHandle<Vec<u8>>>| async move {
        match t {
            Some(t) => tokio::time::timeout(Duration::from_secs(2), t)
                .await
                .ok()
                .and_then(Result::ok)
                .unwrap_or_default(),
            None => Vec::new(),
        }
    };
    let stdout = join(out_task).await;
    let stderr = join(err_task).await;
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::process::Stdio;

    #[allow(clippy::disallowed_methods)] // test helper; `kill -0` returns at once
    fn pid_alive(pid: u32) -> bool {
        std::process::Command::new("/bin/kill")
            .args(["-0", &pid.to_string()])
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    fn sh(script: &str) -> tokio::process::Command {
        let mut cmd = tokio::process::Command::new("/bin/sh");
        cmd.args(["-c", script])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .process_group(0);
        cmd
    }

    async fn wait_gone(pid: u32) -> bool {
        for _ in 0..50 {
            if !pid_alive(pid) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        false
    }

    async fn grandchild_pid(child: &mut tokio::process::Child) -> u32 {
        let mut out = child.stdout.take().unwrap();
        let mut buf = [0u8; 32];
        let n = out.read(&mut buf).await.unwrap();
        String::from_utf8_lossy(&buf[..n]).trim().parse().unwrap()
    }

    #[tokio::test]
    async fn timeout_kills_the_whole_group_not_just_the_leader() {
        // The leader backgrounds a grandchild (like cargo → rustc) and waits.
        let mut child = sh("sleep 60 & echo $!; wait").spawn().unwrap();
        let grandchild = grandchild_pid(&mut child).await;
        let leader = child.id().unwrap();
        let err = wait_bounded(child, Duration::from_millis(200), 1024)
            .await
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert!(
            wait_gone(grandchild).await,
            "grandchild {grandchild} survived"
        );
        assert!(!is_tracked(leader));
    }

    #[tokio::test]
    async fn dropping_the_wait_kills_the_group() {
        let mut child = sh("sleep 60 & echo $!; wait").spawn().unwrap();
        let grandchild = grandchild_pid(&mut child).await;
        let leader = child.id().unwrap();
        let fut = wait_bounded(child, BUILD_STEP_TIMEOUT, 1024);
        let _ = tokio::time::timeout(Duration::from_millis(150), fut).await;
        assert!(
            wait_gone(grandchild).await,
            "grandchild {grandchild} survived"
        );
        assert!(!is_tracked(leader));
    }

    #[tokio::test]
    async fn output_is_capped_to_its_tail_and_the_pipe_is_drained() {
        let child =
            sh("i=0; while [ $i -lt 5000 ]; do echo line-$i; i=$((i+1)); done; echo tail-end >&2")
                .spawn()
                .unwrap();
        let out = wait_bounded(child, Duration::from_secs(30), 64)
            .await
            .unwrap();
        assert!(out.status.success());
        assert!(out.stdout.len() <= 64);
        assert!(String::from_utf8_lossy(&out.stdout).ends_with("line-4999\n"));
        assert_eq!(String::from_utf8_lossy(&out.stderr), "tail-end\n");
    }

    // `kill_all_helper_groups` itself is not exercised here: it is process-wide
    // and would kill the helpers of tests running alongside under `cargo test`.
    #[test]
    fn tracked_group_is_registered_until_dropped_and_killed_on_drop() {
        use std::os::unix::process::CommandExt;
        let mut child = std::process::Command::new("/bin/sh")
            .args(["-c", "sleep 60"])
            .process_group(0)
            .spawn()
            .unwrap();
        let pid = child.id();
        let group = HelperGroup::track(Some(pid));
        assert!(is_tracked(pid));
        assert_eq!(group.pgid(), Some(pid));
        drop(group);
        assert!(!is_tracked(pid));
        let status = child.wait().unwrap();
        assert!(!status.success(), "sleep should have been killed");
    }
}
