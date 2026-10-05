//! The review engine's pure core: diff rendering + partial-diff detection,
//! review-config defaults and budgets, the fan-out / orchestrator agent-run
//! expansion, reviewer prompts, draft-comment parsing, severity ranking and
//! the run-completeness rules. No I/O against the server — the ctx-bound
//! orchestration in otto-server (`modules.rs`) drives these.

use std::sync::Arc;
use std::time::Duration;

use otto_core::api::ReviewConfig;
use otto_core::domain::{ReviewAgentCfg, ReviewComment};
use otto_core::{Error, Id};
use serde::Deserialize;
use serde_json::Value;

/// Render a `DiffResp` into a unified-diff string capped at `cap` chars (plus
/// a short trailer naming every omitted file). Returns `(text, partial)`:
/// `partial` ⇔ some file's hunks are missing (size cap, binary, too large, or
/// the provider's own budget) — such a run must not resolve absent findings.
pub fn render_diff(diff: &otto_core::api::DiffResp, cap: usize) -> (String, bool) {
    use otto_core::api::LineOrigin;
    let mut out = String::with_capacity(cap.min(65536));
    // Files whose hunks the agents will NOT see. Each is still named, with a
    // `(diff omitted: …)` line, so reviewers know the gap exists and
    // `diff_is_partial` can keep the run from resolving findings there.
    let mut omitted: Vec<(&str, &str)> = Vec::new();
    let mut capped = false;
    for file in &diff.files {
        let path = file.path.as_str();
        if capped {
            omitted.push((path, "review size cap reached"));
            continue;
        }
        let why = if file.is_binary {
            Some("binary file")
        } else if file.too_large == Some(true) {
            Some("file diff too large")
        } else if file.hunks_omitted == Some(true) && file.hunks.is_empty() {
            Some("provider diff budget reached")
        } else {
            None
        };
        if let Some(why) = why {
            omitted.push((path, why));
            continue;
        }
        // Render the whole file into a scratch buffer and keep it only if it
        // fits — a file is never half-shown.
        let mut buf = format!("--- a/{path}\n+++ b/{path}\n");
        for hunk in &file.hunks {
            buf.push_str(&hunk.header);
            buf.push('\n');
            for line in &hunk.lines {
                buf.push(match line.origin {
                    LineOrigin::Add => '+',
                    LineOrigin::Del => '-',
                    LineOrigin::Context => ' ',
                });
                buf.push_str(&line.content);
                if !buf.ends_with('\n') {
                    buf.push('\n');
                }
            }
        }
        if out.len() + buf.len() > cap {
            capped = true;
            omitted.push((path, "review size cap reached"));
            continue;
        }
        out.push_str(&buf);
    }
    let partial = !omitted.is_empty() || diff.truncated == Some(true);
    for (path, why) in &omitted {
        out.push_str(&format!(
            "--- a/{path}\n+++ b/{path}\n{DIFF_OMITTED_MARKER}{why} — read the file on disk)\n"
        ));
    }
    if diff.truncated == Some(true) && omitted.is_empty() {
        out.push_str(&format!(
            "{DIFF_OMITTED_MARKER}the provider truncated this diff — some files may be missing)\n"
        ));
    }
    (out, partial)
}

/// Append every `references/*.md` file sitting beside `skill_md` to `out`
/// (sorted for determinism), so agents that cannot read files still get the
/// skill's full method. Best-effort: a missing/unreadable dir is ignored.
#[allow(clippy::disallowed_methods)] // pre-existing sync fs reached from async code without offload (perf2 N3 follow-up)
pub fn append_skill_references(out: &mut String, skill_md: &std::path::Path) {
    let Some(refs_dir) = skill_md.parent().map(|d| d.join("references")) else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(&refs_dir) else {
        return;
    };
    let mut files: Vec<std::path::PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("md"))
        .collect();
    files.sort();
    for p in files {
        if let Ok(content) = std::fs::read_to_string(&p) {
            let fname = p
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_default();
            out.push_str("\n\n---\n\n# Reference: ");
            out.push_str(&fname);
            out.push_str("\n\n");
            out.push_str(&content);
        }
    }
}

/// Resolve a skill's full instructional text for inlining into a review prompt:
/// its `SKILL.md` body plus every `references/*.md` beside it, so the method
/// travels *in the prompt* and runs on any provider (claude/codex/agy/…), not
/// only ones with a native skill loader. Mirrors how `product_run` feeds skills
/// to its agents. Looked up in order: the Otto Library (canonical store), the
/// compiled-in bundled skills (body only), then the operator's global Claude
/// skills dir (`~/.claude/skills/<name>/`) so skills authored there — e.g.
/// `golang-feature-implementation` — work too. Empty/unknown → empty string.
pub fn resolve_skill_inline(library: &otto_context::Library, name: &str) -> String {
    // Skill names come from user-editable review configs and are used as path
    // components below (the library lookups re-check, but fail closed here too).
    if otto_core::paths::safe_component(name).is_none() {
        return String::new();
    }
    // 1. Otto Library (multi-file skills with references on disk).
    if let Some(skill) = library.get_skill(name) {
        let mut out = skill.body;
        if let Some(md) = library.skill_path(name) {
            append_skill_references(&mut out, &md);
        }
        return out;
    }
    // 2. Compiled-in bundled skill body (no separate references).
    if let Some(body) = otto_product::skill_body(name) {
        return body.to_string();
    }
    // 2b. `otto-skills` bundles (`otto-design-2d`, `otto-design-3d`, …). These
    //     are never auto-installed into the Library, so the assist prompts inline
    //     them from the compiled-in asset when the user hasn't installed them.
    if let Some(body) = otto_skills::bundled_body(name) {
        return body;
    }
    // 3. Operator's global Claude skills dir, for skills authored outside the
    //    Library (e.g. `golang-feature-implementation`). Inline the `SKILL.md`
    //    body ONLY — these are general skills, not review-tuned, and their
    //    `references/` can be hundreds of KB of implementation templates that
    //    would bloat every review prompt (and are about *writing* code, not
    //    reviewing it). Reject path-y names so we stay inside ~/.claude/skills.
    if !name.contains(['/', '\\']) && !name.contains("..") {
        if let Some(home) = std::env::var_os("HOME") {
            let md = std::path::Path::new(&home)
                .join(".claude/skills")
                .join(name)
                .join("SKILL.md");
            if let Ok(body) = std::fs::read_to_string(&md) {
                return body;
            }
        }
    }
    String::new()
}

/// Slugify a review agent's display name into a skill name: lowercase, runs of
/// non-alphanumerics collapse to a single `-`, trimmed. "Correctness review" →
/// `correctness-review`, "Grill" → `grill`. Used to find the lens skill when a
/// review agent leaves its `skill` field empty and carries the lens in its name
/// (which matches how the bundled lens skills are named).
pub fn slug_skill_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut prev_dash = false;
    for ch in name.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !out.is_empty() && !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    out.trim_end_matches('-').to_string()
}

/// Registry key for one review agent's cancel flag ("{review_id}:{index}").
pub fn review_agent_cancel_key(review_id: &str, index: usize) -> String {
    format!("{review_id}:{index}")
}

/// Default cap on review agents (claude/codex PTYs) running at once across
/// the daemon — `OTTO_REVIEWER_CONCURRENCY` overrides (≥ 1). Each lens is a CLI
/// process + whole-diff read + transcript watcher; a 10-lens review next to a
/// workflow's `review_run` node used to start them all at once (perf SI-11).
pub const DEFAULT_REVIEWER_CONCURRENCY: usize = 4;

pub fn reviewer_concurrency(env: Option<&str>) -> usize {
    env.and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|n| *n >= 1)
        .unwrap_or(DEFAULT_REVIEWER_CONCURRENCY)
}

/// The daemon-wide reviewer slots. A reviewer takes its permit INSIDE its
/// spawned task, so queued lenses cost nothing but a parked future; its own
/// timeout starts only once it runs.
pub fn reviewer_slots() -> Arc<tokio::sync::Semaphore> {
    static SLOTS: std::sync::OnceLock<Arc<tokio::sync::Semaphore>> = std::sync::OnceLock::new();
    SLOTS
        .get_or_init(|| {
            let n =
                reviewer_concurrency(std::env::var("OTTO_REVIEWER_CONCURRENCY").ok().as_deref());
            Arc::new(tokio::sync::Semaphore::new(n))
        })
        .clone()
}

