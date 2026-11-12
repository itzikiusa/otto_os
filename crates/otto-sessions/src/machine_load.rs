//! How busy the machine is right now: 1-minute load average, logical CPU
//! count, macOS memory pressure and swap in use. The admission gate
//! ([`crate::admission`]) reads it before it lets an engine-owned background
//! agent start, so a review fan-out next to a nightly job no longer drives an
//! 8-core / 16 GB Mac to a load of 60 and 22 GB of swap (2026-11-12).
//!
//! Every reading is a syscall (`getloadavg`, `sysctlbyname`) — no process is
//! spawned — and [`MachineMonitor`] caches one sample for [`SAMPLE_TTL`] so a
//! queue of waiters polling together costs one read. The sampler is a trait
//! so the decision code is unit-tested with fixed samples.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;

/// How long one sample is reused. The load average itself is a 5-second
/// kernel EWMA, so sampling faster than this only re-reads the same number.
pub const SAMPLE_TTL: Duration = Duration::from_secs(2);

/// macOS memory pressure (`kern.memorystatus_vm_pressure_level`: 1 normal,
/// 2 warn, 4 critical). `Unknown` off macOS or when the read fails — it never
/// holds anything back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryPressure {
    Unknown,
    Normal,
    Warn,
    Critical,
}

impl MemoryPressure {
    /// The kernel's level number → pressure (anything unexpected is `Unknown`).
    pub fn from_level(level: i32) -> Self {
        match level {
            1 => Self::Normal,
            2 => Self::Warn,
            4 => Self::Critical,
            _ => Self::Unknown,
        }
    }

    /// Warn or worse: the system is compressing/swapping to keep up.
    pub fn is_elevated(self) -> bool {
        matches!(self, Self::Warn | Self::Critical)
    }
}

/// One reading of the machine.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MachineSample {
    /// 1-minute load average; `None` when unreadable.
    pub load1: Option<f64>,
    /// Logical CPUs (≥ 1).
    pub cores: usize,
    pub pressure: MemoryPressure,
    /// Swap in use, bytes (macOS `vm.swapusage`); `None` elsewhere.
    pub swap_used_bytes: Option<u64>,
    /// When it was taken (drives the cache).
    pub taken: Instant,
}

impl MachineSample {
    /// Load per logical core (0 when the load is unknown).
    pub fn load_per_core(&self) -> f64 {
        self.load1.unwrap_or(0.0) / self.cores.max(1) as f64
    }
}

/// Source of samples — the OS in production, a fixed value in tests.
pub trait Sampler: Send + Sync + 'static {
    fn sample(&self) -> MachineSample;
}

/// Logical CPU count of this machine (≥ 1).
pub fn logical_cores() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

/// The real machine.
#[derive(Debug, Default, Clone, Copy)]
pub struct OsSampler;

impl Sampler for OsSampler {
    fn sample(&self) -> MachineSample {
        MachineSample {
            load1: os::load1(),
            cores: logical_cores(),
            pressure: os::pressure(),
            swap_used_bytes: os::swap_used(),
            taken: Instant::now(),
        }
    }
}

#[cfg(target_os = "macos")]
mod os {
    use super::MemoryPressure;
    use std::ffi::CStr;

    pub fn load1() -> Option<f64> {
        let mut avg = [0f64; 3];
        // SAFETY: `avg` holds 3 doubles and we ask for at most 3.
        let n = unsafe { libc::getloadavg(avg.as_mut_ptr(), 3) };
        (n >= 1 && avg[0].is_finite()).then_some(avg[0])
    }

