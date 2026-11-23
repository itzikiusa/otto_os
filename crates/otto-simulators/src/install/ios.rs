//! iOS Simulator runtime installation (`xcodebuild -downloadPlatform iOS`).
//!
//! Captures both stdout and stderr, verifies exit code == 0, AND parses
//! `xcrun simctl list runtimes -j` to confirm at least one available iOS runtime
//! exists before marking `Done`.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;

use crate::detect::count_available_ios_runtimes;
use crate::exec::{CmdSpec, CommandRunner};
use crate::types::SimInstallStage;

use super::{append_log, update_stage, InstallInner};

pub(crate) async fn run_ios_runtime_install(
    state: &Arc<Mutex<InstallInner>>,
    run_id: u64,
    runner: &dyn CommandRunner,
) -> Result<String, String> {
    update_stage(
        state,
        run_id,
        SimInstallStage::Downloading,
        Some(5),
        "Downloading iOS Simulator runtime via `xcodebuild -downloadPlatform iOS`…",
    )
    .await;

    let mut handle = runner
        .spawn_log_stream(CmdSpec::new("xcodebuild", ["-downloadPlatform", "iOS"]))
        .await
        .map_err(|e| format!("failed to launch xcodebuild: {e}"))?;

    let mut last_err_line: Option<String> = None;
    while let Some(line) = handle.rx.recv().await {
        let pct = parse_percent_hint(&line);
        let lc = line.to_ascii_lowercase();
        if lc.contains("error:") || lc.contains("failed") {
            last_err_line = Some(line.clone());
        }
        append_log(state, run_id, &line, pct).await;
    }

    let exit_code = handle.wait_exit().await;
    if exit_code != 0 {
        let detail = last_err_line
            .map(|l| format!(" (exit {exit_code}: {l})"))
            .unwrap_or_else(|| format!(" (exit code {exit_code})"));
        return Err(format!("xcodebuild -downloadPlatform iOS failed{detail}"));
    }
    if let Some(err) = last_err_line {
        return Err(err);
    }

    // Verify at least one available iOS runtime actually exists.
    let verify = runner
        .run(
            CmdSpec::new("xcrun", ["simctl", "list", "runtimes", "-j"])
                .with_timeout(Duration::from_secs(15)),
        )
        .await
        .map_err(|e| format!("failed to verify iOS runtime: {e}"))?;
    if !verify.success() {
        return Err(format!(
            "simctl verification failed: {}",
            verify.combined_err()
        ));
    }
    let avail = count_available_ios_runtimes(&verify.stdout);
    if avail == 0 {
        return Err(
            "xcodebuild completed, but no available iOS Simulator runtime was detected by simctl"
                .into(),
        );
    }
    Ok(format!(
        "iOS Simulator runtime installed and ready ({avail} runtime(s) available)."
    ))
}

pub(crate) fn parse_percent_hint(line: &str) -> Option<u8> {
    let idx = line.find('%')?;
    let prefix = &line[..idx];
    let num_str: String = prefix
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    let val: f32 = num_str.parse().ok()?;
    Some((val.round() as u32).min(99) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_percent_hints() {
        assert_eq!(parse_percent_hint("Downloading: 42.6% complete"), Some(43));
        assert_eq!(parse_percent_hint("No percent here"), None);
    }
}
