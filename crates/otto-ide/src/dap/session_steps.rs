//! Debug All steps: one visible session that debugs several processes one after
//! another (one .NET test project per target framework, one Zig test suite per
//! test binary). Only one step is live at a time; breakpoints stay on the
//! session, so the ones edited during a step apply to the next (they are synced
//! on every boot). Restart runs the whole plan again from step 1.

use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use futures_util::future::BoxFuture;
use otto_core::Result;

use super::adapters::ResolvedDapAdapter;
use super::session::{DapStartContext, SessionEntry};
use super::session_entry::ActiveConnection;
use super::session_inline::teardown_conn;
use super::session_transport::{boot_session_connection, evict_terminated_sessions};
use crate::environments::runner_parsers::parse_test_output;
use crate::types::{
    IdeDebugState, IdeDebugStep, IdeDebugStepState, IdeRunnerKind, IdeStartDebugReq,
    IdeTestCaseStatus,
};

/// How long a step's companion (`dotnet test`) may take to exit after its
/// test host ended, for its exit code and summary.
const COMPANION_EXIT_WAIT: Duration = Duration::from_secs(10);
/// A step's own output kept for its result (older text is dropped first).
const MAX_STEP_CAPTURE: usize = 512 * 1024;

/// How a step's exit code and output read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StepKind {
    /// `dotnet test` + a VSTest host: 0 all passed, 1 some failed.
    VsTest,
    /// A Microsoft.Testing.Platform test app: 0 passed, 2 failed, 8 no tests.
    Mtp,
    /// A Zig test binary: 0 passed, 1 failed.
    Zig,
}

impl StepKind {
    fn runner(self) -> IdeRunnerKind {
        match self {
            Self::VsTest | Self::Mtp => IdeRunnerKind::DotnetTest,
            Self::Zig => IdeRunnerKind::ZigTest,
        }
    }
}

/// One step of a Debug All plan.
#[derive(Debug, Clone)]
pub(crate) struct PlannedStep {
    pub(crate) label: String,
    /// The step's own start request (`launch.debug_step` set).
    pub(crate) req: IdeStartDebugReq,
    pub(crate) kind: StepKind,
    /// The plan already knows this step cannot start (why, in one line).
    pub(crate) error: Option<String>,
    /// What a single-test Debug of one of its failed tests launches.
    pub(crate) program: String,
}

/// A Debug All plan: the steps and the console lines planning produced.
#[derive(Debug, Default)]
pub(crate) struct DebugPlan {
    /// The session title: `Debug all tests · 3 test projects`.
    pub(crate) title: String,
    pub(crate) steps: Vec<PlannedStep>,
    pub(crate) output: Vec<String>,
}

/// Plans and resolves debug sessions: the language adapters, or a test double.
pub(crate) trait DebugResolver: Send + Sync {
    /// The steps of a Debug All run, or `None` for an ordinary one-process session.
    fn plan<'a>(
        &'a self,
        req: &'a IdeStartDebugReq,
        ctx: &'a DapStartContext,
    ) -> BoxFuture<'a, Result<Option<DebugPlan>>>;
    fn resolve<'a>(
        &'a self,
        req: &'a IdeStartDebugReq,
        ctx: &'a DapStartContext,
    ) -> BoxFuture<'a, Result<ResolvedDapAdapter>>;
}

/// The real [`DebugResolver`]: the language adapters.
pub(super) struct AdapterResolver {
    pub(super) data_dir: std::path::PathBuf,
    pub(super) environments: crate::environments::EnvironmentManager,
}

impl DebugResolver for AdapterResolver {
    fn plan<'a>(
        &'a self,
        req: &'a IdeStartDebugReq,
        ctx: &'a DapStartContext,
    ) -> BoxFuture<'a, Result<Option<DebugPlan>>> {
        Box::pin(super::adapters::plan_debug_steps(
            &self.data_dir,
            req,
            ctx,
            Some(&self.environments),
        ))
    }

    fn resolve<'a>(
        &'a self,
        req: &'a IdeStartDebugReq,
        ctx: &'a DapStartContext,
    ) -> BoxFuture<'a, Result<ResolvedDapAdapter>> {
        Box::pin(super::adapters::resolve_dap_adapter_with_manager(
            &self.data_dir,
            req,
            ctx,
            Some(&self.environments),
        ))
    }
}

