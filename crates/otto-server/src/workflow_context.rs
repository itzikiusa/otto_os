//! Per-run workflow context files — the file-based step-handoff layer.
//!
//! Every workflow run owns `<data_dir>/workflow-context/<run_id>/`:
//!
//! ```text
//! instructions.md                 # standing instructions (workflow.instructions), verbatim — only when non-empty
//! prompt.md                       # this run's ask, verbatim — only when a prompt exists
//! run-brief.md                    # mission brief written at run start (renamed from wf-<run_id>-instruction.md)
//! repos.json                      # live registry of repos/branches/worktrees — only when repos are declared
//! step1-gather-info.md            # curated handoff summary per executed node
//! step1-gather-info.output.json   # raw node output (capped, never inlined-truncated)
//! step3-review-iter2.md           # loop inner steps, per iteration
//! final-output.md                 # on success: copy of the last content-bearing step's .md — the run's deliverable
//! ```
//!
//! Agents are pointed at the directory in their prompt and asked to read, in
//! order, `instructions.md` → `prompt.md` → `run-brief.md` → `repos.json` →
//! prior `step*.md` (each named only when it exists), then write their own
//! step summary; the engine writes a full-fidelity fallback when they don't.
//! All I/O here is best-effort: a failure logs a warning and the run
//! continues on the legacy inline-prompt behavior — context files never fail
//! a node.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Auto-generated run brief (mission, repos, planned steps) — replaces the
/// old `wf-<run_id>-instruction.md`.
pub(crate) const RUN_BRIEF_FILE: &str = "run-brief.md";
/// Standing instructions for the workflow (verbatim copy of
/// `workflow.instructions`) — written only when non-empty.
pub(crate) const INSTRUCTIONS_FILE: &str = "instructions.md";
/// This run's ask, verbatim — written only when a prompt exists.
pub(crate) const PROMPT_FILE: &str = "prompt.md";
/// Copy of the last content-bearing, error-free step's `.md` — the run's
/// deliverable, written on run success.
pub(crate) const FINAL_OUTPUT_FILE: &str = "final-output.md";

/// Kinds whose step `.md` is never the run deliverable — bookkeeping/control
/// nodes that don't produce content worth surfacing as `final-output.md`.
pub(crate) fn is_utility_kind(kind: &str) -> bool {
    matches!(
        kind,
        "manual_trigger" | "log" | "delay" | "channel_notify" | "budget_gate" | "human_approval"
    )
}

/// Cap for `*.output.json` — loop outputs embed full iteration history and a
/// runaway node must not fill the disk. A truncated file gets an explicit
/// trailing marker (it stops being strict JSON; the file targets humans and
/// agents, which is why the marker is loud instead of silent).
const OUTPUT_JSON_CAP: usize = 5 * 1024 * 1024;

/// One declared (or discovered) repo the run operates on. Serialized verbatim
/// into `repos.json`. The user-facing input schema is
/// `{repo, type: "branch"|"worktree", name, source}`; resolution fills the
/// rest. `error` marks an entry that could not be resolved — kept visible in
/// the file rather than dropped, so a failing declaration is diagnosable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RepoEntry {
    /// As declared: a repo id, name, or path.
    pub repo: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo_id: Option<String>,
    /// "branch" | "worktree".
    #[serde(rename = "type")]
    pub kind: String,
    /// Branch name (`kind=branch`) or worktree path (`kind=worktree`).
    pub name: String,
    /// Declared source/destination branch — what the work diffs/PRs against.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Resolved checkout directory the work lives in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree: Option<String>,
    /// Resolved base branch: declared `source`, else the detected default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Parse the run input's `repos` array (tolerant: non-object items and
