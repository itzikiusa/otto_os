//! The `check` node's and the loop's pure decisions (goal.md §2.1–§2.2,
//! DR-107), ported from otto-automation's Goal Loops policy
//! (`goal_loop_policy`, `goal_loop_parse`, `goal_loop/completion`), which
//! keeps working for the old controller until the goal wave deletes it.
//!
//! One pass over a check: the evaluator's parsed answer is kept immutable as
//! `raw`; [`decide`] starts from a clone of it every time and applies
//! [`require_evidence`] → [`reconcile`] → [`complete_when_all_met`], so a
//! person's record that arrives later can never meet a verdict that an
//! earlier pass already changed. Command and rule results are ground truth
//! (decided before the evaluator runs); a human criterion is met only by a
//! person's record bound to the criterion's exact text and the current
//! iteration. [`loop_stop`] is every stop rule of a loop.

use otto_core::workflows::{
    Criterion, CriterionProgress, HoldKind, HumanEntry, LoopLimits, VerifyKind,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// Separates a criterion's evidence headline from the verify command's
/// output tail stored after it; signatures keep only the headline.
pub const VERIFY_OUTPUT_MARKER: &str = "\n--- verify output ---\n";
/// How much of a verify command's output a criterion keeps (its tail).
pub const VERIFY_OUTPUT_TAIL_BYTES: usize = 8 * 1024;
/// Evidence for a criterion the evaluator called met without saying why.
pub const NO_EVIDENCE: &str =
    "Marked met without evidence, so it counts as not met. The evaluator must cite what it checked.";
/// Evidence for every agent criterion when the evaluator's answer is missing
/// or not valid JSON (the step does not fail; the repeated-failure stop
/// catches a checker that keeps failing).
pub const NO_VERDICT: &str = "The checker gave no valid verdict";
pub const AWAITING_HUMAN: &str = "Awaiting human verification";
pub const NOT_ASSESSED: &str = "not assessed";
pub const UNKNOWN_CHECK: &str = "not met: unknown check";
/// The repeated-failure stop's hold reason.
pub const REPEATED_FAILURE: &str =
    "Two checks in a row failed the same way — a new approach is needed";

/// The independent evaluator's instructions (Goal Loops' evaluator prompt,
/// reworded for the check's own JSON shape).
pub const EVALUATOR_PROMPT: &str = "You are the independent checker of a workflow's work. The work was just done in this working directory. For EACH criterion below decide met=true/false with concrete EVIDENCE (run a command, read the file, check the behaviour). Be skeptical: do not mark a criterion met without evidence; do not edit anything. Then give a verdict (achieved only if ALL criteria are met; continue if more work remains; blocked if progress needs a person) and feedback for the next attempt.";
/// The completion review (`final_review: true`), on its own fresh thread.
pub const COMPLETION_REVIEW_PROMPT: &str = "Independently review the final work against the goal. Report concrete unresolved defects. Return JSON {\"approved\":true|false,\"findings\":[string]}. Approve only when no material issue remains.";

/// `headline` plus the last [`VERIFY_OUTPUT_TAIL_BYTES`] of `output`.
pub fn with_output(headline: &str, output: &str) -> String {
    let output = output.trim_end();
    if output.trim().is_empty() {
        return headline.to_string();
    }
    let mut start = output.len().saturating_sub(VERIFY_OUTPUT_TAIL_BYTES);
    while !output.is_char_boundary(start) {
        start += 1;
    }
    let cut = if start > 0 {
        "… earlier output omitted\n"
    } else {
        ""
    };
    format!("{headline}{VERIFY_OUTPUT_MARKER}{cut}{}", &output[start..])
}

/// The evidence without any stored verify output.
pub fn evidence_headline(evidence: &str) -> &str {
    evidence
        .split_once(VERIFY_OUTPUT_MARKER)
        .map_or(evidence, |(head, _)| head)
}

/// A command criterion's evidence: exit status first, output tail after.
pub fn command_evidence(exit_code: i32, output: &str) -> (bool, String) {
    let met = exit_code == 0;
    let headline = if met {
        "verify command exited 0".to_string()
    } else {
        format!("verify command failed (exit {exit_code})")
    };
    (met, with_output(&headline, output))
}

/// The last non-blank, non-decoration line of a command's output, clipped.
pub fn last_meaningful_line(output: &str) -> Option<String> {
    output.lines().rev().find_map(|raw| {
        let line = raw.trim();
        if line.is_empty() {
            return None;
        }
        let only_decor = line.chars().all(|c| {
            c.is_whitespace()
                || matches!(c, '-' | '=' | '_' | '*' | '~' | '#' | '|' | '+' | '.' | ':')
        });
        if only_decor {
            return None;
        }
        Some(otto_core::agent_runtime::clip(line, 160))
    })
}

/// A command criterion's structured progress: exit code and last meaningful line.
pub fn command_progress(exit_code: i32, output: &str) -> CriterionProgress {
    let mut values = BTreeMap::new();
    values.insert("exit_code".into(), json!(exit_code));
    let last_line = last_meaningful_line(output);
    if let Some(ref line) = last_line {
        values.insert("last_line".into(), json!(line));
    }
    let label = if exit_code == 0 {
        None
    } else {
        Some(match last_line {
            Some(line) => format!("exit {exit_code} — {line}"),
            None => format!("exit {exit_code}"),
        })
    };
    CriterionProgress {
        value: None,
        target: None,
        unit: None,
        label,
        values,
    }
}

/// A command criterion's timeout: default 600 s, 10..=3,600.
pub fn command_timeout_s(c: &Criterion) -> u64 {
    c.timeout_s.unwrap_or(600).clamp(10, 3_600)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    #[default]
    Continue,
    Achieved,
    Blocked,
}

/// One criterion's assessment.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Assessed {
    pub id: String,
    #[serde(default)]
    pub met: bool,
    #[serde(default)]
    pub evidence: String,
    /// `command` | `rule` | `agent` | the person's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified_by: Option<String>,
    /// Numeric progress, blocker label, and values inspected by the check.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<CriterionProgress>,
}