/// The default config used when no `pr_review` setting has been stored. The
/// reviewer agents follow the configured default agent (`default_provider`);
/// the summarizer stays on claude because its run path is hard-wired to the
/// claude PTY driver (see `run_review_core`).
pub fn default_review_config(default_provider: &str) -> ReviewConfig {
    ReviewConfig {
        agents: vec![
            ReviewAgentCfg {
                name: "Correctness & bugs".to_string(),
                provider: default_provider.to_string(),
                providers: vec![default_provider.to_string()],
                model: "".to_string(),
                prompt: "You are reviewing a pull request diff. Output ONLY a JSON array \
                         (no prose, no markdown fence) of objects \
                         {\"path\":string,\"line\":number,\"severity\":\"info\"|\"warn\"|\"bug\",\
                         \"body\":string}. Focus on correctness and bugs: logic errors, off-by-one, \
                         nullability, panics, data races, incorrect assumptions."
                    .to_string(),
                skill: "correctness-review".to_string(),
            },
            ReviewAgentCfg {
                name: "Security & error handling".to_string(),
                provider: default_provider.to_string(),
                providers: vec![default_provider.to_string()],
                model: "".to_string(),
                prompt: "You are reviewing a pull request diff. Output ONLY a JSON array \
                         (no prose, no markdown fence) of objects \
                         {\"path\":string,\"line\":number,\"severity\":\"info\"|\"warn\"|\"bug\",\
                         \"body\":string}. Focus on security and error handling: injection, \
                         unhandled errors, missing auth checks, sensitive data exposure."
                    .to_string(),
                skill: "security-review".to_string(),
            },
        ],
        summarizer: ReviewAgentCfg {
            name: "Summarizer".to_string(),
            provider: "claude".to_string(),
            providers: vec![],
            model: "".to_string(),
            prompt: "You are deduplicating and prioritizing code review comments. Output ONLY \
                     a JSON array (no prose, no markdown fence) of objects \
                     {\"path\":string,\"line\":number,\"severity\":\"info\"|\"warn\"|\"bug\",\
                     \"category\":string,\"title\":string,\"body\":string,\"evidence\":string,\
                     \"reasoning\":string,\"suggested_fix\":string}. `title` is a short one-line \
                     summary; `category` is one of security|correctness|performance|architecture|\
                     devex|test|docs|style|other; `evidence` is the offending code excerpt or quote \
                     that proves the issue; `reasoning` is why it is a problem; `suggested_fix` is a \
                     concrete fix. Deduplicate: merge findings that describe the SAME defect at \
                     the same file:line into the single best-evidenced version, and drop trivial \
                     duplicates. There is NO cap on the number of items — return EVERY distinct \
                     verified finding, however many that is (on a large diff, 100+ is normal). \
                     Never drop a finding merely to shorten the list, and never drop one just \
                     because other findings are more severe. Rank by severity (bug first). \
                     Here are the batches of comments from each agent:"
                .to_string(),
            skill: String::new(),
        },
        custom_presets: vec![],
        max_attempts: None,
        timeout_secs: None,
        mode: None,
    }
}

/// Settings key holding one repo's review-config binding.
pub fn repo_review_binding_key(repo_id: &Id) -> String {
    format!("pr_review_repo:{repo_id}")
}

/// `(mode, source tag)` for a loaded review config: an explicitly stored mode
/// keeps its tag so the step log can say WHERE the mode came from, which is the
/// whole point of `ReviewConfig.mode` being an `Option`. Pure — the ctx-bound
/// lookup above is the only I/O.
pub fn mode_source(cfg: &ReviewConfig) -> (otto_core::domain::ReviewMode, &'static str) {
    match cfg.mode {
        Some(m) => (m, "stored config"),
        None => (otto_core::domain::ReviewMode::default(), "default"),
    }
}

/// Per-agent grace period before an agent is marked stuck/failed. 30 min for
/// large diffs, scaled down for small ones so short PRs fail fast. An explicit
/// `override_secs` (from `ReviewConfig.timeout_secs`) wins over the heuristic.
pub fn review_agent_timeout(diff_len: usize, override_secs: Option<u64>) -> Duration {
    if let Some(s) = override_secs {
        return Duration::from_secs(s);
    }
    // The top tier used to be a flat 30 min for "≥ 20k chars", which lumped a
    // 20 KB diff together with a 1 MB one — and a reviewer told to sweep EVERY
    // changed file (see the coverage mandate in the agent prompt) cannot do that
    // for 170 files in 30 minutes. Scale the ceiling with the diff instead, so
    // the grace period tracks the work actually being asked for.
    let secs = if diff_len < 4_000 {
        600 // ≲ small diff: 10 min
    } else if diff_len < 20_000 {
        1_200 // medium: 20 min
    } else if diff_len < 100_000 {
        1_800 // large: 30 min
    } else if diff_len < 400_000 {
        3_600 // very large: 1 h
    } else {
        7_200 // huge (a whole module landing at once): 2 h
    };
    Duration::from_secs(secs)
}

/// Ceiling on the summarizer's budget (see `summarizer_timeout` in
/// `finish_review`): `(300 + 3 s/finding).clamp(300, 1_800)`.
pub const SUMMARIZER_BUDGET_CAP: Duration = Duration::from_secs(1_800);

/// Pure half of [`review_wait_budget`].
pub fn review_wait_budget_for(cfg: &ReviewConfig, diff_len: usize) -> Duration {
    let agents = review_agent_timeout(diff_len, cfg.timeout_secs);
    let agents = if mode_source(cfg).0 == otto_core::domain::ReviewMode::Orchestrator {
        orchestrator_budget(agents, cfg.agents.len())
    } else {
        agents
    };
    agents + SUMMARIZER_BUDGET_CAP + Duration::from_secs(600)
}

/// Display name of the engine's synthesized CI-gate reviewer — the one lens
/// that is ALLOWED to run commands. Must match `checks_review_agent` in
/// `workflow_engine.rs`.
pub const CHECKS_REVIEWER_NAME: &str = "Required checks";

/// Gap between two reviewer spawns (R3). The JoinSet stays uncapped — the only
/// thing being bounded is the cold-start storm: a dozen CLIs booting in the same
/// second is what pushes a codex launch past two minutes.
pub const REVIEW_SPAWN_STAGGER: Duration = Duration::from_millis(1_500);

/// The order `run_review_core` does its per-agent work in: `(register step,
/// spawn step)` per reviewer. EVERY cancel flag is registered before the first
/// session is spawned, so a Stop landing during the stagger reaches reviewers
/// that have not started yet. Pure, so the ordering guarantee is testable
/// without a ctx.
#[cfg(test)]
pub fn spawn_plan(run_count: usize) -> Vec<(usize, usize)> {
    (0..run_count).map(|i| (i, run_count + i)).collect()
}

/// Hard ceiling on an orchestrator reviewer's grace period. It runs every lens,
/// so its budget scales with the lens count — but never past this.
pub const ORCHESTRATOR_BUDGET_CAP: Duration = Duration::from_secs(18_000);

/// One reviewer session to run: a single lens on a single provider (fan-out),
/// or one provider's ORCHESTRATOR running every lens as its own sub-agents.
pub struct AgentRun {
    pub display_name: String,
    /// The configured reviewer name — shared by every provider expansion of
    /// the same `ReviewAgentCfg`, so siblings can find each other. EMPTY for an
    /// orchestrator run, which covers every lens at once: `lens_covered_by`
    /// must never retire it because one lens finished somewhere else.
    pub lens: String,
    pub provider: String,
    pub model: String,
    /// Fan-out: the composed lens prompt. Orchestrator: empty — the prompt
    /// names per-lens output paths, which need the run's index, so it is
    /// composed in the spawn loop from `lenses`.
    pub prompt_lens: String,
    /// Orchestrator only: every lens this run delegates to a sub-agent.
    pub lenses: Vec<OrchestratorLens>,
}

impl AgentRun {
    /// Lens slugs whose per-lens files the watch guard tracks for the row's
    /// progress note. Empty for fan-out (one lens, no sub-agents).
    pub fn lens_slugs(&self) -> Vec<String> {
        self.lenses.iter().map(|l| l.slug.clone()).collect()
    }
}

/// One lens an orchestrator reviewer hands to a sub-agent of its own.
#[derive(Clone)]
pub struct OrchestratorLens {
    pub name: String,
    /// Sanitised `[a-z0-9-]{1,40}` — a path component and the finding label.
    pub slug: String,
    /// The lens method, inlined. Empty when it could not be resolved (claude
    /// can still load it by name from the `--add-dir` bundle).
    pub skill_text: String,
    /// The reviewer's own instructions from the config.
    pub instructions: String,
    /// False for the CI-gate lens, which must be allowed to RUN its commands.
    pub read_only: bool,
}