/// entries without `repo`+`name` are skipped — normalization later marks
/// resolution problems on the entries that DID parse).
pub(crate) fn parse_repo_entries(v: &Value) -> Vec<RepoEntry> {
    let Some(arr) = v.as_array() else {
        return vec![];
    };
    let mut out = Vec::new();
    for item in arr {
        let Some(obj) = item.as_object() else { continue };
        let get = |k: &str| {
            obj.get(k)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        let (Some(repo), Some(name)) = (get("repo"), get("name")) else {
            continue;
        };
        out.push(RepoEntry {
            repo,
            repo_id: get("repo_id"),
            kind: get("type").unwrap_or_else(|| "worktree".into()),
            name,
            source: get("source"),
            worktree: get("worktree"),
            base: get("base"),
            error: None,
        });
    }
    out
}

/// When repos WERE declared but not one of them resolved, git-aware steps
/// must FAIL with the per-entry reasons — falling back to "whatever is
/// checked out at the run cwd" would silently review/PR the wrong target
/// (and an empty diff there reads as a false-green). `None` ⇒ no declaration
/// at all (legacy fallback is fine) or at least one valid entry.
pub(crate) fn all_declared_errored(declared: &[RepoEntry]) -> Option<String> {
    if declared.is_empty() || declared.iter().any(|e| e.error.is_none()) {
        return None;
    }
    Some(
        declared
            .iter()
            .filter_map(|e| e.error.as_ref().map(|err| format!("{}: {err}", e.repo)))
            .collect::<Vec<_>>()
            .join("; "),
    )
}

/// `{repo_id, worktree, base}` — the reference shape `collect_pr_targets`
/// and the loop's ref harvest already consume; nulls omitted.
pub(crate) fn entry_to_target(e: &RepoEntry) -> Value {
    let mut m = serde_json::Map::new();
    if let Some(r) = &e.repo_id {
        m.insert("repo_id".into(), Value::String(r.clone()));
    }
    if let Some(w) = &e.worktree {
        m.insert("worktree".into(), Value::String(w.clone()));
    }
    if let Some(b) = &e.base {
        m.insert("base".into(), Value::String(b.clone()));
    }
    Value::Object(m)
}

/// File-name slug for a step name: lowercase, `[a-z0-9-]`, ≤ 40 chars.
pub(crate) fn slug(name: &str) -> String {
    let mut s: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    while s.contains("--") {
        s = s.replace("--", "-");
    }
    let s: String = s.trim_matches('-').chars().take(40).collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() {
        "step".into()
    } else {
        s
    }
}

/// Base file name (no extension) for a step: `step{N}-{slug}`, with the loop
/// inner index (only when the caller needs disambiguation) and the iteration
/// suffix the user's convention mandates: `step3-review-iter2`.
pub(crate) fn step_base_name(
    n: usize,
    name: &str,
    iter: Option<u64>,
    inner_idx: Option<usize>,
) -> String {
    let mut out = format!("step{n}-{}", slug(name));
    if let Some(k) = inner_idx {
        out.push_str(&format!("-{k}"));
    }
    if let Some(i) = iter {
        out.push_str(&format!("-iter{i}"));
    }
    out
}

/// The mission brief written to `run-brief.md` at run start. Pure so it is
/// unit-testable; `steps` is (display name, kind) in execution order —
/// in-scope nodes only, so numbering matches what actually runs.
/// `has_instructions`/`has_prompt` reflect whether `instructions.md`/
/// `prompt.md` were written for this run — the "How to use this directory"
/// section only points agents at files that actually exist.
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_brief(
    wf_name: &str,
    wf_desc: &str,
    run_id: &str,
    input: &Value,
    repos: &[RepoEntry],
    steps: &[(String, String)],
    has_instructions: bool,
    has_prompt: bool,
) -> String {
    let mut md = String::new();
    md.push_str(&format!("# Workflow run: {wf_name}\n\n"));
    if !wf_desc.trim().is_empty() {
        md.push_str(&format!("{}\n\n", wf_desc.trim()));
    }
    md.push_str(&format!("- Run id: `{run_id}`\n"));
    if let Some(t) = input.get("trigger").and_then(Value::as_str) {
        md.push_str(&format!("- Trigger: {t}\n"));
    }
    md.push('\n');

    // Mission: the free-text fields a trigger provides. `prompt` is filled
    // from `msg` when absent (see `normalize_prompt` in the engine) — when
    // that happened the two are identical, so the `msg` row is redundant and
    // skipped rather than showing the same line twice.
    let prompt_val = input.get("prompt").and_then(Value::as_str).map(str::trim);
    let mut mission = String::new();
    for (key, label) in [
        ("msg", "Message"),
        ("prompt", "Prompt"),
        ("jira_ticket", "Jira ticket"),
    ] {
        if let Some(v) = input.get(key).and_then(Value::as_str).filter(|s| !s.trim().is_empty()) {
            let v = v.trim();
            if key == "msg" && Some(v) == prompt_val {
                continue;
            }
            mission.push_str(&format!("- **{label}:** {v}\n"));
        }
    }
    for (key, label) in [("goals", "Goals"), ("relevant_info", "Relevant info")] {
        if let Some(items) = input.get(key).and_then(Value::as_array) {
            let strs: Vec<&str> = items.iter().filter_map(Value::as_str).collect();
            if !strs.is_empty() {
                mission.push_str(&format!("- **{label}:**\n"));
                for s in strs {
                    mission.push_str(&format!("  - {s}\n"));
                }
            }
        }
    }
    if !mission.is_empty() {
        md.push_str("## Mission\n\n");
        md.push_str(&mission);
        md.push('\n');
    }

    // The heading always renders — its presence/absence is itself a signal an
    // agent shouldn't have to infer from silence. No repos declared ⇒ no
    // `repos.json` at all (`set_repos` is never called with a non-empty
    // registry), so the table and its guidance sentence are skipped in favor
    // of an explicit placeholder rather than pointing at a file that doesn't
    // exist.
    md.push_str("## Repos & branches\n\n");
    if repos.is_empty() {
        md.push_str("_No repos declared for this run._\n\n");
    } else {
        md.push_str("| repo | type | work | source | worktree |\n|---|---|---|---|---|\n");
        for e in repos {
            let status = e.error.as_deref().map(|err| format!(" ⚠ {err}")).unwrap_or_default();
            md.push_str(&format!(
                "| {} | {} | {} | {} | {}{status} |\n",
                e.repo,
                e.kind,
                e.name,
                e.source.as_deref().or(e.base.as_deref()).unwrap_or("(auto)"),
                e.worktree.as_deref().unwrap_or("-"),
            ));
        }
        md.push('\n');
        md.push_str("The machine-readable version of this table is `repos.json` in this directory — it is kept up to date as the run progresses and is the authoritative list.\n\n");
    }

    if !steps.is_empty() {
        md.push_str("## Planned steps\n\n");
        for (i, (name, kind)) in steps.iter().enumerate() {
            md.push_str(&format!("{}. {name} ({kind})\n", i + 1));
        }
        md.push('\n');
    }

    // Layout, enumerated in the order agents should read it — only naming
    // files that actually exist for this run.
    md.push_str("## How to use this directory\n\n");
    if has_instructions {
        md.push_str("- `instructions.md` — standing instructions for this workflow; follow them **by the letter** in every step.\n");
    }
    if has_prompt {
        md.push_str("- `prompt.md` — the ask that started this run.\n");
    }
    md.push_str(&format!(
        "- This file (`{RUN_BRIEF_FILE}`) is the run's mission brief.\n"
    ));
    if !repos.is_empty() {
        md.push_str("- `repos.json` — machine-readable registry of every repo/branch/worktree in play.\n");
    }
    md.push_str(
        "- Each finished step leaves `step{N}-{name}.md` (its handoff summary) and `step{N}-{name}.output.json` (its raw output). Loop iterations add `-iter{X}`.\n\
         - Read the prior step files you need before starting your own work — they are complete, unlike any inline excerpt in your prompt.\n\
         - Before you finish, write YOUR step's `.md` file (the exact path is given in your prompt): what you did/found/changed, files touched, decisions, and anything the next step needs.\n"
    );
    md
}

/// Engine-side rendering of a step's `.md` when the node didn't write its own:
/// full fidelity — an agent `reply` lands verbatim (never truncated), other
/// outputs as pretty JSON.
pub(crate) fn render_step_md(
    kind: &str,
    name: &str,
    out: &Value,
    logs: &[String],
    error: Option<&str>,
) -> String {
    let mut md = format!("# {name} ({kind})\n\n");
    if let Some(e) = error {
        md.push_str(&format!("## Error\n\n{e}\n\n"));
    }
    if !logs.is_empty() {
        md.push_str("## Logs\n\n");
        for l in logs {
            md.push_str(&format!("- {l}\n"));
        }
        md.push('\n');
    }
    if !out.is_null() {
        md.push_str("## Output\n\n");
        // An agent reply is the payload — inline it as text, full length.
        if let Some(reply) = out.get("reply").and_then(Value::as_str) {
            md.push_str(reply);
            md.push_str("\n\n");
            let mut rest = out.clone();
            if let Some(m) = rest.as_object_mut() {
                m.remove("reply");
                if !m.is_empty() {
                    md.push_str(&format!(
                        "```json\n{}\n```\n",
                        serde_json::to_string_pretty(&rest).unwrap_or_default()
                    ));
                }
            }
        } else {
            md.push_str(&format!(
                "```json\n{}\n```\n",
                serde_json::to_string_pretty(out).unwrap_or_default()
            ));
        }
    }
    md
}

/// The context block prepended to agent-backed step prompts: where the files
/// are, what to read, and the exact handoff file this step must write.
pub(crate) fn agent_preamble(
    dir: &str,
    has_instructions: bool,
    has_prompt: bool,
    repos_present: bool,
    prior_mds: &[String],
    own_md: &str,
) -> String {
    let prior = if prior_mds.is_empty() {
        "(none yet — you are the first step)".to_string()
    } else {
        prior_mds.join(", ")
    };
    let mut md = format!("[workflow context]\nContext directory: {dir}\nRead, in order:\n");
    if has_instructions {
        md.push_str(
            "- instructions.md — standing instructions for this workflow; follow them **by the letter** in every step.\n",
        );
    }
    if has_prompt {
        md.push_str("- prompt.md — the ask that started this run.\n");
    }
    md.push_str(&format!(
        "- {RUN_BRIEF_FILE} — the run's mission, goals, and the repos/branches (source and destination) it operates on.\n"
    ));
    if repos_present {
        md.push_str("- repos.json — machine-readable list of every repo/branch/worktree in play.\n");
    }
    md.push_str(&format!(
        "- Prior step summaries: {prior}\n\
         Read the files you need before starting.\n\n\
         [your handoff — required]\n\
         When finished, write a complete summary of what you did/found/changed (files touched, decisions, anything the next step needs) to: {dir}/{own_md}\n\n"
    ));
    md
}

/// Handle on one run's context directory. `dir = None` = disabled (creation
/// failed or intentionally off) — every method becomes a no-op so callers
/// never branch. The repos registry lives here because the engine is the
/// single writer (nodes run sequentially); a `std::sync::Mutex` (never held
/// across an await) keeps it `Sync` for the shared `RunEnv`.
pub(crate) struct RunContextFiles {
    dir: Option<PathBuf>,
    run_id: String,
    repos: Mutex<Vec<RepoEntry>>,
    /// `base_name` (no extension) of the last step `persist_step` recorded as
    /// content-bearing: `error.is_none() && !is_utility_kind(kind)`. Source
    /// for `write_final_output` — the run's deliverable is whatever the last
    /// substantive, successful step left behind.
    content_step: Mutex<Option<String>>,
}

impl RunContextFiles {
    /// Create `<data_dir>/workflow-context/<run_id>/`; on failure log and
    /// return a disabled handle (the run proceeds without files).
    pub fn create(data_dir: &Path, run_id: &str) -> Self {
        let dir = data_dir.join("workflow-context").join(run_id);
        match std::fs::create_dir_all(&dir) {
            Ok(()) => Self {
                dir: Some(dir),
                run_id: run_id.to_string(),
                repos: Mutex::new(vec![]),
                content_step: Mutex::new(None),
            },
            Err(e) => {
                tracing::warn!("workflow-context: create {} failed: {e}", dir.display());
                Self::disabled(run_id)
            }
        }
    }

    pub fn disabled(run_id: &str) -> Self {
        Self {
            dir: None,
            run_id: run_id.to_string(),
            repos: Mutex::new(vec![]),
            content_step: Mutex::new(None),
        }
    }

    pub fn dir_str(&self) -> Option<String> {
        self.dir.as_ref().map(|d| d.to_string_lossy().into_owned())
    }

    /// Name of the auto-generated run brief — always `RUN_BRIEF_FILE`.
    // TODO(task 5): unused until an API/UI surface wants to name this file
    // without reaching for the constant directly.
    #[allow(dead_code)]
    pub fn brief_name(&self) -> String {
        RUN_BRIEF_FILE.to_string()
    }

    pub fn write_brief(&self, content: &str) {
        self.write_file(RUN_BRIEF_FILE, content);
    }

    /// Verbatim copy of the workflow's standing instructions. Caller only
    /// calls this when the field is non-empty (see `has_file`/callers).
    pub fn write_instructions_md(&self, content: &str) {
        self.write_file(INSTRUCTIONS_FILE, content);
    }

    /// Verbatim copy of this run's prompt/ask.
    pub fn write_prompt_md(&self, content: &str) {
        self.write_file(PROMPT_FILE, content);
    }

    /// General-purpose named write (e.g. `jira-<KEY>.md`) — the public face
    /// of the internal best-effort writer. True on success. Consumed by
    /// `prepare_context` for `jira-<KEY>.md`.
    pub fn write_named(&self, name: &str, content: &str) -> bool {
        self.write_file(name, content)
    }

    /// Whether `name` exists in this run's context dir (disabled ⇒ false).
    pub fn has_file(&self, name: &str) -> bool {
        self.dir.as_ref().is_some_and(|d| d.join(name).exists())
    }

    /// Replace the registry (run start) and persist `repos.json`.
    pub fn set_repos(&self, entries: Vec<RepoEntry>) {
        *self.repos.lock().unwrap() = entries;
        self.write_repos_json();
    }

    pub fn repos(&self) -> Vec<RepoEntry> {
        self.repos.lock().unwrap().clone()
    }

    /// Merge a reference a node published (`repo_id` + optional base/worktree)
    /// into the registry. A DECLARED `source` always wins over a published
    /// base — the user said what the destination is; a step can only fill
    /// blanks or add newly discovered repos.
    pub fn merge_published(&self, repo_id: &str, base: Option<&str>, worktree: Option<&str>) {
        {
            let mut repos = self.repos.lock().unwrap();
            if let Some(e) = repos.iter_mut().find(|e| e.repo_id.as_deref() == Some(repo_id)) {
                if let Some(w) = worktree.map(str::trim).filter(|s| !s.is_empty()) {
                    e.worktree = Some(w.to_string());
                }
                if e.source.is_none() {
                    if let Some(b) = base.map(str::trim).filter(|s| !s.is_empty()) {
                        e.base = Some(b.to_string());
                    }
                }
            } else {
                repos.push(RepoEntry {
                    repo: repo_id.to_string(),
                    repo_id: Some(repo_id.to_string()),
                    kind: "worktree".into(),
                    name: worktree.unwrap_or_default().to_string(),
                    source: None,
                    worktree: worktree.map(str::to_string).filter(|s| !s.is_empty()),
                    base: base.map(str::to_string).filter(|s| !s.is_empty()),
                    error: None,
                });
            }
        }
        self.write_repos_json();
    }

    pub fn step_md_path(&self, base_name: &str) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join(format!("{base_name}.md")))
    }

    pub fn step_md_mtime(&self, base_name: &str) -> Option<SystemTime> {
        let p = self.step_md_path(base_name)?;
        std::fs::metadata(p).ok()?.modified().ok()
    }

    /// `step*.md` file names present in the dir (what an agent can read),
    /// sorted by step NUMBER (lexicographic would put step10 before step2).
    pub fn list_step_mds(&self) -> Vec<String> {
        let Some(dir) = &self.dir else { return vec![] };
        let Ok(rd) = std::fs::read_dir(dir) else { return vec![] };
        let mut out: Vec<String> = rd
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| n.starts_with("step") && n.ends_with(".md"))
            .collect();
        out.sort_by_key(|n| {
            let num: u64 = n
                .strip_prefix("step")
                .and_then(|r| r.split('-').next())
                .and_then(|d| d.parse().ok())
                .unwrap_or(u64::MAX);
            (num, n.clone())
        });
        out
    }

    /// Engine-side persistence after a node attempt concludes. Always writes
    /// `{base}.output.json` (capped). Writes `{base}.md` from `render_step_md`
    /// UNLESS the file exists and was modified at/after `attempt_started` —
    /// i.e. the agent wrote its own handoff during the WINNING attempt. A file
    /// left behind by a failed earlier attempt (older mtime) is replaced, so
    /// downstream steps never read a failed attempt's summary.
    /// Returns log lines for the node's run log.
    #[allow(clippy::too_many_arguments)]
    pub fn persist_step(
        &self,
        base_name: &str,
        kind: &str,
        name: &str,
        out: &Value,
        logs: &[String],
        error: Option<&str>,
        attempt_started: Option<SystemTime>,
    ) -> Vec<String> {
        if self.dir.is_none() {
            return vec![];
        }
        let mut lines = Vec::new();
        // Raw output, capped.
        let mut raw = serde_json::to_string_pretty(out).unwrap_or_else(|_| out.to_string());
        if raw.len() > OUTPUT_JSON_CAP {
            let mut end = OUTPUT_JSON_CAP;
            while end > 0 && !raw.is_char_boundary(end) {
                end -= 1;
            }
            raw.truncate(end);
            raw.push_str("\n… [truncated by otto: output exceeded 5 MiB]");
        }
        if self.write_file(&format!("{base_name}.output.json"), &raw) {
            lines.push(format!("context: wrote {base_name}.output.json"));
        }
        // Curated summary — engine fallback unless the agent's own file won.
        let agent_wrote_own = match (attempt_started, self.step_md_mtime(base_name)) {
            (Some(t0), Some(m)) => m >= t0,
            _ => false,
        };
        if agent_wrote_own {
            lines.push(format!("context: kept agent-written {base_name}.md"));
        } else {
            let md = render_step_md(kind, name, out, logs, error);
            if self.write_file(&format!("{base_name}.md"), &md) {
                lines.push(format!("context: wrote {base_name}.md"));
            }
        }
        // Track the run's current deliverable candidate: the latest step that
        // succeeded (no error) and isn't bookkeeping/control-only. Overwritten
        // by every later qualifying step — `write_final_output` reads whatever
        // this points at when the run concludes.
        if error.is_none() && !is_utility_kind(kind) {
            *self.content_step.lock().unwrap() = Some(base_name.to_string());
        }
        lines
    }

    /// Copy the last content-bearing, error-free step's `.md` to
    /// `FINAL_OUTPUT_FILE`. `None` when disabled, no qualifying step ran yet,
    /// or the source file is unreadable — best-effort, like every write here.
    /// Returns the copied bytes (the caller uses them for delivery) on success.
    pub fn write_final_output(&self) -> Option<Vec<u8>> {
        let dir = self.dir.as_ref()?;
        let base = self.content_step.lock().unwrap().clone()?;
        let bytes = match std::fs::read(dir.join(format!("{base}.md"))) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!("workflow-context({}): read {base}.md for final-output failed: {e}", self.run_id);
                return None;
            }
        };
        if let Err(e) = std::fs::write(dir.join(FINAL_OUTPUT_FILE), &bytes) {
            tracing::warn!("workflow-context({}): write {FINAL_OUTPUT_FILE} failed: {e}", self.run_id);
        }
        Some(bytes)
    }

    /// The full `[workflow context]` preamble for an agent-backed step: reads
    /// what's actually on disk for this run (instructions.md/prompt.md/repos)
    /// so every call site stays in sync without re-deriving the flags itself,
    /// builds the step's own handoff-file name via `step_base_name`, and hands
    /// off to `agent_preamble`. Disabled context files ⇒ `""` (no dir to point
    /// an agent at), matching every other best-effort method here.
    pub fn preamble_for(
        &self,
        step_no: usize,
        display_name: &str,
        iter: Option<u64>,
        inner_idx: Option<usize>,
    ) -> String {
        let Some(dir) = self.dir_str() else { return String::new() };
        let own_md = format!("{}.md", step_base_name(step_no, display_name, iter, inner_idx));
        agent_preamble(
            &dir,
            self.has_file(INSTRUCTIONS_FILE),
            self.has_file(PROMPT_FILE),
            !self.repos().is_empty(),
            &self.list_step_mds(),
            &own_md,
        )
    }

    fn write_repos_json(&self) {
        let repos = self.repos.lock().unwrap().clone();
        let json = serde_json::to_string_pretty(&repos).unwrap_or_else(|_| "[]".into());
        self.write_file("repos.json", &json);
    }

    /// Best-effort write; warns (never errors). True on success.
    fn write_file(&self, name: &str, content: &str) -> bool {
        let Some(dir) = &self.dir else { return false };
        match std::fs::write(dir.join(name), content) {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!("workflow-context({}): write {name} failed: {e}", self.run_id);
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn slug_sanitizes_and_caps() {
        assert_eq!(slug("Write tests for a story"), "write-tests-for-a-story");
        assert_eq!(slug("  Fix / retry #2  "), "fix-retry-2");
        assert!(slug(&"x".repeat(100)).len() <= 40);
        assert_eq!(slug(""), "step");
        assert_eq!(slug("---"), "step");
    }

    #[test]
    fn step_names_match_user_convention() {
        assert_eq!(step_base_name(1, "Gather info", None, None), "step1-gather-info");
        assert_eq!(step_base_name(3, "review", Some(2), None), "step3-review-iter2");
        assert_eq!(step_base_name(3, "review", Some(2), Some(1)), "step3-review-1-iter2");
    }

    #[test]
    fn parse_entries_tolerant_and_shaped() {
        let v = json!([
            {"repo": "otto_os", "type": "branch", "name": "feat/x", "source": "main"},
            {"repo": "~/wt/k", "type": "worktree", "name": "~/wt/k"},
            {"nonsense": true},
            "junk"
        ]);
        let e = parse_repo_entries(&v);
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].kind, "branch");
        assert_eq!(e[0].source.as_deref(), Some("main"));
        assert_eq!(e[1].kind, "worktree");
        assert!(e[1].source.is_none());
        assert!(parse_repo_entries(&json!(null)).is_empty());
        assert!(parse_repo_entries(&json!({"repo": "x"})).is_empty());
    }

    #[test]
    fn entry_to_target_skips_nulls() {
        let e = RepoEntry {
            repo: "r".into(),
            repo_id: Some("R1".into()),
            kind: "branch".into(),
            name: "feat".into(),
            source: None,
            worktree: Some("/w".into()),
            base: Some("develop".into()),
            error: None,
        };
        let t = entry_to_target(&e);
        assert_eq!(t, json!({"repo_id": "R1", "worktree": "/w", "base": "develop"}));
    }

    /// One resolved repo entry — a sample non-empty `repos` list for tests
    /// that only care that repos WERE declared, not the specific shape.
    fn sample_repos() -> Vec<RepoEntry> {
        vec![RepoEntry {
            repo: "otto_os".into(),
            repo_id: Some("r1".into()),
            kind: "branch".into(),
            name: "feat/x".into(),
            source: Some("main".into()),
            worktree: Some("/w/x".into()),
            base: Some("main".into()),
            error: None,
        }]
    }

    #[test]
    fn instruction_lists_mission_repos_steps() {
        let repos = sample_repos();
        let input = json!({"msg": "Write tests", "goals": ["tests pass"], "trigger": "chat"});
        let md = render_brief(
            "UI-TEST",
            "desc",
            "run1",
            &input,
            &repos,
            &[
                ("Start".into(), "manual_trigger".into()),
                ("Write tests".into(), "agent_prompt".into()),
            ],
            false,
            false,
        );
        assert!(md.contains("run-brief.md"), "self-references its filename");
        assert!(md.contains("feat/x") && md.contains("main"), "repos table has work + source");
        assert!(md.contains("Write tests"));
        assert!(md.contains("repos.json"));
        assert!(md.contains("Trigger: chat"));
        assert!(md.contains("tests pass"));
        assert!(md.contains("step{N}-{name}.md"), "explains the step-file protocol");
    }

    #[test]
    fn brief_conditional_sections() {
        let md = render_brief("W", "", "r1", &json!({"prompt":"do it"}), &[], &[], true, true);
        assert!(md.contains("run-brief.md") && !md.contains("wf-r1-instruction"));
        assert!(md.contains("instructions.md") && md.contains("prompt.md"));
        assert!(!md.contains("repos.json"), "no repos declared → no repos.json guidance");
        assert!(md.contains("## Repos & branches") && md.contains("_No repos declared for this run._"), "empty case still gets the heading + placeholder as signal");
        let md2 = render_brief("W", "", "r1", &json!({}), &sample_repos(), &[], false, false);
        assert!(md2.contains("repos.json"));
        assert!(!md2.contains("instructions.md — standing"), "no instructions → not referenced");
    }

    #[test]
    fn brief_mission_dedups_msg_equal_to_prompt() {
        // normalize_prompt copies msg into prompt when prompt is absent — the
        // brief must not then show the identical line twice under two labels.
        let md = render_brief("W", "", "r1", &json!({"msg": "do it", "prompt": "do it"}), &[], &[], false, false);
        assert!(md.contains("**Prompt:** do it"));
        assert!(!md.contains("**Message:** do it"), "msg row skipped when identical to prompt");
        // Distinct msg/prompt still both render.
        let md2 = render_brief("W", "", "r1", &json!({"msg": "raw msg", "prompt": "distinct prompt"}), &[], &[], false, false);
        assert!(md2.contains("**Message:** raw msg") && md2.contains("**Prompt:** distinct prompt"));
    }

    #[test]
    fn step_md_full_reply_never_truncated() {
        let long = "x".repeat(20_000);
        let md = render_step_md("agent_prompt", "fix", &json!({"reply": long.clone()}), &[], None);
        assert!(md.contains(&long), "full reply, no truncation");
        let md = render_step_md(
            "review_run",
            "review",
            &json!({"score": 55, "passed": false, "findings": ["a", "b"]}),
            &["log1".into()],
            None,
        );
        assert!(md.contains("55") && md.contains("\"a\"") && md.contains("\"b\""));
        assert!(md.contains("log1"));
        let md = render_step_md("http_request", "call", &json!({"status": 200}), &[], Some("boom"));
        assert!(md.contains("boom"));
    }

    #[test]
    fn preamble_names_files_and_own_target() {
        let p = agent_preamble("/d", true, true, true, &["step1-a.md".into()], "step2-b.md");
        assert!(p.contains("/d") && p.contains("run-brief.md"));
        assert!(p.contains("step1-a.md") && p.contains("step2-b.md"));
        let p = agent_preamble("/d", false, false, false, &[], "step1-a.md");
        assert!(p.contains("first step"));
    }

    #[test]
    fn preamble_conditional_files() {
        let p = agent_preamble("/d", true, true, false, &[], "step1-a.md");
        assert!(p.contains("instructions.md") && p.contains("by the letter"));
        assert!(p.contains("prompt.md") && p.contains("run-brief.md"));
        assert!(!p.contains("repos.json"));
        let p2 = agent_preamble("/d", false, false, true, &[], "step1-a.md");
        assert!(!p2.contains("instructions.md") && !p2.contains("prompt.md"));
        assert!(p2.contains("repos.json"));
    }

    #[test]
    fn files_lifecycle_write_and_fallback() {
        let td = tempfile::tempdir().unwrap();
        let f = RunContextFiles::create(td.path(), "r1");
        assert!(f.dir_str().unwrap().ends_with("workflow-context/r1"));
        f.write_brief("# hi");
        assert!(td.path().join("workflow-context/r1/run-brief.md").exists());
        let logs = f.persist_step("step1-a", "log", "a", &json!({"k": "v"}), &[], None, None);
        assert!(td.path().join("workflow-context/r1/step1-a.md").exists());
        assert!(td.path().join("workflow-context/r1/step1-a.output.json").exists());
        assert!(!logs.is_empty());
        assert_eq!(f.list_step_mds(), vec!["step1-a.md".to_string()]);
        // Disabled handle: everything no-ops.
        let d = RunContextFiles::disabled("rX");
        assert!(d.dir_str().is_none());
        assert!(d.persist_step("s", "log", "a", &json!({}), &[], None, None).is_empty());
    }

    #[test]
    fn named_writes_and_has_file() {
        let td = tempfile::tempdir().unwrap();
        let f = RunContextFiles::create(td.path(), "r6");
        assert_eq!(f.brief_name(), "run-brief.md");
        assert!(!f.has_file("instructions.md"));
        f.write_instructions_md("standing rules");
        assert!(f.has_file("instructions.md"));
        f.write_prompt_md("the ask");
        assert!(f.has_file("prompt.md"));
        assert!(f.write_named("jira-ABC-1.md", "ticket body"));
        assert!(f.has_file("jira-ABC-1.md"));
        assert_eq!(
            std::fs::read_to_string(td.path().join("workflow-context/r6/jira-ABC-1.md")).unwrap(),
            "ticket body"
        );
        // Disabled handle: no-ops, never panics.
        let d = RunContextFiles::disabled("rY");
        assert!(!d.has_file("prompt.md"));
        assert!(!d.write_named("x.md", "y"));
    }

    #[test]
    fn final_output_last_content_step() {
        let td = tempfile::tempdir().unwrap();
        let f = RunContextFiles::create(td.path(), "r9");
        f.persist_step("step1-prep", "prepare_context", "prep", &json!({"jira":{"found":false}}), &[], None, None);
        f.persist_step("step2-report", "agent_prompt", "report", &json!({"reply":"THE DELIVERABLE"}), &[], None, None);
        f.persist_step("step3-notify", "channel_notify", "notify", &json!({"sent":true}), &[], None, None);
        let bytes = f.write_final_output().expect("copied");
        let s = String::from_utf8(bytes).unwrap();
        assert!(s.contains("THE DELIVERABLE"));
        assert!(td.path().join("workflow-context/r9/final-output.md").exists());
    }

    #[test]
    fn final_output_skips_errored_and_handles_none() {
        let td = tempfile::tempdir().unwrap();
        let f = RunContextFiles::create(td.path(), "r10");
        f.persist_step("step1-x", "agent_prompt", "x", &Value::Null, &[], Some("boom"), None);
        assert!(f.write_final_output().is_none(), "errored step is not a deliverable");
        let d = RunContextFiles::disabled("r11");
        assert!(d.write_final_output().is_none());
    }

    #[test]
    fn persist_step_respects_agent_file_only_from_winning_attempt() {
        let td = tempfile::tempdir().unwrap();
        let f = RunContextFiles::create(td.path(), "r2");
        let p = td.path().join("workflow-context/r2/step1-b.md");
        // Stale file from a FAILED earlier attempt (mtime before the winning
        // attempt started) must be replaced by the engine fallback.
        std::fs::write(&p, "stale attempt summary").unwrap();
        let stale_mtime = std::fs::metadata(&p).unwrap().modified().unwrap();
        let attempt_started = stale_mtime + std::time::Duration::from_secs(1);
        f.persist_step(
            "step1-b",
            "agent_prompt",
            "b",
            &json!({"reply": "fresh full reply"}),
            &[],
            None,
            Some(attempt_started),
        );
        let got = std::fs::read_to_string(&p).unwrap();
        assert!(got.contains("fresh full reply"), "stale file replaced: {got}");
        // A file written DURING the winning attempt is the agent's handoff — kept.
        std::fs::write(&p, "agent wrote this").unwrap();
        let before = std::fs::metadata(&p).unwrap().modified().unwrap() - std::time::Duration::from_secs(1);
        f.persist_step(
            "step1-b",
            "agent_prompt",
            "b",
            &json!({"reply": "engine fallback"}),
            &[],
            None,
            Some(before),
        );
        assert!(std::fs::read_to_string(&p).unwrap().contains("agent wrote this"));
    }

    #[test]
    fn output_json_capped_with_marker() {
        let td = tempfile::tempdir().unwrap();
        let f = RunContextFiles::create(td.path(), "r3");
        let big = json!({"blob": "y".repeat(6 * 1024 * 1024)});
        f.persist_step("step1-c", "transform", "c", &big, &[], None, None);
        let raw = std::fs::read_to_string(td.path().join("workflow-context/r3/step1-c.output.json")).unwrap();
        assert!(raw.len() <= OUTPUT_JSON_CAP + 1024);
        assert!(raw.contains("truncated"), "cap leaves an explicit marker");
    }

    #[test]
    fn all_declared_errored_guards_only_total_failure() {
        let ok = RepoEntry {
            repo: "good".into(),
            repo_id: Some("G".into()),
            kind: "branch".into(),
            name: "feat".into(),
            source: None,
            worktree: Some("/w".into()),
            base: None,
            error: None,
        };
        let bad = RepoEntry {
            repo: "typo".into(),
            repo_id: None,
            kind: "branch".into(),
            name: "feat/typo".into(),
            source: None,
            worktree: None,
            base: None,
            error: Some("branch 'feat/typo' is not checked out anywhere in typo".into()),
        };
        // No declarations at all → legacy fallback allowed.
        assert!(all_declared_errored(&[]).is_none());
        // A valid entry among errors → proceed on the valid ones.
        assert!(all_declared_errored(&[bad.clone(), ok]).is_none());
        // EVERY declaration errored → the actionable message, never a silent
        // fallback to the run cwd.
        let msg = all_declared_errored(&[bad]).unwrap();
        assert!(msg.contains("typo") && msg.contains("not checked out"), "{msg}");
    }

    #[test]
    fn list_step_mds_sorts_numerically() {
        let td = tempfile::tempdir().unwrap();
        let f = RunContextFiles::create(td.path(), "r5");
        for n in ["step10-late", "step2-early", "step1-first"] {
            f.persist_step(n, "log", "x", &json!({}), &[], None, None);
        }
        assert_eq!(
            f.list_step_mds(),
            vec!["step1-first.md", "step2-early.md", "step10-late.md"]
        );
    }

    #[test]
    fn merge_published_declared_source_wins() {
        let f = RunContextFiles::disabled("r4");
        f.set_repos(vec![RepoEntry {
            repo: "a".into(),
            repo_id: Some("A".into()),
            kind: "branch".into(),
            name: "feat".into(),
            source: Some("develop".into()),
            worktree: None,
            base: Some("develop".into()),
            error: None,
        }]);
        f.merge_published("A", Some("main"), Some("/w/a")); // published base must NOT clobber declared source
        let r = f.repos();
        assert_eq!(r[0].base.as_deref(), Some("develop"));
        assert_eq!(r[0].worktree.as_deref(), Some("/w/a"));
        f.merge_published("B", Some("master"), Some("/w/b")); // unknown repo appends a discovered entry
        assert_eq!(f.repos().len(), 2);
        assert_eq!(f.repos()[1].base.as_deref(), Some("master"));
    }
}
