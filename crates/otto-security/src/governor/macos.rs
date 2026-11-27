use std::io;
use std::mem::{size_of, MaybeUninit};
use std::process::Stdio;
use std::time::{Duration, Instant};

use otto_sandbox::SandboxPolicy;
use tokio::process::Command;

use super::*;

const SAMPLE_INTERVAL: Duration = Duration::from_millis(100);
pub(super) const REPORT_LIMIT: u64 = 32 * 1024 * 1024;

struct Group(libc::pid_t);

impl Drop for Group {
    fn drop(&mut self) {
        // setpgid(0, 0) at spawn makes the leader PID the unique group ID.
        unsafe { libc::kill(-self.0, libc::SIGKILL) };
    }
}

pub(crate) async fn run(
    program: &Path,
    args: &[String],
    project: &Path,
    cache: &Path,
    output: &Path,
    settings: &ScanSettings,
    cancel: &CancellationToken,
) -> Result<ScanMetrics> {
    if cancel.is_cancelled() {
        return Err(cancelled());
    }
    let tool_dir = program
        .parent()
        .ok_or_else(|| Error::Internal("scanner executable has no parent directory".into()))?;
    let policy = SandboxPolicy::for_offline_scan(project, tool_dir, cache, output);
    let (launcher, args) = policy.wrap(&program.to_string_lossy(), args);
    let mut command = Command::new("/usr/sbin/taskpolicy");
    command
        .args(["-b", "/usr/bin/nice", "-n", "19", &launcher])
        .args(args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", output)
        .env("TMPDIR", output)
        .env("XDG_CACHE_HOME", output)
        .env("GOMAXPROCS", settings.parallelism.to_string())
        .env("GOMEMLIMIT", format!("{}MiB", settings.memory_mib / 2))
        .env("GOENV", "off")
        .env("GOPROXY", "off")
        .env("GOSUMDB", "off")
        .env("GOTOOLCHAIN", "local")
        .env("OPENGREP_SEND_METRICS", "off")
        .env("SEMGREP_SEND_METRICS", "off")
        .current_dir(output)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .kill_on_drop(true);

    let timeout = Duration::from_secs(u64::from(settings.timeout_secs));
    let cpu_limit = timeout.as_secs() * u64::from(settings.parallelism);
    // Only async-signal-safe calls run in the forked child before exec.
    unsafe {
        command.pre_exec(move || {
            limit(libc::RLIMIT_CPU, cpu_limit)?;
            limit(libc::RLIMIT_FSIZE, REPORT_LIMIT)
        });
    }

    let start = Instant::now();
    let mut child = command.spawn().map_err(|e| {
        tracing::warn!(error = %e, "could not spawn security scanner");
        Error::internal_public("Otto could not start the sandboxed scanner.")
    })?;
    let group = Group(child.id().expect("a newly spawned child has a PID") as libc::pid_t);
    let timebase = mach_nanos_per_tick();
    let mut metrics = ScanMetrics::default();
    let mut last = None;
    let mut tick = tokio::time::interval(SAMPLE_INTERVAL);
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);
    let result = loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break Err(cancelled()),
            _ = &mut deadline => {
                break Err(Error::internal_public("The scanner reached the scan time limit."));
            }
            status = child.wait() => {
                match status {
                    Ok(status) if status.success() => break Ok(()),
                    _ => break Err(Error::internal_public(
                        "The scanner could not finish this scan. Check the scan scope and resource limits.",
                    )),
                }
            }
            _ = tick.tick() => {
                let (rss, cpu_ticks) = group_usage(group.0);
                let now = Instant::now();
                metrics.peak_rss_bytes = metrics.peak_rss_bytes.max(rss);
                if let Some((before, cpu_before)) = last {
                    let elapsed = now.duration_since(before).as_secs_f64();
                    if elapsed > 0.0 {
                        let used = cpu_ticks.saturating_sub(cpu_before) as f64 * timebase / 1e9;
                        metrics.peak_cpu_percent = metrics.peak_cpu_percent.max(100.0 * used / elapsed);
                    }
                }
                last = Some((now, cpu_ticks));
                if rss > u64::from(settings.memory_mib) * 1024 * 1024 {
                    break Err(Error::internal_public("The scanner reached the scan memory limit."));
                }
            }
        }
    };

    drop(group);
    let _ = child.wait().await;
    result?;
    metrics.wall_ms = start.elapsed().as_millis() as u64;
    Ok(metrics)
}

fn limit(resource: libc::c_int, max: u64) -> io::Result<()> {
    let limits = libc::rlimit {
        rlim_cur: max,
        rlim_max: max,
    };
    if unsafe { libc::setrlimit(resource, &limits) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[allow(deprecated)] // One stable Darwin syscall via libc, already needed for proc_listpids.
fn mach_nanos_per_tick() -> f64 {
    // PROC_PIDTASKINFO reports CPU time in the Mach absolute clock's units.
    let mut timebase = MaybeUninit::<libc::mach_timebase_info_data_t>::uninit();
    let result = unsafe { libc::mach_timebase_info(timebase.as_mut_ptr()) };
    if result == libc::KERN_SUCCESS {
        let timebase = unsafe { timebase.assume_init() };
        f64::from(timebase.numer) / f64::from(timebase.denom)
    } else {
        0.0
    }
}

fn group_usage(group: libc::pid_t) -> (u64, u64) {
    // Darwin sys/proc_info.h: PROC_PGRP_ONLY and PROC_PIDTASKINFO.
    let mut pids = [0 as libc::pid_t; 64];
    let len = unsafe {
        libc::proc_listpids(
            2,
            group as u32,
            pids.as_mut_ptr().cast(),
            size_of_val(&pids) as i32,
        )
    };
    let mut rss = 0u64;
    let mut cpu = 0u64;
    for pid in pids
        .into_iter()
        .take(len.max(0) as usize / size_of::<libc::pid_t>())
    {
        let mut info = MaybeUninit::<libc::proc_taskinfo>::uninit();
        let size = size_of::<libc::proc_taskinfo>() as i32;
        let got = unsafe { libc::proc_pidinfo(pid, 4, 0, info.as_mut_ptr().cast(), size) };
        if got == size {
            let info = unsafe { info.assume_init() };
            rss = rss.saturating_add(info.pti_resident_size);
            cpu = cpu
                .saturating_add(info.pti_total_user)
                .saturating_add(info.pti_total_system);
        }
    }
    (rss, cpu)
}
