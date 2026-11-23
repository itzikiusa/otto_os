//! Unified `SimulatorsService` coordinating iOS Simulator and Android Emulator
//! discovery, lifecycle, controls, built-in installation, and per-device locks.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use otto_core::keyed_lock::KeyedLocks;
use tokio::sync::{Mutex, RwLock};

use crate::android;
use crate::detect::{detect_all, HostDiscovery};
use crate::exec::{
    CommandRunner, HostProbe, LogStreamHandle, SystemCommandRunner, SystemHostProbe,
};
use crate::install::{BeforePublishFn, InstallManager};
use crate::ios;
use crate::types::{
    ActionOkResp, Appearance, BootSimDeviceReq, ClipboardResp, CreateSimDeviceReq, DeviceId,
    DeviceState, InputAction, OpenSimUrlResp, Orientation, Platform, PortForward, ScreenshotData,
    ScreenshotFormat, SimCatalogResp, SimDevice, SimDevicesResp, SimInstallReq, SimInstallStatus,
    SimulatorsStatus, StatusBarReq,
};

const DISCOVERY_CACHE_TTL: Duration = Duration::from_secs(10);
const DEVICE_CACHE_TTL: Duration = Duration::from_secs(3);

struct CachedDiscovery {
    at: Instant,
    snapshot: HostDiscovery,
}

struct CachedDeviceList {
    at: Instant,
    devices: Vec<SimDevice>,
}

/// Validated booted device handle centralizing UDID/serial resolution.
#[derive(Clone)]
pub(crate) enum BootedTarget {
    Ios { dev: SimDevice, udid: String },
    Android { dev: SimDevice, serial: String },
}

impl BootedTarget {
    pub(crate) fn id(&self) -> &str {
        match self {
            BootedTarget::Ios { dev, .. } | BootedTarget::Android { dev, .. } => &dev.id,
        }
    }
}

/// Thread-safe facade owning iOS & Android drivers, built-in installer, and locks.
pub struct SimulatorsService {
    runner: Arc<dyn CommandRunner>,
    probe: Arc<dyn HostProbe>,
    cache: Arc<RwLock<Option<CachedDiscovery>>>,
    /// Short-TTL per-platform device listing cache invalidated on lifecycle mutations and errors.
    device_cache: Arc<RwLock<HashMap<Platform, CachedDeviceList>>>,
    /// Per-UDID iOS orientation tracked across `set_orientation` calls and screenshot frame dimensions.
    ios_orientations: Arc<RwLock<HashMap<String, Orientation>>>,
    /// Single-flight lock around `detect_all` so older probes never overwrite newer snapshots
    /// and post-install invalidation cannot race an in-flight probe.
    discovery_lock: Arc<Mutex<()>>,
    /// Instance-scoped lock serializing `adb start-server`.
    adb_start_lock: Arc<Mutex<()>>,
    /// Per-device lock preventing overlapping screenshot subprocesses.
    capture_locks: KeyedLocks<String>,
    /// Per-device lock preventing concurrent colliding boot/shutdown/restart calls.
    lifecycle_locks: KeyedLocks<String>,
    installer: Arc<InstallManager>,
}

impl Default for SimulatorsService {
    fn default() -> Self {
        Self::new()
    }
}

impl SimulatorsService {
    pub fn new() -> Self {
        let default_dir = dirs::data_local_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("Otto");
        Self::new_with_data_dir(default_dir)
    }

    pub fn new_with_data_dir(data_dir: PathBuf) -> Self {
        Self::with_deps_and_installer(
            Arc::new(SystemCommandRunner),
            Arc::new(SystemHostProbe),
            Arc::new(InstallManager::new(data_dir)),
        )
    }

    pub fn with_deps(runner: Arc<dyn CommandRunner>, probe: Arc<dyn HostProbe>) -> Self {
        let dir = std::env::temp_dir().join("otto-simulators-test");
        Self::with_deps_and_installer(runner, probe, Arc::new(InstallManager::new(dir)))
    }

    pub fn with_deps_and_installer(
        runner: Arc<dyn CommandRunner>,
        probe: Arc<dyn HostProbe>,
        installer: Arc<InstallManager>,
    ) -> Self {
        Self {
            runner,
            probe,
            cache: Arc::new(RwLock::new(None)),
            device_cache: Arc::new(RwLock::new(HashMap::new())),
            ios_orientations: Arc::new(RwLock::new(HashMap::new())),
            discovery_lock: Arc::new(Mutex::new(())),
            adb_start_lock: Arc::new(Mutex::new(())),
            capture_locks: KeyedLocks::new(),
            lifecycle_locks: KeyedLocks::new(),
            installer,
        }
    }

