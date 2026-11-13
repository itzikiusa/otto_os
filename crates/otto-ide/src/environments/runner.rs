//! Background Test & Run execution engine and structured multi-language test
//! output parsers (`go test -v`, `cargo test`, `pytest -v` / `unittest`,
//! `node --test` / `vitest`, and Maven Surefire / Gradle / JUnit).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::watch;

use crate::target::TargetManager;
use crate::toolchains::{find_executable_in, is_executable_file};
use crate::types::{
    IdeRunConfig, IdeRunState, IdeRunStatus, IdeRunnerKind, IdeSshSpec, IdeTargetKind,
    IdeTargetSpec, IdeTestCaseResult, IdeTestCaseStatus,
};

use super::is_valid_env_key;

const MAX_RUN_OUTPUT_LINES: usize = 2_000;

pub(super) struct ActiveRunHandle {
    pub(super) state: Mutex<IdeRunState>,
    pub(super) stop_tx: watch::Sender<bool>,
}

impl ActiveRunHandle {
    pub(super) fn snapshot(&self) -> IdeRunState {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub(super) fn stop(&self) -> IdeRunState {
        let _ = self.stop_tx.send(true);
        let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if guard.status == IdeRunStatus::Running {
            guard.status = IdeRunStatus::Stopped;
            guard.output.push("[Stopped by user]".into());
        }
        guard.clone()
    }
}

pub(super) async fn execute_run_task(
    handle: Arc<ActiveRunHandle>,
    targets: TargetManager,
    target: IdeTargetSpec,
    ssh: Option<IdeSshSpec>,
    cfg: IdeRunConfig,
    env_map: BTreeMap<String, String>,
    mut stop_rx: watch::Receiver<bool>,
) {
    let started = Instant::now();
    let cwd = resolve_run_cwd(&target, cfg.cwd.as_deref());
    let (program, args) = build_runner_command(&cfg, &cwd, &env_map);

    if let Some(pre) = cfg
        .before_launch
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        append_run_line(&handle, format!("$ {pre}"));
        if let Err(err) =
            run_before_launch(&targets, &target, ssh.as_ref(), pre, &cwd, &env_map).await
        {
            finish_run(
                &handle,
                IdeRunStatus::Failed,
                Some(1),
                started.elapsed(),
                Some(err),
            );
            return;
        }
    }

    append_run_line(
        &handle,
        format!("$ {} {}", program, args.join(" "))
            .trim()
            .to_string(),
    );

    let mut child = match spawn_run_child(&target, ssh.as_ref(), &program, &args, &cwd, &env_map) {
        Ok(c) => c,
        Err(err) => {
            finish_run(
                &handle,
                IdeRunStatus::Failed,
                Some(127),
                started.elapsed(),
                Some(err),
            );
            return;
        }
    };

    let stdout_task = child.stdout.take().map(|out| {
        let h = Arc::clone(&handle);
        tokio::spawn(async move {
            let mut reader = BufReader::new(out).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                append_run_line(&h, line);
            }
        })
    });
    let stderr_task = child.stderr.take().map(|err| {
        let h = Arc::clone(&handle);
        tokio::spawn(async move {
            let mut reader = BufReader::new(err).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                append_run_line(&h, line);
            }
        })
    });

    let wait_res = tokio::select! {
        status = child.wait() => Some(status),
        _ = stop_rx.changed() => {
            let _ = child.kill().await;
            None
        }
    };
    if let Some(t) = stdout_task {
        let _ = t.await;
    }
    if let Some(t) = stderr_task {
        let _ = t.await;
    }

    match wait_res {
        None => finish_run(
            &handle,
            IdeRunStatus::Stopped,
            None,
            started.elapsed(),
            None,
        ),
        Some(Ok(st)) => {
            let code = st.code().unwrap_or(if st.success() { 0 } else { 1 });
            let status = if st.success() {
                IdeRunStatus::Passed
            } else {
                IdeRunStatus::Failed
            };
            finish_run(&handle, status, Some(code), started.elapsed(), None);
        }
        Some(Err(e)) => {
            finish_run(
                &handle,
                IdeRunStatus::Failed,
                Some(1),
                started.elapsed(),
                Some(e.to_string()),
            );
        }
    }
}

