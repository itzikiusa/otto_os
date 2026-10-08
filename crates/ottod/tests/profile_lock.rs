//! Lock-only subprocesses: never boot daemons, sessions or external engines.
#[path = "../src/profile_lock.rs"]
mod profile_lock;

#[test]
fn child_lock_probe() {
    let Some(path) = std::env::var_os("OTTO_TEST_PROFILE_LOCK_DIR") else {
        return;
    };
    let expected = std::env::var("OTTO_TEST_PROFILE_LOCK_EXPECT").unwrap();
    assert_eq!(
        profile_lock::acquire(std::path::Path::new(&path)).is_ok(),
        expected == "free"
    );
}

// Synchronous subprocess integration test; no Tokio worker is blocked.
#[allow(clippy::disallowed_methods)]
#[test]
fn profile_lock_excludes_other_ports_and_directory_aliases_until_release() {
    let root = std::env::temp_dir().join(format!("otto-profile-lock-test-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(root.clone());
    let profile = root.join("profile");
    let lock = profile_lock::acquire(&profile).unwrap();
    let probe = |path: &std::path::Path, expected: &str| {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "child_lock_probe", "--nocapture"])
            .env("OTTO_TEST_PROFILE_LOCK_DIR", path)
            .env("OTTO_TEST_PROFILE_LOCK_EXPECT", expected)
            .env("OTTO_PORT", "17799")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stdout)
        );
    };
    probe(&profile, "busy");
    #[cfg(unix)]
    {
        let alias = root.join("alias");
        std::os::unix::fs::symlink(&profile, &alias).unwrap();
        probe(&alias, "busy");
    }
    drop(lock);
    probe(&profile, "free");
}
