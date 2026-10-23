//! The product runners' prompts and output contracts (pure, unit-testable):
//! the analysis lens and summarizer prompts, the rewrite, test-case and plan
//! prompts, and the JSON extraction their replies go through.

use super::{FoundLearning, FoundQuestion};

/// Append the "write your JSON to this file" instruction to a built prompt.
pub fn augment_with_out_path(base_prompt: &str, out_path: &str) -> String {
    format!(
        "{base_prompt}\n\n---\nWhen done, write ONLY your complete result JSON (no prose) \
         to a sibling temporary file beside {out_path}. Then atomically rename that file \
         to {out_path} as your LAST action. Never write partial output to the final path."
    )
}

// ---------------------------------------------------------------------------
// The verbatim OUTPUT CONTRACT appended to every analysis prompt.
// ---------------------------------------------------------------------------

const OUTPUT_CONTRACT: &str = r#"Investigate as needed, then respond with EXACTLY ONE ```json code block (no prose before or after) matching:
{"summary": "...", "related_repos": ["..."], "functionalities": ["..."],
 "integration_points": ["..."], "risks": ["..."],
 "open_questions": [{"text":"...","rationale":"...","category":"scope|data|ux|edge-case|dependency|other"}],
 "suggested_learnings": [{"kind":"pattern|avoid","title":"...","body":"..."}]}"#;

/// The verbatim consolidation contract for the summarizer agent. It MUST merge,
/// dedupe, and resolve conflicts across all lens outputs into ONE result.
const SUMMARIZER_CONTRACT: &str = r#"You are the SUMMARIZER. Several analysis lenses (possibly run on different AI providers) each produced findings about the SAME product story. Consolidate them into ONE result:
- Write a single cohesive summary.
- Merge related_repos, functionalities, integration_points, and risks across all lenses; remove duplicates.
- Merge AND DEDUPE the open questions (collapse near-identical questions into one; keep the clearest wording).
- RESOLVE conflicts: where lenses or providers disagree, decide the best answer and record what you reconciled in "conflict_notes".
- Merge suggested_learnings; drop duplicates.

Respond with EXACTLY ONE ```json code block (no prose before or after) matching:
{"summary": "...",
 "questions": [{"text":"...","rationale":"...","category":"scope|data|ux|edge-case|dependency|other"}],
 "related_repos": ["..."], "functionalities": ["..."], "integration_points": ["..."], "risks": ["..."],
 "suggested_learnings": [{"kind":"pattern|avoid","title":"...","body":"..."}],
 "conflict_notes": "..."}"#;

// ---------------------------------------------------------------------------
// Pure helpers (unit-testable without an agent)
// ---------------------------------------------------------------------------

/// Extract the first JSON value from an agent reply (```json fence first,
/// else the first balanced `{…}` that parses). The shared implementation:
/// see [`otto_core::text::extract_json`].
pub use otto_core::text::extract_json as extract_json_block;

/// The first JSON candidate in an agent reply (the ```` ```json ```` fence,
/// then each balanced `{…}` in order) that deserializes into `T`. Taking the
/// first span and THEN deserializing let a quoted `{}` or config snippet in a
/// preamble shadow the real answer, so the run read as "no parseable output".
pub fn extract_json_as<T: serde::de::DeserializeOwned>(raw: &str) -> Option<T> {
    let mut out = None;
    otto_core::text::extract_json_where(raw, |v| match serde_json::from_value::<T>(v.clone()) {
        Ok(t) => {
            out = Some(t);
            true
        }
        Err(_) => false,
    });
    out
}

/// The summarizer's reply. Every [`SummaryFindings`] field is optional, so a
/// `{}` decoy would deserialize: the candidate must carry an anchor key too.
pub(super) fn parse_summary_findings(raw: &str) -> Option<SummaryFindings> {
    otto_core::text::extract_json_where(raw, |v| {
        v.get("summary").is_some() || v.get("questions").is_some()
    })
    .and_then(|v| serde_json::from_value(v).ok())
}

/// Build the full analysis prompt for one agent.
///
/// The story context (body + learnings + Jira details) now lives in a separate
/// CONTEXT file that agents read. This keeps the prompt compact while allowing
/// the context to be arbitrarily large.
///
/// The prompt body is: skill instructions + file-read directive + OUTPUT CONTRACT.
/// An optional `prior_summary` is still inlined (it's a short text, not a large blob).
pub fn build_analysis_prompt(
    skill_body: &str,
    context_path: &str,
    prior_summary: Option<&str>,
) -> String {
    let mut prompt = String::new();

    // 1. Skill body (instructions for this agent role)
    prompt.push_str(skill_body);
    prompt.push_str("\n\n---\n\n");

    // 2. Context file directive — agents must read the file before answering.
    prompt.push_str(
        "The full story context is in this file — it may be LARGE, read it fully \
         (in chunks if needed) before answering:\n",
    );
    prompt.push_str(context_path);
    prompt.push_str("\n\n");

    // 3. Prior analysis summary (if re-running)
    if let Some(summary) = prior_summary {
        if !summary.is_empty() {
            prompt.push_str("## Prior Analysis Summary\n\n");
            prompt.push_str(summary);
            prompt.push_str("\n\n");
        }
    }

    // 4. OUTPUT CONTRACT (verbatim, required by the brief)
    prompt.push_str("---\n\n");
    prompt.push_str(OUTPUT_CONTRACT);
    prompt.push('\n');

    prompt
}