    /// `sysctlbyname` into a plain-old-data `T`; `None` unless the kernel
    /// wrote exactly `size_of::<T>()` bytes.
    fn sysctl<T: Copy>(name: &CStr, mut value: T) -> Option<T> {
        let mut len = std::mem::size_of::<T>();
        // SAFETY: `value` is a valid, writable `T` of `len` bytes for the
        // duration of the call; no new value is written (null/0).
        let rc = unsafe {
            libc::sysctlbyname(
                name.as_ptr(),
                (&mut value as *mut T).cast(),
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        (rc == 0 && len == std::mem::size_of::<T>()).then_some(value)
    }

    pub fn pressure() -> MemoryPressure {
        sysctl::<libc::c_int>(c"kern.memorystatus_vm_pressure_level", 0)
            .map(MemoryPressure::from_level)
            .unwrap_or(MemoryPressure::Unknown)
    }

    pub fn swap_used() -> Option<u64> {
        // SAFETY: `xsw_usage` is plain integers; all-zero is a valid value.
        let zero: libc::xsw_usage = unsafe { std::mem::zeroed() };
        sysctl(c"vm.swapusage", zero).map(|x| x.xsu_used)
    }
}

#[cfg(not(target_os = "macos"))]
mod os {
    use super::MemoryPressure;

    /// Linux exposes the load average as text; anything else reports none.
    pub fn load1() -> Option<f64> {
        std::fs::read_to_string("/proc/loadavg")
            .ok()
            .and_then(|s| s.split_whitespace().next()?.parse().ok())
    }

    pub fn pressure() -> MemoryPressure {
        MemoryPressure::Unknown
    }

    pub fn swap_used() -> Option<u64> {
        None
    }
}

/// Setting (number of GiB, default [`DEFAULT_LOW_DISK_GB`]; `0` = never):
/// below this much free space on the data volume the status chip turns to
/// warning and one notice is raised. Nothing is held back on it.
pub const LOW_DISK_SETTING: &str = "machine_low_disk_gb";
/// The owner keeps 30 GB free at all times: builds and swap fail hard below.
pub const DEFAULT_LOW_DISK_GB: f64 = 30.0;
const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

/// `machine_low_disk_gb`; anything missing, negative or not a number takes
/// the default (a typo never silences the warning).
pub fn low_disk_gb(value: Option<&serde_json::Value>) -> f64 {
    value
        .and_then(serde_json::Value::as_f64)
        .filter(|v| v.is_finite() && *v >= 0.0)
        .unwrap_or(DEFAULT_LOW_DISK_GB)
}

/// Free space below the floor (`floor_gb` 0 = off; unknown space never is).
pub fn is_low_disk(free_bytes: Option<u64>, floor_gb: f64) -> bool {
    floor_gb > 0.0 && free_bytes.is_some_and(|b| (b as f64) < floor_gb * GIB)
}

/// Bytes available to this (unprivileged) user on the volume holding
/// `path` — what `df` reports as "Avail". A `statfs` syscall, no process. A
/// path not created yet is measured at its nearest existing ancestor (the
/// same volume); `None` only when nothing on the way can be read.
pub fn disk_free_bytes(path: &std::path::Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    path.ancestors().find_map(|p| {
        let c = std::ffi::CString::new(p.as_os_str().as_bytes()).ok()?;
        disk::free(&c)
    })
}

#[cfg(target_os = "macos")]
mod disk {
    /// macOS `statfs` (64-bit counts; `statvfs` there truncates to 32 bits).
    pub fn free(path: &std::ffi::CStr) -> Option<u64> {
        // SAFETY: all-zero is a valid `statfs` (integers and byte arrays).
        let mut st: libc::statfs = unsafe { std::mem::zeroed() };
        // SAFETY: `path` is NUL-terminated; `st` is a valid, writable struct.
        let rc = unsafe { libc::statfs(path.as_ptr(), &mut st) };
        (rc == 0).then_some(st.f_bavail.saturating_mul(u64::from(st.f_bsize)))
    }
}

#[cfg(not(target_os = "macos"))]
mod disk {
    #[allow(clippy::unnecessary_cast)]
    pub fn free(path: &std::ffi::CStr) -> Option<u64> {
        // SAFETY: all-zero is a valid `statvfs` (plain integers).
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        // SAFETY: `path` is NUL-terminated; `st` is a valid, writable struct.
        let rc = unsafe { libc::statvfs(path.as_ptr(), &mut st) };
        (rc == 0).then_some((st.f_bavail as u64).saturating_mul(st.f_frsize as u64))
    }
}

/// A [`Sampler`] behind a [`SAMPLE_TTL`] cache.
pub struct MachineMonitor {
    sampler: Arc<dyn Sampler>,
    ttl: Duration,
    last: Mutex<Option<MachineSample>>,
}

impl MachineMonitor {
    pub fn new(sampler: Arc<dyn Sampler>) -> Self {
        Self {
            sampler,
            ttl: SAMPLE_TTL,
            last: Mutex::new(None),
        }
    }

    /// The OS-backed monitor the daemon uses.
    pub fn os() -> Self {
        Self::new(Arc::new(OsSampler))
    }

    /// The cached sample when it is younger than the TTL at `now`.
    pub fn cached(&self, now: Instant) -> Option<MachineSample> {
        let last = *self.last.lock().unwrap_or_else(|e| e.into_inner());
        last.filter(|s| now.saturating_duration_since(s.taken) < self.ttl)
    }

    fn store(&self, sample: MachineSample) -> MachineSample {
        *self.last.lock().unwrap_or_else(|e| e.into_inner()) = Some(sample);
        sample
    }

    /// Synchronous read through the cache (tests; callers already off the
    /// async workers).
    pub fn sample_at(&self, now: Instant) -> MachineSample {
        match self.cached(now) {
            Some(s) => s,
            None => self.store(self.sampler.sample()),
        }
    }

    /// A fresh-enough sample. A cache miss reads the OS on the blocking pool
    /// so a slow `sysctl` can never stall an async worker.
    pub async fn current(&self) -> MachineSample {
        if let Some(s) = self.cached(Instant::now()) {
            return s;
        }
        let sampler = Arc::clone(&self.sampler);
        match tokio::task::spawn_blocking(move || sampler.sample()).await {
            Ok(s) => self.store(s),
            // The blocking pool is gone (runtime shutdown): read inline.
            Err(_) => self.store(self.sampler.sample()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Counting(AtomicUsize);
    impl Sampler for Counting {
        fn sample(&self) -> MachineSample {
            self.0.fetch_add(1, Ordering::SeqCst);
            MachineSample {
                load1: Some(3.0),
                cores: 4,
                pressure: MemoryPressure::Normal,
                swap_used_bytes: None,
                taken: Instant::now(),
            }
        }
    }

    #[test]
    fn pressure_levels_map_from_the_kernel_numbers() {
        assert_eq!(MemoryPressure::from_level(1), MemoryPressure::Normal);
        assert_eq!(MemoryPressure::from_level(2), MemoryPressure::Warn);
        assert_eq!(MemoryPressure::from_level(4), MemoryPressure::Critical);
        assert_eq!(MemoryPressure::from_level(0), MemoryPressure::Unknown);
        assert_eq!(MemoryPressure::from_level(3), MemoryPressure::Unknown);
        assert!(MemoryPressure::Warn.is_elevated());
        assert!(MemoryPressure::Critical.is_elevated());
        assert!(!MemoryPressure::Normal.is_elevated());
        assert!(!MemoryPressure::Unknown.is_elevated());
    }

    #[test]
    fn one_sample_is_reused_within_the_ttl_and_refreshed_after() {
        let sampler = Arc::new(Counting(AtomicUsize::new(0)));
        let monitor = MachineMonitor::new(sampler.clone());
        let first = monitor.sample_at(Instant::now());
        // Same instant the sample was taken, and just inside the TTL.
        monitor.sample_at(first.taken);
        monitor.sample_at(first.taken + SAMPLE_TTL - Duration::from_millis(1));
        assert_eq!(sampler.0.load(Ordering::SeqCst), 1, "served from cache");
        monitor.sample_at(first.taken + SAMPLE_TTL);
        assert_eq!(sampler.0.load(Ordering::SeqCst), 2, "stale → re-read");
    }

    #[test]
    fn load_per_core_treats_an_unknown_load_as_idle() {
        let s = MachineSample {
            load1: Some(12.0),
            cores: 8,
            pressure: MemoryPressure::Unknown,
            swap_used_bytes: None,
            taken: Instant::now(),
        };
        assert!((s.load_per_core() - 1.5).abs() < 1e-9);
        let unknown = MachineSample { load1: None, ..s };
        assert_eq!(unknown.load_per_core(), 0.0);
    }

    #[test]
    fn free_disk_space_is_read_from_the_volume_and_judged_against_the_floor() {
        let dir = tempfile::tempdir().unwrap();
        let free = disk_free_bytes(dir.path()).expect("statfs on a temp dir");
        assert!(free > 0);
        // Not created yet: measured on the volume it will live on.
        assert!(disk_free_bytes(&dir.path().join("not/yet")).is_some_and(|b| b > 0));
        assert_eq!(disk_free_bytes(std::path::Path::new("")), None);

        let gib = 1024 * 1024 * 1024;
        assert!(is_low_disk(Some(29 * gib), 30.0));
        assert!(
            !is_low_disk(Some(30 * gib), 30.0),
            "exactly at the floor is fine"
        );
        assert!(!is_low_disk(Some(gib), 0.0), "0 = no warning");
        assert!(!is_low_disk(None, 30.0), "unknown space never warns");

        assert_eq!(low_disk_gb(None), DEFAULT_LOW_DISK_GB);
        assert_eq!(low_disk_gb(Some(&serde_json::json!(50))), 50.0);
        assert_eq!(low_disk_gb(Some(&serde_json::json!(0))), 0.0);
        for bad in [
            serde_json::json!(-1),
            serde_json::json!("lots"),
            serde_json::json!(null),
        ] {
            assert_eq!(low_disk_gb(Some(&bad)), DEFAULT_LOW_DISK_GB, "{bad}");
        }
    }

    /// The real sampler reads without panicking and reports sane values.
    #[test]
    fn the_os_sampler_reads_this_machine() {
        let s = OsSampler.sample();
        assert!(s.cores >= 1);
        if let Some(l) = s.load1 {
            assert!(l >= 0.0 && l.is_finite());
        }
    }
}
