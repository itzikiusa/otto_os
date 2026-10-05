use crate::{now_seconds, ResourcePoint};
use std::collections::BTreeMap;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

struct DesktopProcess {
    name: String,
    pid: u32,
    start_time: u64,
}

pub(crate) struct Sampler {
    system: System,
    sampled_pids: std::collections::BTreeSet<(u32, u64)>,
    desktop_processes: Vec<DesktopProcess>,
    last_discovery: Option<std::time::Instant>,
}
impl Sampler {
    pub fn new() -> Self {
        Self {
            system: System::new(),
            sampled_pids: Default::default(),
            desktop_processes: Vec::new(),
            last_discovery: None,
        }
    }
    pub fn sample(&mut self, processes: &[(String, u32)]) -> Vec<ResourcePoint> {
        if self
            .last_discovery
            .is_none_or(|last| last.elapsed() >= std::time::Duration::from_secs(30))
        {
            self.desktop_processes = desktop_processes();
            self.last_discovery = Some(std::time::Instant::now());
        }
        let processes: Vec<_> = processes
            .iter()
            .cloned()
            .chain(
                self.desktop_processes
                    .iter()
                    .map(|p| (p.name.clone(), p.pid)),
            )
            .collect();
        let pids: Vec<Pid> = processes.iter().map(|(_, id)| Pid::from_u32(*id)).collect();
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&pids),
            true,
            ProcessRefreshKind::nothing()
                .with_cpu()
                .with_memory()
                .without_tasks(),
        );
        let time = now_seconds();
        let mut points = Vec::new();
        for (name, pid) in &processes {
            if let Some(process) = self.system.process(Pid::from_u32(*pid)) {
                if self
                    .desktop_processes
                    .iter()
                    .any(|cached| cached.pid == *pid && cached.start_time != process.start_time())
                {
                    continue;
                }
                points.push(ResourcePoint {
                    timestamp: time,
                    process: name.clone(),
                    cpu_percent: self
                        .sampled_pids
                        .contains(&(*pid, process.start_time()))
                        .then_some(process.cpu_usage() as f64),
                    rss_mb: Some(process.memory() as f64 / 1048576.0),
                    host_load: None,
                });
            }
        }
        let mut grouped: BTreeMap<String, ResourcePoint> = BTreeMap::new();
        for point in points {
            let existing = grouped
                .entry(point.process.clone())
                .or_insert(ResourcePoint {
                    timestamp: time,
                    process: point.process,
                    cpu_percent: None,
                    rss_mb: None,
                    host_load: None,
                });
            if let Some(cpu) = point.cpu_percent {
                existing.cpu_percent = Some(existing.cpu_percent.unwrap_or_default() + cpu);
            }
            if let Some(rss) = point.rss_mb {
                existing.rss_mb = Some(existing.rss_mb.unwrap_or_default() + rss);
            }
        }
        let mut points: Vec<_> = grouped.into_values().collect();
        points.push(ResourcePoint {
            timestamp: time,
            process: "host".into(),
            cpu_percent: None,
            rss_mb: None,
            host_load: Some(System::load_average().one),
        });
        self.sampled_pids = processes
            .iter()
            .filter_map(|(_, pid)| {
                self.system
                    .process(Pid::from_u32(*pid))
                    .map(|p| (*pid, p.start_time()))
            })
            .collect();
        points
    }
}
#[derive(Default)]
pub(crate) struct SpikeDetector {
    previous: BTreeMap<String, ResourcePoint>,
    high: BTreeMap<String, u32>,
    last: BTreeMap<String, i64>,
}
impl SpikeDetector {
    pub fn observe(&mut self, p: &ResourcePoint, c: &crate::TelemetryConfig) -> bool {
        let high = p.cpu_percent.is_some_and(|v| v >= c.cpu_spike_percent);
        let sustained = self.high.entry(p.process.clone()).or_default();
        *sustained = if high { sustained.saturating_add(1) } else { 0 };
        let jump = self
            .previous
            .get(&p.process)
            .and_then(|old| Some(p.rss_mb? - old.rss_mb?))
            .is_some_and(|delta| delta >= c.rss_spike_mb);
        let spike = (*sustained >= 2 || jump)
            && self
                .last
                .get(&p.process)
                .is_none_or(|last| p.timestamp - *last >= 60);
        self.previous.insert(p.process.clone(), p.clone());
        if spike {
            self.last.insert(p.process.clone(), p.timestamp);
        }
        spike
    }
}
/// Keep only function symbols from sample's call tree. Headers, image lists,
/// absolute paths, addresses, source locations and raw text are never returned.
pub(crate) fn sanitize_profile(text: &str) -> Vec<String> {
    let mut frames = Vec::new();
    for line in text.lines() {
        if line.starts_with("Binary Images:") {
            break;
        }
        let prefix = line
            .chars()
            .take_while(|c| c.is_whitespace() || "+|!:".contains(*c))
            .take(64)
            .collect::<String>();
        let line = line.trim_start_matches(|c: char| c.is_whitespace() || "+|!:".contains(c));
        let Some((count, rest)) = line.split_once(' ') else {
            continue;
        };
        if count.parse::<u64>().is_err() {
            continue;
        }
        let symbol = rest.trim().split(" (in ").next().unwrap_or("").trim();
        if symbol.is_empty()
            || symbol.len() > 200
            || symbol.contains('/')
            || symbol.contains("0x")
            || symbol.contains('[')
            || symbol.contains('"')
        {
            continue;
        }
        if !symbol
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_:<>~*&.,() -+".contains(c))
        {
            continue;
        }
        // Preserve weights and tree position: repeated symbols on different
        // branches are meaningful evidence, not duplicates to discard.
        frames.push(format!("{prefix}{count} {symbol}"));
        if frames.len() == 2000 {
            break;
        }
    }
    frames
}