/// The consolidated result the summarizer agent must produce.
#[derive(serde::Deserialize, Default)]
pub struct SummaryFindings {
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub questions: Vec<FoundQuestion>,
    #[serde(default)]
    pub related_repos: Vec<String>,
    #[serde(default)]
    pub functionalities: Vec<String>,
    #[serde(default)]
    pub integration_points: Vec<String>,
    #[serde(default)]
    pub risks: Vec<String>,
    #[serde(default)]
    pub suggested_learnings: Vec<FoundLearning>,
    #[serde(default)]
    pub conflict_notes: String,
}

/// Build the summarizer prompt, feeding every successful lens's findings JSON
/// (each labelled `Lens <name> (<provider>):`) and instructing the agent to
/// consolidate, merge, dedupe, and resolve conflicts into ONE result.
///
/// `lenses` is `(name, provider, findings_json)` per successful lens agent.
pub fn build_summarizer_prompt(
    skill_body: &str,
    story_title: &str,
    lenses: &[(String, String, String)],
) -> String {
    let mut prompt = String::new();

    if !skill_body.is_empty() {
        prompt.push_str(skill_body);
        prompt.push_str("\n\n---\n\n");
    }

    prompt.push_str("## Story: ");
    prompt.push_str(story_title);
    prompt.push_str("\n\n");

    prompt.push_str("## Lens Findings to Consolidate\n\n");
    for (name, provider, json) in lenses {
        prompt.push_str("### Lens ");
        prompt.push_str(name);
        prompt.push_str(" (");
        prompt.push_str(provider);
        prompt.push_str("):\n\n");
        prompt.push_str(json);
        prompt.push_str("\n\n");
    }

    prompt.push_str("---\n\n");
    prompt.push_str(SUMMARIZER_CONTRACT);
    prompt.push('\n');

    prompt
}

// ---------------------------------------------------------------------------
// Rewrite output contract appended to the writer prompt.
// ---------------------------------------------------------------------------

const REWRITE_OUTPUT_CONTRACT: &str = r#"Respond with EXACTLY ONE ```json code block (no prose) matching:
{"title":"...","body_markdown":"...","change_notes":"..."}"#;

// ---------------------------------------------------------------------------
// build_rewrite_prompt — pure, unit-testable
// ---------------------------------------------------------------------------

/// Build the full rewrite prompt for the writer agent.
///
/// The story context (body, learnings, Jira details, answered questions) now
/// lives in a separate CONTEXT file. This keeps the prompt compact while allowing
/// arbitrarily large context.
///
/// Prompt body: writer skill body + file-read directive + OUTPUT CONTRACT.
pub fn build_rewrite_prompt(writer_skill_body: &str, context_path: &str) -> String {
    let mut prompt = String::new();

    // 1. Writer skill body
    prompt.push_str(writer_skill_body);
    prompt.push_str("\n\n---\n\n");

    // 2. Context file directive
    prompt.push_str(
        "The full story context (body, answered questions, analysis summary, learnings) is in \
         this file — it may be LARGE, read it fully (in chunks if needed) before writing:\n",
    );
    prompt.push_str(context_path);
    prompt.push_str("\n\n");

    // 3. OUTPUT CONTRACT (verbatim)
    prompt.push_str("---\n\n");
    prompt.push_str(REWRITE_OUTPUT_CONTRACT);
    prompt.push('\n');

    prompt
}

// ---------------------------------------------------------------------------
// Tests output contract appended to the test-cases prompt.
// ---------------------------------------------------------------------------

const TESTS_OUTPUT_CONTRACT: &str = r#"Respond with EXACTLY ONE ```json code block (no prose) matching:
{"testcases":[{"title":"...","category":"happy|validation|error|edge","priority":"high|medium|low","preconditions":["..."],"steps":["..."],"expected":"..."}]}"#;

// ---------------------------------------------------------------------------
// build_tests_prompt — pure, unit-testable
// ---------------------------------------------------------------------------

/// Build the full test-case generation prompt.
///
/// The story context (body, answered questions, analysis summary, learnings)
/// now lives in a separate CONTEXT file read by the agent.
///
/// Prompt body: tests skill body + file-read directive + OUTPUT CONTRACT.
pub fn build_tests_prompt(tests_skill_body: &str, context_path: &str) -> String {
    let mut prompt = String::new();

    // 1. Tests skill body
    prompt.push_str(tests_skill_body);
    prompt.push_str("\n\n---\n\n");

    // 2. Context file directive
    prompt.push_str(
        "The full story context (body, answered questions, analysis summary, learnings) is in \
         this file — it may be LARGE, read it fully (in chunks if needed) before generating tests:\n",
    );
    prompt.push_str(context_path);
    prompt.push_str("\n\n");

    // 3. OUTPUT CONTRACT (verbatim)
    prompt.push_str("---\n\n");
    prompt.push_str(TESTS_OUTPUT_CONTRACT);
    prompt.push('\n');

    prompt
}