/// The evaluator's answer, and the decided result after a pass.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Evaluation {
    #[serde(default)]
    pub criteria: Vec<Assessed>,
    #[serde(default)]
    pub verdict: Verdict,
    #[serde(default)]
    pub feedback: String,
    #[serde(default)]
    pub rationale: String,
}

/// The evaluator prompt: instructions, the agent criteria with what to
/// inspect, the command and rule results already decided, the author's
/// `instructions`, and where to write the JSON.
pub fn evaluator_prompt(
    criteria: &[Criterion],
    decided: &[Assessed],
    instructions: Option<&str>,
    handoff: &str,
) -> String {
    let mut p = format!("{EVALUATOR_PROMPT}\n\nCriteria to judge:\n");
    for c in criteria.iter().filter(|c| c.verify == VerifyKind::Agent) {
        p.push_str(&format!("- {}: {}", c.id, c.text));
        if !c.how.trim().is_empty() {
            p.push_str(&format!(" (inspect: {})", c.how.trim()));
        }
        p.push('\n');
    }
    if !decided.is_empty() {
        p.push_str(
            "\nAlready decided by commands and rules (context only — decided, do not re-judge):\n",
        );
        for d in decided {
            let word = if d.met { "met" } else { "not met" };
            p.push_str(&format!(
                "- {}: {word} — {}\n",
                d.id,
                evidence_headline(&d.evidence)
            ));
        }
    }
    if let Some(i) = instructions.map(str::trim).filter(|i| !i.is_empty()) {
        p.push_str(&format!("\n{i}\n"));
    }
    p.push_str(&format!(
        "\nWrite JSON to {handoff}: {{\"criteria\":[{{\"id\",\"met\",\"evidence\"}}],\"verdict\":\"continue|achieved|blocked\",\"feedback\",\"rationale\"}}"
    ));
    p
}

