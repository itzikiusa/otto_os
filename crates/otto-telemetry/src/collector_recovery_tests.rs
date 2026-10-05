//! Isolated process-lifecycle fixtures. No ClickHouse, downloads, user state,
//! process-name matching, or broad kill commands are involved.
use super::*;
use std::{fs, process::Child as StdChild};

/// A tiny executable accepts precisely the production argv while doing no
/// collection. Its optional parent forks+execs it, reproducing the actual gap:
/// parent SIGKILL cannot run a destructor or write a post-spawn PID record.
struct Fixture {
    root: tempfile::TempDir,
    executable: PathBuf,
    config: PathBuf,
    children: Vec<StdChild>,
    orphans: Vec<Candidate>,
}
impl Fixture {
    fn new() -> Self {
        Self::with_root(tempfile::tempdir().unwrap())
    }
    fn with_root(root: tempfile::TempDir) -> Self {
        let dir = root.path().join("telemetry");
        let bin_dir = dir.join(format!("collector-{VERSION}"));
        fs::create_dir_all(&bin_dir).unwrap();
        let source = root.path().join("fixture.c");
        fs::write(
            &source,
            r#"
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
int main(int argc, char **argv) {
    const char *pidfile = getenv("OTTO_RECOVERY_FIXTURE_PARENT");
    if (pidfile) {
        unsetenv("OTTO_RECOVERY_FIXTURE_PARENT");
        pid_t pid = fork();
        if (pid < 0) return 2;
        if (pid == 0) { execv(argv[0], argv); _exit(3); }
        FILE *file = fopen(pidfile, "w");
        if (!file) return 4;
        fprintf(file, "%d", pid); fclose(file);
    }
    for (;;) pause();
}
"#,
        )
        .unwrap();
        let executable = bin_dir.join("otelcol-contrib");
        let status = std::process::Command::new("cc")
            .arg(&source)
            .arg("-o")
            .arg(&executable)
            .status()
            .unwrap();
        assert!(status.success(), "compile isolated process fixture");
        let config = dir.join("collector.json");
        fs::write(&config, "{}").unwrap();
        Self {
            root,
            executable,
            config,
            children: Vec::new(),
            orphans: Vec::new(),
        }
    }
    fn dir(&self) -> PathBuf {
        self.root.path().join("telemetry")
    }
    fn spawn(&mut self, config: &Path) -> u32 {
        let child = std::process::Command::new(&self.executable)
            .arg("--config")
            .arg(config)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();
        self.children.push(child);
        pid
    }
    fn crash_parent(&mut self) -> Candidate {
        let pidfile = self.root.path().join("child.pid");
        let mut parent = std::process::Command::new(&self.executable)
            .arg("--config")
            .arg(&self.config)
            .env("OTTO_RECOVERY_FIXTURE_PARENT", &pidfile)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let pid = loop {
            if let Ok(text) = fs::read_to_string(&pidfile) {
                if let Ok(pid) = text.parse::<u32>() {
                    break Pid::from_u32(pid);
                }
            }
            assert!(Instant::now() < deadline, "fixture child not launched");
            std::thread::sleep(Duration::from_millis(20));
        };
        let mut system = System::new();
        system.refresh_processes_specifics(ProcessesToUpdate::Some(&[pid]), true, refresh_kind());
        let candidate = Candidate {
            pid,
            start_time: system.process(pid).unwrap().start_time(),
        };
        self.orphans.push(candidate);
        parent.kill().unwrap();
        parent.wait().unwrap();
        let identity = managed_identity(&self.dir()).unwrap().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if snapshot(&identity).is_ok_and(|candidates| candidates.iter().any(|c| c.pid == pid)) {
                break;
            }
            assert!(Instant::now() < deadline, "fixture child was not orphaned");
            std::thread::sleep(Duration::from_millis(20));
        }
        candidate
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        for child in &mut self.children {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Ok(Some(identity)) = managed_identity(&self.dir()) {
            for candidate in &self.orphans {
                let _ = revalidate_and_signal(&identity, *candidate, Some(Signal::Kill));
            }
        }
    }
}