fn append_run_line(handle: &ActiveRunHandle, line: String) {
    let mut guard = handle.state.lock().unwrap_or_else(|e| e.into_inner());
    if guard.output.len() >= MAX_RUN_OUTPUT_LINES {
        guard.output.remove(0);
    }
    guard.output.push(line);
    guard.tests = parse_test_output(guard.runner, &guard.output);
}

fn finish_run(
    handle: &ActiveRunHandle,
    status: IdeRunStatus,
    exit_code: Option<i32>,
    elapsed: Duration,
    err_msg: Option<String>,
) {
    let mut guard = handle.state.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(msg) = err_msg {
        guard.output.push(format!("ERROR: {msg}"));
    }
    guard.tests = parse_test_output(guard.runner, &guard.output);
    if guard.status != IdeRunStatus::Stopped {
        guard.status = status;
    }
    guard.exit_code = exit_code;
    guard.duration_ms = Some(elapsed.as_millis() as u64);
}

async fn run_before_launch(
    targets: &TargetManager,
    target: &IdeTargetSpec,
    ssh: Option<&IdeSshSpec>,
    script: &str,
    cwd: &str,
    env_map: &BTreeMap<String, String>,
) -> std::result::Result<(), String> {
    match target.kind {
        IdeTargetKind::Local => {
            let out = Command::new("sh")
                .args(["-c", script])
                .current_dir(cwd)
                .envs(env_map)
                .output()
                .await
                .map_err(|e| e.to_string())?;
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
                .resolve_with_ssh(target, "ide-run", spec.clone())
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

fn spawn_run_child(
    target: &IdeTargetSpec,
    ssh: Option<&IdeSshSpec>,
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
                .envs(env_map)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            cmd.spawn().map_err(|e| format!("spawn '{program}': {e}"))
        }
        IdeTargetKind::Ssh => {
            let spec = ssh.ok_or("missing SSH connection spec for remote run")?;
            if !otto_ssh::valid_ssh_host(&spec.host) || !otto_ssh::valid_ssh_user(&spec.user) {
                return Err("invalid SSH host or user".into());
            }
            let mut remote_cmd = format!("cd {} && ", shell_words::quote(cwd));
            for (k, v) in env_map {
                if is_valid_env_key(k) {
                    remote_cmd.push_str(&format!("export {}={} && ", k, shell_words::quote(v)));
                }
            }
            remote_cmd.push_str(&shell_words::quote(program));
            for a in args {
                remote_cmd.push(' ');
                remote_cmd.push_str(&shell_words::quote(a));
            }
            let mut cmd = Command::new("ssh");
            cmd.args([
                "-T",
                "-o",
                "BatchMode=yes",
                "-o",
                "StrictHostKeyChecking=accept-new",
            ]);
            if spec.port != 0 && spec.port != 22 {
                cmd.arg("-p").arg(spec.port.to_string());
            }
            if let Some(id_file) = spec
                .identity_file
                .as_deref()
                .filter(|s| !s.trim().is_empty())
            {
                cmd.arg("-i").arg(id_file);
            }
            if let Some(jump) = spec.proxy_jump.as_deref().filter(|s| !s.trim().is_empty()) {
                if !otto_ssh::valid_proxy_jump(jump) {
                    return Err("invalid SSH ProxyJump".into());
                }
                cmd.arg("-J").arg(jump);
            }
            let dest = if spec.user.trim().is_empty() {
                spec.host.clone()
            } else {
                format!("{}@{}", spec.user.trim(), spec.host)
            };
            cmd.arg("--")
                .arg(dest)
                .arg(remote_cmd)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            cmd.spawn().map_err(|e| format!("spawn ssh: {e}"))
        }
    }
}

fn resolve_program_in_env(program: &str, env_map: &BTreeMap<String, String>) -> PathBuf {
    if program.contains('/') {
        return PathBuf::from(program);
    }
    if let Some(path_str) = env_map.get("PATH") {
        let dirs: Vec<PathBuf> = std::env::split_paths(path_str).collect();
        if let Some(hit) = find_executable_in(&dirs, &[program]) {
            return hit;
        }
    }
    PathBuf::from(program)
}

fn resolve_run_cwd(target: &IdeTargetSpec, raw_cwd: Option<&str>) -> String {
    let Some(c) = raw_cwd
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != ".")
    else {
        return target.root.clone();
    };
    if c.starts_with('/') {
        if target.kind == IdeTargetKind::Local {
            if let (Ok(canon_root), Ok(canon_c)) = (
                std::fs::canonicalize(&target.root),
                std::fs::canonicalize(c),
            ) {
                if canon_c.starts_with(&canon_root) {
                    return canon_c.to_string_lossy().into_owned();
                }
            }
        }
        if c == target.root || c.starts_with(&format!("{}/", target.root.trim_end_matches('/'))) {
            return c.to_string();
        }
        return target.root.clone();
    }
    let rel = c.trim_start_matches("./");
    format!("{}/{rel}", target.root.trim_end_matches('/'))
}