/// JSON objects in agent output: fenced ```json bodies first, then every
/// balanced `{…}` span.
fn json_candidates(text: &str) -> Vec<Value> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("```json") {
        let body = &rest[start + 7..];
        let Some(end) = body.find("```") else { break };
        out.extend(serde_json::from_str::<Value>(body[..end].trim()).ok());
        rest = &body[end + 3..];
    }
    let mut rest = text;
    while let Some(span) = otto_core::text::find_json_object(rest) {
        out.extend(serde_json::from_str::<Value>(span).ok());
        let next = span.as_ptr() as usize - rest.as_ptr() as usize + span.len();
        rest = &rest[next..];
    }
    out
}

/// The evaluator's verdict from its (possibly fenced or prose-wrapped)
/// output: the first object with `verdict` and `criteria` that parses.
pub fn parse_evaluation(text: &str) -> Option<Evaluation> {
    json_candidates(text)
        .into_iter()
        .filter(|v| v.get("verdict").is_some() && v.get("criteria").is_some())
        .find_map(|v| serde_json::from_value(v).ok())
}

/// The completion review passes only with `approved == true` and no
/// findings (DR-116); otherwise, or with no valid JSON, its findings block.
pub fn review_passes(text: &str) -> (bool, Value) {
    let review = json_candidates(text).into_iter().find(|v| {
        v.get("approved").and_then(Value::as_bool).is_some()
            && v.get("findings").and_then(Value::as_array).is_some()
    });
    let Some(review) = review else {
        return (
            false,
            serde_json::json!({"approved": false, "findings": ["The completion review gave no valid answer"]}),
        );
    };
    let ok = review["approved"] == true && review["findings"].as_array().is_some_and(Vec::is_empty);
    (ok, review)
}

/// An unpassed review turns an achieved verdict into blocked with the
/// findings as feedback.
pub fn apply_review(eval: &mut Evaluation, passed: bool, review: &Value) {
    if passed || eval.verdict != Verdict::Achieved {
        return;
    }
    let findings: Vec<String> = review["findings"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|f| f.as_str().map_or_else(|| f.to_string(), str::to_string))
        .collect();
    eval.verdict = Verdict::Blocked;
    eval.feedback = format!(
        "The completion review has unresolved findings: {}",
        findings.join("; ")
    );
}

/// A `met: true` with blank evidence is a claim, not a check.
pub fn require_evidence(eval: &mut Evaluation) {
    for c in eval.criteria.iter_mut() {
        if c.met && c.evidence.trim().is_empty() {
            c.met = false;
            c.evidence = NO_EVIDENCE.into();
        }
    }
}

/// The revision a person's record is bound to: SHA-256 of the criterion's
/// canonical JSON — editing its text (or how it is checked) invalidates it.
pub fn revision(c: &Criterion) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(
        serde_json::to_vec(c).unwrap_or_default(),
    ))
}

/// The person's valid record for `c`: the newest `verify` entry for its id
/// at its current revision and in the current iteration, with evidence.
pub fn human_record<'a>(
    c: &Criterion,
    records: &'a [HumanEntry],
    iteration: Option<u32>,
) -> Option<&'a HumanEntry> {
    let rev = revision(c);
    records.iter().rev().find(|r| {
        r.kind == "verify"
            && r.ask.as_deref() == Some(c.id.as_str())
            && r.revision.as_deref() == Some(rev.as_str())
            && r.iteration == iteration
            && !r.text.trim().is_empty()
    })
}