#[tokio::test]
#[ignore = "isolated lifecycle fixture compiles a tiny C executable; run serially"]
async fn disabled_start_reaps_crashed_parent_collector_but_preserves_decoy_config() {
    let mut fixture = Fixture::new();
    let decoy_config = fixture.dir().join("another-collector.json");
    fs::write(&decoy_config, "{}").unwrap();
    let decoy = fixture.spawn(&decoy_config);
    let orphan = fixture.crash_parent();
    let usage = otto_usage::UsageEngine::start(
        otto_usage::UsageConfig {
            enabled: false,
            ..Default::default()
        },
        fixture.root.path().join("usage"),
    )
    .await;
    let service = crate::TelemetryService::start(
        usage.clone(),
        fixture.root.path().to_owned(),
        TelemetryConfig::default(),
    )
    .await;
    assert!(!service.enabled());
    assert!(!service.status().collector_ready);
    assert!(
        service.status().last_error.is_none(),
        "{:?}",
        service.status()
    );
    let identity = managed_identity(&fixture.dir()).unwrap().unwrap();
    assert!(
        !revalidate_and_signal(&identity, orphan, None).unwrap(),
        "orphan survived disabled startup"
    );
    assert!(
        fixture
            .children
            .iter_mut()
            .find(|c| c.id() == decoy)
            .unwrap()
            .try_wait()
            .unwrap()
            .is_none(),
        "different-config process was terminated"
    );
    assert_eq!(
        fs::read_to_string(&fixture.config).unwrap(),
        "{}",
        "disabled startup rewrote config"
    );
    service.shutdown().await;
    usage.shutdown().await;
}

#[tokio::test]
#[ignore = "isolated lifecycle fixture compiles a tiny C executable; run serially"]
async fn live_owner_blocks_duplicate_start_without_overwriting_config() {
    let mut fixture = Fixture::new();
    let config = fixture.config.clone();
    let pid = fixture.spawn(&config);
    let error = Collector::start(
        &fixture.dir(),
        "http://127.0.0.1:1",
        &TelemetryConfig::default(),
    )
    .await
    .err()
    .expect("live owner must block startup before download/config rewrite");
    assert!(error.to_string().contains("live owner"), "{error}");
    assert_eq!(fs::read_to_string(config).unwrap(), "{}");
    assert!(fixture
        .children
        .iter_mut()
        .find(|c| c.id() == pid)
        .unwrap()
        .try_wait()
        .unwrap()
        .is_none());
}

#[tokio::test]
#[ignore = "isolated lifecycle fixture compiles a tiny C executable; run serially"]
async fn changed_start_time_is_never_signaled() {
    let mut fixture = Fixture::new();
    let orphan = fixture.crash_parent();
    let identity = managed_identity(&fixture.dir()).unwrap().unwrap();
    let stale = Candidate {
        pid: orphan.pid,
        start_time: orphan.start_time.wrapping_add(1),
    };
    assert!(!revalidate_and_signal(&identity, stale, Some(Signal::Kill)).unwrap());
    assert!(
        revalidate_and_signal(&identity, orphan, None).unwrap(),
        "stale identity killed current process"
    );
    recover_orphans(&fixture.dir()).await.unwrap();
}

#[test]
fn relative_config_arguments_cannot_claim_managed_ownership() {
    assert!(canonical_argument(Path::new("collector.json")).is_none());
    assert!(canonical_argument(Path::new("../telemetry/collector.json")).is_none());
}

#[tokio::test]
async fn ownership_lock_excludes_concurrent_starts_and_releases_on_drop() {
    let temp = tempfile::tempdir().unwrap();
    let first = acquire_owner_lock(temp.path()).await.unwrap();
    assert!(acquire_owner_lock(temp.path()).await.is_err());
    drop(first);
    assert!(acquire_owner_lock(temp.path()).await.is_ok());
}

#[tokio::test]
#[ignore = "isolated lifecycle fixture compiles a tiny C executable; run serially"]
async fn relative_data_directory_launches_with_recoverable_absolute_config() {
    // Keep cwd unchanged: a relative fixture path exercises the actual setting
    // without introducing process-global cwd races with other tests.
    let fixture = Fixture::with_root(tempfile::tempdir_in(".").unwrap());
    let absolute = fixture.dir().canonicalize().unwrap();
    let cwd = std::env::current_dir().unwrap().canonicalize().unwrap();
    let relative = absolute.strip_prefix(cwd).unwrap().to_owned();
    assert!(!relative.is_absolute());
    fs::write(
        fixture.executable.with_file_name("binary.sha256"),
        hash_file(&fixture.executable).unwrap(),
    )
    .unwrap();
    let identity = managed_identity(&absolute).unwrap().unwrap();
    let configuration = TelemetryConfig::default();
    {
        let starting = Collector::start(&relative, "http://127.0.0.1:1", &configuration);
        tokio::pin!(starting);
        let observed = async {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let mut system = System::new();
                system.refresh_processes_specifics(ProcessesToUpdate::All, true, refresh_kind());
                if system
                    .processes()
                    .values()
                    .any(|process| ownership(process, &identity) == Ownership::Managed)
                {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "relative setting did not launch a collector with absolute recoverable argv"
                );
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        };
        tokio::select! {
            result=&mut starting=>panic!("fixture exited before argv observation: {:?}",result.err()),
            _=observed=>{},
        }
        // Dropping startup here exercises its cancellation cleanup too. The
        // fixture deliberately exposes no health server, so it cannot be ready.
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    while snapshot(&identity).is_err() {
        assert!(
            Instant::now() < deadline,
            "canceled startup left its child running"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(snapshot(&identity).unwrap().is_empty());
}
