use std::io;
use std::mem::{size_of, MaybeUninit};
use std::process::Stdio;
use std::time::{Duration, Instant};

use otto_sandbox::SandboxPolicy;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use super::*;

const SAMPLE_INTERVAL: Duration = Duration::from_millis(100);
pub(super) const REPORT_LIMIT: u64 = 32 * 1024 * 1024;
const MAX_STDOUT_LINE_BYTES: usize = 1024 * 1024;
const TEMPLATE_UPDATE_FSIZE_LIMIT: u64 = 256 * 1024 * 1024;

type LineHandler = Box<dyn FnMut(&str) -> Result<()> + Send>;

enum RunAbort {
    Error(Error),
    ReaderError,
}

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
    run_with_network(
        program,
        args,
        project,
        cache,
        output,
        settings,
        NetworkScope::Offline,
        cancel,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_with_network(
    program: &Path,
    args: &[String],
    project: &Path,
    cache: &Path,
    output: &Path,
    settings: &ScanSettings,
    network: NetworkScope,
    cancel: &CancellationToken,
) -> Result<ScanMetrics> {
    run_inner(
        program, args, project, cache, output, settings, network, cancel, None,
    )
    .await
}

/// Runs the sandboxed scanner with stdout connected to an anonymous pipe and
/// streams complete lines to `on_line` under bounded per-line and total byte
/// limits, scrubbing each raw line buffer immediately after processing so raw
/// output never touches disk.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_with_network_lines<F>(
    program: &Path,
    args: &[String],
    project: &Path,
    cache: &Path,
    output: &Path,
    settings: &ScanSettings,
    network: NetworkScope,
    cancel: &CancellationToken,
    on_line: F,
) -> Result<ScanMetrics>
where
    F: FnMut(&str) -> Result<()> + Send + 'static,
{
    run_inner(
        program,
        args,
        project,
        cache,
        output,
        settings,
        network,
        cancel,
        Some(Box::new(on_line)),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn run_inner(
    program: &Path,
    args: &[String],
    project: &Path,
    cache: &Path,
    output: &Path,
    settings: &ScanSettings,
    network: NetworkScope,
    cancel: &CancellationToken,
    on_line: Option<LineHandler>,
) -> Result<ScanMetrics> {
    if cancel.is_cancelled() {
        return Err(cancelled());
    }
    let tool_dir = program
        .parent()
        .ok_or_else(|| Error::Internal("scanner executable has no parent directory".into()))?;
    let policy = match network {
        NetworkScope::Offline => SandboxPolicy::for_offline_scan(project, tool_dir, cache, output),
        NetworkScope::LoopbackPort(port) => {
            SandboxPolicy::for_dast_scan(tool_dir, cache, output, port)
        }
        NetworkScope::TemplateUpdate => SandboxPolicy::for_template_update(tool_dir, cache, output),
    };
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
        .env("XDG_CONFIG_HOME", output)
        .env("NUCLEI_TEMPLATES_DIR", cache)
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
        .stdout(if on_line.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stderr(Stdio::null())
        .process_group(0)
        .kill_on_drop(true);

    let timeout = Duration::from_secs(u64::from(settings.timeout_secs));
    let cpu_limit = timeout.as_secs() * u64::from(settings.parallelism);
    let fsize_limit = match network {
        NetworkScope::TemplateUpdate => TEMPLATE_UPDATE_FSIZE_LIMIT,
        NetworkScope::Offline | NetworkScope::LoopbackPort(_) => REPORT_LIMIT,
    };
    // Only async-signal-safe calls run in the forked child before exec.
    unsafe {
        command.pre_exec(move || {
            limit(libc::RLIMIT_CPU, cpu_limit)?;
            limit(libc::RLIMIT_FSIZE, fsize_limit)
        });
    }

    let start = Instant::now();
    let mut child = command.spawn().map_err(|e| {
        tracing::warn!(error = %e, "could not spawn security scanner");
        Error::internal_public("Otto could not start the sandboxed scanner.")
    })?;
    let group = Group(child.id().expect("a newly spawned child has a PID") as libc::pid_t);
    let reader_abort = CancellationToken::new();
    let reader_task = if let Some(handler) = on_line {
        let stdout = child.stdout.take().expect("piped stdout");
        let abort_signal = reader_abort.clone();
        Some(tokio::spawn(async move {
            let res = read_bounded_lines(stdout, REPORT_LIMIT as usize, handler).await;
            if res.is_err() {
                abort_signal.cancel();
            }
            res
        }))
    } else {
        None
    };

    let timebase = mach_nanos_per_tick();
    let mut metrics = ScanMetrics::default();
    let mut last = None;
    let mut tick = tokio::time::interval(SAMPLE_INTERVAL);
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);
    let result = loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break Err(RunAbort::Error(cancelled())),
            _ = reader_abort.cancelled() => break Err(RunAbort::ReaderError),
            _ = &mut deadline => {
                break Err(RunAbort::Error(Error::internal_public(
                    "The scanner reached the scan time limit.",
                )));
            }
            status = child.wait() => {
                match status {
                    Ok(status) if status.success() => break Ok(()),
                    _ => break Err(RunAbort::Error(Error::internal_public(
                        "The scanner could not finish this scan. Check the scan scope and resource limits.",
                    ))),
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
                    break Err(RunAbort::Error(Error::internal_public(
                        "The scanner reached the scan memory limit.",
                    )));
                }
            }
        }
    };

    drop(group);
    let _ = child.wait().await;
    let reader_res = if let Some(handle) = reader_task {
        handle
            .await
            .map_err(|_| Error::Internal("scanner output worker stopped".into()))?
    } else {
        Ok(())
    };
    match result {
        Ok(()) => reader_res?,
        Err(RunAbort::ReaderError) => {
            reader_res?;
            return Err(Error::internal_public(
                "Scanner output stream aborted unexpectedly.",
            ));
        }
        Err(RunAbort::Error(error)) => return Err(error),
    }
    metrics.wall_ms = start.elapsed().as_millis() as u64;
    Ok(metrics)
}

