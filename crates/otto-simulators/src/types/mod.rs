//! Domain DTOs, readiness diagnostics, request/response payloads, device ID
//! parsing, and binary screencast frame codec for `otto-simulators`.
//!
//! Keep in lockstep with `ui/src/lib/api/types/simulators.ts` and
//! `docs/contracts/api.md` + `docs/contracts/ws.md`.

pub mod frame;
pub mod requests;

pub use frame::*;
pub use requests::*;

use serde::{Deserialize, Serialize};

/// Binary frame format version (matches `otto_browser::live::protocol::FRAME_VERSION`).
pub const FRAME_VERSION: u8 = 1;

/// Maximum text frame accepted from a WebSocket client.
pub const MAX_CLIENT_FRAME_BYTES: usize = 64 * 1024;

/// Maximum clipboard / text input length (chars).
pub const MAX_INPUT_TEXT_CHARS: usize = 16_384;

/// Upper pixel coordinate bound for non-normalized Tap/Swipe input.
pub const MAX_INPUT_COORD_PX: f64 = 16_384.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    Ios,
    Android,
}

impl Platform {
    pub fn as_str(self) -> &'static str {
        match self {
            Platform::Ios => "ios",
            Platform::Android => "android",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceState {
    Booted,
    Booting,
    Shutdown,
    ShuttingDown,
    Offline,
    Unknown,
}

impl DeviceState {
    pub fn is_live(self) -> bool {
        matches!(self, DeviceState::Booted)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FormFactor {
    Phone,
    Tablet,
    Watch,
    Tv,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Orientation {
    Portrait,
    Landscape,
    PortraitUpsideDown,
    LandscapeRight,
    #[default]
    Unknown,
}

impl Orientation {
    pub fn is_landscape(self) -> bool {
        matches!(self, Orientation::Landscape | Orientation::LandscapeRight)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Appearance {
    Light,
    Dark,
}

impl Appearance {
    pub fn as_str(self) -> &'static str {
        match self {
            Appearance::Light => "light",
            Appearance::Dark => "dark",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadinessState {
    Ready,
    Degraded,
    Unavailable,
}

/// Closed diagnostic readiness code, mirrored as `SimReadinessCode` in TS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadinessCode {
    Ready,
    XcodeNotInstalled,
    XcodeDownloading,
    XcodeSelectRequired,
    XcodeLicenseRequired,
    IosRuntimeMissing,
    NoDevices,
    SdkNotFound,
    StudioInstalledSdkMissing,
    AdbMissing,
    EmulatorMissing,
    NoAvds,
    UnsupportedOs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueSeverity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolStatus {
    pub name: String,
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadinessIssue {
    pub code: String,
    pub severity: IssueSeverity,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetupStep {
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformReadiness {
    pub platform: Platform,
    pub state: ReadinessState,
    pub code: ReadinessCode,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sdk_path: Option<String>,
    pub tools: Vec<ToolStatus>,
    pub issues: Vec<ReadinessIssue>,
    pub setup_steps: Vec<SetupStep>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimulatorsStatus {
    pub ios: PlatformReadiness,
    pub android: PlatformReadiness,
    pub checked_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScreenMetrics {
    pub width: u32,
    pub height: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dpi: Option<u32>,
}

impl ScreenMetrics {
    /// Return `(width, height)` adjusted for the current device orientation
    /// (swapping portrait physical dimensions when rotated to landscape).
    pub fn oriented_dims(&self, orientation: Orientation) -> (f64, f64) {
        let (w, h) = (f64::from(self.width), f64::from(self.height));
        let needs_swap = (orientation.is_landscape() && h > w)
            || (!orientation.is_landscape() && orientation != Orientation::Unknown && w > h);
        if needs_swap {
            (h, w)
        } else {
            (w, h)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DeviceCapabilities {
    pub boot: bool,
    pub shutdown: bool,
    pub delete: bool,
    pub open_url: bool,
    pub screenshot: bool,
    pub stream: bool,
    pub tap: bool,
    pub swipe: bool,
    pub text: bool,
    pub key_event: bool,
    pub rotate: bool,
    pub appearance: bool,
    pub status_bar: bool,
    pub clipboard: bool,
    pub port_forward: bool,
    pub app_install: bool,
    pub logs: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimDevice {
    pub id: String,
    pub platform: Platform,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub udid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avd_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serial: Option<String>,
    pub state: DeviceState,
    pub os_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_level: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_type: Option<String>,
    pub form_factor: FormFactor,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screen: Option<ScreenMetrics>,
    pub orientation: Orientation,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub appearance: Option<Appearance>,
    pub is_physical: bool,
    pub capabilities: DeviceCapabilities,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimDevicesResp {
    pub devices: Vec<SimDevice>,
    pub status: SimulatorsStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimRuntime {
    pub id: String,
    pub name: String,
    pub version: String,
    pub platform: Platform,
    pub available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimDeviceType {
    pub id: String,
    pub name: String,
    pub form_factor: FormFactor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AndroidSystemImage {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_level: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub abi: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    pub installed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AndroidDeviceProfile {
    pub id: String,
    pub name: String,
    pub form_factor: FormFactor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SimCatalogResp {
    pub ios_runtimes: Vec<SimRuntime>,
    pub ios_device_types: Vec<SimDeviceType>,
    pub android_system_images: Vec<AndroidSystemImage>,
    pub android_device_profiles: Vec<AndroidDeviceProfile>,
}

// ---------------------------------------------------------------------------
// Built-in SDK / Runtime Installer DTOs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SimInstallTarget {
    AndroidEmulator,
    AndroidPlatformTools,
    IosRuntime,
}

impl SimInstallTarget {
    pub fn label(self) -> &'static str {
        match self {
            SimInstallTarget::AndroidEmulator => "Android Emulator & SDK",
            SimInstallTarget::AndroidPlatformTools => "Android Platform Tools (adb)",
            SimInstallTarget::IosRuntime => "iOS Simulator Runtime",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SimInstallStage {
    Idle,
    Downloading,
    Extracting,
    Licensing,
    InstallingPackages,
    CreatingAvds,
    Done,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SimInstallReq {
    pub target: SimInstallTarget,
    #[serde(default)]
    pub create_default_avds: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimInstallStatus {
    pub running: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<SimInstallTarget>,
    pub stage: SimInstallStage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress_pct: Option<u8>,
    pub message: String,
    pub log_tail: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub updated_at: String,
}

impl Default for SimInstallStatus {
    fn default() -> Self {
        Self {
            running: false,
            target: None,
            stage: SimInstallStage::Idle,
            progress_pct: None,
            message: "No installation running.".into(),
            log_tail: Vec::new(),
            error: None,
            updated_at: chrono::Utc::now().to_rfc3339(),
        }
    }
}
