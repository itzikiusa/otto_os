//! The `script` step (engine.md §7.2, DR-002, DR-029, DR-202): one shell
//! command — once, or once per valid `repos.json` entry — through
//! otto-automation's `verify_command` runner: `/bin/sh -c` in its own process
//! group (killed on timeout, cancel or drop, and when the shell exits), the
//! daemon's environment scrubbed, `PATH` from the cached login shell,
//! Seatbelt like a shell task. Run values reach the command only as `OTTO_*`
//! environment variables. The daemon hands in repo paths, the sandbox and the
//! login `PATH` ([`ScriptHost`]).

use crate::context::{RepoEntry, RunContextFiles};
use otto_automation::verify_command::{self, End, Spec};
use serde_json::{json, Value};
use std::time::Duration;

/// Output kept in memory per stream (and shown in the node log).
const TAIL_BYTES: usize = 64 * 1024;

/// What the step needs from the daemon.
#[async_trait::async_trait]
pub trait ScriptHost: Send + Sync {
    /// A registered repo's `(name, main clone path)`.
    async fn repo(&self, repo_id: &str) -> Option<(String, String)>;
    /// The Seatbelt policy a shell task in `cwd` gets (None = unconfined).
    async fn sandbox(&self, cwd: &str) -> Option<otto_sandbox::SandboxPolicy>;
    /// `PATH` of the cached login shell, so go/node/nx resolve as in an agent's shell.
    fn login_path(&self) -> Option<String>;
}

/// Where the step runs: the run's context files, its id and working
/// directory, and its step file's base name (`<base>.log` gets the output).
pub struct ScriptRun<'a> {
    pub files: &'a RunContextFiles,
    pub run_id: &'a str,
    pub run_cwd: &'a str,
    pub step_base: &'a str,
}