pub(super) fn build_runner_command(
    cfg: &IdeRunConfig,
    cwd: &str,
    env_map: &BTreeMap<String, String>,
) -> (String, Vec<String>) {
    let cmd_opt = cfg
        .command
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let mut args = cfg.args.clone();
    let filter = cfg
        .test_filter
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    match cfg.runner {
        IdeRunnerKind::GoTest => {
            let prog = cmd_opt.unwrap_or("go").to_string();
            let mut out = Vec::new();
            if args.first().map(String::as_str) != Some("test") {
                out.push("test".into());
                if !args.iter().any(|a| a == "-v") {
                    out.push("-v".into());
                }
            }
            if let Some(f) = filter {
                if !args.iter().any(|a| a == "-run") {
                    out.extend(["-run".into(), f.to_string()]);
                }
            }
            if args.is_empty() {
                out.push("./...".into());
            } else {
                out.extend(args);
            }
            (prog, out)
        }
        IdeRunnerKind::GoRun => {
            let prog = cmd_opt.unwrap_or("go").to_string();
            if args.first().map(String::as_str) != Some("run") {
                args.insert(0, "run".into());
            }
            if args.len() == 1 {
                args.push(cfg.target_path.clone().unwrap_or_else(|| ".".into()));
            }
            (prog, args)
        }
        IdeRunnerKind::CargoTest => {
            let prog = cmd_opt.unwrap_or("cargo").to_string();
            if args.first().map(String::as_str) != Some("test") {
                args.insert(0, "test".into());
            }
            if let Some(f) = filter {
                if !args.iter().any(|a| a == f) {
                    args.push(f.to_string());
                }
            }
            (prog, args)
        }
        IdeRunnerKind::CargoRun => {
            let prog = cmd_opt.unwrap_or("cargo").to_string();
            if args.first().map(String::as_str) != Some("run") {
                args.insert(0, "run".into());
            }
            (prog, args)
        }
        IdeRunnerKind::Pytest => {
            let py = env_map
                .get("VIRTUAL_ENV")
                .map(|v| format!("{v}/bin/python"))
                .unwrap_or_else(|| "python3".into());
            if let Some(explicit) = cmd_opt {
                return (explicit.to_string(), args);
            }
            let pytest_bin = resolve_program_in_env("pytest", env_map);
            if is_executable_file(&pytest_bin) {
                if !args.iter().any(|a| a == "-v") {
                    args.insert(0, "-v".into());
                }
                if let Some(f) = filter {
                    args.extend(["-k".into(), f.to_string()]);
                }
                ("pytest".into(), args)
            } else {
                let mut u = vec![
                    "-m".into(),
                    "unittest".into(),
                    "discover".into(),
                    "-v".into(),
                ];
                u.extend(args);
                (py, u)
            }
        }
        IdeRunnerKind::Python => {
            let prog = cmd_opt.unwrap_or("python3").to_string();
            if args.is_empty() {
                if let Some(tp) = &cfg.target_path {
                    args.push(tp.clone());
                }
            }
            (prog, args)
        }
        IdeRunnerKind::Node => {
            let prog = cmd_opt.unwrap_or("node").to_string();
            if args.is_empty() {
                if let Some(tp) = &cfg.target_path {
                    args.push(tp.clone());
                }
            }
            (prog, args)
        }
        IdeRunnerKind::Npm => (
            cmd_opt.unwrap_or("npm").to_string(),
            if args.is_empty() {
                vec!["test".into()]
            } else {
                args
            },
        ),
        IdeRunnerKind::Vitest => {
            let prog = cmd_opt.unwrap_or("npx").to_string();
            if args.first().map(String::as_str) != Some("vitest") {
                let mut v = vec!["vitest".into(), "run".into()];
                v.extend(args);
                args = v;
            }
            (prog, args)
        }
        IdeRunnerKind::JavaMain => (cmd_opt.unwrap_or("java").to_string(), args),
        IdeRunnerKind::JavaJunit | IdeRunnerKind::Maven => {
            let mvnw = Path::new(cwd).join("mvnw");
            let prog = cmd_opt.map(str::to_string).unwrap_or_else(|| {
                if is_executable_file(&mvnw) {
                    "./mvnw".into()
                } else {
                    "mvn".into()
                }
            });
            if args.is_empty() {
                args.push("test".into());
            }
            if let Some(f) = filter {
                args.push(format!("-Dtest={f}"));
            }
            (prog, args)
        }
        IdeRunnerKind::Gradle => {
            let gradlew = Path::new(cwd).join("gradlew");
            let prog = cmd_opt.map(str::to_string).unwrap_or_else(|| {
                if is_executable_file(&gradlew) {
                    "./gradlew".into()
                } else {
                    "gradle".into()
                }
            });
            if args.is_empty() {
                args.push("test".into());
            }
            if let Some(f) = filter {
                args.extend(["--tests".into(), f.to_string()]);
            }
            (prog, args)
        }
        IdeRunnerKind::Custom => {
            let c = cmd_opt.unwrap_or("echo");
            if args.is_empty() && c.contains(' ') {
                ("sh".into(), vec!["-c".into(), c.to_string()])
            } else {
                (c.to_string(), args)
            }
        }
    }
}