/// The running plan of a session (`SessionEntry::steps`).
pub(super) struct StepRun {
    pub(super) resolver: Arc<dyn DebugResolver>,
    total: usize,
    pending: VecDeque<PlannedStep>,
    /// The live step's kind, while one runs.
    current: Option<StepKind>,
    /// The live step's output since its separator.
    pub(super) capture: String,
    /// The debuggee's exit code from DAP `exited`.
    pub(super) exit_code: Option<i64>,
    /// Skip to next step was asked for the live step.
    skip: bool,
}

pub(super) fn capture_step_output(run: &mut StepRun, text: &str) {
    if run.current.is_none() {
        return;
    }
    run.capture.push_str(text);
    if run.capture.len() > MAX_STEP_CAPTURE {
        let mut cut = run.capture.len() - MAX_STEP_CAPTURE;
        while !run.capture.is_char_boundary(cut) {
            cut += 1;
        }
        run.capture.drain(..cut);
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Put `plan` on the session as its steps (all pending) and its planning output.
pub(super) fn install_plan(
    entry: &Arc<SessionEntry>,
    resolver: Arc<dyn DebugResolver>,
    plan: DebugPlan,
) {
    for line in &plan.output {
        entry.append_output("console", format!("{line}\n"));
    }
    let steps: Vec<IdeDebugStep> = plan
        .steps
        .iter()
        .map(|s| IdeDebugStep {
            label: s.label.clone(),
            state: IdeDebugStepState::Pending,
            exit_code: None,
            summary: None,
            failed_tests: Vec::new(),
            program: s.program.clone(),
            test_framework: s.req.launch.test_framework.clone(),
            started_ms: None,
            ended_ms: None,
        })
        .collect();
    let n = steps.len();
    let title = plan.title;
    *entry.steps.lock().unwrap_or_else(|e| e.into_inner()) = Some(StepRun {
        resolver,
        total: n,
        pending: plan.steps.into(),
        current: None,
        capture: String::new(),
        exit_code: None,
        skip: false,
    });
    entry.mutate_and_broadcast(|snap| {
        snap.title = title;
        snap.steps = steps;
        snap.current_step = None;
    });
}

/// Start the plan's next step that starts (a step that cannot start is marked
/// `error` and the run goes on), or end the session when none is left. The
/// caller holds the lifecycle lock; a Stop or Skip superseding `my_gen` ends it.
pub(super) async fn start_next_step(
    sessions: &Arc<DashMap<String, Arc<SessionEntry>>>,
    entry: &Arc<SessionEntry>,
    my_gen: u64,
) {
    loop {
        if entry.boot_generation.load(Ordering::SeqCst) != my_gen || entry.is_terminal() {
            return;
        }
        let Some((step, resolver)) = begin_next_step(entry) else {
            finish_run(sessions, entry);
            return;
        };
        if let Some(err) = step.error.clone() {
            end_step(entry, None, Some(err));
            continue;
        }
        let ctx = entry
            .orig_ctx
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let resolve = resolver.resolve(&step.req, &ctx);
        let resolved = match entry.unless_superseded(my_gen, resolve).await {
            None => return,
            Some(Ok(r)) => r,
            Some(Err(err)) => {
                end_step(entry, None, Some(first_line(&err.to_string())));
                continue;
            }
        };
        for item in &resolved.initial_output {
            entry.append_output(&item.category, item.text.clone());
        }
        entry.mutate_and_broadcast(|snap| {
            snap.language = resolved.language.clone();
            snap.adapter = resolved.adapter_name.clone();
        });
        let boot = boot_session_connection(sessions, entry, resolved, &step.req, &ctx);
        match entry.unless_superseded(my_gen, boot).await {
            None | Some(Ok(())) => return,
            Some(Err(err)) => {
                entry.disconnect_and_kill();
                end_step(entry, None, Some(first_line(&err.to_string())));
            }
        }
    }
}

fn first_line(msg: &str) -> String {
    let msg = msg
        .split_once(": ")
        .filter(|(head, _)| matches!(*head, "invalid" | "upstream" | "not found" | "internal"))
        .map_or(msg, |(_, rest)| rest);
    msg.lines().next().unwrap_or("").trim().to_string()
}

/// Take the next planned step: it becomes the live one and its separator is printed.
fn begin_next_step(entry: &Arc<SessionEntry>) -> Option<(PlannedStep, Arc<dyn DebugResolver>)> {
    let (step, idx, total, resolver) = {
        let mut guard = entry.steps.lock().unwrap_or_else(|e| e.into_inner());
        let run = guard.as_mut()?;
        let step = run.pending.pop_front()?;
        run.current = Some(step.kind);
        run.capture.clear();
        run.exit_code = None;
        run.skip = false;
        let idx = run.total - run.pending.len() - 1;
        (step, idx, run.total, Arc::clone(&run.resolver))
    };
    entry.append_output(
        "console",
        format!("── Step {}/{total} · {} ──\n", idx + 1, step.label),
    );
    entry.mutate_and_broadcast(|snap| {
        snap.state = IdeDebugState::Starting;
        snap.stopped_reason = None;
        snap.paused_file = None;
        snap.paused_line = None;
        snap.selected_frame_file = None;
        snap.selected_frame_line = None;
        snap.threads.clear();
        snap.stack_frames.clear();
        snap.scopes.clear();
        snap.inline_values.clear();
        snap.current_step = Some(idx as u32);
        if let Some(s) = snap.steps.get_mut(idx) {
            s.state = IdeDebugStepState::Running;
            s.started_ms = Some(now_ms());
        }
    });
    Some((step, resolver))
}

/// Record the live step's result: skipped, could not start (`error`), or what
/// its exit code and output say.
fn end_step(entry: &SessionEntry, exit_code: Option<i64>, start_error: Option<String>) {
    let (kind, capture, skip, exit_code) = {
        let mut guard = entry.steps.lock().unwrap_or_else(|e| e.into_inner());
        let Some(run) = guard.as_mut() else { return };
        let Some(kind) = run.current.take() else {
            return;
        };
        (
            kind,
            std::mem::take(&mut run.capture),
            run.skip,
            exit_code.or(run.exit_code),
        )
    };
    let result = if skip {
        StepResult::of(IdeDebugStepState::Skipped, Some("skipped".into()))
    } else if let Some(err) = start_error {
        StepResult::of(IdeDebugStepState::Error, Some(err))
    } else {
        step_result(kind, exit_code, &capture)
    };
    entry.mutate_and_broadcast(|snap| {
        if let Some(s) = snap
            .current_step
            .and_then(|i| snap.steps.get_mut(i as usize))
        {
            s.state = result.state;
            s.summary = result.summary;
            s.failed_tests = result.failed_tests;
            s.exit_code = exit_code;
            s.ended_ms = Some(now_ms());
        }
        snap.current_step = None;
    });
}

/// The last step is done: print the run's result and end the session.
fn finish_run(sessions: &DashMap<String, Arc<SessionEntry>>, entry: &Arc<SessionEntry>) {
    let steps = entry
        .snapshot
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .steps
        .clone();
    entry.append_output("console", format!("{}\n", run_summary(&steps)));
    entry.mark_terminated();
    evict_terminated_sessions(sessions);
}

/// `3 steps · 2 passed · 1 failed (Api.Tests (net11.0): 1 of 17 failed)`.
pub(crate) fn run_summary(steps: &[IdeDebugStep]) -> String {
    let count = |st: IdeDebugStepState| steps.iter().filter(|s| s.state == st).count();
    let mut parts = vec![format!("{} steps", steps.len())];
    for (st, word) in [
        (IdeDebugStepState::Passed, "passed"),
        (IdeDebugStepState::Failed, "failed"),
        (IdeDebugStepState::NoTests, "with no tests"),
        (IdeDebugStepState::Skipped, "skipped"),
    ] {
        let n = count(st);
        if n > 0 {
            parts.push(format!("{n} {word}"));
        }
    }
    match count(IdeDebugStepState::Error) {
        0 => {}
        1 => parts.push("1 step couldn't start".into()),
        n => parts.push(format!("{n} steps couldn't start")),
    }
    let failed: Vec<String> = steps
        .iter()
        .filter(|s| s.state == IdeDebugStepState::Failed)
        .map(|s| format!("{}: {}", s.label, s.summary.as_deref().unwrap_or("failed")))
        .collect();
    let mut out = parts.join(" · ");
    if !failed.is_empty() {
        out.push_str(&format!(" ({})", failed.join("; ")));
    }
    out
}

struct StepResult {
    state: IdeDebugStepState,
    summary: Option<String>,
    failed_tests: Vec<String>,
}

impl StepResult {
    fn of(state: IdeDebugStepState, summary: Option<String>) -> Self {
        Self {
            state,
            summary,
            failed_tests: Vec::new(),
        }
    }
}

/// The exit code decides pass or fail; the output only gives the counts.
fn step_result(kind: StepKind, exit_code: Option<i64>, capture: &str) -> StepResult {
    let lines: Vec<String> = capture.lines().map(str::to_string).collect();
    let parsed = parse_test_output(kind.runner(), &lines);
    let mut failed_tests: Vec<String> = parsed
        .iter()
        .filter(|t| t.status == IdeTestCaseStatus::Failed)
        .map(|t| t.name.clone())
        .take(5)
        .collect();
    if kind == StepKind::Zig && failed_tests.is_empty() {
        failed_tests = zig_failed_tests(capture);
    }
    let counts = summary_counts(capture).or_else(|| {
        (!parsed.is_empty()).then(|| {
            let n = |st: IdeTestCaseStatus| parsed.iter().filter(|t| t.status == st).count();
            Counts {
                passed: n(IdeTestCaseStatus::Passed),
                failed: n(IdeTestCaseStatus::Failed),
                total: parsed.len(),
            }
        })
    });
    let state = match (kind, exit_code) {
        (_, Some(0)) if kind != StepKind::Zig && counts.is_some_and(|c| c.total == 0) => {
            IdeDebugStepState::NoTests
        }
        (_, Some(0)) => IdeDebugStepState::Passed,
        (StepKind::Mtp, Some(2)) | (StepKind::VsTest | StepKind::Zig, Some(1)) => {
            IdeDebugStepState::Failed
        }
        (StepKind::Mtp, Some(8)) => IdeDebugStepState::NoTests,
        _ => IdeDebugStepState::Error,
    };
    let summary = match state {
        IdeDebugStepState::Passed => Some(match counts {
            Some(c) => format!("{} passed", c.passed),
            None => "passed".into(),
        }),
        IdeDebugStepState::Failed => Some(match counts {
            Some(c) if c.failed > 0 => format!("{} of {} failed", c.failed, c.total),
            _ => format!("failed (exit {})", exit_code.unwrap_or(1)),
        }),
        IdeDebugStepState::NoTests => Some("no tests".into()),
        _ => Some(match exit_code {
            Some(code) => format!("exited with code {code}"),
            None => "ended without an exit code".into(),
        }),
    };
    StepResult {
        state,
        summary,
        failed_tests,
    }
}

/// A Zig test binary run in a terminal prints `2/3 extra.test.extra fails...` and,
/// after the test's own output, `FAIL (TestExpectedEqual)`: the failed tests'
/// names (`extra fails`, as a gutter Debug filters them), at most 5.
fn zig_failed_tests(capture: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut last: Option<&str> = None;
    for line in capture.lines() {
        let t = line.trim();
        if let Some((head, rest)) = t.split_once(' ') {
            if head.contains('/')
                && head
                    .split('/')
                    .all(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
            {
                last = rest.split_once("...").map(|(name, _)| name);
            }
        }
        if t.contains("FAIL (") {
            if let Some(name) = last.take() {
                let short = name.split_once(".test.").map_or(name, |(_, n)| n);
                if out.len() < 5 {
                    out.push(short.to_string());
                }
            }
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Counts {
    passed: usize,
    failed: usize,
    total: usize,
}

/// The runner's own summary: VSTest `Failed: 1, Passed: 16, Skipped: 0, Total: 17`,
/// Microsoft.Testing.Platform `total: 3` / `failed: 0` / `succeeded: 3` lines,
/// Zig `All 12 tests passed.` / `3 passed; 0 skipped; 1 failed.`.
fn summary_counts(capture: &str) -> Option<Counts> {
    fn num_after(text: &str, key: &str) -> Option<usize> {
        let at = text.find(key)? + key.len();
        let digits: String = text[at..]
            .trim_start()
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        digits.parse().ok()
    }
    fn num_before(text: &str, key: &str) -> Option<usize> {
        let head = text[..text.find(key)?].trim_end();
        let digits: String = head
            .chars()
            .rev()
            .take_while(char::is_ascii_digit)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        digits.parse().ok()
    }
    let mut mtp = (None, None, None);
    let mut found = None;
    for line in capture.lines() {
        let t = line.trim();
        if t.contains("Total:") && t.contains("Passed:") {
            if let (Some(failed), Some(passed), Some(total)) = (
                num_after(t, "Failed:"),
                num_after(t, "Passed:"),
                num_after(t, "Total:"),
            ) {
                found = Some(Counts {
                    passed,
                    failed,
                    total,
                });
            }
        } else if let Some(rest) = t.strip_prefix("All ") {
            if let Some(n) =
                num_before(rest, " tests passed").or_else(|| num_before(rest, " test passed"))
            {
                found = Some(Counts {
                    passed: n,
                    failed: 0,
                    total: n,
                });
            }
        } else if t.ends_with(" failed.") && t.contains(" passed; ") {
            if let (Some(passed), Some(failed)) =
                (num_before(t, " passed;"), num_before(t, " failed."))
            {
                let skipped = num_before(t, " skipped;").unwrap_or(0);
                found = Some(Counts {
                    passed,
                    failed,
                    total: passed + failed + skipped,
                });
            }
        } else if let Some(n) = t.strip_prefix("total:").and_then(|r| r.trim().parse().ok()) {
            mtp.0 = Some(n);
        } else if let Some(n) = t
            .strip_prefix("failed:")
            .and_then(|r| r.trim().parse().ok())
        {
            mtp.1 = Some(n);
        } else if let Some(n) = t
            .strip_prefix("succeeded:")
            .and_then(|r| r.trim().parse().ok())
        {
            mtp.2 = Some(n);
        }
    }
    if let (Some(total), Some(failed), Some(passed)) = mtp {
        return Some(Counts {
            passed,
            failed,
            total,
        });
    }
    found
}

/// The live step's process ended (`terminated`, adapter EOF, or the `exited`
/// fallback on `wire`): record its result and start the next step. Returns
/// `false` for a session without steps (the caller ends it as before).
pub(super) fn on_step_end(
    sessions: &Arc<DashMap<String, Arc<SessionEntry>>>,
    entry: &Arc<SessionEntry>,
    wire: &Arc<super::session::DapWireClient>,
) -> bool {
    if entry
        .steps
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_none()
    {
        return false;
    }
    // Only the live step's own connection ends it, and only once.
    let conn = {
        let mut slot = entry.conn.write().unwrap_or_else(|e| e.into_inner());
        match slot.as_ref() {
            Some(c) if Arc::ptr_eq(&c.wire, wire) => slot.take(),
            _ => None,
        }
    };
    let Some(conn) = conn else {
        return true;
    };
    conn.wire.retire();
    let my_gen = entry.boot_generation.load(Ordering::SeqCst);
    let sessions = Arc::clone(sessions);
    let entry = Arc::clone(entry);
    tokio::spawn(async move {
        let exit = entry
            .unless_superseded(my_gen, companion_exit_code(&conn))
            .await
            .flatten();
        teardown_conn(&conn).await;
        let _guard = entry.lifecycle_lock.lock().await;
        if entry.boot_generation.load(Ordering::SeqCst) != my_gen || entry.is_terminal() {
            return;
        }
        end_step(&entry, exit, None);
        start_next_step(&sessions, &entry, my_gen).await;
    });
    true
}

/// A VSTest step's result is `dotnet test`'s exit code: wait for it a little.
async fn companion_exit_code(conn: &ActiveConnection) -> Option<i64> {
    if !conn.guard.has_companion() {
        return None;
    }
    let deadline = tokio::time::Instant::now() + COMPANION_EXIT_WAIT;
    loop {
        if let Some(code) = conn.guard.companion_exit_code() {
            return Some(i64::from(code));
        }
        if tokio::time::Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Skip to next step: end the live step (marked `skipped`) and start the next.
/// `false` when no step is left to run (the caller stops the session).
pub(super) async fn skip_step(
    sessions: &Arc<DashMap<String, Arc<SessionEntry>>>,
    entry: &Arc<SessionEntry>,
) -> bool {
    {
        let mut guard = entry.steps.lock().unwrap_or_else(|e| e.into_inner());
        match guard.as_mut() {
            Some(run) if !run.pending.is_empty() => run.skip = true,
            _ => return false,
        }
    }
    let my_gen = entry.bump_generation();
    let conn = entry.conn.write().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(conn) = conn {
        conn.wire.retire();
        teardown_conn(&conn).await;
    }
    let _guard = entry.lifecycle_lock.lock().await;
    if entry.boot_generation.load(Ordering::SeqCst) != my_gen || entry.is_terminal() {
        return true;
    }
    end_step(entry, None, None);
    start_next_step(sessions, entry, my_gen).await;
    true
}

#[cfg(test)]
#[path = "session_steps_tests.rs"]
mod tests;