/// Run the step: `(output, logs)` or the error.
pub async fn run(
    host: &dyn ScriptHost,
    at: &ScriptRun<'_>,
    p: &Value,
) -> Result<(Value, Vec<String>), String> {
    let env_files = at.files;
    let command = p
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if command.is_empty() {
        return Err("script: empty command".into());
    }
    let timeout_s = p
        .get("timeout_s")
        .and_then(Value::as_u64)
        .unwrap_or(900)
        .clamp(10, 7_200);
    let ok_codes: Vec<i32> = p
        .get("ok_exit_codes")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_i64)
                .map(|c| c as i32)
                .collect()
        })
        .filter(|c: &Vec<i32>| !c.is_empty())
        .unwrap_or_else(|| vec![0]);
    let allow_failure = p.get("allow_failure").and_then(Value::as_bool) == Some(true);
    let output_file = p
        .get("output_file")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if output_file.is_some_and(|f| !crate::context::is_bare_name(f)) {
        return Err("script: output_file must be a bare file name".into());
    }
    let ctx_dir = env_files.dir_str().unwrap_or_default();
    let in_context = p.get("cwd").and_then(Value::as_str) == Some("context");
    let log_file = env_files
        .step_md_path(at.step_base)
        .map(|p| p.with_extension("log"));
    // A re-run (retry, restart of an idempotent step) starts the output file
    // afresh: a command that appends to it must not append twice.
    if let Some(name) = output_file {
        let _ = tokio::fs::remove_file(std::path::Path::new(&ctx_dir).join(name)).await;
    }
    if let Some(log) = &log_file {
        let _ = tokio::fs::remove_file(log).await;
    }
    let mut common = vec![
        ("OTTO_RUN_ID".to_string(), at.run_id.to_string()),
        ("OTTO_CONTEXT_DIR".to_string(), ctx_dir.clone()),
    ];
    if let Some(path) = host.login_path() {
        common.push(("PATH".into(), path));
    }
    if let Some(name) = output_file {
        common.push(("OTTO_OUT".into(), name.to_string()));
    }
    let foreach = p.get("foreach").and_then(Value::as_str) == Some("repos");
    let entries: Vec<_> = env_files
        .repos()
        .into_iter()
        .filter(|e| e.error.is_none())
        .collect();
    let targets: Vec<Option<&RepoEntry>> = if foreach {
        entries.iter().map(Some).collect()
    } else {
        vec![None]
    };
    if targets.is_empty() {
        let reply = "no repositories to run on";
        return Ok((
            json!({"exit_code": 0, "timed_out": false, "duration_ms": 0, "runs": [], "reply": reply}),
            vec![format!("script: {reply}")],
        ));
    }
    let (mut runs, mut logs, mut tails, mut total_ms, mut last_code) =
        (Vec::new(), Vec::new(), Vec::new(), 0u64, 0i32);
    for target in targets {
        let mut vars = common.clone();
        let mut label = String::new();
        let mut cwd = at.run_cwd.to_string();
        if let Some(e) = target {
            let repo = match &e.repo_id {
                Some(id) => host.repo(id).await,
                None => None,
            };
            label = e
                .repo_name
                .clone()
                .or_else(|| repo.as_ref().map(|r| r.0.clone()))
                .unwrap_or_else(|| e.repo.clone());
            let wt = e.worktree.clone().unwrap_or_default();
            if !wt.is_empty() {
                cwd = wt.clone();
            }
            for (k, v) in [
                ("OTTO_WORKTREE", Some(wt)),
                ("OTTO_BASE", e.base.clone()),
                ("OTTO_BASE_BRANCH", e.base_branch.clone()),
                ("OTTO_SOURCE", e.source.clone()),
                ("OTTO_REPO", repo.as_ref().map(|r| r.1.clone())),
                ("OTTO_REPO_NAME", Some(label.clone())),
                ("OTTO_REPO_ID", e.repo_id.clone()),
            ] {
                vars.push((k.to_string(), v.unwrap_or_default()));
            }
        }
        if in_context {
            cwd = ctx_dir.clone();
        }
        let sandbox = host.sandbox(&cwd).await;
        let spec = Spec {
            env: &vars,
            sandbox: sandbox.as_ref(),
            cache_tag: Some(at.run_id),
            output_cap: TAIL_BYTES,
            log_file: log_file.as_deref(),
            ..Spec::new(&cwd, command, Duration::from_secs(timeout_s))
        };
        // Cancel and skip drop this future, which kills the process group.
        let done = verify_command::execute(&spec, std::future::pending()).await;
        total_ms += done.duration_ms;
        let who = if label.is_empty() {
            "script".to_string()
        } else {
            format!("script [{label}]")
        };
        let code = match &done.end {
            End::Exited(c) => *c,
            End::TimedOut => return Err(format!("{who}: timed out after {timeout_s} s")),
            End::Canceled => return Err(format!("{who}: canceled")),
            End::Failed(m) => return Err(format!("{who}: {m}")),
        };
        last_code = code;
        logs.push(format!("{who}: exit {code} in {} ms", done.duration_ms));
        runs.push(json!({"repo_name": label, "exit_code": code, "duration_ms": done.duration_ms}));
        if !done.exited_with(&ok_codes) && !allow_failure {
            let tail: Vec<&str> = done.output.lines().rev().take(20).collect();
            let tail: Vec<&str> = tail.into_iter().rev().collect();
            return Err(format!("{who}: exit code {code}\n{}", tail.join("\n")));
        }
        tails.push(done.output);
    }
    let output = tails.join("\n");
    logs.push(otto_core::text::clip_bytes(&output, TAIL_BYTES).to_string());
    let reply = match output_file {
        Some(name) => env_files.read_confined(name).unwrap_or_else(|| {
            logs.push(format!("⚠ script: {name} was not written"));
            output.clone()
        }),
        None => output,
    };
    Ok((
        json!({
            "exit_code": last_code, "timed_out": false, "duration_ms": total_ms,
            "runs": runs, "reply": reply, "output_file": output_file,
        }),
        logs,
    ))
}