    async fn discovery(&self, refresh: bool) -> HostDiscovery {
        if !refresh {
            if let Some(cached) = self.cache.read().await.as_ref() {
                if cached.at.elapsed() < DISCOVERY_CACHE_TTL {
                    return cached.snapshot.clone();
                }
            }
        }
        let _flight = self.discovery_lock.lock().await;
        if !refresh {
            if let Some(cached) = self.cache.read().await.as_ref() {
                if cached.at.elapsed() < DISCOVERY_CACHE_TTL {
                    return cached.snapshot.clone();
                }
            }
        }
        let managed = self.installer.managed_sdk_dir();
        let fresh = detect_all(
            self.runner.as_ref(),
            self.probe.as_ref(),
            self.adb_start_lock.as_ref(),
            Some(&managed),
        )
        .await;
        *self.cache.write().await = Some(CachedDiscovery {
            at: Instant::now(),
            snapshot: fresh.clone(),
        });
        fresh
    }

    async fn invalidate_cache(&self) {
        let _flight = self.discovery_lock.lock().await;
        *self.cache.write().await = None;
        self.device_cache.write().await.clear();
    }

    pub(crate) async fn invalidate_device_cache(&self) {
        self.device_cache.write().await.clear();
    }

    /// Record an observed orientation for an iOS simulator UDID and update any
    /// cached device entry in place so subsequent normalized taps scale accurately.
    pub(crate) async fn record_ios_orientation(&self, udid: &str, orientation: Orientation) {
        if orientation == Orientation::Unknown {
            return;
        }
        self.ios_orientations
            .write()
            .await
            .insert(udid.to_string(), orientation);
        if let Some(cached) = self.device_cache.write().await.get_mut(&Platform::Ios) {
            for dev in &mut cached.devices {
                if dev.udid.as_deref() == Some(udid) {
                    dev.orientation = orientation;
                }
            }
        }
    }

    async fn update_ios_orientation_from_frame(&self, udid: &str, width: u32, height: u32) {
        if width == 0 || height == 0 || width == height {
            return;
        }
        let current = self.ios_orientations.read().await.get(udid).copied();
        if width > height {
            if !current.is_some_and(Orientation::is_landscape) {
                self.record_ios_orientation(udid, Orientation::Landscape)
                    .await;
            }
        } else if current.is_some_and(Orientation::is_landscape) {
            self.record_ios_orientation(udid, Orientation::Portrait)
                .await;
        }
    }

    pub async fn install_status(&self) -> SimInstallStatus {
        self.installer.status().await
    }

    pub async fn start_install(&self, req: SimInstallReq) -> otto_core::Result<SimInstallStatus> {
        // Acquire `discovery_lock` and clear caches synchronously BEFORE InstallManager
        // publishes terminal status so an in-flight probe cannot overwrite with stale state.
        let cache = self.cache.clone();
        let device_cache = self.device_cache.clone();
        let discovery_lock = self.discovery_lock.clone();
        let before_publish: BeforePublishFn = Arc::new(move || {
            let cache = cache.clone();
            let device_cache = device_cache.clone();
            let discovery_lock = discovery_lock.clone();
            Box::pin(async move {
                let _flight = discovery_lock.lock().await;
                *cache.write().await = None;
                device_cache.write().await.clear();
            })
        });
        self.installer
            .start(
                req,
                self.runner.clone(),
                self.probe.clone(),
                self.adb_start_lock.clone(),
                before_publish,
            )
            .await
    }

    pub async fn cancel_install(&self) -> SimInstallStatus {
        let st = self.installer.cancel().await;
        self.invalidate_cache().await;
        st
    }

    pub async fn status(&self, refresh: bool) -> SimulatorsStatus {
        self.discovery(refresh).await.status
    }

    async fn list_platform_devices(
        &self,
        platform: Platform,
        disc: &HostDiscovery,
        refresh: bool,
    ) -> otto_core::Result<Vec<SimDevice>> {
        if !refresh {
            if let Some(cached) = self.device_cache.read().await.get(&platform) {
                if cached.at.elapsed() < DEVICE_CACHE_TTL {
                    return Ok(cached.devices.clone());
                }
            }
        }
        let fresh = match platform {
            Platform::Ios => {
                let orientations = self.ios_orientations.read().await.clone();
                ios::list_devices(self.runner.as_ref(), &disc.ios_tools, &orientations).await?
            }
            Platform::Android => {
                android::list_devices(
                    self.runner.as_ref(),
                    self.probe.as_ref(),
                    self.adb_start_lock.as_ref(),
                    &disc.android_tools,
                )
                .await?
            }
        };
        self.device_cache.write().await.insert(
            platform,
            CachedDeviceList {
                at: Instant::now(),
                devices: fresh.clone(),
            },
        );
        Ok(fresh)
    }