/// Parse raw runner output lines into structured per-test pass/fail/duration items.
pub fn parse_test_output(runner: IdeRunnerKind, lines: &[String]) -> Vec<IdeTestCaseResult> {
    match runner {
        IdeRunnerKind::GoTest => parse_go_test_lines(lines),
        IdeRunnerKind::CargoTest => parse_cargo_test_lines(lines),
        IdeRunnerKind::Pytest => parse_python_test_lines(lines),
        IdeRunnerKind::Node | IdeRunnerKind::Vitest | IdeRunnerKind::Npm => {
            parse_js_test_lines(lines)
        }
        IdeRunnerKind::JavaJunit | IdeRunnerKind::Maven | IdeRunnerKind::Gradle => {
            parse_jvm_test_lines(lines)
        }
        _ => Vec::new(),
    }
}

fn parse_go_test_lines(lines: &[String]) -> Vec<IdeTestCaseResult> {
    let mut map: BTreeMap<String, IdeTestCaseResult> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    for line in lines {
        let t = line.trim();
        if let Some(name) = t
            .strip_prefix("=== RUN")
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if !map.contains_key(name) {
                order.push(name.to_string());
            }
            map.insert(
                name.to_string(),
                simple_test_case(name, IdeTestCaseStatus::Running, None),
            );
        } else if let Some((status, rest)) = t
            .strip_prefix("--- PASS:")
            .map(|r| (IdeTestCaseStatus::Passed, r))
            .or_else(|| {
                t.strip_prefix("--- FAIL:")
                    .map(|r| (IdeTestCaseStatus::Failed, r))
            })
            .or_else(|| {
                t.strip_prefix("--- SKIP:")
                    .map(|r| (IdeTestCaseStatus::Skipped, r))
            })
        {
            let rest = rest.trim();
            let (name, dur_ms) = match rest.split_once(' ') {
                Some((n, tail)) => (n.trim(), parse_seconds_parens(tail)),
                None => (rest, None),
            };
            if !map.contains_key(name) {
                order.push(name.to_string());
            }
            let entry = map
                .entry(name.to_string())
                .or_insert_with(|| simple_test_case(name, status, dur_ms));
            entry.status = status;
            entry.duration_ms = dur_ms;
        } else if let Some((file, ln, msg)) = parse_go_diag_line(t) {
            if let Some(last_name) = order.last() {
                if let Some(entry) = map.get_mut(last_name) {
                    entry.file.get_or_insert(file);
                    entry.line.get_or_insert(ln);
                    entry.message = Some(msg);
                }
            }
        }
    }
    order.into_iter().filter_map(|k| map.remove(&k)).collect()
}