/// Exactly one assessment per defined criterion, in definition order:
/// commands and rules from `decided` (the evaluator cannot change them),
/// agent criteria from the evaluator, human criteria only from a person's
/// valid record (`met: false` is unmet with their note, never blocked). An
/// achieved verdict with anything unmet goes back to continue.
pub fn reconcile(
    criteria: &[Criterion],
    decided: &[Assessed],
    records: &[HumanEntry],
    iteration: Option<u32>,
    eval: &mut Evaluation,
) {
    let claimed = std::mem::take(&mut eval.criteria);
    let unmet = |id: &str, why: &str| Assessed {
        id: id.to_string(),
        met: false,
        evidence: why.to_string(),
        verified_by: None,
        progress: None,
    };
    eval.criteria = criteria
        .iter()
        .map(|c| match c.verify {
            VerifyKind::Command | VerifyKind::Expr => decided
                .iter()
                .find(|d| d.id == c.id)
                .cloned()
                .unwrap_or_else(|| unmet(&c.id, NOT_ASSESSED)),
            VerifyKind::Agent => claimed
                .iter()
                .find(|a| a.id == c.id)
                .map(|a| Assessed {
                    verified_by: Some("agent".into()),
                    ..a.clone()
                })
                .unwrap_or_else(|| unmet(&c.id, NOT_ASSESSED)),
            VerifyKind::Human => match human_record(c, records, iteration) {
                Some(r) if r.met == Some(true) => Assessed {
                    id: c.id.clone(),
                    met: true,
                    evidence: format!("Verified by {}: {}", r.by, r.text.trim()),
                    verified_by: Some(r.by.clone()),
                    progress: None,
                },
                Some(r) => Assessed {
                    id: c.id.clone(),
                    met: false,
                    evidence: format!("Not met, says {}: {}", r.by, r.text.trim()),
                    verified_by: Some(r.by.clone()),
                    progress: None,
                },
                None => unmet(&c.id, AWAITING_HUMAN),
            },
            VerifyKind::Unknown => unmet(&c.id, UNKNOWN_CHECK),
        })
        .collect();
    if eval.verdict == Verdict::Achieved && eval.criteria.iter().any(|c| !c.met) {
        eval.verdict = Verdict::Continue;
    }
}

/// Every criterion met with evidence means done, whatever the evaluator
/// would still like to do (its wish is kept in the rationale). A blocked
/// verdict is left alone: it asks a person for something.
pub fn complete_when_all_met(eval: &mut Evaluation) -> bool {
    let all_met = !eval.criteria.is_empty()
        && eval
            .criteria
            .iter()
            .all(|c| c.met && !c.evidence.trim().is_empty());
    if eval.verdict != Verdict::Continue || !all_met {
        return false;
    }
    let wish = [eval.feedback.trim(), eval.rationale.trim()]
        .into_iter()
        .find(|s| !s.is_empty())
        .unwrap_or("no reason given");
    eval.verdict = Verdict::Achieved;
    eval.rationale = format!(
        "Every acceptance criterion is met with evidence, so the goal is done. The evaluator asked to continue: {wish}"
    );
    true
}

/// One pass's result: the decided evaluation and, when every non-human
/// criterion is met, the human criterion the step parks on (a person is never
/// asked to verify unfinished work).
#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    pub eval: Evaluation,
    pub park_on: Option<String>,
}

/// One pass from `raw` (`None`: the checker gave no valid verdict).
pub fn decide(
    raw: Option<&Evaluation>,
    criteria: &[Criterion],
    decided: &[Assessed],
    records: &[HumanEntry],
    iteration: Option<u32>,
) -> Decision {
    let mut eval = raw.cloned().unwrap_or_else(|| Evaluation {
        criteria: criteria
            .iter()
            .filter(|c| c.verify == VerifyKind::Agent)
            .map(|c| Assessed {
                id: c.id.clone(),
                evidence: NO_VERDICT.into(),
                ..Assessed::default()
            })
            .collect(),
        ..Evaluation::default()
    });
    require_evidence(&mut eval);
    reconcile(criteria, decided, records, iteration, &mut eval);
    complete_when_all_met(&mut eval);
    let others_met = criteria
        .iter()
        .zip(&eval.criteria)
        .all(|(c, a)| c.verify == VerifyKind::Human || a.met);
    let park_on = others_met
        .then(|| {
            criteria.iter().find(|c| {
                c.verify == VerifyKind::Human && human_record(c, records, iteration).is_none()
            })
        })
        .flatten()
        .map(|c| c.id.clone());
    Decision { eval, park_on }
}