    pub async fn list_devices(
        &self,
        platform: Option<Platform>,
        state: Option<DeviceState>,
        refresh: bool,
    ) -> otto_core::Result<SimDevicesResp> {
        let disc = self.discovery(refresh).await;
        let mut devices = Vec::new();
        if platform.is_none() || platform == Some(Platform::Ios) {
            match self
                .list_platform_devices(Platform::Ios, &disc, refresh)
                .await
            {
                Ok(ios_devs) => devices.extend(ios_devs),
                Err(err) => {
                    tracing::warn!(platform = "ios", error = %err, "failed to list iOS simulators");
                    if platform == Some(Platform::Ios) {
                        return Err(err);
                    }
                }
            }
        }
        if platform.is_none() || platform == Some(Platform::Android) {
            match self
                .list_platform_devices(Platform::Android, &disc, refresh)
                .await
            {
                Ok(and_devs) => devices.extend(and_devs),
                Err(err) => {
                    tracing::warn!(platform = "android", error = %err, "failed to list Android devices");
                    if platform == Some(Platform::Android) {
                        return Err(err);
                    }
                }
            }
        }
        if let Some(want_state) = state {
            devices.retain(|d| d.state == want_state);
        }
        Ok(SimDevicesResp {
            devices,
            status: disc.status,
        })
    }

    pub async fn catalog(&self, platform: Option<Platform>) -> otto_core::Result<SimCatalogResp> {
        let disc = self.discovery(false).await;
        let mut resp = SimCatalogResp::default();
        if platform.is_none() || platform == Some(Platform::Ios) {
            match ios::list_catalog(self.runner.as_ref(), &disc.ios_tools).await {
                Ok((runtimes, types)) => {
                    resp.ios_runtimes = runtimes;
                    resp.ios_device_types = types;
                }
                Err(err) => {
                    tracing::warn!(platform = "ios", error = %err, "failed to load iOS simulator catalog");
                    if platform == Some(Platform::Ios) {
                        return Err(err);
                    }
                }
            }
        }
        if platform.is_none() || platform == Some(Platform::Android) {
            match android::list_catalog(
                self.runner.as_ref(),
                self.probe.as_ref(),
                &disc.android_tools,
            )
            .await
            {
                Ok((imgs, profs)) => {
                    resp.android_system_images = imgs;
                    resp.android_device_profiles = profs;
                }
                Err(err) => {
                    tracing::warn!(platform = "android", error = %err, "failed to load Android catalog");
                    if platform == Some(Platform::Android) {
                        return Err(err);
                    }
                }
            }
        }
        Ok(resp)
    }

    pub async fn get_device(&self, raw_id: &str) -> otto_core::Result<SimDevice> {
        let parsed = DeviceId::parse(raw_id)?;
        let resp = self
            .list_devices(Some(parsed.platform()), None, false)
            .await?;
        find_matching_device(&resp.devices, &parsed)
            .cloned()
            .ok_or_else(|| otto_core::Error::NotFound(format!("simulator {raw_id} not found")))
    }

    pub async fn create_device(&self, req: CreateSimDeviceReq) -> otto_core::Result<SimDevice> {
        let disc = self.discovery(true).await;
        let created_id = match req {
            CreateSimDeviceReq::Ios {
                name,
                device_type_id,
                runtime_id,
            } => {
                let udid = ios::create_device(
                    self.runner.as_ref(),
                    &disc.ios_tools,
                    &name,
                    &device_type_id,
                    &runtime_id,
                )
                .await?;
                format!("ios:{udid}")
            }
            CreateSimDeviceReq::Android {
                name,
                system_image_id,
                device_profile,
            } => {
                android::create_avd(
                    self.runner.as_ref(),
                    self.probe.as_ref(),
                    &disc.android_tools,
                    &name,
                    &system_image_id,
                    device_profile.as_deref(),
                )
                .await?;
                format!("android:avd:{name}")
            }
        };
        self.invalidate_cache().await;
        self.get_device(&created_id).await
    }