fn parse_go_diag_line(trimmed: &str) -> Option<(String, u32, String)> {
    let (file_part, rest) = trimmed.split_once(".go:")?;
    if file_part.contains(' ') {
        return None;
    }
    let (line_s, msg) = rest.split_once(':')?;
    let line_no = line_s.trim().parse::<u32>().ok()?;
    Some((format!("{file_part}.go"), line_no, msg.trim().to_string()))
}

fn parse_cargo_test_lines(lines: &[String]) -> Vec<IdeTestCaseResult> {
    let mut out = Vec::new();
    for line in lines {
        let Some(rest) = line.trim().strip_prefix("test ") else {
            continue;
        };
        let Some((name, outcome)) = rest.rsplit_once(" ... ") else {
            continue;
        };
        let status = if outcome.starts_with("ok") {
            IdeTestCaseStatus::Passed
        } else if outcome.starts_with("FAILED") {
            IdeTestCaseStatus::Failed
        } else if outcome.starts_with("ignored") {
            IdeTestCaseStatus::Skipped
        } else {
            continue;
        };
        out.push(simple_test_case(name.trim(), status, None));
    }
    out
}

fn parse_python_test_lines(lines: &[String]) -> Vec<IdeTestCaseResult> {
    let mut out = Vec::new();
    for line in lines {
        let t = line.trim();
        if t.contains("::") {
            let parts: Vec<&str> = t.split_whitespace().collect();
            if parts.len() >= 2 {
                let st = match parts[1] {
                    "PASSED" => Some(IdeTestCaseStatus::Passed),
                    "FAILED" | "ERROR" => Some(IdeTestCaseStatus::Failed),
                    "SKIPPED" | "XFAIL" => Some(IdeTestCaseStatus::Skipped),
                    _ => None,
                };
                if let Some(status) = st {
                    let (file, name) = parts[0].split_once("::").unwrap_or(("", parts[0]));
                    let mut item = simple_test_case(name, status, None);
                    if !file.is_empty() {
                        item.suite = Some(file.to_string());
                        item.file = Some(file.to_string());
                    }
                    out.push(item);
                    continue;
                }
            }
        }
        if let Some((left, right)) = t.rsplit_once(" ... ") {
            let st = match right.trim() {
                "ok" => Some(IdeTestCaseStatus::Passed),
                "FAIL" | "ERROR" => Some(IdeTestCaseStatus::Failed),
                s if s.starts_with("skipped") => Some(IdeTestCaseStatus::Skipped),
                _ => None,
            };
            if let Some(status) = st {
                out.push(simple_test_case(left.trim(), status, None));
            }
        }
    }
    out
}

fn parse_js_test_lines(lines: &[String]) -> Vec<IdeTestCaseResult> {
    let mut out = Vec::new();
    for line in lines {
        let t = line.trim();
        if let Some(rest) = t
            .strip_prefix("ok ")
            .or_else(|| t.strip_prefix("✓ "))
            .or_else(|| t.strip_prefix("✔ "))
        {
            let name = rest
                .split_once(" - ")
                .map(|(_, n)| n)
                .unwrap_or(rest)
                .trim();
            if !name.is_empty() {
                out.push(simple_test_case(
                    name,
                    IdeTestCaseStatus::Passed,
                    parse_ms_suffix(name),
                ));
            }
        } else if let Some(rest) = t
            .strip_prefix("not ok ")
            .or_else(|| t.strip_prefix("✖ "))
            .or_else(|| t.strip_prefix("× "))
        {
            let name = rest
                .split_once(" - ")
                .map(|(_, n)| n)
                .unwrap_or(rest)
                .trim();
            if !name.is_empty() {
                out.push(simple_test_case(
                    name,
                    IdeTestCaseStatus::Failed,
                    parse_ms_suffix(name),
                ));
            }
        }
    }
    out
}