/// The unmet criteria's signature (ids + evidence headlines — a verify
/// command's output tail differs on every run and would hide a repeat) and
/// the repeated-failure count after this check, from the previous check's.
pub fn record_progress(previous: Option<(&str, u32)>, eval: &Evaluation) -> (String, u32) {
    let mut unmet: Vec<_> = eval
        .criteria
        .iter()
        .filter(|c| !c.met)
        .map(|c| (c.id.as_str(), evidence_headline(&c.evidence).trim()))
        .collect();
    if unmet.is_empty() {
        return (String::new(), 0);
    }
    unmet.sort();
    let signature = serde_json::to_string(&unmet).unwrap_or_default();
    let count = match previous {
        Some((sig, n)) if sig == signature => n + 1,
        _ => 1,
    };
    (signature, count)
}

/// One row of the check's output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CriterionRow {
    pub id: String,
    pub text: String,
    pub verify: VerifyKind,
    pub met: bool,
    pub evidence: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<CriterionProgress>,
}

/// The `check` node's output (goal.md §2.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckOutput {
    pub criteria: Vec<CriterionRow>,
    pub all_met: bool,
    pub met: usize,
    pub total: usize,
    pub progress_pct: u32,
    pub verdict: Verdict,
    pub feedback: String,
    /// ≤ 3 KiB: unmet criteria with evidence headlines, then the feedback.
    pub feedback_md: String,
    pub repeat_signature: String,
    pub repeated_failures: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<Value>,
}

pub const FEEDBACK_MD_MAX: usize = 3 * 1024;

impl CheckOutput {
    pub fn new(
        criteria: &[Criterion],
        eval: &Evaluation,
        (repeat_signature, repeated_failures): (String, u32),
        review: Option<Value>,
    ) -> Self {
        let rows: Vec<CriterionRow> = criteria
            .iter()
            .zip(&eval.criteria)
            .map(|(c, a)| CriterionRow {
                id: c.id.clone(),
                text: c.text.clone(),
                verify: c.verify,
                met: a.met,
                evidence: a.evidence.clone(),
                verified_by: a.verified_by.clone(),
                progress: a.progress.clone(),
            })
            .collect();
        let (met, total) = (rows.iter().filter(|r| r.met).count(), rows.len());
        let mut md = String::new();
        for r in rows.iter().filter(|r| !r.met) {
            md.push_str(&format!(
                "- Not met: {} — {}\n",
                r.text,
                evidence_headline(&r.evidence)
            ));
        }
        if !eval.feedback.trim().is_empty() {
            md.push_str(&format!("\n{}\n", eval.feedback.trim()));
        }
        Self {
            all_met: total > 0 && met == total,
            progress_pct: (met * 100).checked_div(total).unwrap_or(0) as u32,
            met,
            total,
            verdict: eval.verdict,
            feedback: eval.feedback.clone(),
            feedback_md: clip_bytes(&md, FEEDBACK_MD_MAX).to_string(),
            repeat_signature,
            repeated_failures,
            review,
            criteria: rows,
        }
    }

    /// The step file: the same table the next worker and the person read.
    pub fn step_file(&self) -> String {
        let mut md = format!(
            "**{} of {} criteria met** · verdict {}\n\n| Criterion | Check | Result | Evidence |\n|---|---|---|---|\n",
            self.met,
            self.total,
            serde_json::to_value(self.verdict).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
        );
        for r in &self.criteria {
            let how = match r.verify {
                VerifyKind::Command => "command",
                VerifyKind::Agent => "agent",
                VerifyKind::Human => "person",
                VerifyKind::Expr => "rule",
                VerifyKind::Unknown => "unknown",
            };
            let cell = |s: &str| s.replace('|', "\\|").replace('\n', " ");
            md.push_str(&format!(
                "| {} | {how} | {} | {} |\n",
                cell(&r.text),
                if r.met { "Met" } else { "Not met" },
                cell(evidence_headline(&r.evidence))
            ));
        }
        if !self.feedback.trim().is_empty() {
            md.push_str(&format!("\n{}\n", self.feedback.trim()));
        }
        md
    }
}