    pub async fn delete_device(&self, raw_id: &str) -> otto_core::Result<()> {
        let parsed = DeviceId::parse(raw_id)?;
        let disc = self.discovery(false).await;
        match parsed {
            DeviceId::IosUdid(udid) => {
                ios::delete_device(self.runner.as_ref(), &disc.ios_tools, &udid).await?;
            }
            DeviceId::AndroidAvd(avd) => {
                android::delete_avd(
                    self.runner.as_ref(),
                    self.probe.as_ref(),
                    &disc.android_tools,
                    &avd,
                )
                .await?;
            }
            DeviceId::AndroidSerial(_) => {
                return Err(otto_core::Error::Invalid(
                    "cannot delete a connected serial device directly; delete by android:avd:<name>"
                        .into(),
                ));
            }
        }
        self.invalidate_cache().await;
        Ok(())
    }

    pub async fn boot_device(
        &self,
        raw_id: &str,
        req: &BootSimDeviceReq,
    ) -> otto_core::Result<SimDevice> {
        let parsed = DeviceId::parse(raw_id)?;
        let _guard = self.lifecycle_locks.lock(&parsed.as_canonical()).await;
        self.boot_device_unlocked(&parsed, raw_id, req).await
    }

    async fn boot_device_unlocked(
        &self,
        parsed: &DeviceId,
        raw_id: &str,
        req: &BootSimDeviceReq,
    ) -> otto_core::Result<SimDevice> {
        let disc = self.discovery(false).await;
        match parsed {
            DeviceId::IosUdid(udid) => {
                ios::boot_device(self.runner.as_ref(), &disc.ios_tools, udid, req).await?;
            }
            DeviceId::AndroidAvd(avd) => {
                android::boot_avd(
                    self.runner.as_ref(),
                    self.probe.as_ref(),
                    self.adb_start_lock.as_ref(),
                    &disc.android_tools,
                    avd,
                    req,
                )
                .await?;
            }
            DeviceId::AndroidSerial(_) => {
                return Err(otto_core::Error::Invalid(
                    "cannot boot by serial; boot an Android AVD via android:avd:<name>".into(),
                ));
            }
        }
        self.invalidate_device_cache().await;
        self.get_device(raw_id).await
    }

    pub async fn shutdown_device(&self, raw_id: &str) -> otto_core::Result<SimDevice> {
        let parsed = DeviceId::parse(raw_id)?;
        let _guard = self.lifecycle_locks.lock(&parsed.as_canonical()).await;
        self.shutdown_device_unlocked(raw_id).await
    }

    async fn shutdown_device_unlocked(&self, raw_id: &str) -> otto_core::Result<SimDevice> {
        let dev = self.get_device(raw_id).await?;
        let disc = self.discovery(false).await;
        match dev.platform {
            Platform::Ios => {
                let udid = dev
                    .udid
                    .as_deref()
                    .ok_or_else(|| otto_core::Error::Invalid("missing iOS UDID".into()))?;
                ios::shutdown_device(self.runner.as_ref(), &disc.ios_tools, udid).await?;
            }
            Platform::Android => {
                if let Some(serial) = dev.serial.as_deref() {
                    android::shutdown_serial(self.runner.as_ref(), &disc.android_tools, serial)
                        .await?;
                }
            }
        }
        self.invalidate_device_cache().await;
        self.get_device(raw_id).await
    }

    pub async fn restart_device(
        &self,
        raw_id: &str,
        req: &BootSimDeviceReq,
    ) -> otto_core::Result<SimDevice> {
        let parsed = DeviceId::parse(raw_id)?;
        // Reject unbootable serial IDs before shutting down the device.
        if matches!(parsed, DeviceId::AndroidSerial(_)) {
            return Err(otto_core::Error::Invalid(
                "cannot restart by serial; restart an Android AVD via android:avd:<name>".into(),
            ));
        }
        let _guard = self.lifecycle_locks.lock(&parsed.as_canonical()).await;
        let _ = self.shutdown_device_unlocked(raw_id).await;
        let mut boot_req = req.clone();
        boot_req.wipe_data = Some(false);
        self.boot_device_unlocked(&parsed, raw_id, &boot_req).await
    }

    pub async fn open_url(
        &self,
        raw_id: &str,
        url: &str,
        auto_forward_localhost: bool,
    ) -> otto_core::Result<OpenSimUrlResp> {
        let target = self.require_booted_target(raw_id).await?;
        let disc = self.discovery(false).await;
        match target {
            BootedTarget::Ios { udid, .. } => {
                ios::open_url(self.runner.as_ref(), &disc.ios_tools, &udid, url).await
            }
            BootedTarget::Android { serial, .. } => {
                android::open_url(
                    self.runner.as_ref(),
                    &disc.android_tools,
                    &serial,
                    url,
                    auto_forward_localhost,
                )
                .await
            }
        }
    }

