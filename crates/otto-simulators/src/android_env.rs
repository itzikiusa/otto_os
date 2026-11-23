//! Shared Android environment, ADB daemon startup, AVD-list parsing, and JDK
//! discovery helpers.
//!
//! Kept as a leaf module depending only on [`crate::exec`] and [`crate::types`]
//! so `detect`, `android`, and `install` form a strictly acyclic module graph.

use std::path::Path;
use std::time::Duration;

use tokio::sync::Mutex;

use crate::exec::{CmdSpec, CommandRunner, HostProbe};
use crate::types::is_safe_avd_name;

/// Parse `emulator -list-avds` stdout cleanly, filtering empty lines, emulator
/// `INFO`/`WARNING` banners, and unsafe AVD names.
pub fn parse_avd_list(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|line| {
            !line.is_empty()
                && !line.starts_with("INFO")
                && !line.starts_with("WARNING")
                && is_safe_avd_name(line)
        })
        .map(ToOwned::to_owned)
        .collect()
}

/// Attach `ANDROID_SDK_ROOT`, `ANDROID_HOME`, and optional `JAVA_HOME` to a
/// [`CmdSpec`].
pub fn with_android_sdk_env(
    mut spec: CmdSpec,
    sdk_root: Option<&Path>,
    java_home: Option<&str>,
) -> CmdSpec {
    if let Some(root) = sdk_root {
        let r = root.to_string_lossy().into_owned();
        spec = spec
            .with_env("ANDROID_SDK_ROOT", &r)
            .with_env("ANDROID_HOME", &r);
    }
    if let Some(jh) = java_home {
        spec = spec.with_env("JAVA_HOME", jh);
    }
    spec
}

/// Ensure the adb daemon is running using null stdio (`run_silent`) serialized
/// behind an injected per-service `adb_lock`.
pub async fn ensure_adb_server(runner: &dyn CommandRunner, adb_lock: &Mutex<()>, adb_path: &str) {
    let _guard = adb_lock.lock().await;
    let _ = runner
        .run_silent(CmdSpec::new(adb_path, ["start-server"]).with_timeout(Duration::from_secs(15)))
        .await;
}

/// Resolve a JDK `JAVA_HOME` when `ottod` runs under launchd / Tauri sidecar
/// without a login shell's `JAVA_HOME`.
pub async fn discover_java_home(
    runner: &dyn CommandRunner,
    probe: &dyn HostProbe,
) -> Option<String> {
    if let Some(jh) = probe.env_var("JAVA_HOME") {
        if probe.is_executable(&Path::new(&jh).join("bin/java")) {
            return Some(jh);
        }
    }
    for args in [vec!["-v", "17+"], vec![]] {
        if let Ok(out) = runner
            .run(CmdSpec::new("/usr/libexec/java_home", args).with_timeout(Duration::from_secs(5)))
            .await
        {
            if out.success() {
                let p = out.stdout_str().trim().to_string();
                if !p.is_empty() && probe.is_executable(&Path::new(&p).join("bin/java")) {
                    return Some(p);
                }
            }
        }
    }
    let jvm_root = Path::new("/Library/Java/JavaVirtualMachines");
    let mut jvm_entries = probe.list_dir(jvm_root);
    jvm_entries.sort();
    for entry in jvm_entries.into_iter().rev() {
        let home = jvm_root.join(&entry).join("Contents/Home");
        if probe.is_executable(&home.join("bin/java")) {
            return Some(home.to_string_lossy().into_owned());
        }
    }
    for candidate in [
        "/opt/homebrew/opt/openjdk@21/libexec/openjdk.jdk/Contents/Home",
        "/opt/homebrew/opt/openjdk@17/libexec/openjdk.jdk/Contents/Home",
        "/opt/homebrew/opt/openjdk/libexec/openjdk.jdk/Contents/Home",
        "/usr/local/opt/openjdk@21/libexec/openjdk.jdk/Contents/Home",
        "/usr/local/opt/openjdk@17/libexec/openjdk.jdk/Contents/Home",
        "/Applications/Android Studio.app/Contents/jbr/Contents/Home",
    ] {
        if probe.is_executable(&Path::new(candidate).join("bin/java")) {
            return Some(candidate.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::fake::{FakeCommandRunner, FakeHostProbe};

    #[tokio::test]
    async fn discovers_temurin_jdk_when_java_home_unset() {
        let runner = FakeCommandRunner::new();
        let probe = FakeHostProbe::new();
        probe.add_dir(
            "/Library/Java/JavaVirtualMachines",
            vec!["temurin-21.jdk".into(), "temurin-17.jdk".into()],
        );
        probe.add_executable(
            "/Library/Java/JavaVirtualMachines/temurin-21.jdk/Contents/Home/bin/java",
        );
        let found = discover_java_home(&runner, &probe).await;
        assert_eq!(
            found.as_deref(),
            Some("/Library/Java/JavaVirtualMachines/temurin-21.jdk/Contents/Home")
        );
    }

    #[test]
    fn parses_avd_list_filtering_banners_and_unsafe_names() {
        let parsed = parse_avd_list(
            "INFO    | Storing crashdata in: /tmp\n\
             WARNING | Something noisy\n\
             Pixel_9_API_35\n\
             -bad-leading-dash\n\
             Otto_Pixel_Tablet\n",
        );
        assert_eq!(parsed, vec!["Pixel_9_API_35", "Otto_Pixel_Tablet"]);
    }
}