/// A loop's stop rules, from its params with the run's raised limits
/// (`limits_json.loops[<node id>]`) over them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopBudget {
    pub max_iterations: u32,
    pub max_active_s: Option<u64>,
    pub stop_on_repeat_failure: bool,
    pub hold_when_unmet: bool,
    /// The iteration a person resumed a Blocked hold at.
    pub unblocked_at: Option<u32>,
}

impl LoopBudget {
    /// `max_iterations` defaults to 3 and clamps to 1..=10, or 1..=50 when a
    /// time budget (300..=28,800 s) bounds the loop.
    pub fn new(params: &Value, limits: Option<&LoopLimits>) -> Self {
        let limits = limits.cloned().unwrap_or_default();
        let max_active_s = limits
            .max_active_s
            .or_else(|| params.get("max_active_s").and_then(Value::as_u64))
            .map(|s| s.clamp(300, 28_800));
        let cap = if max_active_s.is_some() { 50 } else { 10 };
        let max_iterations = limits
            .max_iterations
            .map(u64::from)
            .or_else(|| params.get("max_iterations").and_then(Value::as_u64))
            .unwrap_or(3)
            .clamp(1, cap) as u32;
        let flag = |k: &str| params.get(k).and_then(Value::as_bool).unwrap_or(false);
        Self {
            max_iterations,
            max_active_s,
            stop_on_repeat_failure: flag("stop_on_repeat_failure"),
            hold_when_unmet: flag("hold_when_unmet"),
            unblocked_at: limits.unblocked_at,
        }
    }
}

/// Where in a loop a stop is decided.
#[derive(Debug, Clone, Copy)]
pub enum StopPoint<'a> {
    /// Before iteration `used + 1` starts.
    BeforeIteration { used: u32 },
    /// Before an inner step starts; `active_ms` = the loop's banked time.
    BeforeStep { active_ms: u64 },
    /// After an inner `check` settled in `iteration`.
    AfterCheck {
        iteration: u32,
        check: &'a CheckOutput,
    },
    /// An inner step failed (without `continue_on_error`).
    StepFailed {
        iteration: u32,
        step: &'a str,
        error: &'a str,
    },
}

/// How a loop ends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stop {
    /// The check passed: the loop ends satisfied now.
    Satisfied,
    /// Ends with `satisfied: false` (today's ending), the log line why.
    Unmet(String),
    /// `hold_when_unmet`: hold the run (`output._workflow_hold`).
    Hold { kind: HoldKind, reason: String },
}

/// Every stop rule of a loop (goal.md §2.2). `None` = go on.
pub fn loop_stop(b: &LoopBudget, at: StopPoint<'_>) -> Option<Stop> {
    let unmet = |kind, reason: String| {
        Some(if b.hold_when_unmet {
            Stop::Hold { kind, reason }
        } else {
            Stop::Unmet(reason)
        })
    };
    match at {
        StopPoint::BeforeIteration { used } if used >= b.max_iterations => unmet(
            HoldKind::Exhausted,
            format!("Out of iterations: {used} of {} used", b.max_iterations),
        ),
        StopPoint::BeforeStep { active_ms } => {
            let cap = b.max_active_s?;
            (active_ms >= cap * 1000).then_some(())?;
            unmet(
                HoldKind::Exhausted,
                format!(
                    "Out of time: {} of {} active",
                    span(active_ms / 1000),
                    span(cap)
                ),
            )
        }
        StopPoint::AfterCheck { iteration, check } => {
            if check.verdict == Verdict::Achieved {
                return Some(Stop::Satisfied);
            }
            if b.unblocked_at.is_some_and(|at| iteration <= at) {
                return None;
            }
            if check.verdict == Verdict::Blocked {
                let first = first_sentence(&check.feedback);
                let reason = if first.is_empty() {
                    "The check needs a person".to_string()
                } else {
                    first
                };
                return unmet(HoldKind::Blocked, reason);
            }
            (b.stop_on_repeat_failure && check.repeated_failures >= 2).then_some(())?;
            unmet(HoldKind::Blocked, REPEATED_FAILURE.to_string())
        }
        StopPoint::StepFailed {
            iteration,
            step,
            error,
        } => b.hold_when_unmet.then(|| Stop::Hold {
            kind: HoldKind::Blocked,
            reason: otto_core::agent_runtime::clip(
                &format!(
                    "{step} failed at iteration {iteration}: {}",
                    first_sentence(error)
                ),
                300,
            ),
        }),
        StopPoint::BeforeIteration { .. } => None,
    }
}