async fn read_bounded_lines<R>(
    mut reader: R,
    max_total: usize,
    mut on_line: LineHandler,
) -> Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut chunk = [0u8; 8192];
    let mut line = Vec::new();
    let mut total = 0usize;
    let outcome = async {
        loop {
            let n = reader
                .read(&mut chunk)
                .await
                .map_err(|_| Error::internal_public("Could not read scanner output stream."))?;
            if n == 0 {
                break;
            }
            total = total.saturating_add(n);
            if total > max_total {
                return Err(Error::internal_public(
                    "Scanner output exceeded the scan report limit.",
                ));
            }
            let mut slice = &chunk[..n];
            while let Some(pos) = slice.iter().position(|&b| b == b'\n') {
                let part = &slice[..pos];
                if line.len().saturating_add(part.len()) > MAX_STDOUT_LINE_BYTES {
                    return Err(Error::internal_public(
                        "Scanner output line exceeded the maximum allowed size.",
                    ));
                }
                line.extend_from_slice(part);
                let res = std::str::from_utf8(&line)
                    .map_err(|_| Error::internal_public("Nuclei returned an unreadable report."))
                    .and_then(&mut on_line);
                line.fill(0);
                line.clear();
                res?;
                slice = &slice[pos + 1..];
            }
            if !slice.is_empty() {
                if line.len().saturating_add(slice.len()) > MAX_STDOUT_LINE_BYTES {
                    return Err(Error::internal_public(
                        "Scanner output line exceeded the maximum allowed size.",
                    ));
                }
                line.extend_from_slice(slice);
            }
        }
        if !line.is_empty() {
            let res = std::str::from_utf8(&line)
                .map_err(|_| Error::internal_public("Nuclei returned an unreadable report."))
                .and_then(&mut on_line);
            line.fill(0);
            line.clear();
            res?;
        }
        Ok(())
    }
    .await;
    line.fill(0);
    line.clear();
    chunk.fill(0);
    outcome
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
