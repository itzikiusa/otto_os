//! Host + process telemetry sampling via `sysinfo` (CPU, RAM, load, the ottod
//! process's own RSS/CPU). One sample maps to a `system_metrics` row.

use sysinfo::{Pid, ProcessesToUpdate, System};

/// A single point-in-time sample. Fields mirror the `system_metrics` columns.
#[derive(Debug, Clone, Default)]
pub struct Metric {
    pub cpu_pct: f64,
    pub mem_used_mb: f64,
    pub mem_total_mb: f64,
    pub mem_pct: f64,
    pub load_avg_1: f64,
    pub process_rss_mb: f64,
    pub process_cpu_pct: f64,
    pub active_sessions: u32,
}

/// Holds a `sysinfo::System` and the current process id. `sample` is blocking
/// (it sleeps a CPU-refresh interval to compute a real %), so callers run it on
/// a blocking thread.
pub struct MetricsSampler {
    sys: System,
    pid: Option<Pid>,
}

impl Default for MetricsSampler {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsSampler {
    pub fn new() -> Self {
        Self {
            sys: System::new(),
            pid: sysinfo::get_current_pid().ok(),
        }
    }

    /// Hostname (for the `host` column / multi-host dashboards).
    pub fn host() -> String {
        System::host_name().unwrap_or_else(|| "localhost".to_string())
    }

    /// Take one sample. CPU % needs two refreshes a short interval apart, so
    /// this sleeps `MINIMUM_CPU_UPDATE_INTERVAL` (~200ms) — **blocking**.
    pub fn sample(&mut self, active_sessions: u32) -> Metric {
        self.sys.refresh_cpu_usage();
        std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
        self.sys.refresh_cpu_usage();
        self.sys.refresh_memory();
        if let Some(pid) = self.pid {
            self.sys
                .refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
        }

        let mb = 1024.0 * 1024.0;
        let total = self.sys.total_memory() as f64;
        let used = self.sys.used_memory() as f64;
        let (rss, pcpu) = self
            .pid
            .and_then(|pid| self.sys.process(pid))
            .map(|p| (p.memory() as f64 / mb, p.cpu_usage() as f64))
            .unwrap_or((0.0, 0.0));

        Metric {
            cpu_pct: self.sys.global_cpu_usage() as f64,
            mem_used_mb: used / mb,
            mem_total_mb: total / mb,
            mem_pct: if total > 0.0 { used / total * 100.0 } else { 0.0 },
            load_avg_1: System::load_average().one,
            process_rss_mb: rss,
            process_cpu_pct: pcpu,
            active_sessions,
        }
    }
}