/// How a loop's stop shows: `(satisfied, log line, _workflow_hold)`.
pub fn stop_outcome(stop: Option<Stop>, iteration: u64) -> (bool, Option<String>, Option<Value>) {
    match stop {
        Some(Stop::Satisfied) => (
            true,
            Some(format!(
                "loop: every criterion met at iteration {iteration}"
            )),
            None,
        ),
        Some(Stop::Unmet(reason)) => (false, Some(format!("loop: {reason}")), None),
        Some(Stop::Hold { kind, reason }) => (
            false,
            Some(format!("loop: holds the run — {reason}")),
            Some(serde_json::json!({"kind": kind, "reason": reason, "iteration": iteration})),
        ),
        None => (false, None, None),
    }
}

/// Resume's limits for the loop a hold stopped (goal.md §2.3): the person's
/// new totals over the run's earlier ones; a Blocked hold records the
/// iteration it resumes at (`unblocked_at`). An Exhausted hold resumes only
/// with room left above what is used — else the refusal the person reads.
pub fn resume_limits(
    params: &Value,
    prev: Option<&LoopLimits>,
    hold: &otto_core::workflows::RunHold,
    (used_iterations, used_ms): (u32, u64),
    (iterations, active_s): (Option<u32>, Option<u64>),
) -> Result<LoopLimits, String> {
    let mut limits = prev.cloned().unwrap_or_default();
    limits.max_iterations = iterations.or(limits.max_iterations);
    limits.max_active_s = active_s.or(limits.max_active_s);
    let b = LoopBudget::new(params, Some(&limits));
    match hold.kind {
        HoldKind::Exhausted if used_iterations >= b.max_iterations => Err(format!(
            "Raise the iteration limit to resume: {used_iterations} of {} iterations are used.",
            b.max_iterations
        )),
        HoldKind::Exhausted if b.max_active_s.is_some_and(|s| used_ms >= s * 1000) => Err(format!(
            "Raise the time limit to resume: {} of {} active time is used.",
            span(used_ms / 1000),
            span(b.max_active_s.unwrap_or_default())
        )),
        HoldKind::Blocked => {
            limits.unblocked_at = hold.iteration;
            Ok(limits)
        }
        _ => Ok(limits),
    }
}

fn clip_bytes(s: &str, max: usize) -> &str {
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn first_sentence(text: &str) -> String {
    let t = text.trim();
    let end = t
        .char_indices()
        .find(|&(i, c)| c == '\n' || (c == '.' && t[i + 1..].starts_with(char::is_whitespace)))
        .map_or(t.len(), |(i, c)| if c == '.' { i + 1 } else { i });
    otto_core::agent_runtime::clip(&t[..end], 300)
}

/// "2h", "1h 30m", "45m", "20s".
fn span(secs: u64) -> String {
    match (secs / 3600, secs % 3600 / 60) {
        (0, 0) => format!("{secs}s"),
        (0, m) => format!("{m}m"),
        (h, 0) if secs.is_multiple_of(3600) => format!("{h}h"),
        (h, m) => format!("{h}h {m}m"),
    }
}

#[cfg(test)]
#[path = "criteria_tests.rs"]
mod tests;