    pub async fn screenshot(
        &self,
        raw_id: &str,
        format: ScreenshotFormat,
    ) -> otto_core::Result<ScreenshotData> {
        let target = self.require_booted_target(raw_id).await?;
        self.screenshot_target(&target, format).await
    }

    pub(crate) async fn screenshot_target(
        &self,
        target: &BootedTarget,
        format: ScreenshotFormat,
    ) -> otto_core::Result<ScreenshotData> {
        let _guard = self.capture_locks.lock(target.id()).await;
        let disc = self.discovery(false).await;
        let res = match target {
            BootedTarget::Ios { udid, .. } => {
                let shot =
                    ios::screenshot(self.runner.as_ref(), &disc.ios_tools, udid, format).await;
                if let Ok(ref data) = shot {
                    self.update_ios_orientation_from_frame(udid, data.width, data.height)
                        .await;
                }
                shot
            }
            BootedTarget::Android { serial, .. } => {
                android::screenshot(self.runner.as_ref(), &disc.android_tools, serial).await
            }
        };
        if res.is_err() {
            self.invalidate_device_cache().await;
        }
        res
    }

    pub async fn send_input(
        &self,
        raw_id: &str,
        action: &InputAction,
    ) -> otto_core::Result<ActionOkResp> {
        let target = self.require_booted_target(raw_id).await?;
        self.send_input_target(&target, action).await
    }

    pub(crate) async fn send_input_target(
        &self,
        target: &BootedTarget,
        action: &InputAction,
    ) -> otto_core::Result<ActionOkResp> {
        let disc = self.discovery(false).await;
        let res = match target {
            BootedTarget::Ios { dev, udid } => {
                let orientation = self
                    .ios_orientations
                    .read()
                    .await
                    .get(udid)
                    .copied()
                    .unwrap_or(dev.orientation);
                ios::send_input(
                    self.runner.as_ref(),
                    &disc.ios_tools,
                    udid,
                    dev.screen.as_ref(),
                    orientation,
                    action,
                )
                .await
            }
            BootedTarget::Android { dev, serial } => {
                android::send_input(
                    self.runner.as_ref(),
                    &disc.android_tools,
                    serial,
                    dev.screen.as_ref(),
                    dev.orientation,
                    action,
                )
                .await
            }
        };
        if res.is_err() {
            self.invalidate_device_cache().await;
        }
        res
    }

    pub async fn set_orientation(
        &self,
        raw_id: &str,
        orientation: Orientation,
    ) -> otto_core::Result<ActionOkResp> {
        let target = self.require_booted_target(raw_id).await?;
        let disc = self.discovery(false).await;
        match target {
            BootedTarget::Ios { udid, .. } => {
                let res =
                    ios::set_orientation(self.runner.as_ref(), &disc.ios_tools, &udid, orientation)
                        .await?;
                self.record_ios_orientation(&udid, orientation).await;
                Ok(res)
            }
            BootedTarget::Android { serial, .. } => {
                let res = android::set_orientation(
                    self.runner.as_ref(),
                    &disc.android_tools,
                    &serial,
                    orientation,
                )
                .await?;
                self.invalidate_device_cache().await;
                Ok(res)
            }
        }
    }

    pub async fn set_appearance(
        &self,
        raw_id: &str,
        appearance: Appearance,
    ) -> otto_core::Result<ActionOkResp> {
        let target = self.require_booted_target(raw_id).await?;
        let disc = self.discovery(false).await;
        match target {
            BootedTarget::Ios { udid, .. } => {
                ios::set_appearance(self.runner.as_ref(), &disc.ios_tools, &udid, appearance).await
            }
            BootedTarget::Android { serial, .. } => {
                android::set_appearance(
                    self.runner.as_ref(),
                    &disc.android_tools,
                    &serial,
                    appearance,
                )
                .await
            }
        }
    }

    pub async fn set_status_bar(
        &self,
        raw_id: &str,
        req: &StatusBarReq,
    ) -> otto_core::Result<ActionOkResp> {
        let udid = self
            .require_booted_ios(
                raw_id,
                "status bar override is only supported on iOS Simulator",
            )
            .await?;
        let disc = self.discovery(false).await;
        ios::set_status_bar(self.runner.as_ref(), &disc.ios_tools, &udid, req).await
    }