// ---------------------------------------------------------------------------
// Plan output contract appended to the task-breakdown prompt.
// ---------------------------------------------------------------------------

const PLAN_OUTPUT_CONTRACT: &str = r#"Respond with EXACTLY ONE ```json code block (no prose) matching:
{"plan_markdown":"..."}
where plan_markdown is the full implementation plan as Markdown using level-3 headings of the form `### Task N: <title>`, each followed by `**Goal:** ...`, a checklist of steps as `- [ ]` items, and `**Verify:** ...`. Emit every checkbox as `- [ ]` (todo)."#;

/// Prepended to a planning prompt when the run is NON-interactive (the default).
/// Instructs the agent to work fully autonomously and NOT ask the user anything —
/// the operator is away and will only review the finished plan.
pub(super) const PLAN_AUTONOMY_DIRECTIVE: &str = r#"AUTONOMY: You are running UNATTENDED. The user is NOT available and will only review the finished plan at the end. Do NOT ask the user any questions, and do NOT wait for input. Where something is ambiguous, make reasonable assumptions, STATE those assumptions explicitly in the plan, and continue. Produce the complete plan autonomously and write the final JSON to the output path.

---

"#;

/// The verbatim consolidation contract for the PLAN summarizer agent. It merges
/// the candidate plans (possibly from different providers) into ONE plan obeying
/// the same `plan_markdown` output shape.
const PLAN_SUMMARIZER_CONTRACT: &str = r#"You are the SUMMARIZER. Several planning agents (possibly on different AI providers) each produced an implementation plan for the SAME story. Consolidate them into ONE best plan:
- Merge the tasks across all candidate plans; remove duplicates and collapse near-identical tasks.
- Keep the clearest wording and the most complete, correct steps; reconcile any disagreements, choosing the best approach.
- Preserve good coverage (setup, implementation, tests, verification) without redundant busywork.
- Keep the SAME Markdown structure: level-3 headings `### Task N: <title>`, each with `**Goal:** ...`, a `- [ ]` checklist of steps, and `**Verify:** ...`. Emit every checkbox as `- [ ]` (todo).

Respond with EXACTLY ONE ```json code block (no prose) matching:
{"plan_markdown":"..."}"#;

/// Build the plan-summarizer prompt: feed every candidate plan (labelled by
/// provider) and instruct consolidation into one plan, same output contract.
/// `candidates` is `(provider, plan_markdown)` per successful planner.
pub(super) fn build_plan_summarizer_prompt(
    skill_body: &str,
    story_title: &str,
    context_path: &str,
    candidates: &[(String, String)],
) -> String {
    let mut prompt = String::new();

    if !skill_body.is_empty() {
        prompt.push_str(skill_body);
        prompt.push_str("\n\n---\n\n");
    }

    prompt.push_str("## Story: ");
    prompt.push_str(story_title);
    prompt.push_str("\n\n");

    prompt.push_str(
        "The full story context (body, answered questions, analysis summary, approved test \
         cases, learnings) is in this file — read it fully before consolidating:\n",
    );
    prompt.push_str(context_path);
    prompt.push_str("\n\n");

    prompt.push_str("## Candidate Plans to Consolidate\n\n");
    for (provider, plan) in candidates {
        prompt.push_str("### Plan (");
        prompt.push_str(provider);
        prompt.push_str("):\n\n");
        prompt.push_str(plan);
        prompt.push_str("\n\n");
    }

    prompt.push_str("---\n\n");
    prompt.push_str(PLAN_SUMMARIZER_CONTRACT);
    prompt.push('\n');

    prompt
}

// ---------------------------------------------------------------------------
// build_plan_prompt — pure, unit-testable
// ---------------------------------------------------------------------------

/// Build the full implementation-plan prompt for the task-breakdown agent.
///
/// The story context (body, answered questions, analysis summary, approved test
/// cases, learnings) lives in a separate CONTEXT file the agent reads. The prompt
/// body is: task-breakdown skill body + file-read directive + OUTPUT CONTRACT.
pub fn build_plan_prompt(skill_body: &str, context_path: &str) -> String {
    let mut prompt = String::new();

    // 1. Task-breakdown skill body
    prompt.push_str(skill_body);
    prompt.push_str("\n\n---\n\n");

    // 2. Context file directive
    prompt.push_str(
        "The full story context (body, answered questions, analysis summary, approved test \
         cases, learnings) is in this file — it may be LARGE, read it fully (in chunks if \
         needed) before planning:\n",
    );
    prompt.push_str(context_path);
    prompt.push_str("\n\n");

    // 3. OUTPUT CONTRACT (verbatim)
    prompt.push_str("---\n\n");
    prompt.push_str(PLAN_OUTPUT_CONTRACT);
    prompt.push('\n');

    prompt
}