/// The providers a reviewer runs on: its explicit list, else its single one.
pub fn effective_providers(a: &ReviewAgentCfg) -> Vec<String> {
    if a.providers.is_empty() {
        vec![a.provider.clone()]
    } else {
        a.providers.clone()
    }
}

/// The lens SKILL name for a reviewer: the explicit `skill` field, falling back
/// to the slugified agent name ("Grill" -> grill, "Correctness review" ->
/// correctness-review) since configs usually carry the lens in the name with
/// `skill` empty.
pub fn lens_of(a: &ReviewAgentCfg) -> String {
    if a.skill.trim().is_empty() {
        slug_skill_name(&a.name)
    } else {
        a.skill.clone()
    }
}

/// An orchestrator reviewer's grace period: the fan-out budget stretched by
/// half the lens count (its sub-agents run in parallel, so the lenses cost far
/// less than serially), capped so a wedged one still fails.
pub fn orchestrator_budget(base: Duration, n_lenses: usize) -> Duration {
    let factor = n_lenses.div_ceil(2).max(1) as u32;
    base.saturating_mul(factor).min(ORCHESTRATOR_BUDGET_CAP)
}

/// Expand a review config into the sessions to run.
///
/// `FanOut` (the default) is one session per lens × provider — byte-for-byte
/// what reviews have always done. `Orchestrator` is one session per provider,
/// each running EVERY lens as its own sub-agent and merging the per-lens files
/// (§1.3): N sessions instead of N × lenses, which is what makes a 6-lens
/// 2-provider review survivable on file descriptors and CPU.
///
/// Takes the library (not a ctx) so the expansion is testable on its own.
pub fn expand_agent_runs(
    cfg: &ReviewConfig,
    mode: otto_core::domain::ReviewMode,
    library: &otto_context::Library,
) -> Vec<AgentRun> {
    if mode == otto_core::domain::ReviewMode::Orchestrator {
        return orchestrator_runs(cfg, library);
    }
    cfg.agents
        .iter()
        .flat_map(|a| {
            let providers = effective_providers(a);
            let multi = providers.len() > 1;
            // Inline the agent's skill (body + references) ahead of its lens
            // prompt so EVERY provider runs the full method, not just claude (which
            // also gets it registered out-of-tree via `--add-dir`; codex/agy do
            // not register `--add-dir` skills and would otherwise have to scavenge
            // the bundle). Resolved once per agent, reused per provider.
            let lens = lens_of(a);
            let skill_text = resolve_skill_inline(library, &lens);
            providers.into_iter().map(move |p| {
                let display_name = if multi {
                    format!("{} \u{00b7} {}", a.name, p)
                } else {
                    a.name.clone()
                };
                let prompt_lens = compose_review_lens_prompt(&lens, &skill_text, &a.prompt);
                AgentRun {
                    display_name,
                    lens: a.name.clone(),
                    provider: p,
                    model: a.model.clone(),
                    prompt_lens,
                    lenses: Vec::new(),
                }
            })
        })
        .collect()
}

/// The orchestrator expansion: one run per DISTINCT provider (in first-seen
/// order), each carrying every lens.
pub fn orchestrator_runs(cfg: &ReviewConfig, library: &otto_context::Library) -> Vec<AgentRun> {
    let lenses: Vec<OrchestratorLens> = cfg
        .agents
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let lens = lens_of(a);
            let slug = {
                let s = crate::session::sanitize_lens_slug(&lens);
                if s.is_empty() {
                    format!("lens{i}")
                } else {
                    s
                }
            };
            OrchestratorLens {
                name: a.name.clone(),
                slug,
                skill_text: resolve_skill_inline(library, &lens),
                instructions: a.prompt.clone(),
                // The CI gate is the one lens told to RUN things; every other
                // lens keeps the read-only contract.
                read_only: a.name != CHECKS_REVIEWER_NAME,
            }
        })
        .collect();

    let mut providers: Vec<String> = Vec::new();
    for a in &cfg.agents {
        for p in effective_providers(a) {
            if !providers.contains(&p) {
                providers.push(p);
            }
        }
    }

    let n = lenses.len();
    providers
        .into_iter()
        .map(|p| {
            // The first configured model that actually applies to this provider
            // (a per-lens model on another provider must not leak across).
            let model = cfg
                .agents
                .iter()
                .find(|a| !a.model.trim().is_empty() && effective_providers(a).contains(&p))
                .map(|a| a.model.clone())
                .unwrap_or_default();
            AgentRun {
                display_name: format!("{p} \u{00b7} orchestrator ({n} lenses)"),
                lens: String::new(),
                provider: p,
                model,
                prompt_lens: String::new(),
                lenses: lenses.clone(),
            }
        })
        .collect()
}

/// Background wrapper: runs `run_review_core` for a PR and sets the final
/// status on the review row.
#[allow(clippy::too_many_arguments)]
/// The source (head) and destination (base) branch names a review is about.
/// Surfaced to reviewers so they know what they're looking at, and (for PR
/// reviews) used to materialize the source branch's real code into an isolated
/// worktree. Either field may be empty when a flow can't resolve it.
#[derive(Clone, Default)]
pub struct ReviewBranches {
    pub source: String,
    pub dest: String,
    /// The review's cwd really is a checkout of `source` (always for local
    /// reviews; for PR reviews only when the isolated worktree was built).
    /// `false` ⇒ the prompt must not claim the files on disk are the change's
    /// code, and the run is partial (it resolves no absent findings).
    pub checked_out: bool,
}

/// Prefix each finding's body with its lens when it carries one. An
/// orchestrator batch is one provider's SIX lenses merged into a single array,
/// so without the label the summarizer cannot tell which method produced what
/// (fan-out batches are one lens each and are unaffected — their findings have
/// no `lens`).
pub fn label_findings_with_lens(
    findings: &[otto_core::domain::ReviewFinding],
) -> Vec<otto_core::domain::ReviewFinding> {
    findings
        .iter()
        .map(|f| match f.lens.as_deref() {
            Some(lens) if !lens.is_empty() => otto_core::domain::ReviewFinding {
                body: format!("[lens: {lens}] {}", f.body),
                ..f.clone()
            },
            _ => f.clone(),
        })
        .collect()
}

/// A draft review comment as emitted by the summarizer.
#[derive(Deserialize)]
pub struct DraftComment {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub line: Option<u32>,
    /// Optional end of a multi-line finding range. Anchors the finding to a
    /// span, not just a single line, when the agent reports one.
    #[serde(default)]
    pub line_end: Option<u32>,
    #[serde(default = "default_draft_severity")]
    pub severity: String,
    pub body: String,
    // Enriched workflow fields (optional; derived from `body` when absent).
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub evidence: Option<String>,
    #[serde(default)]
    pub reasoning: Option<String>,
    #[serde(default)]
    pub suggested_fix: Option<String>,
}

pub fn default_draft_severity() -> String {
    "info".to_string()
}

/// Whether a freshly summarized comment is the same one as an already-decided
/// comment of this review: same file and either the same anchored line or the
/// same (whitespace-normalized) text. The summarizer re-words between runs, so
/// the line is the primary key and the text covers path-/line-less comments.
pub fn same_draft_comment(kept: &ReviewComment, c: &DraftComment) -> bool {
    if kept.path != c.path {
        return false;
    }
    if kept.line.is_some() && kept.line == c.line {
        return true;
    }
    let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    norm(&kept.body) == norm(&c.body)
}

/// Parse the summarizer's reply into draft comments (tolerates fences/prose).
pub fn parse_draft_comments(review_id: &Id, summary_text: &str) -> Vec<DraftComment> {
    let stripped = summary_text
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    let start = stripped.find('[').unwrap_or(0);
    let end = stripped.rfind(']').map(|i| i + 1).unwrap_or(stripped.len());
    // Prose like "see item ] … [draft]" puts the last `]` BEFORE the first
    // `[`; slicing `start..end` then panicked and killed the review task.
    if start >= end {
        tracing::warn!(review = %review_id, "final reply holds no JSON array; no comments stored");
        return vec![];
    }
    let slice = &stripped[start..end];
    serde_json::from_str(slice).unwrap_or_else(|e| {
        tracing::warn!(review = %review_id, "failed to parse final JSON ({e}); no comments stored");
        vec![]
    })
}

/// Whether every reviewer row finished cleanly and the real summarizer ran —
/// the precondition for treating "absent this run" as "resolved". `skipped`
/// counts as finished (a sibling row covered that lens); a row whose note
/// starts with `partial` (an orchestrator adopted at its cap with lenses still
/// running) does not.
pub fn review_run_complete(
    reviewers: &[otto_core::domain::ReviewAgentState],
    summary_fallback: bool,
) -> bool {
    !summary_fallback
        && reviewers.iter().all(|a| {
            matches!(a.status.as_str(), "done" | "skipped") && !a.note.starts_with("partial")
        })
}