fn parse_jvm_test_lines(lines: &[String]) -> Vec<IdeTestCaseResult> {
    let mut out = Vec::new();
    for line in lines {
        let t = line
            .trim()
            .trim_start_matches("[INFO] ")
            .trim_start_matches("[ERROR] ");
        if let Some((left, right)) = t.rsplit_once(' ') {
            if left.contains(" > ") {
                let st = match right {
                    "PASSED" => Some(IdeTestCaseStatus::Passed),
                    "FAILED" => Some(IdeTestCaseStatus::Failed),
                    "SKIPPED" => Some(IdeTestCaseStatus::Skipped),
                    _ => None,
                };
                if let Some(status) = st {
                    let (suite, name) = left.split_once(" > ").unwrap_or(("", left));
                    let mut item = simple_test_case(name.trim(), status, None);
                    item.suite = Some(suite.trim().to_string());
                    out.push(item);
                    continue;
                }
            }
        }
        if t.starts_with("Tests run:") && t.contains("-- in ") {
            if let Some((_, suite)) = t.split_once("-- in ") {
                let failed = !t.contains("Failures: 0, Errors: 0");
                let status = if failed {
                    IdeTestCaseStatus::Failed
                } else {
                    IdeTestCaseStatus::Passed
                };
                let mut item = simple_test_case(suite.trim(), status, None);
                item.suite = Some(suite.trim().to_string());
                out.push(item);
            }
        }
    }
    out
}

fn simple_test_case(
    name: &str,
    status: IdeTestCaseStatus,
    duration_ms: Option<u64>,
) -> IdeTestCaseResult {
    IdeTestCaseResult {
        name: name.to_string(),
        suite: None,
        status,
        duration_ms,
        file: None,
        line: None,
        message: None,
    }
}

fn parse_seconds_parens(s: &str) -> Option<u64> {
    let inner = s
        .trim()
        .trim_start_matches('(')
        .trim_end_matches(')')
        .trim_end_matches('s');
    inner
        .parse::<f64>()
        .ok()
        .map(|sec| (sec * 1000.0).round() as u64)
}

fn parse_ms_suffix(s: &str) -> Option<u64> {
    let start = s.rfind('(')?;
    let inner = s[start + 1..].trim_end_matches(')').trim();
    inner
        .strip_suffix("ms")?
        .trim()
        .parse::<f64>()
        .ok()
        .map(|v| v.round() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_structured_test_results_across_go_rust_python_js_and_jvm() {
        let go_out = vec![
            "=== RUN   TestAdd".to_string(),
            "--- PASS: TestAdd (0.02s)".to_string(),
            "=== RUN   TestSub".to_string(),
            "    calc_test.go:19: expected 3, got 2".to_string(),
            "--- FAIL: TestSub (0.01s)".to_string(),
        ];
        let go_res = parse_test_output(IdeRunnerKind::GoTest, &go_out);
        assert_eq!(go_res.len(), 2);
        assert_eq!(go_res[0].status, IdeTestCaseStatus::Passed);
        assert_eq!(go_res[0].duration_ms, Some(20));
        assert_eq!(go_res[1].status, IdeTestCaseStatus::Failed);
        assert_eq!(go_res[1].line, Some(19));

        let cargo_res = parse_test_output(
            IdeRunnerKind::CargoTest,
            &[
                "test math::adds ... ok".into(),
                "test math::fails ... FAILED".into(),
            ],
        );
        assert_eq!(cargo_res.len(), 2);
        assert_eq!(cargo_res[0].status, IdeTestCaseStatus::Passed);
        assert_eq!(cargo_res[1].status, IdeTestCaseStatus::Failed);

        let py_res = parse_test_output(
            IdeRunnerKind::Pytest,
            &["tests/test_app.py::test_ok PASSED [100%]".into()],
        );
        assert_eq!(py_res[0].name, "test_ok");
        assert_eq!(py_res[0].status, IdeTestCaseStatus::Passed);

        let js_res = parse_test_output(
            IdeRunnerKind::Node,
            &[
                "ok 1 - adds numbers (3ms)".into(),
                "not ok 2 - divides by zero".into(),
            ],
        );
        assert_eq!(js_res.len(), 2);
        assert_eq!(js_res[0].status, IdeTestCaseStatus::Passed);
        assert_eq!(js_res[1].status, IdeTestCaseStatus::Failed);

        let jvm_res = parse_test_output(
            IdeRunnerKind::Gradle,
            &["com.acme.CalcTest > addsNumbers() PASSED".into()],
        );
        assert_eq!(jvm_res.len(), 1);
        assert_eq!(jvm_res[0].status, IdeTestCaseStatus::Passed);
    }
}