    pub async fn get_clipboard(&self, raw_id: &str) -> otto_core::Result<ClipboardResp> {
        let udid = self
            .require_booted_ios(raw_id, "clipboard read is only supported on iOS Simulator")
            .await?;
        let disc = self.discovery(false).await;
        ios::get_clipboard(self.runner.as_ref(), &disc.ios_tools, &udid).await
    }

    pub async fn set_clipboard(&self, raw_id: &str, text: &str) -> otto_core::Result<ActionOkResp> {
        let udid = self
            .require_booted_ios(raw_id, "clipboard write is only supported on iOS Simulator")
            .await?;
        let disc = self.discovery(false).await;
        ios::set_clipboard(self.runner.as_ref(), &disc.ios_tools, &udid, text).await
    }

    pub async fn install_app(&self, raw_id: &str, path: &str) -> otto_core::Result<ActionOkResp> {
        let target = self.require_booted_target(raw_id).await?;
        let disc = self.discovery(false).await;
        match target {
            BootedTarget::Ios { udid, .. } => {
                ios::install_app(
                    self.runner.as_ref(),
                    self.probe.as_ref(),
                    &disc.ios_tools,
                    &udid,
                    path,
                )
                .await
            }
            BootedTarget::Android { serial, .. } => {
                android::install_app(
                    self.runner.as_ref(),
                    self.probe.as_ref(),
                    &disc.android_tools,
                    &serial,
                    path,
                )
                .await
            }
        }
    }

    pub async fn launch_app(
        &self,
        raw_id: &str,
        app_id: &str,
        args: &[String],
    ) -> otto_core::Result<ActionOkResp> {
        let target = self.require_booted_target(raw_id).await?;
        let disc = self.discovery(false).await;
        match target {
            BootedTarget::Ios { udid, .. } => {
                ios::launch_app(self.runner.as_ref(), &disc.ios_tools, &udid, app_id, args).await
            }
            BootedTarget::Android { serial, .. } => {
                android::launch_app(self.runner.as_ref(), &disc.android_tools, &serial, app_id)
                    .await
            }
        }
    }

    pub async fn terminate_app(
        &self,
        raw_id: &str,
        app_id: &str,
    ) -> otto_core::Result<ActionOkResp> {
        let target = self.require_booted_target(raw_id).await?;
        let disc = self.discovery(false).await;
        match target {
            BootedTarget::Ios { udid, .. } => {
                ios::terminate_app(self.runner.as_ref(), &disc.ios_tools, &udid, app_id).await
            }
            BootedTarget::Android { serial, .. } => {
                android::terminate_app(self.runner.as_ref(), &disc.android_tools, &serial, app_id)
                    .await
            }
        }
    }

    pub async fn list_ports(&self, raw_id: &str) -> otto_core::Result<Vec<PortForward>> {
        match self.require_booted_target(raw_id).await? {
            BootedTarget::Ios { .. } => Ok(Vec::new()),
            BootedTarget::Android { serial, .. } => {
                let disc = self.discovery(false).await;
                android::list_port_forwards(self.runner.as_ref(), &disc.android_tools, &serial)
                    .await
            }
        }
    }

    pub async fn add_port_forward(
        &self,
        raw_id: &str,
        remote_port: u16,
        local_port: u16,
    ) -> otto_core::Result<PortForward> {
        let serial = self
            .require_booted_android(
                raw_id,
                "port forwarding (adb reverse) is only needed on Android; iOS Simulator reaches localhost directly",
            )
            .await?;
        let disc = self.discovery(false).await;
        android::add_port_forward(
            self.runner.as_ref(),
            &disc.android_tools,
            &serial,
            remote_port,
            local_port,
        )
        .await
    }

    pub async fn remove_port_forward(
        &self,
        raw_id: &str,
        remote_port: u16,
    ) -> otto_core::Result<()> {
        let serial = self
            .require_booted_android(raw_id, "port forwarding is only supported on Android")
            .await?;
        let disc = self.discovery(false).await;
        android::remove_port_forward(
            self.runner.as_ref(),
            &disc.android_tools,
            &serial,
            remote_port,
        )
        .await
    }

    pub async fn spawn_logs(&self, raw_id: &str) -> otto_core::Result<LogStreamHandle> {
        let target = self.require_booted_target(raw_id).await?;
        let disc = self.discovery(false).await;
        match target {
            BootedTarget::Ios { udid, .. } => {
                ios::spawn_logs(self.runner.as_ref(), &disc.ios_tools, &udid).await
            }
            BootedTarget::Android { serial, .. } => {
                android::spawn_logs(self.runner.as_ref(), &disc.android_tools, &serial).await
            }
        }
    }