/// Why a run may NOT resolve findings absent from it, independent of how its
/// agents fared (see [`review_run_complete`]):
/// - the diff the agents saw was cut (size cap / binary / too-large files are
///   rendered as `(diff omitted: …)`) — files they never saw prove nothing;
/// - a PR review whose source could not be checked out ran in the user's tree,
///   so line anchors (and so fingerprints) came from the wrong revision.
pub fn resolve_blocker(
    pr_number: u64,
    branches: Option<&ReviewBranches>,
    diff_text: &str,
) -> Option<&'static str> {
    if diff_is_partial(diff_text) {
        return Some("partial diff (files omitted)");
    }
    if pr_number != 0 && !branches.is_some_and(|b| b.checked_out) {
        return Some("PR source not checked out");
    }
    None
}

/// Marker [`render_diff`] writes for a file whose hunks the agents did NOT get.
pub const DIFF_OMITTED_MARKER: &str = "(diff omitted: ";

/// Whether a rendered review diff is missing content (see [`render_diff`]).
pub fn diff_is_partial(diff_text: &str) -> bool {
    diff_text
        .lines()
        .any(|l| l.starts_with(DIFF_OMITTED_MARKER))
}

/// Repo-relative paths a unified diff touches (both sides, so a deleted or
/// renamed file's findings are in scope too). Reads the `--- a/…` + `+++ b/…`
/// header PAIR (a lone `--- ` is a removed `-- …` content line, not a header)
/// and git's `rename from/to` lines; `/dev/null` is skipped and a quoted
/// header is unquoted.
pub fn diff_file_paths(diff: &str) -> Vec<String> {
    fn push(out: &mut Vec<String>, raw: &str, strip_side: bool) {
        // Drop a trailing tab-separated timestamp (plain `diff -u` output).
        let raw = raw.split('\t').next().unwrap_or(raw).trim();
        let raw = raw
            .strip_prefix('"')
            .and_then(|r| r.strip_suffix('"'))
            .unwrap_or(raw);
        if raw == "/dev/null" || raw.is_empty() {
            return;
        }
        let path = if strip_side {
            raw.strip_prefix("a/")
                .or_else(|| raw.strip_prefix("b/"))
                .unwrap_or(raw)
        } else {
            raw
        };
        let path = otto_state::review_findings::normalize_finding_path(path);
        if !path.is_empty() && !out.iter().any(|p| p == path) {
            out.push(path.to_string());
        }
    }
    let lines: Vec<&str> = diff.lines().collect();
    let mut out: Vec<String> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if let Some(old) = line.strip_prefix("--- ") {
            if let Some(new) = lines.get(i + 1).and_then(|n| n.strip_prefix("+++ ")) {
                push(&mut out, old, true);
                push(&mut out, new, true);
            }
        } else if let Some(p) = line
            .strip_prefix("rename from ")
            .or_else(|| line.strip_prefix("rename to "))
        {
            push(&mut out, p, false);
        }
    }
    out
}

/// Text of 1-based `line` in the repo-relative `path` under `repo_path`, for
/// fingerprint anchoring. Absolute or `..` paths are refused (the path comes
/// from agent output). Each file is read at most once per run via `cache`.
pub async fn anchored_line_text(
    repo_path: &str,
    path: &str,
    line: u32,
    cache: &mut std::collections::HashMap<String, Option<Vec<String>>>,
) -> Option<String> {
    let rel = otto_state::review_findings::normalize_finding_path(path);
    let rel_path = std::path::Path::new(rel);
    if rel.is_empty()
        || rel_path.is_absolute()
        || rel_path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return None;
    }
    if !cache.contains_key(rel) {
        let lines = tokio::fs::read_to_string(std::path::Path::new(repo_path).join(rel_path))
            .await
            .ok()
            .map(|s| s.lines().map(str::to_string).collect::<Vec<String>>());
        cache.insert(rel.to_string(), lines);
    }
    let idx = (line as usize).checked_sub(1)?;
    cache.get(rel)?.as_ref()?.get(idx).cloned()
}

/// Whether a stored finding severity is a blocker (critical/high). Rows are
/// persisted in the normalized vocabulary (`critical|high|medium|low|info`,
/// see [`otto_core::finding::FindingSeverity::normalize`]), so comparing the
/// raw string against the reviewer token `"bug"` never matched and every
/// review reported 0 blockers. Normalizing first also covers legacy
/// `bug`/`blocker` rows.
pub fn is_blocking_severity(severity: &str) -> bool {
    use otto_core::finding::FindingSeverity;
    matches!(
        FindingSeverity::normalize(severity),
        FindingSeverity::Critical | FindingSeverity::High
    )
}

/// Sort rank for a stored severity: critical first, info last. Shares
/// [`is_blocking_severity`]'s normalization so legacy `bug`/`warn` rows and the
/// normalized `high`/`medium` rows rank the same.
pub fn severity_rank(severity: &str) -> u8 {
    use otto_core::finding::FindingSeverity;
    match FindingSeverity::normalize(severity) {
        FindingSeverity::Critical => 0,
        FindingSeverity::High => 1,
        FindingSeverity::Medium => 2,
        FindingSeverity::Low => 3,
        FindingSeverity::Info => 4,
    }
}

#[cfg(test)]
mod reviewer_slot_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn concurrency_env_parses_with_a_floor_of_one() {
        assert_eq!(reviewer_concurrency(None), DEFAULT_REVIEWER_CONCURRENCY);
        assert_eq!(reviewer_concurrency(Some("2")), 2);
        assert_eq!(reviewer_concurrency(Some(" 7 ")), 7);
        assert_eq!(
            reviewer_concurrency(Some("0")),
            DEFAULT_REVIEWER_CONCURRENCY
        );
        assert_eq!(
            reviewer_concurrency(Some("lots")),
            DEFAULT_REVIEWER_CONCURRENCY
        );
    }

    /// The fan-out pattern (permit taken inside the spawned task) never runs
    /// more than N reviewers at once, however many lenses are spawned.
    #[tokio::test]
    async fn spawned_reviewers_never_exceed_the_cap() {
        let slots = Arc::new(tokio::sync::Semaphore::new(reviewer_concurrency(Some("3"))));
        let live = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let mut set = tokio::task::JoinSet::new();
        for _ in 0..12 {
            let (slots, live, peak) = (slots.clone(), live.clone(), peak.clone());
            set.spawn(async move {
                let _slot = slots.acquire_owned().await.ok();
                let now = live.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(15)).await;
                live.fetch_sub(1, Ordering::SeqCst);
            });
        }
        while set.join_next().await.is_some() {}
        assert_eq!(peak.load(Ordering::SeqCst), 3);
        assert!(
            reviewer_slots().available_permits() >= 1,
            "the daemon pool exists and is non-empty"
        );
    }
}

#[cfg(test)]
mod review_r10_tests {
    use super::*;
    use otto_core::domain::CommentSeverity;

    fn file(path: &str, binary: bool, too_large: bool, lines: usize) -> serde_json::Value {
        let body: Vec<serde_json::Value> = (0..lines)
            .map(|i| serde_json::json!({"origin": "add", "content": format!("line {i}\n"), "old_line": null, "new_line": i + 1}))
            .collect();
        serde_json::json!({
            "path": path, "old_path": null, "is_binary": binary,
            "too_large": too_large,
            "hunks": if binary || too_large { serde_json::json!([]) } else {
                serde_json::json!([{"header": "@@ -0,0 +1 @@", "lines": body}])
            }
        })
    }

    /// Binary / too-large / over-cap files are NAMED with an omission
    /// marker (never silently dropped) and the render reports partial — which
    /// is what keeps such a run from resolving findings in unseen files.
    #[test]
    fn render_diff_marks_omitted_files_and_reports_partial() {
        let diff: otto_core::api::DiffResp = serde_json::from_value(serde_json::json!({
            "files": [file("a.rs", false, false, 2), file("img.png", true, false, 0),
                      file("huge.rs", false, true, 0), file("b.rs", false, false, 400)]
        }))
        .unwrap();
        let (text, partial) = render_diff(&diff, 1_000);
        assert!(partial);
        assert!(text.contains("+line 1"), "{text}");
        for p in ["img.png", "huge.rs", "b.rs"] {
            assert!(
                text.contains(&format!("+++ b/{p}")),
                "{p} not named: {text}"
            );
        }
        assert!(text.contains("(diff omitted: binary file"));
        assert!(text.contains("(diff omitted: review size cap reached"));
        assert!(diff_is_partial(&text));
        assert!(resolve_blocker(0, None, &text).is_some());

        let small: otto_core::api::DiffResp = serde_json::from_value(serde_json::json!({
            "files": [file("a.rs", false, false, 2)]
        }))
        .unwrap();
        let (text, partial) = render_diff(&small, 1_000);
        assert!(!partial && !diff_is_partial(&text));
        assert!(resolve_blocker(0, None, &text).is_none());
    }

