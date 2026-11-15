//! Process-group lifecycle guards and scrubbed child spawning for [`super::runner`].

use std::collections::BTreeMap;
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

use super::runner_cmd::resolve_program_in_env;
use super::shell_env::collect_safe_non_root_base_env;
use crate::target::TargetManager;
use crate::types::{IdeSshSpec, IdeTargetKind, IdeTargetSpec};

pub(super) struct ChildProcessGroupGuard {
    pub(super) pid: Option<u32>,
    pub(super) is_local: bool,
    pub(super) disarmed: bool,
}

impl Drop for ChildProcessGroupGuard {
    #[allow(clippy::disallowed_methods)] // synchronous Drop safeguard against orphan child processes
    fn drop(&mut self) {
        if !self.disarmed && self.is_local {
            #[cfg(unix)]
            if let Some(pid) = self.pid {
                let _ = std::process::Command::new("/bin/kill")
                    .args(["-KILL", &format!("-{pid}")])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
        }
    }
}

pub(super) async fn kill_child_process_group(
    child: &mut tokio::process::Child,
    pid: Option<u32>,
    is_local: bool,
) {
    #[cfg(unix)]
    if is_local {
        if let Some(p) = pid {
            let neg = format!("-{p}");
            let _ = Command::new("/bin/kill")
                .args(["-TERM", &neg])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await;
            if tokio::time::timeout(Duration::from_millis(150), child.wait())
                .await
                .is_ok()
            {
                let _ = Command::new("/bin/kill")
                    .args(["-KILL", &neg])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .await;
                return;
            }
            let _ = Command::new("/bin/kill")
                .args(["-KILL", &neg])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await;
        }
    }
    #[cfg(not(unix))]
    let _ = (pid, is_local);

    let _ = child.kill().await;
    let _ = tokio::time::timeout(Duration::from_millis(120), child.wait()).await;
}

pub(super) async fn cleanup_remaining_process_group(pid: Option<u32>, is_local: bool) {
    #[cfg(unix)]
    if is_local {
        if let Some(p) = pid {
            let _ = Command::new("/bin/kill")
                .args(["-KILL", &format!("-{p}")])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await;
        }
    }
    #[cfg(not(unix))]
    let _ = (pid, is_local);
}

fn apply_scrubbed_local_env(cmd: &mut Command, env_map: &BTreeMap<String, String>) {
    // Clear inherited `ottod` daemon environment first so non-root runs only
    // receive variables present in `env_map` (with a minimal safe fallback if empty).
    cmd.env_clear();
    if !env_map.contains_key("PATH") {
        cmd.envs(collect_safe_non_root_base_env());
    }
    cmd.envs(env_map);
}

pub(super) async fn run_before_launch(
    targets: &TargetManager,
    target: &IdeTargetSpec,
    ssh: Option<&IdeSshSpec>,
    actor_id: &str,
    script: &str,
    cwd: &str,
    env_map: &BTreeMap<String, String>,
) -> std::result::Result<(), String> {
    match target.kind {
        IdeTargetKind::Local => {
            let mut cmd = Command::new("sh");
            cmd.args(["-c", script])
                .current_dir(cwd)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            apply_scrubbed_local_env(&mut cmd, env_map);
            #[cfg(unix)]
            cmd.process_group(0);

            let out = cmd.output().await.map_err(|e| e.to_string())?;
            if !out.status.success() {
                return Err(format!(
                    "before_launch failed: {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                ));
            }
            Ok(())
        }
        IdeTargetKind::Ssh => {
            let spec = ssh.ok_or("missing SSH spec for before_launch")?;
            let res = targets
                .resolve_with_ssh(target, actor_id, spec.clone())
                .map_err(|e| e.to_string())?;
            let out = targets
                .exec_shell(&res, script, Some(cwd), env_map, Duration::from_secs(120))
                .await
                .map_err(|e| e.to_string())?;
            if !out.success() {
                return Err(format!("before_launch failed: {}", out.stderr.trim()));
            }
            Ok(())
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn spawn_run_child(
    targets: &TargetManager,
    target: &IdeTargetSpec,
    ssh: Option<&IdeSshSpec>,
    actor_id: &str,
    program: &str,
    args: &[String],
    cwd: &str,
    env_map: &BTreeMap<String, String>,
) -> std::result::Result<tokio::process::Child, String> {
    match target.kind {
        IdeTargetKind::Local => {
            let resolved_prog = resolve_program_in_env(program, env_map);
            let mut cmd = Command::new(resolved_prog);
            cmd.args(args)
                .current_dir(cwd)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            apply_scrubbed_local_env(&mut cmd, env_map);
            #[cfg(unix)]
            cmd.process_group(0);
            cmd.spawn().map_err(|e| format!("spawn '{program}': {e}"))
        }
        IdeTargetKind::Ssh => {
            let spec = ssh.ok_or("missing SSH connection spec for remote run")?;
            let mut cmd = targets
                .build_ssh_run_command(spec, actor_id, cwd, program, args, env_map)
                .map_err(|e| e.to_string())?;
            #[cfg(unix)]
            cmd.process_group(0);
            cmd.spawn().map_err(|e| format!("spawn ssh: {e}"))
        }
    }
}
