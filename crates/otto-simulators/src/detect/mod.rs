//! Dynamic auto-detection and readiness diagnostics for iOS Simulator (Xcode)
//! and Android Emulator (Android SDK).
//!
//! Never fails daemon startup when tools are missing, mid-download, or missing
//! runtimes/AVDs — returns structured readiness states and actionable setup
//! commands instead.

pub mod android;
pub mod ios;

pub use android::detect_android;
pub use ios::{count_available_ios_runtimes, detect_ios};

use std::path::{Path, PathBuf};

use tokio::sync::Mutex;

use crate::exec::{CommandRunner, HostProbe};
use crate::types::SimulatorsStatus;

/// Resolved Android SDK tool paths discovered on the host.
#[derive(Debug, Clone, Default)]
pub struct ResolvedAndroidTools {
    pub sdk_root: Option<PathBuf>,
    pub studio_app: Option<PathBuf>,
    pub adb: Option<PathBuf>,
    pub emulator: Option<PathBuf>,
    pub avdmanager: Option<PathBuf>,
    pub sdkmanager: Option<PathBuf>,
}

/// Resolved iOS / Xcode tool paths discovered on the host.
#[derive(Debug, Clone, Default)]
pub struct ResolvedIosTools {
    pub xcode_app: Option<PathBuf>,
    pub xcode_downloading: Option<PathBuf>,
    pub developer_dir: Option<String>,
    pub xcrun: Option<PathBuf>,
    pub idb: Option<PathBuf>,
    pub simctl_ok: bool,
    pub license_required: bool,
}

/// Combined host discovery snapshot.
#[derive(Debug, Clone)]
pub struct HostDiscovery {
    pub ios_tools: ResolvedIosTools,
    pub android_tools: ResolvedAndroidTools,
    pub status: SimulatorsStatus,
}

/// Run full dynamic discovery across both iOS and Android.
pub async fn detect_all(
    runner: &dyn CommandRunner,
    probe: &dyn HostProbe,
    adb_lock: &Mutex<()>,
    managed_android_sdk: Option<&Path>,
) -> HostDiscovery {
    let (ios_tools, ios) = detect_ios(runner, probe).await;
    let (android_tools, android) =
        detect_android(runner, probe, adb_lock, managed_android_sdk).await;
    HostDiscovery {
        ios_tools,
        android_tools,
        status: SimulatorsStatus {
            ios,
            android,
            checked_at: chrono::Utc::now().to_rfc3339(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::fake::{FakeCommandRunner, FakeHostProbe};
    use crate::types::{ReadinessCode, ReadinessState};

    #[tokio::test]
    async fn detects_xcode_mid_download_and_missing_android_sdk() {
        let runner = FakeCommandRunner::new();
        let probe = FakeHostProbe::new();
        let adb_lock = Mutex::new(());
        probe.add_dir("/Applications", vec!["Xcode.appdownload".into()]);
        probe.add_path("/Applications/Xcode.appdownload");

        let disc = detect_all(&runner, &probe, &adb_lock, None).await;
        assert_eq!(disc.status.ios.state, ReadinessState::Unavailable);
        assert_eq!(disc.status.ios.code, ReadinessCode::XcodeDownloading);
        assert!(!disc.status.ios.setup_steps.is_empty());

        assert_eq!(disc.status.android.state, ReadinessState::Unavailable);
        assert_eq!(disc.status.android.code, ReadinessCode::SdkNotFound);
        assert!(!disc.status.android.setup_steps.is_empty());
    }
}