    /// A PR review that could not check out the source must not resolve.
    #[test]
    fn resolve_blocker_requires_a_pr_checkout() {
        let b = |checked_out| ReviewBranches {
            source: "f".into(),
            dest: "main".into(),
            checked_out,
        };
        assert!(resolve_blocker(7, None, "").is_some());
        assert!(resolve_blocker(7, Some(&b(false)), "").is_some());
        assert!(resolve_blocker(7, Some(&b(true)), "").is_none());
    }

    /// Prose with a `]` before the first `[` must not panic the review task.
    #[test]
    fn parse_draft_comments_survives_inverted_brackets() {
        let id: Id = "r".into();
        assert!(parse_draft_comments(&id, "see ] then [ nothing").is_empty());
        assert!(parse_draft_comments(&id, "no json at all").is_empty());
        let ok = parse_draft_comments(&id, r#"[{"body": "x", "severity": "Critical"}]"#);
        assert_eq!(ok.len(), 1);
        assert_eq!(
            CommentSeverity::normalize(&ok[0].severity),
            CommentSeverity::Bug
        );
    }

    /// The blocking wait covers the reviewers' budget plus the summarizer's,
    /// so it can never read a still-running review's counts.
    #[test]
    fn review_wait_budget_exceeds_agent_plus_summarizer() {
        let cfg = default_review_config("claude");
        let small = review_wait_budget_for(&cfg, 100);
        assert!(small >= Duration::from_secs(600 + 1_800), "{small:?}");
        let huge = review_wait_budget_for(&cfg, 1_000_000);
        assert!(huge >= Duration::from_secs(7_200 + 1_800), "{huge:?}");
    }
}

#[cfg(test)]
mod review_scope_tests {
    use super::{diff_file_paths, review_run_complete};

    fn agent(status: &str, note: &str) -> otto_core::domain::ReviewAgentState {
        serde_json::from_value(serde_json::json!({
            "name": "lens", "provider": "claude", "model": "",
            "status": status, "note": note, "comment_count": 0
        }))
        .unwrap()
    }

    #[test]
    fn diff_paths_cover_both_sides_and_skip_content_lines() {
        let diff = "diff --git a/src/a.rs b/src/a.rs\n\
                    index 1..2 100644\n\
                    --- a/src/a.rs\n\
                    +++ b/src/a.rs\n\
                    @@ -1,2 +1,2 @@\n\
                    --- a removed SQL comment line\n\
                    +new\n\
                    diff --git a/gone.rs b/gone.rs\n\
                    deleted file mode 100644\n\
                    --- a/gone.rs\n\
                    +++ /dev/null\n\
                    diff --git a/old name.rs b/new name.rs\n\
                    similarity index 100%\n\
                    rename from old name.rs\n\
                    rename to new name.rs\n";
        assert_eq!(
            diff_file_paths(diff),
            vec!["src/a.rs", "gone.rs", "old name.rs", "new name.rs"]
        );
    }

    fn kept(path: Option<&str>, line: Option<u32>, body: &str) -> otto_core::domain::ReviewComment {
        otto_core::domain::ReviewComment {
            id: "c1".into(),
            review_id: "r1".into(),
            path: path.map(str::to_string),
            line,
            severity: otto_core::domain::CommentSeverity::Warn,
            body: body.into(),
            state: otto_core::domain::CommentState::Approved,
            posted: true,
            created_at: chrono::Utc::now(),
        }
    }

    fn draft(path: Option<&str>, line: Option<u32>, body: &str) -> super::DraftComment {
        serde_json::from_value(serde_json::json!({
            "path": path, "line": line, "severity": "warn", "body": body
        }))
        .unwrap()
    }

    #[test]
    fn decided_comments_are_not_redrafted_by_a_summarizer_rerun() {
        use super::same_draft_comment;
        let k = kept(Some("a.rs"), Some(10), "Unchecked unwrap");
        // Re-worded on the same line → the same comment.
        assert!(same_draft_comment(
            &k,
            &draft(Some("a.rs"), Some(10), "unwrap may panic")
        ));
        // Another line / file → a different comment.
        assert!(!same_draft_comment(
            &k,
            &draft(Some("a.rs"), Some(11), "unwrap may panic")
        ));
        assert!(!same_draft_comment(
            &k,
            &draft(Some("b.rs"), Some(10), "Unchecked unwrap")
        ));
        // A general comment matches on its text.
        let g = kept(None, None, "Overall:  add tests");
        assert!(same_draft_comment(
            &g,
            &draft(None, None, "Overall: add tests")
        ));
    }

    #[test]
    fn only_an_anchor_rejection_falls_back_to_a_general_comment() {
        use super::{general_comment_body, inline_anchor_rejected};
        use otto_core::Error;
        assert!(inline_anchor_rejected(&Error::Conflict(
            "github 422: pull_request_review_thread.line must be part of the diff".into()
        )));
        assert!(inline_anchor_rejected(&Error::Upstream(
            "gitlab 400: position is invalid".into()
        )));
        // Auth / 5xx are not anchor problems — never re-post on those.
        assert!(!inline_anchor_rejected(&Error::Forbidden(
            "github 403: bad token".into()
        )));
        assert!(!inline_anchor_rejected(&Error::Upstream(
            "github 502: bad gateway".into()
        )));
        assert_eq!(
            general_comment_body(&kept(Some("a.rs"), Some(3), "x")),
            "**`a.rs:3`**\n\nx"
        );
    }

    #[test]
    fn only_clean_runs_are_complete() {
        let ok = [agent("done", "3 findings"), agent("skipped", "skipped — x")];
        assert!(review_run_complete(&ok, false));
        assert!(
            !review_run_complete(&ok, true),
            "fallback summarizer = partial"
        );
        assert!(!review_run_complete(&[agent("error", "timed out")], false));
        assert!(!review_run_complete(
            &[agent("done", "partial — 1 lens still running")],
            false
        ));
    }
}

#[cfg(test)]
mod severity_tests {
    use super::{is_blocking_severity, severity_rank};

    #[test]
    fn blockers_are_counted_in_the_stored_vocabulary() {
        // What the store actually holds (normalized on write)…
        assert!(is_blocking_severity("critical"));
        assert!(is_blocking_severity("high"));
        assert!(!is_blocking_severity("medium"));
        assert!(!is_blocking_severity("low"));
        assert!(!is_blocking_severity("info"));
        // …and legacy reviewer tokens on older rows.
        assert!(is_blocking_severity("bug"));
        assert!(is_blocking_severity("blocker"));
        assert!(!is_blocking_severity("warn"));
    }

