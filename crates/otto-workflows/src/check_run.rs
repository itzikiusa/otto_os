//! The `check` step's deterministic half (goal.md §2.1): its params, the
//! command criteria through otto-automation's `verify_command` (exit status
//! is ground truth; the evidence keeps an 8 KiB tail, every byte goes to
//! `<step base>.<criterion id>.log`), the rule criteria over the iteration
//! context, and the evaluator's and completion review's handoffs. The
//! decisions themselves are [`crate::criteria`]; otto-server's
//! `nodes/check.rs` only runs the agent turns and waits for a person.

use crate::criteria::{self, Assessed, Evaluation};
use crate::script_run::{ScriptHost, ScriptRun};
use otto_automation::verify_command::{self, End, Spec};
use otto_core::workflows::{Criterion, VerifyKind};
use serde_json::Value;
use std::time::Duration;

/// A `check` node's params.
#[derive(Debug, Clone)]
pub struct CheckParams {
    pub criteria: Vec<Criterion>,
    /// `evaluator.{provider, model, effort}` (empty = the run default).
    pub provider: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub instructions: Option<String>,
    pub final_review: bool,
}

impl CheckParams {
    pub fn parse(p: &Value) -> Result<Self, String> {
        let criteria: Vec<Criterion> =
            serde_json::from_value(p.get("criteria").cloned().unwrap_or(Value::Null))
                .map_err(|e| format!("check: criteria: {e}"))?;
        if criteria.is_empty() {
            return Err("check: needs at least one criterion".into());
        }
        let text = |v: Option<&Value>| {
            v.and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        let ev = p.get("evaluator");
        Ok(Self {
            provider: text(ev.and_then(|e| e.get("provider"))),
            model: text(ev.and_then(|e| e.get("model"))),
            effort: text(ev.and_then(|e| e.get("effort"))),
            instructions: text(p.get("instructions")),
            final_review: p.get("final_review").and_then(Value::as_bool) == Some(true),
            criteria,
        })
    }

    pub fn needs_agent(&self) -> bool {
        self.criteria.iter().any(|c| c.verify == VerifyKind::Agent)
    }
}

/// The command and rule criteria, decided before any agent runs; `ictx` is
/// the loop's iteration context `{iteration, steps, last, input}`. `Err`
/// only when a command could not run at all.
pub async fn decide_commands_and_rules(
    host: &dyn ScriptHost,
    at: &ScriptRun<'_>,
    criteria: &[Criterion],
    ictx: &Value,
) -> Result<Vec<Assessed>, String> {
    let mut out = vec![];
    for c in criteria {
        let (met, evidence, by) = match c.verify {
            VerifyKind::Command => {
                let (met, evidence) = run_command(host, at, c).await?;
                (met, evidence, "command")
            }
            VerifyKind::Expr => {
                let (met, evidence) = rule(c, ictx);
                (met, evidence, "rule")
            }
            _ => continue,
        };
        out.push(Assessed {
            id: c.id.clone(),
            met,
            evidence,
            verified_by: Some(by.into()),
        });
    }
    Ok(out)
}

async fn run_command(
    host: &dyn ScriptHost,
    at: &ScriptRun<'_>,
    c: &Criterion,
) -> Result<(bool, String), String> {
    let command = c.command.as_deref().unwrap_or("").trim();
    if command.is_empty() {
        return Ok((false, format!("{}: no command to run", c.id)));
    }
    let mut vars = vec![
        ("OTTO_RUN_ID".to_string(), at.run_id.to_string()),
        (
            "OTTO_CONTEXT_DIR".to_string(),
            at.files.dir_str().unwrap_or_default(),
        ),
        ("OTTO_WORKTREE".to_string(), at.run_cwd.to_string()),
    ];
    if let Some(path) = host.login_path() {
        vars.push(("PATH".into(), path));
    }
    let log = at
        .files
        .step_md_path(at.step_base)
        .map(|p| p.with_extension(format!("{}.log", c.id)));
    let sandbox = host.sandbox(at.run_cwd).await;
    let timeout = criteria::command_timeout_s(c);
    let spec = Spec {
        env: &vars,
        sandbox: sandbox.as_ref(),
        cache_tag: Some(at.run_id),
        output_cap: criteria::VERIFY_OUTPUT_TAIL_BYTES,
        log_file: log.as_deref(),
        ..Spec::new(at.run_cwd, command, Duration::from_secs(timeout))
    };
    // Cancel drops this future, which kills the process group.
    let done = verify_command::execute(&spec, std::future::pending()).await;
    match done.end {
        End::Exited(code) => Ok(criteria::command_evidence(code, &done.output)),
        End::TimedOut => Ok((
            false,
            criteria::with_output(
                &format!("verify command timed out after {timeout} s"),
                &done.output,
            ),
        )),
        End::Canceled => Err(format!("{}: canceled", c.id)),
        End::Failed(m) => Err(format!("{}: could not run the command: {m}", c.id)),
    }
}

/// A rule criterion: `expr` over the iteration context, with the brief of
/// a step it names (`steps.review.passed is false — review 72/100 …`).
fn rule(c: &Criterion, ictx: &Value) -> (bool, String) {
    let expr = c.expr.as_deref().unwrap_or("").trim();
    let met = otto_core::expr::eval_bool(expr, ictx);
    let mut evidence = format!("{expr} is {met}");
    if let Some(steps) = ictx.get("steps").and_then(Value::as_object) {
        let named = steps
            .iter()
            .find(|(name, _)| expr.contains(&format!("steps.{name}")));
        if let Some(brief) = named.and_then(|(_, out)| crate::step_util::brief_summary(out)) {
            evidence.push_str(" — ");
            evidence.push_str(&crate::step_util::truncate_sentences(&brief, 300));
        }
    }
    (met, evidence)
}

/// The evaluator's answer: its handoff file, else its reply.
pub fn evaluation(handoff: Option<&str>, reply: &str) -> Option<Evaluation> {
    handoff
        .and_then(criteria::parse_evaluation)
        .or_else(|| criteria::parse_evaluation(reply))
}

/// The completion review's prompt: the goal (criteria and instructions) and
/// where its JSON goes.
pub fn review_prompt(p: &CheckParams, handoff: &str) -> String {
    let mut s = format!(
        "{}\n\nThe goal's criteria:\n",
        criteria::COMPLETION_REVIEW_PROMPT
    );
    for c in &p.criteria {
        s.push_str(&format!("- {}\n", c.text));
    }
    if let Some(i) = &p.instructions {
        s.push_str(&format!("\n{i}\n"));
    }
    s.push_str(&format!("\nWrite the JSON to {handoff}."));
    s
}

/// The checkpoint id of this inner check in the previous iteration
/// (`<loop>#<i-1>.<k>`), whose output carries the repeat signature.
pub fn previous_iteration_id(node_id: &str) -> Option<String> {
    let (loop_id, rest) = node_id.split_once('#')?;
    let (i, k) = rest.split_once('.')?;
    let i: u64 = i.parse().ok()?;
    (i > 1).then(|| format!("{loop_id}#{}.{k}", i - 1))
}

#[cfg(test)]
#[path = "check_run_tests.rs"]
mod tests;