    pub async fn recent_logs(
        &self,
        raw_id: &str,
        filter: Option<&str>,
        min_level: Option<crate::ws::LogLevel>,
        max_lines: usize,
    ) -> otto_core::Result<crate::ws::RecentLogsResp> {
        let target = self.require_booted_target(raw_id).await?;
        let device_id = target.id().to_string();
        let disc = self.discovery(false).await;
        let raw = match target {
            BootedTarget::Ios { udid, .. } => {
                let debug =
                    min_level.is_some_and(|l| l.rank() <= crate::ws::LogLevel::Debug.rank());
                ios::recent_logs(self.runner.as_ref(), &disc.ios_tools, &udid, debug).await?
            }
            BootedTarget::Android { serial, .. } => {
                android::recent_logs(
                    self.runner.as_ref(),
                    &disc.android_tools,
                    &serial,
                    max_lines,
                )
                .await?
            }
        };
        let (lines, truncated) =
            crate::ws::parse_and_filter_logs(&raw, filter, min_level, max_lines);
        Ok(crate::ws::RecentLogsResp {
            device_id,
            lines,
            truncated,
        })
    }

    pub(crate) async fn resolve_booted_target(
        &self,
        raw_id: &str,
    ) -> otto_core::Result<BootedTarget> {
        self.require_booted_target(raw_id).await
    }

    async fn require_booted_target(&self, raw_id: &str) -> otto_core::Result<BootedTarget> {
        let dev = self.get_device(raw_id).await?;
        if !dev.state.is_live() {
            return Err(otto_core::Error::Conflict(format!(
                "simulator {} ({}) is not booted (state: {:?})",
                dev.name, dev.id, dev.state
            )));
        }
        match dev.platform {
            Platform::Ios => {
                let udid = dev
                    .udid
                    .clone()
                    .ok_or_else(|| otto_core::Error::Invalid("missing iOS UDID".into()))?;
                Ok(BootedTarget::Ios { dev, udid })
            }
            Platform::Android => {
                let serial = dev.serial.clone().ok_or_else(|| {
                    otto_core::Error::Conflict("Android device has no active ADB serial".into())
                })?;
                Ok(BootedTarget::Android { dev, serial })
            }
        }
    }

    async fn require_booted_ios(&self, raw_id: &str, msg: &str) -> otto_core::Result<String> {
        match self.require_booted_target(raw_id).await? {
            BootedTarget::Ios { udid, .. } => Ok(udid),
            BootedTarget::Android { .. } => Err(otto_core::Error::Invalid(msg.into())),
        }
    }

    async fn require_booted_android(&self, raw_id: &str, msg: &str) -> otto_core::Result<String> {
        match self.require_booted_target(raw_id).await? {
            BootedTarget::Android { serial, .. } => Ok(serial),
            BootedTarget::Ios { .. } => Err(otto_core::Error::Invalid(msg.into())),
        }
    }
}