    #[test]
    fn rank_orders_highest_first() {
        assert!(severity_rank("critical") < severity_rank("high"));
        assert!(severity_rank("high") < severity_rank("medium"));
        assert!(severity_rank("medium") < severity_rank("info"));
        assert_eq!(severity_rank("bug"), severity_rank("high"));
    }
}

/// Prettify a lens skill name for display: "correctness-review" → "Correctness
/// review", "test_review" → "Test review".
pub fn title_case_lens(s: &str) -> String {
    let spaced = s.replace(['-', '_'], " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Build a [`ReviewConfig`] for a workflow `review_run` step from its declared
/// providers + lenses, reusing the same reviewer/summarizer prompts as the
/// default PR-review config — so a workflow review fans out exactly like a PR
/// review (multi-provider × multi-lens agents, one summarizer that consolidates +
/// scores the findings). `providers` empty → the resolved global default;
/// `lenses` empty → the two default lenses (correctness + security) retargeted to
/// `providers`.
/// The lens prompt shared by every workflow-review reviewer: it pins the JSON
/// output contract; the per-lens `skill` text is prepended at run time.
pub const WORKFLOW_REVIEW_LENS_PROMPT: &str =
    "You are reviewing a pull request diff. Output ONLY a JSON array \
     (no prose, no markdown fence) of objects \
     {\"path\":string,\"line\":number,\"severity\":\"info\"|\"warn\"|\"bug\",\"body\":string}. \
     Review through your lens — the skill prepended above defines what to look for. Open and \
     read the real files around you to verify each finding before reporting it.";

pub fn workflow_review_config(
    default_provider: &str,
    providers: &[String],
    lenses: &[String],
) -> ReviewConfig {
    let mut base = default_review_config(default_provider);
    let provs: Vec<String> = if providers.is_empty() {
        vec![default_provider.to_string()]
    } else {
        providers.to_vec()
    };
    if lenses.is_empty() {
        for a in base.agents.iter_mut() {
            a.provider = provs[0].clone();
            a.providers = provs.clone();
        }
    } else {
        base.agents = lenses
            .iter()
            .map(|lens| ReviewAgentCfg {
                name: title_case_lens(lens),
                provider: provs[0].clone(),
                providers: provs.clone(),
                model: String::new(),
                prompt: WORKFLOW_REVIEW_LENS_PROMPT.to_string(),
                skill: lens.clone(),
            })
            .collect();
    }
    base
}

/// Rich workflow-review config (PR-review parity): each reviewer carries its OWN
/// provider set + optional custom instructions, plus a configurable summarizer —
/// the same model the PR-review pipeline uses. Built from the `review_run` node's
/// `reviewers`/`summarizer` JSON params. Reviewers with no providers fall back to
/// `default_provider`; an empty `reviewers` returns the default config.
pub fn workflow_review_config_from_json(
    default_provider: &str,
    reviewers: &[Value],
    summarizer: Option<&Value>,
) -> ReviewConfig {
    let mut base = default_review_config(default_provider);
    let str_list = |v: &Value, k: &str| -> Vec<String> {
        v.get(k)
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    let agents: Vec<ReviewAgentCfg> = reviewers
        .iter()
        .filter_map(|r| {
            let lens = r
                .get("lens")
                .or_else(|| r.get("skill"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let mut provs = str_list(r, "providers");
            if provs.is_empty() {
                provs = vec![default_provider.to_string()];
            }
            let instr = r.get("instructions").and_then(Value::as_str).unwrap_or("").trim().to_string();
            let name = r
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| {
                    if lens.is_empty() {
                        "Reviewer".to_string()
                    } else {
                        title_case_lens(&lens)
                    }
                });
            let model = r.get("model").and_then(Value::as_str).unwrap_or("").to_string();
            let prompt = if instr.is_empty() {
                WORKFLOW_REVIEW_LENS_PROMPT.to_string()
            } else {
                format!("{WORKFLOW_REVIEW_LENS_PROMPT}\n\n--- Reviewer-specific instructions ---\n{instr}")
            };
            // A reviewer with neither a lens nor a name nor providers is noise.
            if lens.is_empty() && instr.is_empty() {
                return None;
            }
            Some(ReviewAgentCfg {
                name,
                provider: provs[0].clone(),
                providers: provs,
                model,
                prompt,
                skill: lens,
            })
        })
        .collect();
    if !agents.is_empty() {
        base.agents = agents;
    }
    if let Some(s) = summarizer {
        if let Some(p) = s
            .get("provider")
            .and_then(Value::as_str)
            .filter(|x| !x.is_empty())
        {
            base.summarizer.provider = p.to_string();
            base.summarizer.providers = vec![];
        }
        if let Some(m) = s.get("model").and_then(Value::as_str) {
            base.summarizer.model = m.to_string();
        }
        if let Some(instr) = s
            .get("instructions")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|x| !x.is_empty())
        {
            base.summarizer.prompt = format!(
                "{}\n\n--- Additional summarizer guidance ---\n{instr}",
                base.summarizer.prompt
            );
        }
    }
    base
}

/// Compose a review agent's prompt body: prepend the inlined lens method
/// (`skill_text`) ahead of the agent's lens prompt, fronted by a directive that
/// makes the indicated lens AUTHORITATIVE on every provider.
///
/// This directive is the load-bearing fix for the codex skill-propagation bug:
/// codex has no first-class skills, so without it codex reflexively goes looking
/// for "a review skill" and runs the wrong one. The wording NAMES the lens and
/// forbids substituting a *different* skill, while explicitly permitting the
/// named lens itself — so it never suppresses the correct skill on claude (which
/// may still invoke `Skill(<lens>)`, the same method). Empty `skill_text` ⇒ the
/// agent prompt is returned unchanged (mirrors [`compose_draft_prompt`]).
pub fn compose_review_lens_prompt(lens: &str, skill_text: &str, agent_prompt: &str) -> String {
    if skill_text.trim().is_empty() {
        return agent_prompt.to_string();
    }
    let lens = lens.trim();
    let directive = if lens.is_empty() {
        "Use the review method specified in full below. Do not search for, switch to, or \
         substitute a different review skill or style — everything you need is already here."
            .to_string()
    } else {
        format!(
            "Use the `{lens}` review method specified in full below. Do not search for, switch \
             to, or substitute a *different* review skill or style — its full method is already \
             here. (Invoking `{lens}` itself is fine.)"
        )
    };
    format!("{directive}\n\n{skill_text}\n\n---\n\n{agent_prompt}")
}

/// The ORCHESTRATOR reviewer's prompt: one agent, every lens, each lens run as
/// its own sub-agent writing its own file, then a single merge into the path
/// the watch loop polls (design §1.3).
///
/// The merge — not the lenses — is what the watch loop keys on, so the prompt
/// is explicit that the merged file is written LAST; the R1 guard in
/// [`otto_agent_run::agent_run::watch_for_result_guarded`] enforces the same rule from
/// the outside. `per_lens_path` maps a lens slug to that lens's output path,
/// and `merged_path` is the reviewer's own findings file.
pub fn compose_orchestrator_prompt(
    lenses: &[OrchestratorLens],
    provider: &str,
    per_lens_path: impl Fn(&str) -> std::path::PathBuf,
    merged_path: &std::path::Path,
) -> String {
    let mut out = format!(
        "MULTI-LENS CODE REVIEW \u{2014} you are the review ORCHESTRATOR for provider {provider}. \
         READ-ONLY (same rules as below).\n\
         Run EACH lens below as its own sub-agent, all in parallel where your CLI allows it \
         (Claude Code: the Agent tool, one per lens, general-purpose; Codex: spawn sub-agents if \
         available, otherwise run the lenses one after another yourself).\n\
         Give every sub-agent: the lens method verbatim (below), the diff file path, the checkout \
         path, the read-only rules, and its OWN output path (named in its section) \u{2014} a JSON \
         array of {{path,line,severity,body,lens:\"<lens-slug>\"}}.\n\
         Wait for ALL sub-agents to finish. Then read every per-lens file, drop exact duplicates, \
         keep the `lens` field, and write the merged array to:\n  {}\n\
         Writing that merged file is the LAST thing you do; never write it while a sub-agent is \
         still running.\n",
        merged_path.display()
    );
    for (i, l) in lenses.iter().enumerate() {
        out.push_str(&format!(
            "\n--- lens {}: {} ({}) ---\n",
            i + 1,
            l.name,
            l.slug
        ));
        // claude loads the staged lens bundle via `--add-dir`, so an
        // unresolvable method is still reachable there BY NAME; codex/agy have
        // only what the prompt carries.
        let method = if l.skill_text.trim().is_empty() {
            if provider == "claude" {
                format!("use the `{}` skill (registered via --add-dir)", l.slug)
            } else {
                String::new()
            }
        } else {
            l.skill_text.trim().to_string()
        };
        if !method.is_empty() {
            out.push_str(&method);
            out.push_str("\n\n");
        }
        if !l.instructions.trim().is_empty() {
            out.push_str(l.instructions.trim());
            out.push('\n');
        }
        if !l.read_only {
            out.push_str(
                "This lens is the CI gate: you MAY run the listed check commands (and nothing \
                 else that modifies the repo).\n",
            );
        }
        out.push_str(&format!(
            "Sub-agent output path: {}\n",
            per_lens_path(&l.slug).display()
        ));
    }
    out
}

#[cfg(test)]
mod review_lens_prompt_tests {
    use super::compose_review_lens_prompt;

    #[test]
    fn empty_skill_returns_agent_prompt_unchanged() {
        // No lens method resolved ⇒ the agent prompt is returned byte-for-byte
        // (no directive, no separator), identical to the prior behaviour.
        assert_eq!(
            compose_review_lens_prompt("grill", "", "DO REVIEW"),
            "DO REVIEW"
        );
        assert_eq!(
            compose_review_lens_prompt("", "   ", "DO REVIEW"),
            "DO REVIEW"
        );
    }

    #[test]
    fn names_lens_permits_it_and_forbids_substitution() {
        let out = compose_review_lens_prompt("correctness-review", "METHOD BODY", "AGENT TASK");
        let lower = out.to_lowercase();
        // Names the indicated lens so it is authoritative on every provider.
        assert!(out.contains("correctness-review"));
        // Forbids substituting a DIFFERENT skill (this is what stops codex
        // scavenging the wrong one)...
        assert!(lower.contains("do not search for"));
        assert!(lower.contains("different"));
        // ...but explicitly PERMITS invoking the named lens itself, so it can't
        // suppress the correct skill on claude (superpowers "must use a skill").
        assert!(lower.contains("invoking `correctness-review`"));
        // Method body precedes the agent task, separated by the divider.
        let m = out.find("METHOD BODY").expect("method body present");
        let t = out.find("AGENT TASK").expect("agent task present");
        assert!(m < t, "the lens method must precede the agent task");
        assert!(out.contains("\n\n---\n\n"));
    }

    #[test]
    fn blank_lens_name_still_forbids_substitution() {
        // A lens with no resolvable name still gets the anti-substitution guard
        // (just without a name to reference).
        let out = compose_review_lens_prompt("", "METHOD", "TASK");
        let lower = out.to_lowercase();
        assert!(lower.contains("do not search for"));
        assert!(out.contains("METHOD"));
        assert!(out.contains("TASK"));
    }
}

#[cfg(test)]
mod orchestrator_tests {
    use super::*;
    use otto_core::domain::{ReviewAgentCfg, ReviewMode};

    fn agent(name: &str, providers: &[&str], model: &str) -> ReviewAgentCfg {
        ReviewAgentCfg {
            name: name.to_string(),
            provider: providers.first().copied().unwrap_or("claude").to_string(),
            providers: providers.iter().map(|p| p.to_string()).collect(),
            model: model.to_string(),
            prompt: format!("{name} instructions"),
            skill: String::new(),
        }
    }

    /// Lens names no bundled or user-installed skill answers to, so the prompt
    /// text under test is the same everywhere.
    const UNRESOLVED_A: &str = "Alpha lens";
    const UNRESOLVED_B: &str = "Beta lens";

    fn cfg_with(agents: Vec<ReviewAgentCfg>) -> ReviewConfig {
        let mut cfg = default_review_config("claude");
        cfg.agents = agents;
        cfg
    }

    /// A library with nothing installed on disk. NOTE it does not mean "no
    /// skill resolves": `resolve_skill_inline` falls through to the compiled-in
    /// bundles and `~/.claude/skills`, so a REAL lens name (`security-review`)
    /// still inlines its method here. Tests that assert on prompt text use
    /// [`UNRESOLVED_A`]/[`UNRESOLVED_B`] instead, so they read the same on any
    /// machine.
    fn empty_library() -> (tempfile::TempDir, otto_context::Library) {
        let tmp = tempfile::tempdir().unwrap();
        let lib = otto_context::Library::new(tmp.path().join("library"));
        (tmp, lib)
    }

    #[test]
    fn effective_review_mode_tags_stored_vs_default() {
        let mut cfg = default_review_config("claude");
        // Nothing stored ⇒ fan-out, tagged as the default (so the step log can
        // say the user never chose it).
        assert_eq!(mode_source(&cfg), (ReviewMode::FanOut, "default"));
        cfg.mode = Some(ReviewMode::Orchestrator);
        assert_eq!(
            mode_source(&cfg),
            (ReviewMode::Orchestrator, "stored config")
        );
        // An explicitly stored fan-out is still "stored", not "default".
        cfg.mode = Some(ReviewMode::FanOut);
        assert_eq!(mode_source(&cfg), (ReviewMode::FanOut, "stored config"));
    }

    #[test]
    fn orchestrator_mode_expands_one_run_per_provider() {
        let (_tmp, lib) = empty_library();
        let cfg = cfg_with(vec![
            agent("Correctness review", &["claude", "codex"], ""),
            agent("Security review", &["claude", "codex"], "gpt-5"),
            agent("Test review", &["claude"], ""),
        ]);
        let runs = expand_agent_runs(&cfg, ReviewMode::Orchestrator, &lib);
        // One session per DISTINCT provider, in first-seen order — not per
        // lens × provider (which would be five).
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].provider, "claude");
        assert_eq!(runs[1].provider, "codex");
        assert_eq!(
            runs[0].display_name,
            "claude \u{00b7} orchestrator (3 lenses)"
        );
        assert_eq!(
            runs[1].display_name,
            "codex \u{00b7} orchestrator (3 lenses)"
        );
        // Empty lens: `lens_covered_by` must never retire an orchestrator row
        // because one lens finished on the other provider.
        assert!(runs.iter().all(|r| r.lens.is_empty()));
        // Every run carries every lens, slugified from the agent names.
        assert_eq!(
            runs[0].lens_slugs(),
            vec!["correctness-review", "security-review", "test-review"]
        );
        // The model is the first configured one that applies to THIS provider.
        assert_eq!(runs[0].model, "gpt-5");
        assert_eq!(runs[1].model, "gpt-5");
        // The prompt is composed per index in the spawn loop, not here.
        assert!(runs.iter().all(|r| r.prompt_lens.is_empty()));
    }

    #[test]
    fn orchestrator_prompt_lists_every_lens_and_per_lens_paths() {
        let (_tmp, lib) = empty_library();
        let cfg = cfg_with(vec![
            agent(UNRESOLVED_A, &["claude"], ""),
            agent(UNRESOLVED_B, &["claude"], ""),
        ]);
        let runs = expand_agent_runs(&cfg, ReviewMode::Orchestrator, &lib);
        let dir = std::path::PathBuf::from("/tmp");
        let compose = |provider: &str| {
            compose_orchestrator_prompt(
                &runs[0].lenses,
                provider,
                |slug| crate::session::lens_findings_path_in(&dir, "R1", 0, slug),
                &crate::session::findings_path_in(&dir, "R1", 0),
            )
        };
        let out = compose("claude");
        assert!(out.starts_with("MULTI-LENS CODE REVIEW"));
        assert!(out.contains("ORCHESTRATOR for provider claude"));
        // Every lens gets its own numbered section AND its own output path.
        assert!(out.contains("--- lens 1: Alpha lens (alpha-lens) ---"));
        assert!(out.contains("--- lens 2: Beta lens (beta-lens) ---"));
        assert!(out.contains("/tmp/otto-review-R1-0-alpha-lens.json"));
        assert!(out.contains("/tmp/otto-review-R1-0-beta-lens.json"));
        // The merged file is the one the watch loop polls, and it is written LAST.
        assert!(out.contains("/tmp/otto-review-R1-0.json"));
        assert!(out.contains("LAST thing you do"));
        // Unresolvable method ⇒ claude is pointed at the `--add-dir` bundle it
        // alone can load…
        assert!(out.contains("use the `alpha-lens` skill (registered via --add-dir)"));
        // …while codex/agy, which never register those skills, are not sent
        // after a bundle they cannot open.
        let codex = compose("codex");
        assert!(!codex.contains("--add-dir"));
        // The per-reviewer instructions travel with their lens either way.
        assert!(out.contains("Beta lens instructions"));
        assert!(codex.contains("Beta lens instructions"));
    }

    #[test]
    fn orchestrator_prompt_inlines_a_resolved_lens_method() {
        // A lens whose method resolved gets the METHOD verbatim, not a pointer
        // to it — that is what lets codex/agy run the same review as claude.
        let lenses = vec![OrchestratorLens {
            name: "Alpha lens".into(),
            slug: "alpha-lens".into(),
            skill_text: "  ALPHA METHOD BODY  ".into(),
            instructions: "alpha instructions".into(),
            read_only: true,
        }];
        let dir = std::path::PathBuf::from("/tmp");
        let out = compose_orchestrator_prompt(
            &lenses,
            "claude",
            |slug| crate::session::lens_findings_path_in(&dir, "R1", 0, slug),
            &crate::session::findings_path_in(&dir, "R1", 0),
        );
        assert!(out.contains("ALPHA METHOD BODY"));
        assert!(!out.contains("registered via --add-dir"));
        assert!(out.contains("alpha instructions"));
    }

    #[test]
    fn checks_lens_is_not_read_only_in_orchestrator_prompt() {
        let (_tmp, lib) = empty_library();
        let cfg = cfg_with(vec![
            agent(UNRESOLVED_A, &["claude"], ""),
            agent(CHECKS_REVIEWER_NAME, &["claude"], ""),
        ]);
        let runs = expand_agent_runs(&cfg, ReviewMode::Orchestrator, &lib);
        assert!(runs[0].lenses[0].read_only);
        // The CI gate is the ONE lens that must be allowed to run commands.
        assert!(!runs[0].lenses[1].read_only);
        let dir = std::path::PathBuf::from("/tmp");
        let out = compose_orchestrator_prompt(
            &runs[0].lenses,
            "claude",
            |slug| crate::session::lens_findings_path_in(&dir, "R1", 0, slug),
            &crate::session::findings_path_in(&dir, "R1", 0),
        );
        assert!(out.contains("you MAY run the listed check commands"));
        // …and only once: the read-only lens must not inherit the licence.
        assert_eq!(
            out.matches("you MAY run the listed check commands").count(),
            1
        );
    }

    #[test]
    fn orchestrator_budget_scales_with_lenses_and_caps() {
        // 6 lenses ⇒ ceil(6/2) = 3 × the fan-out budget.
        assert_eq!(
            orchestrator_budget(Duration::from_secs(600), 6),
            Duration::from_secs(1_800)
        );
        // Odd counts round up (5 lenses ⇒ 3×).
        assert_eq!(
            orchestrator_budget(Duration::from_secs(600), 5),
            Duration::from_secs(1_800)
        );
        // A single lens still gets at least the fan-out budget.
        assert_eq!(
            orchestrator_budget(Duration::from_secs(600), 1),
            Duration::from_secs(600)
        );
        assert_eq!(
            orchestrator_budget(Duration::from_secs(600), 0),
            Duration::from_secs(600)
        );
        // Capped: a 5h base × 4 is still 5h.
        assert_eq!(
            orchestrator_budget(Duration::from_secs(18_000), 8),
            Duration::from_secs(18_000)
        );
    }

    #[test]
    fn fan_out_is_default_and_unchanged() {
        let (_tmp, lib) = empty_library();
        let cfg = cfg_with(vec![
            agent(UNRESOLVED_A, &["claude", "codex"], "sonnet"),
            agent(UNRESOLVED_B, &["claude"], ""),
        ]);
        let runs = expand_agent_runs(&cfg, ReviewMode::default(), &lib);
        // One run per lens × provider, with the provider suffixed only on the
        // multi-provider lens — exactly the pre-batch expansion.
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].display_name, "Alpha lens \u{00b7} claude");
        assert_eq!(runs[1].display_name, "Alpha lens \u{00b7} codex");
        assert_eq!(runs[2].display_name, "Beta lens");
        // `lens` is the CONFIGURED name (shared by the provider expansions) so
        // siblings can cover for each other.
        assert_eq!(runs[0].lens, "Alpha lens");
        assert_eq!(runs[1].lens, "Alpha lens");
        assert_eq!(runs[0].model, "sonnet");
        assert_eq!(runs[2].model, "");
        // No sub-agent lenses, and the lens prompt is composed up front — with
        // an unresolvable method that is the reviewer's own instructions alone,
        // exactly as `compose_review_lens_prompt` has always returned.
        assert!(runs.iter().all(|r| r.lenses.is_empty()));
        assert!(runs.iter().all(|r| r.lens_slugs().is_empty()));
        assert_eq!(runs[2].prompt_lens, "Beta lens instructions");
    }