#[cfg(not(target_os = "macos"))]
fn desktop_processes() -> Vec<DesktopProcess> {
    Vec::new()
}

#[cfg(target_os = "macos")]
fn desktop_processes() -> Vec<DesktopProcess> {
    use sysinfo::UpdateKind;
    // Development/isolated daemons cannot establish ownership of the installed
    // app. They expose absent UI samples rather than claiming its measurements.
    let deployed = std::env::current_exe().ok().is_some_and(|path| {
        path.ends_with("Otto/bin/ottod") || path.ends_with("Otto.app/Contents/MacOS/ottod")
    });
    if !deployed {
        return Vec::new();
    }
    let mut discover = System::new();
    discover.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_exe(UpdateKind::Always)
            .with_user(UpdateKind::Always)
            .without_tasks(),
    );
    let me = Pid::from_u32(std::process::id());
    let Some(uid) = discover.process(me).and_then(|p| p.user_id()) else {
        return Vec::new();
    };
    let mut desktops = Vec::new();
    for (pid, process) in discover.processes() {
        if process.user_id() == Some(uid)
            && process
                .exe()
                .is_some_and(|path| path.ends_with("Otto.app/Contents/MacOS/otto-desktop"))
        {
            if let Some(coalition) = resource_coalition(pid.as_u32()) {
                desktops.push((*pid, coalition));
            }
        }
    }
    let mut owned = Vec::new();
    for (pid, process) in discover.processes() {
        if owned.len() >= 16 {
            break;
        }
        if process.user_id() != Some(uid) {
            continue;
        }
        if desktops.iter().any(|(desktop, _)| desktop == pid) {
            owned.push(DesktopProcess {
                name: "desktop".into(),
                pid: pid.as_u32(),
                start_time: process.start_time(),
            });
            continue;
        }
        let Some(path) = process.exe() else { continue };
        if !path.starts_with("/System/Library/Frameworks/WebKit.framework/") {
            continue;
        }
        let label = match path.file_name().and_then(|s| s.to_str()) {
            Some(
                "com.apple.WebKit.WebContent" | "com.apple.WebKit.WebContent.EnhancedSecurity",
            ) => "ui-webcontent",
            Some("com.apple.WebKit.GPU") => "ui-gpu",
            Some("com.apple.WebKit.Networking") => "ui-network",
            _ => continue,
        };
        if resource_coalition(pid.as_u32())
            .is_some_and(|id| desktops.iter().any(|(_, owned)| *owned == id))
        {
            owned.push(DesktopProcess {
                name: label.into(),
                pid: pid.as_u32(),
                start_time: process.start_time(),
            });
        }
    }
    owned
}

#[cfg(target_os = "macos")]
fn resource_coalition(pid: u32) -> Option<u64> {
    // Apple xnu bsd/sys/proc_info_private.h: two coalition IDs plus three
    // reserved u64s. Resource coalition is index 0; failed/changed ABI = absent.
    #[link(name = "proc")]
    unsafe extern "C" {
        fn proc_pidinfo(
            pid: i32,
            flavor: i32,
            arg: u64,
            buffer: *mut std::ffi::c_void,
            size: i32,
        ) -> i32;
    }
    let mut info = [0u64; 5];
    // SAFETY: the live writable buffer is exactly the advertised 40 bytes.
    let size = unsafe {
        proc_pidinfo(
            pid as i32,
            20,
            0,
            info.as_mut_ptr().cast(),
            std::mem::size_of_val(&info) as i32,
        )
    };
    (size == std::mem::size_of_val(&info) as i32 && info[0] != 0).then_some(info[0])
}