fn find_matching_device<'a>(devices: &'a [SimDevice], id: &DeviceId) -> Option<&'a SimDevice> {
    let canonical = id.as_canonical();
    if let Some(d) = devices.iter().find(|d| d.id == canonical) {
        return Some(d);
    }
    if let DeviceId::AndroidSerial(serial) = id {
        return devices
            .iter()
            .find(|d| d.serial.as_deref() == Some(serial.as_str()));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::fake::{ok_out, FakeCommandRunner, FakeHostProbe};

    fn fake_png_bytes(w: u32, h: u32) -> Vec<u8> {
        let mut png = Vec::new();
        png.extend_from_slice(b"\x89PNG\r\n\x1a\n");
        png.extend_from_slice(&13u32.to_be_bytes());
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&w.to_be_bytes());
        png.extend_from_slice(&h.to_be_bytes());
        png.extend_from_slice(&[8, 6, 0, 0, 0]);
        png
    }

    fn setup_ios_service(w: u32, h: u32) -> (Arc<FakeCommandRunner>, SimulatorsService) {
        let runner = Arc::new(FakeCommandRunner::new());
        let probe = Arc::new(FakeHostProbe::new());
        probe.add_path("/Applications/Xcode.app");
        probe.add_which("xcrun", "/usr/bin/xcrun");
        probe.add_which("idb", "/opt/homebrew/bin/idb");
        runner.on_exact(
            "xcode-select",
            &["-p"],
            ok_out(b"/Applications/Xcode.app/Contents/Developer\n"),
        );
        runner.on_exact("xcodebuild", &["-version"], ok_out(b"Xcode 16.2\n"));
        runner.on_exact(
            "xcrun",
            &["simctl", "list", "runtimes", "-j"],
            ok_out(
                br#"{"runtimes":[{"identifier":"com.apple.CoreSimulator.SimRuntime.iOS-18-2","name":"iOS 18.2","isAvailable":true}]}"#,
            ),
        );
        runner.on_exact(
            "xcrun",
            &["simctl", "list", "devices", "-j"],
            ok_out(
                br#"{"devices":{"com.apple.CoreSimulator.SimRuntime.iOS-18-2":[{"udid":"IOS-1","name":"iPhone 16 Pro","state":"Booted","isAvailable":true}]}}"#,
            ),
        );
        runner.on_exact(
            "xcrun",
            &["simctl", "list", "-j"],
            ok_out(
                br#"{"runtimes":[{"identifier":"com.apple.CoreSimulator.SimRuntime.iOS-18-2","name":"iOS 18.2","version":"18.2","isAvailable":true}],"devicetypes":[],"devices":{"com.apple.CoreSimulator.SimRuntime.iOS-18-2":[{"udid":"IOS-1","name":"iPhone 16 Pro","state":"Booted","isAvailable":true,"deviceTypeIdentifier":"com.apple.CoreSimulator.SimDeviceType.iPhone-16-Pro"}]}}"#,
            ),
        );
        runner.on(move |spec| {
            if spec.program.ends_with("xcrun")
                && spec.args.len() == 6
                && spec.args[0] == "simctl"
                && spec.args[1] == "io"
                && spec.args[2] == "IOS-1"
                && spec.args[3] == "screenshot"
            {
                std::fs::write(&spec.args[5], fake_png_bytes(w, h)).unwrap();
                Some(ok_out(b""))
            } else {
                None
            }
        });
        let svc = SimulatorsService::with_deps(runner.clone(), probe);
        (runner, svc)
    }

    #[tokio::test]
    async fn tracks_ios_orientation_across_set_orientation_screenshots_and_send_input() {
        let (runner, svc) = setup_ios_service(2556, 1179);
        runner.on(|spec| {
            if spec.program == "osascript" {
                Some(ok_out(b""))
            } else {
                None
            }
        });
        // iPhone 16 Pro portrait screen is 393x852 → landscape is 852x393.
        // Normalized (0.5, 0.25) in landscape => (426, 98).
        runner.on_exact(
            "idb",
            &["ui", "tap", "--udid", "IOS-1", "426", "98"],
            ok_out(b""),
        );

        svc.set_orientation("ios:IOS-1", Orientation::Landscape)
            .await
            .unwrap();
        let dev = svc.get_device("ios:IOS-1").await.unwrap();
        assert_eq!(dev.orientation, Orientation::Landscape);

        svc.send_input(
            "ios:IOS-1",
            &InputAction::Tap {
                x: 0.5,
                y: 0.25,
                normalized: true,
            },
        )
        .await
        .unwrap();

        // Also verify landscape screenshot dimensions update tracked orientation from scratch.
        let (runner2, svc2) = setup_ios_service(2556, 1179);
        runner2.on_exact(
            "idb",
            &["ui", "tap", "--udid", "IOS-1", "426", "98"],
            ok_out(b""),
        );
        svc2.screenshot("ios:IOS-1", ScreenshotFormat::Png)
            .await
            .unwrap();
        let dev2 = svc2.get_device("ios:IOS-1").await.unwrap();
        assert_eq!(dev2.orientation, Orientation::Landscape);
        svc2.send_input(
            "ios:IOS-1",
            &InputAction::Tap {
                x: 0.5,
                y: 0.25,
                normalized: true,
            },
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn ten_frames_use_cached_target_without_per_frame_device_reenumeration() {
        let (runner, svc) = setup_ios_service(1179, 2556);
        let target = svc.resolve_booted_target("ios:IOS-1").await.unwrap();
        runner.clear_recorded();

        for _ in 0..10 {
            svc.screenshot_target(&target, ScreenshotFormat::Png)
                .await
                .unwrap();
        }

        let recorded = runner.recorded();
        let screenshot_calls = recorded
            .iter()
            .filter(|(_, args)| args.get(1).map(String::as_str) == Some("io"))
            .count();
        let list_calls = recorded
            .iter()
            .filter(|(_, args)| args.get(1).map(String::as_str) == Some("list"))
            .count();
        assert_eq!(screenshot_calls, 10);
        assert_eq!(
            list_calls, 0,
            "10 screencast frames must not re-enumerate devices per frame"
        );
    }
}