    #[test]
    fn cancel_flags_registered_before_staggered_spawns() {
        for n in [1usize, 2, 7] {
            let plan = spawn_plan(n);
            assert_eq!(plan.len(), n);
            let last_register = plan.iter().map(|(r, _)| *r).max().unwrap();
            let first_spawn = plan.iter().map(|(_, s)| *s).min().unwrap();
            // R3: a Stop that lands mid-stagger must reach every reviewer, so
            // no flag may be registered after the first spawn.
            assert!(
                last_register < first_spawn,
                "register must precede every spawn"
            );
            // One stagger sleep between consecutive spawns, none before the first.
            assert_eq!(plan.iter().filter(|(_, s)| *s > first_spawn).count(), n - 1);
        }
        // No cap and no semaphore — the stagger is the only pacing (R3).
        assert_eq!(REVIEW_SPAWN_STAGGER, Duration::from_millis(1_500));
    }
}

/// Whether a failed inline post was the forge rejecting the ANCHOR (the line
/// is not in the PR's current diff) rather than the request as a whole.
pub fn inline_anchor_rejected(e: &Error) -> bool {
    match e {
        Error::Conflict(_) => true,
        Error::Upstream(m) | Error::Invalid(m) => m.contains(" 400:"),
        _ => false,
    }
}

/// Body of the general-comment fallback: the location it was meant for, then
/// the finding.
pub fn general_comment_body(comment: &ReviewComment) -> String {
    match (&comment.path, comment.line) {
        (Some(p), Some(l)) => format!("**`{p}:{l}`**\n\n{}", comment.body),
        (Some(p), None) => format!("**`{p}`**\n\n{}", comment.body),
        _ => comment.body.clone(),
    }
}

/// Add `/.otto/` to the repo's `info/exclude` (idempotent). Resolves a linked
/// worktree's `.git` FILE to its common dir (where `info/exclude` lives) by
/// reading files only — no git process runs over the user's config here.
pub fn exclude_otto_dir(repo: &std::path::Path) -> std::io::Result<()> {
    let dot_git = repo.join(".git");
    let git_dir = if dot_git.is_dir() {
        dot_git
    } else {
        let text = std::fs::read_to_string(&dot_git)?;
        let Some(dir) = text.trim().strip_prefix("gitdir:") else {
            return Ok(());
        };
        let dir = repo.join(dir.trim());
        match std::fs::read_to_string(dir.join("commondir")) {
            Ok(common) => dir.join(common.trim()),
            Err(_) => dir,
        }
    };
    let info = git_dir.join("info");
    std::fs::create_dir_all(&info)?;
    let exclude = info.join("exclude");
    let current = std::fs::read_to_string(&exclude).unwrap_or_default();
    if current
        .lines()
        .any(|l| matches!(l.trim(), "/.otto/" | ".otto/" | "/.otto" | ".otto"))
    {
        return Ok(());
    }
    let mut next = current;
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    next.push_str("# Otto review notes (local only)\n/.otto/\n");
    std::fs::write(&exclude, next)
}

#[cfg(test)]
mod exclude_tests {
    use super::*;

    #[test]
    fn otto_dir_is_excluded_once_in_main_and_linked_worktrees() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("r");
        std::fs::create_dir_all(repo.join(".git/info")).unwrap();
        std::fs::write(repo.join(".git/info/exclude"), "*.log").unwrap();
        exclude_otto_dir(&repo).unwrap();
        exclude_otto_dir(&repo).unwrap();
        let ex = std::fs::read_to_string(repo.join(".git/info/exclude")).unwrap();
        assert!(ex.starts_with("*.log\n"));
        assert_eq!(ex.matches("/.otto/").count(), 1);
        // A linked worktree: `.git` is a file → gitdir → commondir.
        let wt = tmp.path().join("wt");
        std::fs::create_dir_all(repo.join(".git/worktrees/wt")).unwrap();
        std::fs::write(repo.join(".git/worktrees/wt/commondir"), "../..").unwrap();
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(
            wt.join(".git"),
            format!("gitdir: {}", repo.join(".git/worktrees/wt").display()),
        )
        .unwrap();
        std::fs::write(repo.join(".git/info/exclude"), "").unwrap();
        exclude_otto_dir(&wt).unwrap();
        let ex = std::fs::read_to_string(repo.join(".git/info/exclude")).unwrap();
        assert_eq!(ex.matches("/.otto/").count(), 1);
    }
}

/// Append an approved review comment as a markdown bullet to
/// `<repo_path>/.otto/pr-<n>-review.md`, creating the file and header if needed.
pub async fn append_to_review_file(
    repo_path: &str,
    pr_number: u64,
    comment: &ReviewComment,
) -> std::io::Result<()> {
    use tokio::io::AsyncWriteExt;

    let otto_dir = std::path::Path::new(repo_path).join(".otto");
    tokio::fs::create_dir_all(&otto_dir).await?;
    // Keep the notes out of git: un-excluded, `.otto/` showed up as an
    // untracked file ("stage all" committed it) and every later LOCAL review
    // reviewed the review notes as new code. Best-effort.
    if let Err(e) = exclude_otto_dir(std::path::Path::new(repo_path)) {
        tracing::warn!("could not add .otto/ to .git/info/exclude: {e}");
    }
    let file_path = otto_dir.join(format!("pr-{pr_number}-review.md"));

    let file_exists = tokio::fs::metadata(&file_path).await.is_ok();
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file_path)
        .await?;

    if !file_exists {
        let header = format!("# PR #{pr_number} Review\n\n");
        file.write_all(header.as_bytes()).await?;
    }

    let loc = match (&comment.path, comment.line) {
        (Some(p), Some(l)) => format!(" (`{p}` line {l})"),
        (Some(p), None) => format!(" (`{p}`)"),
        _ => String::new(),
    };
    let bullet = format!(
        "- **[{}]**{} {}\n",
        comment.severity.as_str(),
        loc,
        comment.body
    );
    file.write_all(bullet.as_bytes()).await?;
    Ok(())
}
