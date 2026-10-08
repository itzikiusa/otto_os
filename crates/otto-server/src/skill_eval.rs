//! Skills Evaluator — test a skill by having a coding agent USE it to implement
//! a task in an isolated git worktree, then run a fleet of validation agents
//! over the result, score it, and (between iterations) have an improver agent
//! edit a *copy* of the skill following best practices and re-run.
//!
//! Mirrors the PR-review subsystem ([`crate::review_session`]): each agent runs
//! as a real, openable [`SessionManager`] session (tagged `meta.source =
//! "skilleval"`, hidden from the main grid) that writes its result to a temp
//! file we poll for. Per-validation live state lives in each iteration's
//! `agents_json` and is persisted one index at a time so the UI's poll shows
//! progress. Resilience: one stuck/failed agent never aborts the others.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{Path as AxPath, Query as AxQuery, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use otto_core::api::{
    CreateSessionReq, EvalTarget, ImplDiffResp, LibrarySkill, PromoteSkillReq, RateIterationReq,
    RegressionReq, SkillEvalConfig, SkillEvalImproverCfg, SkillEvalValidationCfg, SkillSourceInfo,
    SkillSourceReq, SkillSourcesResp, StartSkillEvalReq,
};
use otto_core::domain::{
    EvalFinding, EvalScore, EvalValidationState, GoldenTask, NoticeKind, NoticeSeverity,
    PromoteGate, SessionKind, SkillEval, SkillEvalStatus, User, Workspace, WorkspaceRole,
};
use otto_core::event::Event;
use otto_core::{Error, Id, Result};
use otto_sessions::SessionManager;
use otto_state::{NewNotice, SkillEvalsRepo};

use crate::auth::{require_root, require_ws_role, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

/// Per-run cancellation flags, keyed by eval id. A running eval checks its flag
/// between/within agent steps; cancel/delete set it and kill live sessions.
pub use otto_core::cancel::CancelRegistry;

fn register_cancel(reg: &CancelRegistry, eval_id: &str) -> Arc<AtomicBool> {
    let flag = Arc::new(AtomicBool::new(false));
    if let Ok(mut map) = reg.lock() {
        map.insert(eval_id.to_string(), Arc::clone(&flag));
    }
    flag
}

fn signal_cancel(reg: &CancelRegistry, eval_id: &str) {
    if let Ok(map) = reg.lock() {
        let retries = format!("{eval_id}:retry:");
        for (key, flag) in map.iter() {
            if key == eval_id || key.starts_with(&retries) {
                flag.store(true, Ordering::SeqCst);
            }
        }
    }
}

/// One retry per slot, registered before asynchronous preparation. Drop only
/// removes this attempt, including on request errors or task cancellation.
pub(crate) struct RetryLease {
    registry: CancelRegistry,
    key: String,
    pub(crate) flag: Arc<AtomicBool>,
}

impl RetryLease {
    pub(crate) fn claim(registry: &CancelRegistry, run: &str, slot: &str) -> Result<Self> {
        let key = format!("{run}:retry:{slot}");
        let flag = Arc::new(AtomicBool::new(false));
        let mut map = registry
            .lock()
            .map_err(|_| Error::Internal("retry registry unavailable".into()))?;
        if map.contains_key(&key) {
            return Err(Error::Conflict(
                "this agent retry is already running".into(),
            ));
        }
        map.insert(key.clone(), Arc::clone(&flag));
        Ok(Self {
            registry: Arc::clone(registry),
            key,
            flag,
        })
    }
}

impl Drop for RetryLease {
    fn drop(&mut self) {
        if let Ok(mut map) = self.registry.lock() {
            if map
                .get(&self.key)
                .is_some_and(|flag| Arc::ptr_eq(flag, &self.flag))
            {
                map.remove(&self.key);
            }
        }
    }
}

fn unregister_cancel(reg: &CancelRegistry, eval_id: &str) {
    if let Ok(mut map) = reg.lock() {
        map.remove(eval_id);
    }
}

fn is_cancelled(flag: &Arc<AtomicBool>) -> bool {
    flag.load(Ordering::SeqCst)
}

// ---------------------------------------------------------------------------
// PTY driving constants (match otto_review::session — claude's TUI is slow).
// ---------------------------------------------------------------------------

const OUTPUT_POLL: Duration = Duration::from_millis(1000);
const WAITING_IDLE: Duration = Duration::from_secs(120);

/// Implementation agents get a longer grace period than validators (they edit
/// code), validators a medium one, the improver a short one.
const IMPL_TIMEOUT: Duration = Duration::from_secs(7200); // 120 min
const VALIDATION_TIMEOUT: Duration = Duration::from_secs(3600); // 60 min
const IMPROVER_TIMEOUT: Duration = Duration::from_secs(3600); // 60 min

/// Best-practice guidance handed to the improver so skill edits stay healthy.
///
/// Deliberately biased toward *positive* instruction. The improver runs once per
/// iteration on top of the previous winner, so any nudge toward "add a hard rule"
/// compounds: after a few rounds a skill degenerates into a wall of MUST/NEVER
/// lines, and no validation dimension scores phrasing, so nothing pushes back.
/// Stating the wanted behaviour (plus why, plus how to check it) generalises to
/// situations the rule never anticipated; a prohibition only names one trap.
/// Prohibitions still earn their place for destructive actions and for trigger
/// scope ("when NOT to use this skill") — those are boundaries, not style.
const SKILL_BEST_PRACTICES: &str = "Skill authoring best practices to follow:\n\
- Keep valid YAML frontmatter with at least `name:` and a concise one-line `description:`.\n\
- Preserve the skill's original intent and structure; change only what the findings justify.\n\
- Prefer additive, surgical edits (tighten wording, add a missing rule, add a concrete example) \
over rewrites.\n\
- Be specific and actionable, and prefer stating the behaviour you WANT over listing what to \
avoid: describe the correct action, give the reason it matters in one clause, and where possible \
name the check that confirms it was done (a command to run, an output to look for). An agent that \
knows the goal and can verify it handles cases a prohibition never anticipated.\n\
- Reserve explicit prohibitions for two things: destructive or irreversible actions, and trigger \
scope (when this skill should NOT be used). Elsewhere, replace a `NEVER do X` line with the \
positive rule that makes X unnecessary — and do not restate a rule the skill already makes once.\n\
- Do not introduce contradictions or remove instructions that are still correct.\n\
- Do NOT game the validators: never add instructions that merely tell the agent to satisfy these \
specific checks, hardcode outputs, or name the validation dimensions — the edits must improve how \
the agent works IN GENERAL so the gains hold on unseen tasks.\n\
- Return the COMPLETE new SKILL.md content (frontmatter + body), not a fragment.";

// ---------------------------------------------------------------------------
// Skill source resolution
// ---------------------------------------------------------------------------

struct ResolvedSkill {
    name: String,
    body: String,
}

/// Validate a skill name is a safe single path segment (alphanumeric/-/_).
fn is_safe_name(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Coerce an arbitrary string into a safe skill-name segment.
fn sanitize_name(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "skill".to_string()
    } else {
        trimmed
    }
}

/// First 6 alphanumeric chars of an id, lowercased — a short run tag.
fn short_id(id: &Id) -> String {
    id.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(6)
        .collect::<String>()
        .to_lowercase()
}

/// Locate a `SKILL.md` (or the only `*.md`) under a directory tree.
#[allow(clippy::disallowed_methods)] // sync helper: resolve_skill_source runs via spawn_blocking
fn find_skill_file(dir: &Path) -> Option<PathBuf> {
    // Prefer a SKILL.md anywhere in the tree (shallow-first).
    let mut stack = vec![dir.to_path_buf()];
    let mut md_fallback: Option<PathBuf> = None;
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.file_name().and_then(|n| n.to_str()) == Some("SKILL.md") {
                return Some(p);
            } else if p.extension().and_then(|x| x.to_str()) == Some("md") && md_fallback.is_none()
            {
                md_fallback = Some(p);
            }
        }
    }
    md_fallback
}

/// Extract an archive into `dest` using system tools (macOS ships unzip + tar).
#[allow(clippy::disallowed_methods)] // sync helper: resolve_skill_source runs via spawn_blocking
fn extract_archive(archive: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest)
        .map_err(|e| Error::Internal(format!("create extract dir: {e}")))?;
    let name = archive.to_string_lossy().to_lowercase();
    let status = if name.ends_with(".zip") {
        std::process::Command::new("unzip")
            .arg("-o")
            .arg(archive)
            .arg("-d")
            .arg(dest)
            .status()
    } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") || name.ends_with(".gz") {
        // `tar -xzf` handles .tar.gz/.tgz; for a bare .gz tar still unwraps a
        // single-file gzip stream when it is a tarball, which skill archives are.
        std::process::Command::new("tar")
            .arg("-xzf")
            .arg(archive)
            .arg("-C")
            .arg(dest)
            .status()
    } else {
        return Err(Error::Invalid(format!(
            "unsupported archive type: {}",
            archive.display()
        )));
    };
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(Error::Upstream(format!("extract failed (exit {s})"))),
        Err(e) => Err(Error::Internal(format!("spawn extractor: {e}"))),
    }
}

/// [`resolve_skill_source`] off the runtime worker: it reads skill files and
/// may shell out to `unzip`/`tar` for an uploaded archive.
async fn resolve_skill_source_async(
    library: &otto_context::Library,
    src: &SkillSourceReq,
) -> Result<ResolvedSkill> {
    let (library, src) = (library.clone(), src.clone());
    tokio::task::spawn_blocking(move || resolve_skill_source(&library, &src))
        .await
        .map_err(|e| Error::Internal(format!("skill source task: {e}")))?
}

/// Resolve the skill under test to a `(name, body)` pair.
fn resolve_skill_source(
    library: &otto_context::Library,
    src: &SkillSourceReq,
) -> Result<ResolvedSkill> {
    match src.kind.as_str() {
        "library" => {
            let s = library
                .get_skill(&src.reference)
                .ok_or_else(|| Error::NotFound(format!("library skill '{}'", src.reference)))?;
            Ok(ResolvedSkill {
                name: s.name,
                body: s.body,
            })
        }
        "provider" => {
            let provider = src.provider.as_deref().unwrap_or("claude");
            if !is_safe_name(&src.reference) {
                return Err(Error::Invalid("unsafe skill name".into()));
            }
            let home = dirs::home_dir().ok_or_else(|| Error::Internal("no home dir".into()))?;
            let path = home
                .join(format!(".{provider}"))
                .join("skills")
                .join(&src.reference)
                .join("SKILL.md");
            let body = std::fs::read_to_string(&path).map_err(|e| {
                Error::NotFound(format!("{provider} skill '{}': {e}", src.reference))
            })?;
            Ok(ResolvedSkill {
                name: src.reference.clone(),
                body,
            })
        }
        "path" => {
            let p = PathBuf::from(&src.reference);
            if !p.exists() {
                return Err(Error::NotFound(format!("path '{}'", src.reference)));
            }
            let lower = src.reference.to_lowercase();
            let is_archive = lower.ends_with(".zip")
                || lower.ends_with(".gz")
                || lower.ends_with(".tgz")
                || lower.ends_with(".tar.gz");
            let (skill_file, derived_name): (PathBuf, String) = if is_archive {
                let tmp = std::env::temp_dir().join(format!(
                    "otto-skill-extract-{}",
                    short_id(&otto_core::new_id())
                ));
                extract_archive(&p, &tmp)?;
                let file = find_skill_file(&tmp)
                    .ok_or_else(|| Error::NotFound("no SKILL.md / .md inside archive".into()))?;
                // Name from the archive file stem (minus any .tar).
                let stem = p
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("skill")
                    .trim_end_matches(".tar")
                    .to_string();
                (file, stem)
            } else if p.is_dir() {
                let file = find_skill_file(&p)
                    .ok_or_else(|| Error::NotFound("no SKILL.md / .md in folder".into()))?;
                let name = p
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("skill")
                    .to_string();
                (file, name)
            } else {
                // A file. Name from its parent dir (for SKILL.md) or its stem.
                let name = if p.file_name().and_then(|n| n.to_str()) == Some("SKILL.md") {
                    p.parent()
                        .and_then(|d| d.file_name())
                        .and_then(|s| s.to_str())
                        .unwrap_or("skill")
                        .to_string()
                } else {
                    p.file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("skill")
                        .to_string()
                };
                (p.clone(), name)
            };
            let body = std::fs::read_to_string(&skill_file)
                .map_err(|e| Error::Internal(format!("read skill file: {e}")))?;
            Ok(ResolvedSkill {
                name: sanitize_name(&derived_name),
                body,
            })
        }
        other => Err(Error::Invalid(format!(
            "unknown skill source kind '{other}'"
        ))),
    }
}

// ---------------------------------------------------------------------------
// Git worktree
// ---------------------------------------------------------------------------

/// True if `repo_path` is inside a git work tree.
async fn is_git_repo(repo_path: &str) -> bool {
    otto_git::hardened_command()
        .arg("-C")
        .arg(repo_path)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// True if `repo` has at least one commit (HEAD resolves) — required before
/// `git worktree add … HEAD` can work.
async fn has_head_commit(repo: &str) -> bool {
    otto_git::hardened_command()
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "--verify", "HEAD"])
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// The default scratch repo location, used when the workspace root isn't a git
/// repo. Overridable via `OTTO_SKILLEVAL_DIR`; defaults to `~/Otto/SkillsEvaluator`.
fn scratch_dir() -> Result<PathBuf> {
    if let Ok(custom) = std::env::var("OTTO_SKILLEVAL_DIR") {
        if !custom.trim().is_empty() {
            return Ok(PathBuf::from(custom.trim()));
        }
    }
    dirs::home_dir()
        .map(|h| h.join("Otto").join("SkillsEvaluator"))
        .ok_or_else(|| Error::Internal("cannot resolve home directory for scratch repo".into()))
}

/// Ensure the scratch repo exists, is a git repo, and has an initial commit so
/// worktrees can be created from HEAD. Returns its absolute path.
async fn ensure_scratch_repo() -> Result<String> {
    let dir = scratch_dir()?;
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| Error::Internal(format!("create scratch dir: {e}")))?;
    let path = dir.to_string_lossy().to_string();

    if !is_git_repo(&path).await {
        let out = otto_git::hardened_command()
            .arg("-C")
            .arg(&path)
            .arg("init")
            .output()
            .await
            .map_err(|e| Error::Internal(format!("git init: {e}")))?;
        if !out.status.success() {
            return Err(Error::Upstream(format!(
                "git init failed in {path}: {}",
                String::from_utf8_lossy(&out.stderr)
                    .lines()
                    .next()
                    .unwrap_or("")
                    .trim()
            )));
        }
    }

    if !has_head_commit(&path).await {
        let _ = tokio::fs::write(
            dir.join("README.md"),
            "# Otto Skills Evaluator scratch repo\n\nOtto creates git worktrees here to run skill \
             evaluations when the workspace root is not a git repository. Safe to delete.\n",
        )
        .await;
        let _ = otto_git::hardened_command()
            .arg("-C")
            .arg(&path)
            .args(["add", "-A"])
            .output()
            .await;
        let out = otto_git::hardened_command()
            .arg("-C")
            .arg(&path)
            .args([
                "-c",
                "user.email=otto@localhost",
                "-c",
                "user.name=Otto",
                "commit",
                "-m",
                "Initialize Otto Skills Evaluator scratch repo",
            ])
            .output()
            .await
            .map_err(|e| Error::Internal(format!("git commit: {e}")))?;
        if !out.status.success() {
            return Err(Error::Upstream(format!(
                "could not create initial commit in scratch repo: {}",
                String::from_utf8_lossy(&out.stderr)
                    .lines()
                    .next()
                    .unwrap_or("")
                    .trim()
            )));
        }
    }
    Ok(path)
}

/// Resolve the base repo to create worktrees from: the workspace root when it's
/// a git repo with commits, else a scratch repo (created on demand). Returns
/// `(repo_path, used_scratch)`.
async fn resolve_base_repo(ws_root: &str) -> Result<(String, bool)> {
    if is_git_repo(ws_root).await && has_head_commit(ws_root).await {
        return Ok((ws_root.to_string(), false));
    }
    let scratch = ensure_scratch_repo().await?;
    Ok((scratch, true))
}

/// Create a detached worktree at `dest` checked out at `base_ref`.
async fn add_worktree(repo_path: &str, base_ref: &str, dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let out = otto_git::hardened_command()
        .arg("-C")
        .arg(repo_path)
        .args(["worktree", "add", "--detach"])
        .arg(dest)
        .arg(base_ref)
        .output()
        .await
        .map_err(|e| Error::Internal(format!("spawn git worktree: {e}")))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(Error::Upstream(format!(
            "git worktree add failed: {}",
            stderr.lines().next().unwrap_or("").trim()
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Output parsing
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct RawFinding {
    #[serde(default = "default_severity")]
    severity: String,
    #[serde(default)]
    issue: String,
    /// Tolerate `body`/`message` as aliases for `issue`.
    #[serde(default)]
    body: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    suggestion: String,
    #[serde(default)]
    fix: String,
    #[serde(default)]
    location: Option<String>,
    #[serde(default)]
    path: Option<String>,
}

fn default_severity() -> String {
    "info".to_string()
}

/// Parse an explicit findings verdict, retaining the distinction between a
/// valid clean `[]` and missing/malformed output. Fences/prose and the common
/// `{ "findings": [...] }` wrapper are accepted; unrelated arrays are not.
fn parse_findings(text: &str) -> Option<Vec<EvalFinding>> {
    let start = text.find(['[', '{'])?;
    let value = serde_json::Deserializer::from_str(&text[start..])
        .into_iter::<serde_json::Value>()
        .next()?
        .ok()?;
    let array = match value {
        serde_json::Value::Array(array) => array,
        serde_json::Value::Object(mut object) => object.remove("findings")?.as_array()?.clone(),
        _ => return None,
    };
    let mut findings = Vec::with_capacity(array.len());
    for value in array {
        let r: RawFinding = serde_json::from_value(value).ok()?;
        let issue = first_nonempty(&[&r.issue, &r.body, &r.message]);
        // Dropping an invalid entry would turn malformed findings into a pass.
        if issue.is_empty() {
            return None;
        }
        findings.push(EvalFinding {
            severity: r.severity,
            issue,
            suggestion: first_nonempty(&[&r.suggestion, &r.fix]),
            location: r.location.or(r.path),
        });
    }
    Some(findings)
}

/// A file can be observed between creation and its final write. Leave incomplete
/// validator output in place and keep polling until it contains a verdict.
fn capture_output_ready(text: &str, findings_mode: bool) -> bool {
    !findings_mode || parse_findings(text).is_some()
}

fn read_capture_output(path: &Path, findings_mode: bool) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    if !capture_output_ready(&text, findings_mode) {
        return None;
    }
    let _ = std::fs::remove_file(path);
    Some(text)
}

/// Used by initial validation and retries so transport completion alone cannot
/// create a successful pass or a clean Review artifact.
fn record_validation_pass(
    outcome: &AgentOutcome,
    scores: &mut Vec<f64>,
    union: &mut Vec<EvalFinding>,
) -> bool {
    if outcome.errored {
        return false;
    }
    let Some(findings) = parse_findings(&outcome.text) else {
        return false;
    };
    scores.push(score_findings(&findings).1);
    merge_findings(union, findings);
    true
}

fn finish_validation(
    mut state: EvalValidationState,
    session_id: Option<String>,
    scores: Vec<f64>,
    findings: Vec<EvalFinding>,
    passes: u32,
) -> EvalValidationState {
    state.session_id = session_id;
    if scores.len() != passes as usize {
        state.status = "error".into();
        state.note = format!(
            "validation produced {} of {passes} required findings verdicts",
            scores.len()
        );
        state.passed = false;
        state.score = 0.0;
        state.findings = findings;
    } else {
        state.status = "done".into();
        state.score = scores.iter().sum::<f64>() / scores.len() as f64;
        state.passed = !findings.iter().any(|f| is_fail(&f.severity));
        state.note = format!(
            "{} · {} issue{}{}",
            if state.passed { "passed" } else { "failed" },
            findings.len(),
            if findings.len() == 1 { "" } else { "s" },
            if passes > 1 {
                format!(" · {} passes", scores.len())
            } else {
                String::new()
            },
        );
        state.findings = findings;
    }
    state
}

fn first_nonempty(candidates: &[&str]) -> String {
    candidates
        .iter()
        .map(|s| s.trim())
        .find(|s| !s.is_empty())
        .unwrap_or("")
        .to_string()
}

#[derive(serde::Deserialize)]
struct RawImprovement {
    #[serde(default)]
    base_iter: Option<u32>,
    #[serde(default)]
    skill: String,
    #[serde(default)]
    summary: String,
}

/// Pull `{base_iter, skill, summary}` out of an improver reply.
fn parse_improvement(text: &str) -> Option<RawImprovement> {
    let start = text.find('{')?;
    let end = text.rfind('}').map(|i| i + 1)?;
    if start >= end {
        return None;
    }
    serde_json::from_str::<RawImprovement>(&text[start..end]).ok()
}

/// True for a "fail"-class severity (a real violation of the dimension).
fn is_fail(severity: &str) -> bool {
    matches!(
        severity.to_lowercase().as_str(),
        "fail" | "error" | "critical"
    )
}

/// Severity rank for dedup (higher = more severe).
fn severity_rank(severity: &str) -> u8 {
    match severity.to_lowercase().as_str() {
        "fail" | "error" | "critical" => 3,
        "warn" | "warning" | "major" => 2,
        _ => 1,
    }
}

/// Normalized key for deduping findings across passes (same issue, any wording
/// drift in the tail is ignored).
fn finding_key(f: &EvalFinding) -> String {
    f.issue
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(80)
        .collect()
}

/// Union `incoming` into `acc`, deduping by issue and keeping the higher severity.
fn merge_findings(acc: &mut Vec<EvalFinding>, incoming: Vec<EvalFinding>) {
    for f in incoming {
        let key = finding_key(&f);
        if let Some(existing) = acc.iter_mut().find(|e| finding_key(e) == key) {
            if severity_rank(&f.severity) > severity_rank(&existing.severity) {
                existing.severity = f.severity;
            }
            if existing.suggestion.trim().is_empty() && !f.suggestion.trim().is_empty() {
                existing.suggestion = f.suggestion;
            }
        } else {
            acc.push(f);
        }
    }
}

/// Score one validation from its findings: start at 100 and subtract per
/// finding by severity. `passed` = no `fail`-severity findings.
pub(crate) fn score_findings(findings: &[EvalFinding]) -> (bool, f64) {
    let mut score = 100.0_f64;
    let mut passed = true;
    for f in findings {
        match f.severity.to_lowercase().as_str() {
            "fail" | "error" | "critical" => {
                score -= 25.0;
                passed = false;
            }
            "warn" | "warning" | "major" => score -= 8.0,
            _ => score -= 2.0,
        }
    }
    (passed, score.clamp(0.0, 100.0))
}

/// Produce a GNU unified diff between `old` and `new`, with `context_lines`
/// lines of unchanged context around each hunk.  The output is the standard
/// unified-diff format (`--- a/SKILL.md`, `+++ b/SKILL.md`, `@@ … @@` hunks)
/// so it renders cleanly in the UI's `<DiffView>` component and is instantly
/// readable without any client-side transformation.
fn unified_diff(old: &str, new: &str, context_lines: usize) -> String {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let (n, m) = (a.len(), b.len());

    // Collect edit operations: ('=', old_line), ('-', old_line), ('+', new_line).
    #[derive(Clone)]
    enum Op {
        Keep(usize, usize), // (a_idx, b_idx)
        Delete(usize),      // a_idx
        Insert(usize),      // b_idx
    }
    let mut ops: Vec<Op> = Vec::with_capacity(n + m);

    // Common prefix/suffix are kept verbatim — an improver edit usually
    // touches a few lines, so the O(n·m) LCS below only sees the changed middle.
    let mut pre = 0usize;
    while pre < n && pre < m && a[pre] == b[pre] {
        pre += 1;
    }
    let mut suf = 0usize;
    while suf < n - pre && suf < m - pre && a[n - 1 - suf] == b[m - 1 - suf] {
        suf += 1;
    }
    for k in 0..pre {
        ops.push(Op::Keep(k, k));
    }
    let (an, bm) = (n - pre - suf, m - pre - suf);
    // LCS length table over the middle only, and only below a cell cap (the
    // table is u32 per cell: 4M cells = 16 MB). Past the cap the middle is
    // emitted as a whole delete + insert — still a valid unified diff.
    const MAX_LCS_CELLS: usize = 4_000_000;
    if an.saturating_mul(bm) <= MAX_LCS_CELLS {
        let (a, b) = (&a[pre..pre + an], &b[pre..pre + bm]);
        let mut lcs = vec![vec![0u32; bm + 1]; an + 1];
        for i in (0..an).rev() {
            for j in (0..bm).rev() {
                lcs[i][j] = if a[i] == b[j] {
                    lcs[i + 1][j + 1] + 1
                } else {
                    lcs[i + 1][j].max(lcs[i][j + 1])
                };
            }
        }
        let (mut i, mut j) = (0usize, 0usize);
        while i < an && j < bm {
            if a[i] == b[j] {
                ops.push(Op::Keep(pre + i, pre + j));
                i += 1;
                j += 1;
            } else if lcs[i + 1][j] >= lcs[i][j + 1] {
                ops.push(Op::Delete(pre + i));
                i += 1;
            } else {
                ops.push(Op::Insert(pre + j));
                j += 1;
            }
        }
        while i < an {
            ops.push(Op::Delete(pre + i));
            i += 1;
        }
        while j < bm {
            ops.push(Op::Insert(pre + j));
            j += 1;
        }
    } else {
        ops.extend((pre..pre + an).map(Op::Delete));
        ops.extend((pre..pre + bm).map(Op::Insert));
    }
    for k in 0..suf {
        ops.push(Op::Keep(n - suf + k, m - suf + k));
    }

    if ops.is_empty() {
        return String::new();
    }

    // Group ops into hunks separated by more than `context_lines * 2` unchanged
    // lines.  Each hunk records the op-index range [hunk_start, hunk_end).
    let mut hunks: Vec<(usize, usize)> = Vec::new();
    let mut hunk_start: Option<usize> = None;
    let mut last_change = 0usize;
    for (idx, op) in ops.iter().enumerate() {
        let is_change = !matches!(op, Op::Keep(..));
        if is_change {
            if hunk_start.is_none() {
                // Begin a new hunk with leading context.
                let start = idx.saturating_sub(context_lines);
                hunk_start = Some(start);
            }
            last_change = idx;
        }
        if let Some(hs) = hunk_start {
            // Close the hunk once we've run `context_lines` unchanged lines past
            // the last change (or reached the end).
            let trailing = idx.saturating_sub(last_change);
            if !is_change && trailing > context_lines {
                hunks.push((hs, idx - context_lines + context_lines.min(trailing)));
                hunk_start = None;
            }
        }
    }
    if let Some(hs) = hunk_start {
        let end = (last_change + context_lines + 1).min(ops.len());
        hunks.push((hs, end));
    }

    if hunks.is_empty() {
        return String::new(); // no differences
    }

    // Render.
    let mut out = String::new();
    out.push_str("--- a/SKILL.md\n");
    out.push_str("+++ b/SKILL.md\n");
    for (hs, he) in hunks {
        let slice = &ops[hs..he];
        // Count old/new lines in this hunk for the @@ header.
        let old_count = slice
            .iter()
            .filter(|o| matches!(o, Op::Keep(..) | Op::Delete(_)))
            .count();
        let new_count = slice
            .iter()
            .filter(|o| matches!(o, Op::Keep(..) | Op::Insert(_)))
            .count();
        let old_start = match slice.first() {
            Some(Op::Keep(ai, _)) | Some(Op::Delete(ai)) => ai + 1,
            Some(Op::Insert(_)) => {
                // Leading inserts — find the first keep/delete for old start.
                slice
                    .iter()
                    .find_map(|o| match o {
                        Op::Keep(ai, _) | Op::Delete(ai) => Some(ai + 1),
                        _ => None,
                    })
                    .unwrap_or(1)
            }
            None => 1,
        };
        let new_start = match slice.first() {
            Some(Op::Keep(_, bi)) | Some(Op::Insert(bi)) => bi + 1,
            Some(Op::Delete(_)) => slice
                .iter()
                .find_map(|o| match o {
                    Op::Keep(_, bi) | Op::Insert(bi) => Some(bi + 1),
                    _ => None,
                })
                .unwrap_or(1),
            None => 1,
        };
        out.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            old_start, old_count, new_start, new_count
        ));
        for op in slice {
            match op {
                Op::Keep(ai, _) => {
                    out.push(' ');
                    out.push_str(a[*ai]);
                    out.push('\n');
                }
                Op::Delete(ai) => {
                    out.push('-');
                    out.push_str(a[*ai]);
                    out.push('\n');
                }
                Op::Insert(bi) => {
                    out.push('+');
                    out.push_str(b[*bi]);
                    out.push('\n');
                }
            }
        }
    }
    out
}

/// Convenience wrapper: 3-line context, matching GNU diff defaults.
fn simple_diff(old: &str, new: &str) -> String {
    unified_diff(old, new, 3)
}

// ---------------------------------------------------------------------------
// Agent driver (a real, openable session + temp output file)
// ---------------------------------------------------------------------------

/// How an in-flight agent's live state is persisted (so the UI poll shows it).
enum LiveSlot<'a> {
    /// Update element `index` of an iteration's `agents_json`, cloning `base`.
    Validation {
        repo: &'a SkillEvalsRepo,
        iter_id: &'a Id,
        index: usize,
        base: EvalValidationState,
    },
    /// Record the live implementation session id on the iteration row.
    Implementation {
        repo: &'a SkillEvalsRepo,
        iter_id: &'a Id,
        worktree: String,
    },
    /// No live persistence (improver — fast, status tracked on the iteration).
    None,
}

impl LiveSlot<'_> {
    async fn set(&mut self, status: &str, session_id: Option<&str>, note: &str) {
        match self {
            LiveSlot::Validation {
                repo,
                iter_id,
                index,
                base,
            } => {
                base.status = status.to_string();
                if let Some(sid) = session_id {
                    base.session_id = Some(sid.to_string());
                }
                base.note = note.chars().take(160).collect();
                let _ = repo.set_iter_agent_at(iter_id, *index, base).await;
            }
            LiveSlot::Implementation {
                repo,
                iter_id,
                worktree,
            } => {
                let _ = repo
                    .set_iter_impl(iter_id, session_id, note, Some(worktree.as_str()))
                    .await;
            }
            LiveSlot::None => {}
        }
    }
}

struct AgentOutcome {
    session_id: Option<String>,
    text: String,
    errored: bool,
}

/// Spawn `provider` as a live session in `cwd`, inject `prompt`, and wait for it
/// to write `output_path` (or exit / time out). When `findings_mode` is set, a
/// claude transcript that already contains a findings array is also accepted.
#[allow(clippy::too_many_arguments)]
async fn run_agent_capture(
    manager: &Arc<SessionManager>,
    ws: &Workspace,
    user: &User,
    provider: &str,
    cwd: &str,
    prompt: &str,
    output_path: &Path,
    timeout: Duration,
    findings_mode: bool,
    cancel: &Arc<AtomicBool>,
    mut slot: LiveSlot<'_>,
) -> AgentOutcome {
    let _ = std::fs::remove_file(output_path);
    if is_cancelled(cancel) {
        return AgentOutcome {
            session_id: None,
            text: String::new(),
            errored: true,
        };
    }

    let meta = serde_json::json!({ "source": "skilleval" });
    let req = CreateSessionReq {
        kind: SessionKind::Agent,
        provider: Some(provider.to_string()),
        title: None,
        cwd: Some(cwd.to_string()),
        connection_id: None,
        model: None,
        meta: Some(meta),
    };
    let session = match manager.create(ws, &user.id, req, None).await {
        Ok(s) => s,
        Err(e) => {
            slot.set("error", None, &format!("could not start: {e}"))
                .await;
            return AgentOutcome {
                session_id: None,
                text: String::new(),
                errored: true,
            };
        }
    };
    let sid = session.id.clone();
    slot.set("running", Some(&sid), "").await;

    if is_cancelled(cancel) {
        let _ = manager.archive(&sid).await;
        return AgentOutcome {
            session_id: Some(sid),
            text: String::new(),
            errored: true,
        };
    }

    crate::review_session::submit_prompt(manager, &sid, prompt).await;

    let deadline = Instant::now() + timeout;
    let mut flagged_waiting = false;
    let mut last_turn: Option<String> = None;
    let mut transcript = match session.provider_session_id.as_deref() {
        Some(psid) if provider == "claude" => Some(crate::turn_oracle::ClaudeTail::new(
            otto_orchestrator::claude_pty::session_jsonl_path(cwd, psid),
        )),
        _ => None,
    };

    loop {
        // 0. Cancelled — kill the session and bail.
        if is_cancelled(cancel) {
            let _ = manager.archive(&sid).await;
            return AgentOutcome {
                session_id: Some(sid),
                text: String::new(),
                errored: true,
            };
        }

        // 1. The agent wrote its output file (the reliable, provider-agnostic path).
        if let Some(text) = read_capture_output(output_path, findings_mode) {
            return AgentOutcome {
                session_id: Some(sid),
                text,
                errored: false,
            };
        }

        // 2. claude transcript fallback (read incrementally, off the runtime).
        let scan = crate::turn_oracle::poll_claude_tail(&mut transcript).await;
        if let Some(turn) = scan.and_then(|s| s.last_turn_text) {
            if findings_mode && capture_output_ready(&turn, true) {
                return AgentOutcome {
                    session_id: Some(sid),
                    text: turn,
                    errored: false,
                };
            }
            last_turn = Some(turn);
        }

        match manager.live_handle(&sid) {
            Some(handle) => {
                if handle.on_exit().borrow().is_some() {
                    // Final read of the file, then fall back to the last turn.
                    if let Some(text) = read_capture_output(output_path, findings_mode) {
                        return AgentOutcome {
                            session_id: Some(sid),
                            text,
                            errored: false,
                        };
                    }
                    if let Some(turn) = last_turn {
                        return AgentOutcome {
                            session_id: Some(sid),
                            text: turn,
                            errored: false,
                        };
                    }
                    slot.set("error", Some(&sid), "session exited before writing output")
                        .await;
                    return AgentOutcome {
                        session_id: Some(sid),
                        text: String::new(),
                        errored: true,
                    };
                }
                let idle = handle.last_output_at().elapsed();
                if idle >= WAITING_IDLE && !flagged_waiting {
                    flagged_waiting = true;
                    slot.set(
                        "waiting",
                        Some(&sid),
                        "looks blocked on input — Open it to respond",
                    )
                    .await;
                } else if idle < WAITING_IDLE && flagged_waiting {
                    flagged_waiting = false;
                    slot.set("running", Some(&sid), "").await;
                }
            }
            None => {
                slot.set("error", Some(&sid), "session is no longer live")
                    .await;
                return AgentOutcome {
                    session_id: Some(sid),
                    text: String::new(),
                    errored: true,
                };
            }
        }

        if Instant::now() >= deadline {
            if let Some(turn) = last_turn {
                return AgentOutcome {
                    session_id: Some(sid),
                    text: turn,
                    errored: false,
                };
            }
            slot.set("error", Some(&sid), "timed out (grace period elapsed)")
                .await;
            return AgentOutcome {
                session_id: Some(sid),
                text: String::new(),
                errored: true,
            };
        }
        tokio::time::sleep(OUTPUT_POLL).await;
    }
}

/// Output-file path for one agent within an iteration.
fn output_path(eval_id: &Id, iter: u32, slot: &str) -> PathBuf {
    let dir = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(dir).join(format!("otto-skilleval-{eval_id}-{iter}-{slot}.txt"))
}

// ---------------------------------------------------------------------------
// Orchestration
// ---------------------------------------------------------------------------

/// One agent run within an iteration (validation × provider). The model is
/// carried on the seeded [`EvalValidationState`] for display; the visible PTY
/// session driver itself does not apply a per-agent model override (mirrors the
/// review reviewer-session path).
struct ValRun {
    validation: String,
    provider: String,
    criteria: String,
}

async fn collect_validation_results(
    mut set: tokio::task::JoinSet<(usize, f64, Vec<EvalFinding>)>,
    names: &[String],
) -> (Vec<f64>, Vec<(String, EvalFinding)>) {
    let mut scores = Vec::new();
    let mut findings = Vec::new();
    while let Some(joined) = set.join_next().await {
        if let Ok((index, score, result)) = joined {
            scores.push(score);
            let label = names.get(index).cloned().unwrap_or_default();
            findings.extend(result.into_iter().map(|f| (label.clone(), f)));
        }
    }
    (scores, findings)
}

/// Background wrapper: run the eval, set the final status, notify, and clean up
/// the cancellation token.
async fn run_skill_eval(
    ctx: ServerCtx,
    eval_id: Id,
    ws: Workspace,
    user: User,
    req: StartSkillEvalReq,
    cancel: Arc<AtomicBool>,
) {
    let result = run_skill_eval_core(&ctx, &eval_id, &ws, &user, &req, &cancel).await;
    unregister_cancel(&ctx.skill_eval_cancels, &eval_id);

    let cancelled = is_cancelled(&cancel);
    let (status, error): (SkillEvalStatus, Option<String>) = if cancelled {
        (
            SkillEvalStatus::Cancelled,
            Some("Cancelled by user".to_string()),
        )
    } else {
        match &result {
            Ok(()) => (SkillEvalStatus::Done, None),
            Err(e) => (SkillEvalStatus::Error, Some(e.to_string())),
        }
    };
    if !ctx
        .skill_evals_store
        .finish_running(&eval_id, status, error.as_deref())
        .await
        .unwrap_or(false)
    {
        return;
    }
    match status {
        SkillEvalStatus::Done => tracing::info!(eval = %eval_id, "skill evaluation complete"),
        SkillEvalStatus::Cancelled => tracing::info!(eval = %eval_id, "skill evaluation cancelled"),
        _ => tracing::warn!(eval = %eval_id, "skill evaluation error: {:?}", error),
    }
    notify_complete(&ctx, &eval_id, &req.source.reference, status).await;
    // Broadcast a workspace-scoped event so the Skill-Eval UI can switch from
    // fixed-interval polling to event-driven refresh.
    let ev = Event::SkillEvalUpdated {
        workspace_id: ws.id.clone(),
        run_id: eval_id.clone(),
        status: match status {
            SkillEvalStatus::Done => "done".to_string(),
            SkillEvalStatus::Cancelled => "cancelled".to_string(),
            _ => "error".to_string(),
        },
    };
    if ctx.events.send(ev).is_err() {
        tracing::debug!(%eval_id, "no WS subscribers for SkillEvalUpdated");
    }
}

/// Post a notification when a run finishes (done/error/cancelled).
async fn notify_complete(ctx: &ServerCtx, eval_id: &Id, skill: &str, status: SkillEvalStatus) {
    let eval = ctx.skill_evals_store.get_eval(eval_id).await.ok();
    let (severity, body) = match status {
        SkillEvalStatus::Done => {
            let detail = eval
                .as_ref()
                .and_then(|e| e.best_score.map(|s| (e.best_iteration.unwrap_or(0), s)))
                .map(|(i, s)| format!("best score {s:.0} (iteration {i})"))
                .unwrap_or_else(|| "finished".to_string());
            (NoticeSeverity::Info, format!("Skill '{skill}' — {detail}"))
        }
        SkillEvalStatus::Cancelled => (
            NoticeSeverity::Warn,
            format!("Skill '{skill}' evaluation cancelled"),
        ),
        _ => (
            NoticeSeverity::Error,
            format!("Skill '{skill}' evaluation failed"),
        ),
    };
    let _ = ctx
        .notifications()
        .create(NewNotice {
            kind: NoticeKind::System,
            severity,
            title: "Skill evaluation finished".to_string(),
            body,
            source_key: Some(format!("skilleval:{eval_id}")),
            action: None,
            user_id: None, // global system notice
        })
        .await;
}

#[allow(clippy::too_many_lines)]
async fn run_skill_eval_core(
    ctx: &ServerCtx,
    eval_id: &Id,
    ws: &Workspace,
    user: &User,
    req: &StartSkillEvalReq,
    cancel: &Arc<AtomicBool>,
) -> Result<()> {
    if req.mode == "score_only" {
        return run_score_only_core(ctx, eval_id, ws, req, cancel).await;
    }
    let resolved = resolve_skill_source_async(&ctx.context_library, &req.source).await?;
    // Use the workspace repo when it's a git repo with commits; otherwise fall
    // back to a scratch repo (~/Otto/SkillsEvaluator), created + git-init'd on
    // demand, so the evaluator works even without a git workspace.
    let (repo_path, used_scratch) = resolve_base_repo(&ws.root_path).await?;
    if used_scratch {
        tracing::info!(eval = %eval_id, "workspace root is not a git repo; using scratch repo {repo_path}");
    }
    let base_ref = if used_scratch {
        "HEAD".to_string()
    } else {
        req.base_ref.clone().unwrap_or_else(|| "HEAD".to_string())
    };
    let improver = req
        .improver
        .clone()
        .unwrap_or_else(|| SkillEvalImproverCfg {
            provider: req.impl_cli.clone(),
            model: String::new(),
        });
    let iterations = req.iterations.max(1);
    let passes = req.validator_passes.clamp(1, 3);
    let run_tag = short_id(eval_id);

    // Pre-trust the repo for every provider that will run.
    {
        let mut provs: Vec<String> = vec![req.impl_cli.clone(), improver.provider.clone()];
        for v in &req.validations {
            if v.providers.is_empty() {
                provs.push(req.impl_cli.clone());
            } else {
                provs.extend(v.providers.iter().cloned());
            }
        }
        provs.sort();
        provs.dedup();
        for p in &provs {
            otto_sessions::trust::ensure_trusted(p, &repo_path);
        }
    }

    // Carried between iterations: the skill the next iteration tests, and the
    // iteration it was derived from. Plus a record of (iter, skill, score) so
    // the improver can choose the best base.
    let mut next_skill = resolved.body.clone();
    let mut next_base: Option<u32> = None;
    let mut history: Vec<(u32, String, f64)> = Vec::new();

    // Eval-lab scoring inputs (constant across iterations): golden task, resolved
    // test/lint commands (request → golden → config default), and weights.
    let cfg = load_skill_eval_config(ctx).await;
    let golden = golden_for_workspace(ctx, &ws.id, req.golden_task_id.as_deref()).await?;
    let weights = req.weights.clone().unwrap_or_else(|| cfg.weights.clone());
    let test_cmd = first_nonempty_opt(&[
        req.test_cmd.clone(),
        golden.as_ref().map(|g| g.test_cmd.clone()),
        Some(cfg.default_test_cmd.clone()),
    ]);
    let lint_cmd = first_nonempty_opt(&[
        req.lint_cmd.clone(),
        golden.as_ref().map(|g| g.lint_cmd.clone()),
        Some(cfg.default_lint_cmd.clone()),
    ]);
    let eval_snapshot = ctx.skill_evals_store.get_eval(eval_id).await?;

    for iter in 1..=iterations {
        if is_cancelled(cancel) {
            return Ok(());
        }
        let skill_body = next_skill.clone();
        let base_iter = next_base;
        let skill_name = sanitize_name(&format!("{}-run-{run_tag}-iter{iter}", resolved.name));

        // Seed the per-validation agent rows for this iteration.
        let mut val_runs: Vec<ValRun> = Vec::new();
        let mut seeded: Vec<EvalValidationState> = Vec::new();
        for v in &req.validations {
            let providers: Vec<String> = if v.providers.is_empty() {
                vec![req.impl_cli.clone()]
            } else {
                v.providers.clone()
            };
            let multi = providers.len() > 1;
            for p in providers {
                let display = if multi {
                    format!("{} \u{00b7} {}", v.name, p)
                } else {
                    v.name.clone()
                };
                seeded.push(EvalValidationState {
                    validation: v.name.clone(),
                    name: display,
                    provider: p.clone(),
                    model: v.model.clone(),
                    status: "pending".into(),
                    note: String::new(),
                    passed: false,
                    score: 0.0,
                    session_id: None,
                    findings: Vec::new(),
                });
                val_runs.push(ValRun {
                    validation: v.name.clone(),
                    provider: p,
                    criteria: v.criteria.clone(),
                });
            }
        }

        let iteration = ctx
            .skill_evals_store
            .add_iteration(
                eval_id,
                iter,
                base_iter,
                &skill_name,
                &skill_body,
                &req.impl_cli,
                &seeded,
            )
            .await?;
        let iter_id = iteration.id.clone();

        // Create an isolated worktree and install the skill copy into it.
        let dest = std::env::temp_dir()
            .join("otto-skilleval")
            .join(eval_id)
            .join(format!("iter{iter}"));
        let dest_str = dest.to_string_lossy().to_string();
        if let Err(e) = add_worktree(&repo_path, &base_ref, &dest).await {
            ctx.skill_evals_store
                .set_iter_status(&iter_id, "error", &format!("worktree: {e}"))
                .await?;
            return Err(e);
        }
        install_skill(&dest, &skill_name, &skill_body);
        for p in std::iter::once(req.impl_cli.as_str())
            .chain(val_runs.iter().map(|v| v.provider.as_str()))
        {
            otto_sessions::trust::ensure_trusted(p, &dest_str);
        }

        // --- 1. Implementation ------------------------------------------------
        ctx.skill_evals_store
            .set_iter_status(&iter_id, "implementing", "running implementation agent")
            .await?;
        let impl_out = output_path(eval_id, iter, "impl");
        let impl_prompt = build_impl_prompt(&skill_name, &skill_body, &req.task, &impl_out);
        let impl_outcome = run_agent_capture(
            &ctx.manager,
            ws,
            user,
            &req.impl_cli,
            &dest_str,
            &impl_prompt,
            &impl_out,
            IMPL_TIMEOUT,
            false,
            cancel,
            LiveSlot::Implementation {
                repo: &ctx.skill_evals_store,
                iter_id: &iter_id,
                worktree: dest_str.clone(),
            },
        )
        .await;
        let impl_summary = summarize_impl(&impl_outcome.text);
        ctx.skill_evals_store
            .set_iter_impl(
                &iter_id,
                impl_outcome.session_id.as_deref(),
                &impl_summary,
                Some(&dest_str),
            )
            .await?;

        // --- 2. Validations (concurrent) -------------------------------------
        ctx.skill_evals_store
            .set_iter_status(&iter_id, "validating", "running validation agents")
            .await?;

        // Pre-compute `git diff HEAD` once after the impl agent finishes so
        // every validator gets the same snapshot injected into its prompt
        // instead of each spawning a separate `git diff` call. Best-effort:
        // if the diff can't be produced the validators fall back to running
        // `git diff` themselves (via the prompt instruction).
        let precomputed_diff = validator_diff(&dest_str).await;

        let mut set = tokio::task::JoinSet::new();
        for (index, run) in val_runs.into_iter().enumerate() {
            let manager = Arc::clone(&ctx.manager);
            let repo = ctx.skill_evals_store.clone();
            let ws_c = ws.clone();
            let user_c = user.clone();
            let cwd = dest_str.clone();
            let eval_id_c = eval_id.clone();
            let iter_id_c = iter_id.clone();
            let base = seeded[index].clone();
            let cancel_c = Arc::clone(cancel);
            let diff_c = precomputed_diff.clone();
            set.spawn(async move {
                // Run the validation `passes` times and average — reduces grader
                // noise. Findings are unioned (deduped, highest severity wins).
                let mut pass_scores: Vec<f64> = Vec::new();
                let mut union: Vec<EvalFinding> = Vec::new();
                let mut last_sid: Option<String> = None;
                for pass in 0..passes {
                    if is_cancelled(&cancel_c) {
                        break;
                    }
                    let out_path = output_path(&eval_id_c, iter, &format!("val{index}-p{pass}"));
                    let prompt = build_validation_prompt(
                        &run.validation,
                        &run.criteria,
                        &out_path,
                        diff_c.as_deref(),
                    );
                    let outcome = run_agent_capture(
                        &manager,
                        &ws_c,
                        &user_c,
                        &run.provider,
                        &cwd,
                        &prompt,
                        &out_path,
                        VALIDATION_TIMEOUT,
                        true,
                        &cancel_c,
                        LiveSlot::Validation {
                            repo: &repo,
                            iter_id: &iter_id_c,
                            index,
                            base: base.clone(),
                        },
                    )
                    .await;
                    last_sid = outcome.session_id.clone().or(last_sid);
                    record_validation_pass(&outcome, &mut pass_scores, &mut union);
                }

                let final_state = finish_validation(base, last_sid, pass_scores, union, passes);
                let _ = repo
                    .set_iter_agent_at(&iter_id_c, index, &final_state)
                    .await;
                (index, final_state.score, final_state.findings)
            });
        }

        let val_names: Vec<String> = seeded.iter().map(|s| s.name.clone()).collect();
        let (scores, all_findings) = collect_validation_results(set, &val_names).await;

        let iter_score = if scores.is_empty() {
            100.0
        } else {
            scores.iter().sum::<f64>() / scores.len() as f64
        };
        ctx.skill_evals_store
            .set_iter_score(&iter_id, iter_score)
            .await?;
        history.push((iter, skill_body.clone(), iter_score));

        // --- 2b. Multi-signal scoring (tests/lint/diff/review → proof) -------
        // The impl agent left uncommitted changes in the worktree, so the diff is
        // working-tree vs HEAD (base = None).
        if let Ok(scored_iter) = ctx.skill_evals_store.get_iteration(&iter_id).await {
            if let Err(e) = crate::eval_score::score_iteration(
                ctx,
                &eval_snapshot,
                &scored_iter,
                golden.as_ref(),
                &weights,
                None,
                test_cmd.as_deref(),
                lint_cmd.as_deref(),
                cancel,
            )
            .await
            {
                tracing::warn!(eval = %eval_id, "iteration scoring failed: {e}");
            }
        }

        if is_cancelled(cancel) {
            ctx.skill_evals_store
                .set_iter_status(
                    &iter_id,
                    "done",
                    &format!("score {iter_score:.0} · cancelled"),
                )
                .await?;
            return Ok(());
        }

        // --- 3. Improve (between iterations only) ----------------------------
        let perfect = all_findings.is_empty()
            && scores.len() == seeded.len()
            && scores.iter().all(|score| *score == 100.0);
        if iter < iterations && !perfect {
            ctx.skill_evals_store
                .set_iter_status(&iter_id, "improving", "running improver agent")
                .await?;
            let impr_out = output_path(eval_id, iter, "improve");
            let prompt = build_improver_prompt(&req.task, &history, &all_findings, &impr_out);
            let outcome = run_agent_capture(
                &ctx.manager,
                ws,
                user,
                &improver.provider,
                &repo_path,
                &prompt,
                &impr_out,
                IMPROVER_TIMEOUT,
                false,
                cancel,
                LiveSlot::None,
            )
            .await;

            if let Some(imp) =
                parse_improvement(&outcome.text).filter(|i| !i.skill.trim().is_empty())
            {
                // The base the improver chose to edit (defaults to the best so far).
                let chosen = imp
                    .base_iter
                    .filter(|b| history.iter().any(|(i, _, _)| i == b))
                    .unwrap_or_else(|| best_iter(&history));
                let base_body = history
                    .iter()
                    .find(|(i, _, _)| *i == chosen)
                    .map(|(_, b, _)| b.clone())
                    .unwrap_or_else(|| skill_body.clone());
                // O(n·m) worst case — keep it off the async runtime.
                let diff = {
                    let (old, new) = (base_body.clone(), imp.skill.clone());
                    tokio::task::spawn_blocking(move || simple_diff(&old, &new))
                        .await
                        .unwrap_or_default()
                };
                ctx.skill_evals_store
                    .set_iter_improvement(&iter_id, Some(&imp.skill), &imp.summary, &diff)
                    .await?;
                next_skill = imp.skill;
                next_base = Some(chosen);
            } else {
                // Improver failed — carry the same skill forward unchanged.
                ctx.skill_evals_store
                    .set_iter_improvement(
                        &iter_id,
                        None,
                        "improver produced no usable edit; carried skill forward unchanged",
                        "",
                    )
                    .await?;
                next_skill = skill_body.clone();
                next_base = Some(iter);
            }
        }

        let note = if perfect {
            format!("score {iter_score:.0} · all validations passed")
        } else {
            format!("score {iter_score:.0}")
        };
        ctx.skill_evals_store
            .set_iter_status(&iter_id, "done", &note)
            .await?;

        // Stop early if a perfect score is reached.
        if perfect {
            break;
        }
    }

    // A rating may have changed during later agent iterations. Select from
    // persisted scores while holding the same lock as rating publication.
    let _guard = crate::eval_score::update_guard(eval_id).await;
    let current = ctx.skill_evals_store.get_eval(eval_id).await?;
    let iter_composites: Vec<_> = current
        .iterations
        .iter()
        .filter_map(|it| {
            it.scoring
                .as_ref()
                .filter(|score| score.proof_status != "pending")
                .map(|score| (it.iter, score.composite))
        })
        .collect();

    if iter_composites.is_empty()
        && current.iterations.iter().any(|it| {
            it.scoring
                .as_ref()
                .is_some_and(|score| score.proof_status == "pending")
        })
    {
        return Err(Error::Conflict(
            "rating evidence publication is pending; retry the rating".into(),
        ));
    }

    // Pick the winner by composite score (the eval-lab headline) when scoring
    // produced one, else fall back to the validator-only score.
    let (best_i, best_s) = if !iter_composites.is_empty() {
        iter_composites
            .iter()
            .cloned()
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap_or((1, 0.0))
    } else {
        history
            .iter()
            .max_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(i, _, s)| (*i, *s))
            .unwrap_or((1, 0.0))
    };
    let summary = build_summary(&resolved.name, &history, best_i, best_s);
    ctx.skill_evals_store
        .set_scored_summary(
            eval_id,
            &summary,
            best_i,
            best_s,
            (!iter_composites.is_empty()).then_some(best_s),
        )
        .await?;
    Ok(())
}

/// The first option whose trimmed value is non-empty.
pub(crate) fn first_nonempty_opt(opts: &[Option<String>]) -> Option<String> {
    opts.iter()
        .flatten()
        .find(|s| !s.trim().is_empty())
        .cloned()
}

/// Score-only run: no agent. Resolve the `target` to a directory, build a single
/// iteration around it, and run the scoring pipeline. Used to evaluate an existing
/// branch / working tree / directory against a golden task (and as the engine for
/// score-only matrix cells).
async fn run_score_only_core(
    ctx: &ServerCtx,
    eval_id: &Id,
    ws: &Workspace,
    req: &StartSkillEvalReq,
    cancel: &Arc<AtomicBool>,
) -> Result<()> {
    let eval = ctx.skill_evals_store.get_eval(eval_id).await?;
    let cfg = load_skill_eval_config(ctx).await;
    let golden = golden_for_workspace(ctx, &ws.id, req.golden_task_id.as_deref()).await?;
    let weights = req.weights.clone().unwrap_or_else(|| cfg.weights.clone());
    let test_cmd = first_nonempty_opt(&[
        req.test_cmd.clone(),
        golden.as_ref().map(|g| g.test_cmd.clone()),
        Some(cfg.default_test_cmd.clone()),
    ]);
    let lint_cmd = first_nonempty_opt(&[
        req.lint_cmd.clone(),
        golden.as_ref().map(|g| g.lint_cmd.clone()),
        Some(cfg.default_lint_cmd.clone()),
    ]);

    // Resolve the target into (worktree, diff_base).
    let target = req.target.clone().unwrap_or(EvalTarget {
        kind: "working".into(),
        git_ref: None,
        path: None,
    });
    let base_ref = req.base_ref.clone().unwrap_or_else(|| "HEAD".to_string());
    let (worktree, diff_base): (String, Option<String>) = match target.kind.as_str() {
        "path" => {
            let p = target
                .path
                .clone()
                .filter(|p| !p.trim().is_empty())
                .ok_or_else(|| Error::Invalid("target.path is required for kind=path".into()))?;
            (p, None)
        }
        "branch" => {
            let r = target
                .git_ref
                .clone()
                .filter(|r| !r.trim().is_empty())
                .ok_or_else(|| {
                    Error::Invalid("target.git_ref is required for kind=branch".into())
                })?;
            let (repo_path, _scratch) = resolve_base_repo(&ws.root_path).await?;
            let dest = std::env::temp_dir()
                .join("otto-skilleval")
                .join(eval_id)
                .join("score");
            add_worktree(&repo_path, &r, &dest).await?;
            (dest.to_string_lossy().to_string(), Some(base_ref.clone()))
        }
        // "working": score the workspace repo's live working tree in place.
        _ => (ws.root_path.clone(), None),
    };

    let it = ctx
        .skill_evals_store
        .add_iteration(eval_id, 1, None, "score-only", "", "score-only", &[])
        .await?;
    ctx.skill_evals_store
        .set_iter_status(&it.id, "validating", "scoring target")
        .await?;
    ctx.skill_evals_store
        .set_iter_impl(&it.id, None, "score-only run (no agent)", Some(&worktree))
        .await?;

    let scored_iter = ctx.skill_evals_store.get_iteration(&it.id).await?;
    crate::eval_score::score_iteration(
        ctx,
        &eval,
        &scored_iter,
        golden.as_ref(),
        &weights,
        diff_base.as_deref(),
        test_cmd.as_deref(),
        lint_cmd.as_deref(),
        cancel,
    )
    .await?;
    let _guard = crate::eval_score::update_guard(eval_id).await;
    let score = ctx
        .skill_evals_store
        .get_iteration(&it.id)
        .await?
        .scoring
        .filter(|score| score.proof_status != "pending")
        .ok_or_else(|| {
            Error::Internal("scoring did not publish a result; retry pending rating".into())
        })?;
    ctx.skill_evals_store
        .set_iter_score(&it.id, score.composite)
        .await?;
    ctx.skill_evals_store
        .set_iter_status(
            &it.id,
            "done",
            &format!(
                "composite {:.0} · proof {}",
                score.composite, score.proof_status
            ),
        )
        .await?;
    let summary = format!(
        "score-only: composite {:.0}, proof {}",
        score.composite, score.proof_status
    );
    ctx.skill_evals_store
        .set_scored_summary(eval_id, &summary, 1, score.composite, Some(score.composite))
        .await?;
    Ok(())
}

/// Iteration with the highest score so far (ties → latest).
fn best_iter(history: &[(u32, String, f64)]) -> u32 {
    history
        .iter()
        .max_by(|a, b| {
            a.2.partial_cmp(&b.2)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.0.cmp(&b.0))
        })
        .map(|(i, _, _)| *i)
        .unwrap_or(1)
}

/// Write the skill copy into the worktree so the agent can discover it.
fn install_skill(worktree: &Path, skill_name: &str, body: &str) {
    let dir = worktree.join(".claude").join("skills").join(skill_name);
    if std::fs::create_dir_all(&dir).is_ok() {
        let _ = std::fs::write(dir.join("SKILL.md"), body);
    }
}

fn summarize_impl(text: &str) -> String {
    let t = text.trim();
    if t.is_empty() {
        return "implementation completed".to_string();
    }
    // If the agent wrote JSON {summary}, use that field; else first ~280 chars.
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(t) {
        if let Some(s) = v.get("summary").and_then(|x| x.as_str()) {
            return s.chars().take(400).collect();
        }
    }
    t.chars().take(400).collect()
}

// ---------------------------------------------------------------------------
// Prompts
// ---------------------------------------------------------------------------

fn build_impl_prompt(skill_name: &str, skill_body: &str, task: &str, out: &Path) -> String {
    format!(
        "You are implementing a development task in this worktree, and you MUST follow the skill \
         below exactly — its instructions govern how you implement (logging, docs, naming, \
         structure, etc.). The skill is also installed at .claude/skills/{skill_name}.\n\n\
         ===== SKILL: {skill_name} =====\n{skill_body}\n===== END SKILL =====\n\n\
         TASK:\n{task}\n\n\
         Implement the task fully in this worktree, editing/creating files as the skill directs. \
         When you are completely done, write a short (2-4 sentence) plain-text summary of what you \
         changed to this absolute file path, overwriting any existing content:\n\n{}\n\n\
         Writing that summary file is the LAST thing you do.",
        out.to_string_lossy()
    )
}

/// Build a validation agent prompt. When `diff_context` is `Some(diff)`, the
/// pre-computed `git diff` is injected directly so each validator does not need
/// to re-run `git diff` itself (saves one shell call per validator per pass).
/// The diff is capped at 6 000 chars to keep the prompt bounded.
fn build_validation_prompt(
    validation: &str,
    criteria: &str,
    out: &Path,
    diff_context: Option<&str>,
) -> String {
    const DIFF_CAP: usize = 6_000;
    let diff_section = match diff_context {
        Some(d) if !d.trim().is_empty() => {
            let capped: String = d.chars().take(DIFF_CAP).collect();
            let truncation_note = if d.chars().count() > DIFF_CAP {
                "\n[… diff truncated …]"
            } else {
                ""
            };
            format!(
                "\nPre-computed diff (use this rather than re-running `git diff`):\n\
                 ```\n{capped}{truncation_note}\n```\n"
            )
        }
        _ => "\nInspect the implemented code (use `git diff` / `git status` and read the changed files).".to_string(),
    };
    format!(
        "VALIDATION — STRICTLY READ-ONLY. You are checking a freshly-implemented change in this \
         worktree against one quality dimension. You MUST NOT edit, create, or delete any file \
         except the findings file described below, and MUST NOT run commands that mutate the repo.\n\n\
         Dimension: {validation}\n\
         What to check (and how to judge it):\n{criteria}\n\
         {diff_section}\n\
         Report problems for THIS dimension only. For each problem, give the concrete fix you \
         would suggest.\n\n\
         Output ONLY a JSON array (no prose, no markdown fence) written to this absolute file path, \
         overwriting any existing content:\n\n{}\n\n\
         Each element: {{\"severity\":\"fail\"|\"warn\"|\"info\", \"issue\":\"what is wrong or \
         missing\", \"suggestion\":\"the concrete fix\", \"location\":\"file:line or symbol \
         (optional)\"}}. Use \"fail\" only for real violations of the dimension. Output [] if the \
         implementation fully satisfies this dimension. Writing the file is the LAST thing you do.",
        out.to_string_lossy()
    )
}

fn build_improver_prompt(
    task: &str,
    history: &[(u32, String, f64)],
    findings: &[(String, EvalFinding)],
    out: &Path,
) -> String {
    let mut iters = String::new();
    for (i, body, score) in history {
        iters.push_str(&format!(
            "----- iteration {i} (score {score:.0}) — SKILL.md -----\n{body}\n\n"
        ));
    }
    let mut found = String::new();
    for (dim, f) in findings {
        found.push_str(&format!(
            "- [{}] ({}) {} → fix: {}\n",
            dim,
            f.severity,
            f.issue,
            if f.suggestion.trim().is_empty() {
                "(none given)"
            } else {
                &f.suggestion
            }
        ));
    }
    if found.is_empty() {
        found.push_str("(no findings)\n");
    }
    format!(
        "You improve a coding-agent SKILL so that an agent following it produces better code for \
         this kind of task. An agent used the skill to implement a task; validation agents then \
         found the issues listed below. Edit the skill so a future run avoids these issues.\n\n\
         TASK the skill was used for:\n{task}\n\n\
         SKILL VERSIONS TRIED SO FAR (with their scores):\n{iters}\
         VALIDATION FINDINGS (dimension, severity, issue → suggested fix):\n{found}\n\
         {SKILL_BEST_PRACTICES}\n\n\
         Decide which prior iteration's SKILL.md is the best BASE to edit (usually the \
         highest-scoring one — do NOT build on a version that scored worse). Then produce the \
         improved skill.\n\n\
         Output ONLY a JSON object (no prose, no markdown fence) written to this absolute file \
         path, overwriting any existing content:\n\n{}\n\n\
         Shape: {{\"base_iter\": <iteration number you based the edit on>, \"skill\": \"<the \
         COMPLETE new SKILL.md content>\", \"summary\": \"<2-4 sentences on what you changed and \
         why>\"}}. Writing the file is the LAST thing you do.",
        out.to_string_lossy()
    )
}

fn build_summary(skill: &str, history: &[(u32, String, f64)], best_i: u32, best_s: f64) -> String {
    let mut s = format!(
        "Evaluated skill '{skill}' across {} iteration{}. ",
        history.len(),
        if history.len() == 1 { "" } else { "s" }
    );
    let trail: Vec<String> = history
        .iter()
        .map(|(i, _, sc)| format!("iter {i}: {sc:.0}"))
        .collect();
    s.push_str(&trail.join(", "));
    s.push_str(&format!(". Best: iteration {best_i} (score {best_s:.0})."));
    if history.len() > 1 {
        let first = history.first().map(|(_, _, sc)| *sc).unwrap_or(0.0);
        let delta = best_s - first;
        if delta > 0.5 {
            s.push_str(&format!(" Improvement of +{delta:.0} over the baseline."));
        } else if delta < -0.5 {
            s.push_str(" The baseline skill scored best; later edits did not help.");
        } else {
            s.push_str(" Edits did not materially change the score.");
        }
    }
    s
}

// ---------------------------------------------------------------------------
// Config (settings key `skill_eval`)
// ---------------------------------------------------------------------------

fn default_skill_eval_config(default_provider: &str) -> SkillEvalConfig {
    let mk = |name: &str, criteria: &str| SkillEvalValidationCfg {
        name: name.to_string(),
        criteria: criteria.to_string(),
        providers: vec![default_provider.to_string()],
        model: String::new(),
    };
    SkillEvalConfig {
        validations: vec![
            mk(
                "logging",
                "Logging follows the skill's conventions: context-aware/structured logging is \
                 used, log levels are appropriate, and no sensitive data or noisy logs leak in.",
            ),
            mk(
                "documentation",
                "Public functions/types and the feature are documented as the skill requires \
                 (doc comments, README/OpenAPI updates where applicable). Docs match the code.",
            ),
            mk(
                "properties-config",
                "Any new properties/config/system-parameters the feature needs are actually added \
                 and wired in (not just referenced), following the skill's configuration patterns.",
            ),
            mk(
                "variable-naming",
                "Variable and parameter names are clear, consistent, and follow the skill's and \
                 language's naming conventions; no cryptic or misleading names.",
            ),
            mk(
                "type-naming",
                "Class/struct/receiver/interface names follow the skill's and language's \
                 conventions and accurately describe their responsibility.",
            ),
        ],
        improver: SkillEvalImproverCfg {
            provider: default_provider.to_string(),
            model: String::new(),
        },
        iterations: 2,
        validator_passes: 1,
        weights: otto_core::domain::ScoreWeights::default(),
        promote_min_score: 80.0,
        require_proof_pass: true,
        default_test_cmd: String::new(),
        default_lint_cmd: String::new(),
    }
}

pub(crate) async fn load_skill_eval_config(ctx: &ServerCtx) -> SkillEvalConfig {
    let repo = otto_state::SettingsRepo::new(ctx.pool.clone());
    let default_provider = ctx
        .resolve_provider_or_fallback(None, None, "skill_eval.config")
        .await;
    match repo.get("skill_eval").await {
        Ok(Some(v)) => serde_json::from_value(v)
            .unwrap_or_else(|_| default_skill_eval_config(&default_provider)),
        _ => default_skill_eval_config(&default_provider),
    }
}

// ---------------------------------------------------------------------------
// HTTP handlers
// ---------------------------------------------------------------------------

async fn start_eval(
    AxPath(ws_id): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<StartSkillEvalReq>,
) -> ApiResult<Json<SkillEval>> {
    require_ws_role(&ctx, &user, &ws_id, WorkspaceRole::Editor).await?;
    let eval = launch_eval(&ctx, &ws_id, &user, req, None)
        .await
        .map_err(ApiError)?;
    Ok(Json(eval))
}

/// Create + spawn an eval run. Shared by `start_eval`, golden-task run, and matrix
/// cells. `matrix_cell` is `(matrix_id, provider, skill, prompt)` when this run is
/// one cell of a matrix. Caller is responsible for the workspace role check.
pub(crate) async fn golden_for_workspace(
    ctx: &ServerCtx,
    ws_id: &Id,
    id: Option<&str>,
) -> Result<Option<GoldenTask>> {
    let Some(id) = id else { return Ok(None) };
    let golden = ctx.golden_tasks_store.get(id).await?;
    if golden.workspace_id != *ws_id {
        return Err(Error::Invalid(
            "golden task belongs to another workspace".into(),
        ));
    }
    Ok(Some(golden))
}

/// Upper bound on queued implementation/validation/improvement steps.
pub(crate) fn eval_work_units(req: &StartSkillEvalReq) -> Result<usize> {
    let mut expanded = 0usize;
    let mut names = std::collections::HashSet::new();
    for validation in &req.validations {
        if validation.name.trim().is_empty() || !names.insert(validation.name.trim()) {
            return Err(Error::Invalid(
                "validation names must be non-empty and unique".into(),
            ));
        }
        let mut providers = std::collections::HashSet::new();
        for provider in &validation.providers {
            if provider.trim().is_empty() || !providers.insert(provider.trim()) {
                return Err(Error::Invalid(
                    "validation providers must be non-empty and unique".into(),
                ));
            }
        }
        expanded = expanded.saturating_add(validation.providers.len().max(1));
    }
    if req.iterations > 10 || expanded > 16 {
        return Err(Error::Invalid(
            "an evaluation allows at most 10 iterations and 16 expanded validators".into(),
        ));
    }
    if req.mode.trim() == "score_only" {
        return Ok(1);
    }
    Ok(req.iterations.max(1) as usize * (2 + expanded * req.validator_passes.clamp(1, 3) as usize))
}

/// Shared admission checks run before persistence or any agent launch.
pub(crate) async fn validate_eval_request(
    ctx: &ServerCtx,
    ws_id: &Id,
    req: &mut StartSkillEvalReq,
) -> Result<(String, String)> {
    req.mode = match req.mode.trim() {
        "" | "generate" => "generate".into(),
        "score_only" => "score_only".into(),
        _ => return Err(Error::Invalid("mode must be generate or score_only".into())),
    };
    eval_work_units(req)?;
    req.iterations = req.iterations.max(1);
    let mode = req.mode.as_str();
    // Resolve the run's source-skill name + task. `generate` requires an impl CLI
    // and a resolvable skill; `score_only` runs no agent, so it tolerates an empty
    // impl CLI and derives its task/name from the golden task when not given.
    let golden = golden_for_workspace(ctx, ws_id, req.golden_task_id.as_deref()).await?;
    let (source_name, task) = if mode == "score_only" {
        let task = if req.task.trim().is_empty() {
            golden
                .as_ref()
                .map(|g| g.prompt.clone())
                .unwrap_or_default()
        } else {
            req.task.trim().to_string()
        };
        let name = golden
            .as_ref()
            .map(|g| g.skill.clone())
            .filter(|s| !s.is_empty())
            .or_else(|| Some(req.source.reference.clone()).filter(|r| !r.is_empty()))
            .unwrap_or_else(|| "(score-only)".to_string());
        (name, task)
    } else {
        if req.impl_cli.trim().is_empty() {
            return Err(Error::Invalid(
                "impl_cli is required for generate mode".into(),
            ));
        }
        if req.task.trim().is_empty() {
            return Err(Error::Invalid("task is required".into()));
        }
        let resolved = resolve_skill_source_async(&ctx.context_library, &req.source).await?;
        (resolved.name, req.task.trim().to_string())
    };

    Ok((source_name, task))
}

pub(crate) async fn launch_eval(
    ctx: &ServerCtx,
    ws_id: &Id,
    user: &User,
    mut req: StartSkillEvalReq,
    matrix_cell: Option<(String, String, String, String)>,
) -> Result<SkillEval> {
    let (source_name, task) = validate_eval_request(ctx, ws_id, &mut req).await?;
    let mode = req.mode.clone();
    let ws = ctx.workspaces.get(ws_id).await?;
    let config = serde_json::to_value(&req).unwrap_or(serde_json::Value::Null);
    let dims = matrix_cell
        .as_ref()
        .map(|(_, p, s, pr)| (p.as_str(), s.as_str(), pr.as_str()));
    let eval = ctx
        .skill_evals_store
        .create_eval_ex(
            ws_id,
            &source_name,
            &task,
            req.impl_cli.trim(),
            req.iterations.max(1),
            &config,
            &mode,
            req.golden_task_id.as_deref(),
            matrix_cell.as_ref().map(|(m, _, _, _)| m.as_str()),
            dims,
        )
        .await?;

    // Sessions retain the initiating user's permissions and ownership.
    let run_user = user.clone();

    let ctx_bg = ctx.clone();
    let eval_id = eval.id.clone();
    let cancel = register_cancel(&ctx.skill_eval_cancels, &eval_id);
    tokio::spawn(async move {
        run_skill_eval(ctx_bg, eval_id, ws, run_user, req, cancel).await;
    });

    Ok(eval)
}

/// `?summary=1[&limit=&cursor=&skill=]` → one keyset page of list rows
/// (`SkillEvalSummaryPage`, single query, no iterations). Without `summary`
/// the legacy full list (every run with every iteration) is returned.
#[derive(serde::Deserialize, Default)]
struct ListEvalsQuery {
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    skill: Option<String>,
}

async fn list_evals(
    AxPath(ws_id): AxPath<Id>,
    AxQuery(q): AxQuery<ListEvalsQuery>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<axum::response::Response> {
    use axum::response::IntoResponse;
    require_ws_role(&ctx, &user, &ws_id, WorkspaceRole::Viewer).await?;
    if matches!(q.summary.as_deref(), Some("1" | "true")) {
        let page = ctx
            .skill_evals_store
            .list_summaries(
                &ws_id,
                q.skill.as_deref().filter(|s| !s.is_empty()),
                q.limit.unwrap_or(200),
                q.cursor.as_deref().filter(|s| !s.is_empty()),
            )
            .await
            .map_err(ApiError)?;
        return Ok(Json(page).into_response());
    }
    let evals = ctx
        .skill_evals_store
        .list_for_workspace(&ws_id)
        .await
        .map_err(ApiError)?;
    Ok(Json(evals).into_response())
}

async fn get_eval(
    AxPath(eval_id): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<SkillEval>> {
    let eval = ctx
        .skill_evals_store
        .get_eval(&eval_id)
        .await
        .map_err(ApiError)?;
    require_ws_role(&ctx, &user, &eval.workspace_id, WorkspaceRole::Viewer).await?;
    Ok(Json(eval))
}

async fn list_sources(
    AxPath(ws_id): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<SkillSourcesResp>> {
    require_ws_role(&ctx, &user, &ws_id, WorkspaceRole::Viewer).await?;
    let mut sources: Vec<SkillSourceInfo> = Vec::new();

    // Otto library skills.
    for s in ctx.context_library.list_skills() {
        sources.push(SkillSourceInfo {
            kind: "library".into(),
            name: s.name,
            description: s.description,
            provider: None,
        });
    }
    // Per-provider on-disk skills — a directory walk + SKILL.md reads, so it
    // runs on the blocking pool.
    sources.extend(crate::offload::blocking(provider_skill_sources).await);
    Ok(Json(SkillSourcesResp { sources }))
}

/// Per-provider on-disk skills — the same dirs the Skills Lab lists
/// (~/.claude/skills, $CODEX_HOME/skills, ~/.gemini/skills for agy).
#[allow(clippy::disallowed_methods)] // sync helper: list_sources runs it via offload::blocking
fn provider_skill_sources() -> Vec<SkillSourceInfo> {
    let mut out = Vec::new();
    for provider in otto_context::provider_skills::PROVIDERS {
        let Some(dir) = otto_context::provider_skills::provider_root(provider) else {
            continue;
        };
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            if !e.path().is_dir() {
                continue;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            if !is_safe_name(&name) {
                continue;
            }
            let skill_md = e.path().join("SKILL.md");
            if !skill_md.exists() {
                continue;
            }
            let description = std::fs::read_to_string(&skill_md)
                .ok()
                .map(|b| parse_frontmatter_description(&b))
                .unwrap_or_default();
            out.push(SkillSourceInfo {
                kind: "provider".into(),
                name,
                description,
                provider: Some(provider.to_string()),
            });
        }
    }
    out
}

/// Extract the `description:` from a skill's YAML frontmatter (best-effort).
fn parse_frontmatter_description(body: &str) -> String {
    let mut lines = body.lines();
    if lines.next().map(str::trim) != Some("---") {
        return String::new();
    }
    for line in lines {
        let t = line.trim();
        if t == "---" {
            break;
        }
        if let Some(rest) = t.strip_prefix("description:") {
            let v = rest.trim();
            let v = v
                .strip_prefix('"')
                .and_then(|x| x.strip_suffix('"'))
                .or_else(|| v.strip_prefix('\'').and_then(|x| x.strip_suffix('\'')))
                .unwrap_or(v);
            return v.chars().take(200).collect();
        }
    }
    String::new()
}

async fn get_config(
    State(ctx): State<ServerCtx>,
    CurrentUser(_user): CurrentUser,
) -> ApiResult<Json<SkillEvalConfig>> {
    Ok(Json(load_skill_eval_config(&ctx).await))
}

async fn put_config(
    State(ctx): State<ServerCtx>,
    auth: crate::auth::CurrentAuthContext,
    CurrentUser(user): CurrentUser,
    Json(body): Json<SkillEvalConfig>,
) -> ApiResult<Json<SkillEvalConfig>> {
    require_root(&user)?;
    crate::auth::require_human(&auth.0)?;
    let repo = otto_state::SettingsRepo::new(ctx.pool.clone());
    let value = serde_json::to_value(&body)
        .map_err(|e| ApiError(Error::Internal(format!("serialize: {e}"))))?;
    repo.put("skill_eval", &value).await.map_err(ApiError)?;
    Ok(Json(body))
}

// ---------------------------------------------------------------------------
// Cancel / delete / promote / retry / diff
// ---------------------------------------------------------------------------

/// Archive every live session a run spawned (kills its PTY).
async fn archive_eval_sessions(manager: &Arc<SessionManager>, eval: &SkillEval) {
    for it in &eval.iterations {
        if let Some(sid) = &it.impl_session_id {
            let _ = manager.archive(sid).await;
        }
        for a in &it.agents {
            if let Some(sid) = &a.session_id {
                let _ = manager.archive(sid).await;
            }
        }
    }
}

/// Remove every worktree a run created (best-effort), then prune.
///
/// SAFETY: only ever touches Otto-managed disposable worktrees (under the
/// `otto-skilleval` temp dir). A `score_only` run can point `worktree_path` at the
/// user's real repo (`kind=working`/`path`); those must NEVER be removed — the
/// guard below skips any path that isn't an Otto temp worktree.
async fn remove_eval_worktrees(repo_root: &str, eval: &SkillEval) {
    // Working/path score-only targets are user-owned even below our temp dir.
    if eval.mode == "score_only"
        && eval.config.pointer("/target/kind").and_then(|v| v.as_str()) != Some("branch")
    {
        return;
    }
    let managed_root = std::env::temp_dir().join("otto-skilleval");
    let mut removed_any = false;
    for it in &eval.iterations {
        if let Some(path) = &it.worktree_path {
            let expected = managed_root
                .join(&eval.id)
                .join(if eval.mode == "score_only" {
                    "score".into()
                } else {
                    format!("iter{}", it.iter)
                });
            if Path::new(path) != expected {
                continue;
            }
            // A replaced symlink must not redirect cleanup outside our tree.
            let Ok(temp_root) = tokio::fs::canonicalize(std::env::temp_dir()).await else {
                continue;
            };
            let Ok(actual) = tokio::fs::canonicalize(path).await else {
                continue;
            };
            if actual
                != temp_root
                    .join("otto-skilleval")
                    .join(&eval.id)
                    .join(expected.file_name().unwrap_or_default())
            {
                continue;
            }
            let _ = otto_git::hardened_command()
                .arg("-C")
                .arg(repo_root)
                .args(["worktree", "remove", "--force"])
                .arg(path)
                .output()
                .await;
            // Also drop the now-empty parent dir if it lingers.
            let _ = tokio::fs::remove_dir_all(path).await;
            removed_any = true;
        }
    }
    if removed_any {
        let _ = otto_git::hardened_command()
            .arg("-C")
            .arg(repo_root)
            .args(["worktree", "prune"])
            .output()
            .await;
    }
}

/// Signal-cancel a run, kill its live sessions, and mark it cancelled (idempotent;
/// the background task also finalizes). Used by `cancel_eval` and matrix cancel.
pub(crate) async fn cancel_run(ctx: &ServerCtx, eval_id: &Id) -> Result<()> {
    // Serialize with rating/retry publication and promotion. Persist first: a
    // worker must not stop while its durable row still claims it is running.
    let _guard = crate::eval_score::update_guard(eval_id).await;
    ctx.skill_evals_store
        .set_status(
            eval_id,
            SkillEvalStatus::Cancelled,
            Some("Cancelled by user"),
        )
        .await?;
    signal_cancel(&ctx.skill_eval_cancels, eval_id);
    if let Ok(eval) = ctx.skill_evals_store.get_eval(eval_id).await {
        archive_eval_sessions(&ctx.manager, &eval).await;
    }
    Ok(())
}

async fn cancel_eval(
    AxPath(eval_id): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<SkillEval>> {
    let eval = ctx
        .skill_evals_store
        .get_eval(&eval_id)
        .await
        .map_err(ApiError)?;
    require_ws_role(&ctx, &user, &eval.workspace_id, WorkspaceRole::Editor).await?;

    cancel_run(&ctx, &eval_id).await.map_err(ApiError)?;
    let eval = ctx
        .skill_evals_store
        .get_eval(&eval_id)
        .await
        .map_err(ApiError)?;
    Ok(Json(eval))
}

async fn delete_eval(
    AxPath(eval_id): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<axum::http::StatusCode> {
    let eval = ctx
        .skill_evals_store
        .get_eval(&eval_id)
        .await
        .map_err(ApiError)?;
    require_ws_role(&ctx, &user, &eval.workspace_id, WorkspaceRole::Editor).await?;

    // Stop any in-flight work, kill sessions, remove worktrees, then delete rows.
    cancel_run(&ctx, &eval_id).await.map_err(ApiError)?;
    let stopped = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let active = ctx
                .skill_eval_cancels
                .lock()
                .map(|map| {
                    map.keys()
                        .any(|key| key == &eval_id || key.starts_with(&format!("{eval_id}:retry:")))
                })
                .unwrap_or(true);
            if !active {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    if stopped.is_err() {
        return Err(ApiError(Error::Conflict(
            "evaluation is still stopping; retry deletion shortly".into(),
        )));
    }
    if let Ok(ws) = ctx.workspaces.get(&eval.workspace_id).await {
        remove_eval_worktrees(&ws.root_path, &eval).await;
    }
    ctx.skill_evals_store
        .delete(&eval_id)
        .await
        .map_err(ApiError)?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// The promote gate for one iteration: composite ≥ threshold AND (proof passes OR
/// proof not required). Reads the run's config thresholds.
async fn iteration_gate(
    ctx: &ServerCtx,
    eval: &SkillEval,
    it: &otto_core::domain::EvalIteration,
) -> PromoteGate {
    let cfg = load_skill_eval_config(ctx).await;
    let composite = (eval.status == SkillEvalStatus::Done
        && it.status == "done"
        && it.agents.iter().all(|a| a.status == "done"))
    .then(|| {
        it.scoring
            .as_ref()
            .filter(|s| s.proof_status != "pending")
            .map(|s| s.composite)
    })
    .flatten();
    // Scoring is a snapshot; promotion must use current evidence and policy.
    let proof_status = match it.proof_pack_id.as_deref() {
        Some(id) => match crate::proof::recompute_and_emit(ctx, id).await {
            Ok(pack)
                if pack.workspace_id == eval.workspace_id
                    && pack.work_item_id == it.id
                    && pack.work_item_kind == otto_core::proof::WorkItemKind::Task =>
            {
                pack.status.as_str().to_string()
            }
            _ => String::new(),
        },
        None => String::new(),
    };
    otto_core::eval_score::promote_gate(
        composite,
        &proof_status,
        cfg.promote_min_score,
        cfg.require_proof_pass,
    )
}

/// `GET /skill-evaluations/{id}/promote-gate?iteration_id=…` — whether (and why)
/// the run's best (or a named) iteration may be promoted.
async fn get_promote_gate(
    AxPath(eval_id): AxPath<Id>,
    AxQuery(q): AxQuery<std::collections::HashMap<String, String>>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<PromoteGate>> {
    let eval = ctx
        .skill_evals_store
        .get_eval(&eval_id)
        .await
        .map_err(ApiError)?;
    require_ws_role(&ctx, &user, &eval.workspace_id, WorkspaceRole::Viewer).await?;
    let it = pick_iteration(&eval, q.get("iteration_id").map(|s| s.as_str()))
        .ok_or_else(|| ApiError(Error::NotFound("iteration".into())))?;
    Ok(Json(iteration_gate(&ctx, &eval, it).await))
}

/// The named iteration, or the run's best, or the highest-composite one.
fn pick_iteration<'a>(
    eval: &'a SkillEval,
    iteration_id: Option<&str>,
) -> Option<&'a otto_core::domain::EvalIteration> {
    if let Some(id) = iteration_id {
        return eval.iterations.iter().find(|i| i.id == id);
    }
    if let Some(b) = eval.best_iteration {
        if let Some(it) = eval.iterations.iter().find(|i| i.iter == b) {
            return Some(it);
        }
    }
    eval.iterations.iter().max_by(|a, b| {
        let sa = a.scoring.as_ref().map(|s| s.composite).unwrap_or(a.score);
        let sb = b.scoring.as_ref().map(|s| s.composite).unwrap_or(b.score);
        sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
    })
}

async fn promote_skill(
    AxPath(eval_id): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    auth: crate::auth::CurrentAuthContext,
    CurrentUser(user): CurrentUser,
    Json(req): Json<PromoteSkillReq>,
) -> ApiResult<Json<LibrarySkill>> {
    require_root(&user)?; // writes to the shared Otto library (like PUT /library/skills)
    crate::auth::require_human(&auth.0)?;
    // Eligibility and the library write must use one publication snapshot.
    // A rating or retry may invalidate the score while Proof is recomputed.
    let _score_guard = crate::eval_score::update_guard(&eval_id).await;
    let eval = ctx
        .skill_evals_store
        .get_eval(&eval_id)
        .await
        .map_err(ApiError)?;
    let it = eval
        .iterations
        .iter()
        .find(|i| i.id == req.iteration_id)
        .ok_or_else(|| ApiError(Error::NotFound("iteration".into())))?;

    // Gate: promote only if the iteration's composite score + proof pack pass —
    // unless a root user explicitly forces it (which is audited + waives proof).
    let gate = iteration_gate(&ctx, &eval, it).await;
    if !gate.allowed && !req.force {
        return Err(ApiError(Error::Conflict(format!(
            "promote gate not met: {}",
            gate.reasons.join("; ")
        ))));
    }

    let body = match req.source.as_str() {
        "improved" => it.skill_after.clone().ok_or_else(|| {
            ApiError(Error::Invalid(
                "this iteration has no improved skill".into(),
            ))
        })?,
        _ => it.skill_before.clone(),
    };
    let name = req.name.trim();
    if !is_safe_name(name) {
        return Err(ApiError(Error::Invalid(
            "skill name must be letters, digits, '-' or '_'".into(),
        )));
    }
    if name.is_empty() {
        return Err(ApiError(Error::Invalid("skill name is required".into())));
    }

    // A forced promotion past an unmet gate is an audited override; it also waives
    // the iteration's proof pack so the bypass is visible there too.
    if !gate.allowed && req.force {
        if let Some(pack_id) = &it.proof_pack_id {
            let _ = ctx
                .proof_repo
                .waive(pack_id, &user.id, "force-promoted past the eval-lab gate")
                .await;
        }
        ctx.audit(otto_state::NewAuditEntry {
            user_id: Some(user.id.clone()),
            action: "skill_eval.force_promote".into(),
            target: Some(name.to_string()),
            detail: Some(serde_json::json!({
                "eval_id": eval_id,
                "iteration_id": req.iteration_id,
                "reasons": gate.reasons,
                "score": gate.score,
            })),
            ip: None,
        })
        .await;
    }

    ctx.context_library
        .put_skill(name, &body)
        .map_err(|e| ApiError(Error::Internal(format!("write skill: {e}"))))?;
    let _ = ctx.skill_evals_store.set_promoted(&eval_id, &user.id).await;
    ctx.context_library
        .get_skill(name)
        .map(Json)
        .ok_or_else(|| ApiError(Error::Internal("skill not found after promote".into())))
}

/// `GET /skill-evaluations/{id}/iterations/{iter_id}/score` — the iteration's
/// multi-signal score (computing it on demand if not yet present).
async fn get_iter_score(
    AxPath((eval_id, iter_id)): AxPath<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<EvalScore>> {
    let eval = ctx
        .skill_evals_store
        .get_eval(&eval_id)
        .await
        .map_err(ApiError)?;
    require_ws_role(&ctx, &user, &eval.workspace_id, WorkspaceRole::Viewer).await?;
    let it = eval
        .iterations
        .iter()
        .find(|i| i.id == iter_id)
        .ok_or_else(|| ApiError(Error::NotFound("iteration".into())))?;
    Ok(Json(it.scoring.clone().unwrap_or_default()))
}

/// `GET /skill-evaluations/{id}/iterations/{iter_id}/proof-pack` — the iteration's
/// assembled proof pack (header + evidence artifacts), recomputed live.
async fn get_iter_proof_pack(
    AxPath((eval_id, iter_id)): AxPath<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<serde_json::Value>> {
    let eval = ctx
        .skill_evals_store
        .get_eval(&eval_id)
        .await
        .map_err(ApiError)?;
    require_ws_role(&ctx, &user, &eval.workspace_id, WorkspaceRole::Viewer).await?;
    let it = eval
        .iterations
        .iter()
        .find(|i| i.id == iter_id)
        .ok_or_else(|| ApiError(Error::NotFound("iteration".into())))?;
    let Some(pack_id) = it.proof_pack_id.clone() else {
        return Ok(Json(serde_json::json!({ "exists": false })));
    };
    let pack = ctx.proof_repo.get_pack(&pack_id).await.map_err(ApiError)?;
    let arts = ctx
        .proof_repo
        .list_artifacts(&pack_id)
        .await
        .unwrap_or_default();
    let badges = crate::proof::badge_strings(&pack, &arts);
    let contract = crate::proof::live_contract(&ctx, &pack, &arts).await;
    let artifacts: Vec<serde_json::Value> = arts
        .iter()
        .map(|a| {
            let (preview, truncated) =
                crate::proof::preview(a.content_ref.as_deref().unwrap_or(""));
            serde_json::json!({
                "kind": a.kind.as_str(),
                "title": a.title,
                "status": a.status.as_str(),
                "preview": preview,
                "truncated": truncated,
                "metadata": a.metadata,
            })
        })
        .collect();
    Ok(Json(serde_json::json!({
        "exists": true,
        "id": pack.id,
        "status": pack.status.as_str(),
        "risk_score": pack.risk_score,
        "done_score": pack.done_score,
        "badges": badges,
        "contract": contract,
        "artifacts": artifacts,
    })))
}

/// `POST /skill-evaluations/{id}/iterations/{iter_id}/rate` — record a human
/// rating and re-derive the iteration's score (cheaply — no command re-run).
async fn rate_iteration(
    AxPath((eval_id, iter_id)): AxPath<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<RateIterationReq>,
) -> ApiResult<Json<SkillEval>> {
    let eval = ctx
        .skill_evals_store
        .get_eval(&eval_id)
        .await
        .map_err(ApiError)?;
    require_ws_role(&ctx, &user, &eval.workspace_id, WorkspaceRole::Editor).await?;
    let _score_guard = crate::eval_score::update_guard(&eval_id).await;
    let eval = ctx
        .skill_evals_store
        .get_eval(&eval_id)
        .await
        .map_err(ApiError)?;
    let it = eval
        .iterations
        .iter()
        .find(|i| i.id == iter_id)
        .ok_or_else(|| ApiError(Error::NotFound("iteration".into())))?
        .clone();
    let rating = req.rating.min(5);
    let mut pending = it.scoring.clone().unwrap_or_default();
    pending.human = otto_core::eval_score::human_score(Some(rating), &req.note, &user.id);
    pending.composite = otto_core::eval_score::compute_composite(&pending);
    pending.proof_status = "pending".into();
    pending.done_score = 0;
    ctx.skill_evals_store
        .begin_human_rating(&eval_id, &iter_id, &pending)
        .await
        .map_err(ApiError)?;
    #[cfg(test)]
    recovery_tests::after_rating_pending(&iter_id).await;
    // The pending snapshot durably preserves non-human signals for recovery.
    // Until Proof and the headline publish, promotion must reject this snapshot.
    let (score, pack_id) =
        crate::eval_score::rescore_with_human(&ctx, &eval, &it, rating, &req.note, &user.id)
            .await
            .map_err(ApiError)?;
    ctx.skill_evals_store
        .publish_iter_scoring(
            &eval_id,
            &iter_id,
            &score,
            &pack_id,
            (eval.mode == "score_only").then_some(score.composite),
            "Rating updated",
        )
        .await
        .map_err(ApiError)?;
    let eval = ctx
        .skill_evals_store
        .get_eval(&eval_id)
        .await
        .map_err(ApiError)?;
    Ok(Json(eval))
}

/// `POST /skill-evaluations/{id}/iterations/{iter_id}/regression` — capture a
/// (typically failed) iteration as a regression golden task. Deduped by source
/// iteration.
async fn iteration_regression(
    AxPath((eval_id, iter_id)): AxPath<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<RegressionReq>,
) -> ApiResult<Json<GoldenTask>> {
    let eval = ctx
        .skill_evals_store
        .get_eval(&eval_id)
        .await
        .map_err(ApiError)?;
    require_ws_role(&ctx, &user, &eval.workspace_id, WorkspaceRole::Editor).await?;
    let it = eval
        .iterations
        .iter()
        .find(|i| i.id == iter_id)
        .ok_or_else(|| ApiError(Error::NotFound("iteration".into())))?;

    // Idempotent: a second capture from the same iteration returns the first.
    if let Ok(Some(existing)) = ctx.golden_tasks_store.find_by_source_iter(&iter_id).await {
        return Ok(Json(existing));
    }

    // Recover the run's commands from its config so the regression re-runs the same.
    let cfg: StartSkillEvalReq =
        serde_json::from_value(eval.config.clone()).unwrap_or(StartSkillEvalReq {
            source: SkillSourceReq {
                kind: String::new(),
                reference: String::new(),
                provider: None,
            },
            task: eval.task.clone(),
            impl_cli: eval.impl_cli.clone(),
            validations: Vec::new(),
            iterations: 1,
            improver: None,
            base_ref: None,
            validator_passes: 1,
            mode: eval.mode.clone(),
            golden_task_id: eval.golden_task_id.clone(),
            target: None,
            test_cmd: None,
            lint_cmd: None,
            weights: None,
        });
    let golden = golden_for_workspace(&ctx, &eval.workspace_id, eval.golden_task_id.as_deref())
        .await
        .map_err(ApiError)?;
    let conf = load_skill_eval_config(&ctx).await;
    let test_cmd = first_nonempty_opt(&[
        cfg.test_cmd.clone(),
        golden.as_ref().map(|g| g.test_cmd.clone()),
        Some(conf.default_test_cmd.clone()),
    ])
    .unwrap_or_default();
    let lint_cmd = first_nonempty_opt(&[
        cfg.lint_cmd.clone(),
        golden.as_ref().map(|g| g.lint_cmd.clone()),
        Some(conf.default_lint_cmd.clone()),
    ])
    .unwrap_or_default();
    let repo_key = match it.worktree_path.as_deref() {
        Some(wt) => crate::proof::resolve_repo_for_cwd(&ctx, &eval.workspace_id, wt)
            .await
            .unwrap_or_else(|| eval.workspace_id.clone()),
        None => eval.workspace_id.clone(),
    };
    let proof_status = it
        .scoring
        .as_ref()
        .map(|s| s.proof_status.clone())
        .unwrap_or_default();
    let name = req
        .name
        .clone()
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| format!("regression: {}", eval.source_skill));
    let input = otto_state::GoldenTaskInput {
        name,
        prompt: eval.task.clone(),
        skill: eval.source_skill.clone(),
        test_cmd,
        lint_cmd,
        build_cmd: String::new(),
        rubric: format!(
            "Regression captured from eval {eval_id} iter {} (proof was '{proof_status}'). \
             The produced change must keep the test command passing.",
            it.iter
        ),
        tags: vec!["regression".to_string()],
        enabled: true,
    };
    let task = ctx
        .golden_tasks_store
        .create(
            &eval.workspace_id,
            &repo_key,
            &input,
            "regression",
            Some(&eval_id),
            Some(&iter_id),
            &user.id,
        )
        .await
        .map_err(ApiError)?;
    Ok(Json(task))
}

async fn impl_diff(
    AxPath((eval_id, iter_id)): AxPath<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<ImplDiffResp>> {
    let eval = ctx
        .skill_evals_store
        .get_eval(&eval_id)
        .await
        .map_err(ApiError)?;
    require_ws_role(&ctx, &user, &eval.workspace_id, WorkspaceRole::Viewer).await?;
    let it = eval
        .iterations
        .iter()
        .find(|i| i.id == iter_id)
        .ok_or_else(|| ApiError(Error::NotFound("iteration".into())))?;
    let Some(wt) = it.worktree_path.clone() else {
        return Ok(Json(ImplDiffResp {
            diff: String::new(),
            truncated: false,
        }));
    };
    if tokio::fs::metadata(&wt).await.is_err() {
        return Ok(Json(ImplDiffResp {
            diff: "(worktree no longer available)".into(),
            truncated: false,
        }));
    }
    let (mut diff, truncated) = otto_git::LocalGit::new(&wt)
        .working_diff_with_untracked_capped(IMPL_DIFF_CAP)
        .await
        .map_err(ApiError)?;
    if truncated {
        diff.push_str("\n… (diff truncated)");
    }
    Ok(Json(ImplDiffResp { diff, truncated }))
}

const IMPL_DIFF_CAP: usize = 200 * 1024;

/// Prompts retain 6000 characters; bound command output before allocating or
/// cloning it for the at-most-16 admitted validators.
async fn validator_diff(path: &str) -> Option<String> {
    let (mut diff, truncated) = otto_git::LocalGit::new(path)
        .diff_text_capped(Some("HEAD"), 24_004)
        .await
        .ok()?;
    if truncated {
        diff.push_str("\n… (diff truncated; inspect the repository for remaining changes)");
    }
    Some(diff)
}

async fn retry_validation(
    AxPath((eval_id, iter_id, index)): AxPath<(Id, Id, usize)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<SkillEval>> {
    let eval = ctx
        .skill_evals_store
        .get_eval(&eval_id)
        .await
        .map_err(ApiError)?;
    require_ws_role(&ctx, &user, &eval.workspace_id, WorkspaceRole::Editor).await?;
    let retry =
        RetryLease::claim(&ctx.skill_eval_cancels, &eval_id, "validation").map_err(ApiError)?;
    let score_guard = crate::eval_score::update_guard(&eval_id).await;
    let current = ctx
        .skill_evals_store
        .get_eval(&eval_id)
        .await
        .map_err(ApiError)?;
    if matches!(
        current.status,
        SkillEvalStatus::Running | SkillEvalStatus::Cancelled
    ) {
        return Err(ApiError(Error::Conflict(
            "retry requires a finished, non-cancelled evaluation".into(),
        )));
    }
    let eval = current;
    let it = eval
        .iterations
        .iter()
        .find(|i| i.id == iter_id)
        .ok_or_else(|| ApiError(Error::NotFound("iteration".into())))?;
    let agent = it
        .agents
        .get(index)
        .ok_or_else(|| ApiError(Error::NotFound("validation".into())))?
        .clone();
    let Some(worktree) = it.worktree_path.clone() else {
        return Err(ApiError(Error::Invalid(
            "worktree no longer available — re-run the evaluation".into(),
        )));
    };
    if tokio::fs::metadata(&worktree).await.is_err() {
        return Err(ApiError(Error::Invalid(
            "worktree no longer available — re-run the evaluation".into(),
        )));
    }
    // Recover this validation's criteria from the stored run config.
    let cfg: StartSkillEvalReq = serde_json::from_value(eval.config.clone())
        .map_err(|_| ApiError(Error::Invalid("run config unavailable for retry".into())))?;
    let criteria = cfg
        .validations
        .iter()
        .find(|v| v.name == agent.validation)
        .map(|v| v.criteria.clone())
        .ok_or_else(|| ApiError(Error::NotFound("validation criteria".into())))?;
    let passes = cfg.validator_passes.clamp(1, 3);

    let run_user = user.clone();
    let ws = ctx
        .workspaces
        .get(&eval.workspace_id)
        .await
        .map_err(ApiError)?;

    // Mark pending immediately so the UI shows it re-running.
    let mut pending = agent.clone();
    pending.status = "pending".into();
    pending.note = "retrying…".into();
    let previous_scoring = it.scoring.clone();

    let retry_diff = validator_diff(&worktree).await;

    if !ctx
        .skill_evals_store
        .begin_validation_retry(&eval_id, &iter_id, index, &pending)
        .await
        .map_err(ApiError)?
    {
        return Err(ApiError(Error::Conflict(
            "evaluation changed before retry admission".into(),
        )));
    }

    drop(score_guard);

    let ctx_bg = ctx.clone();
    let iter_id_bg = iter_id.clone();
    let eval_id_bg = eval_id.clone();
    let iter_num = it.iter;
    let provider = agent.provider.clone();
    let base = agent.clone();
    tokio::spawn(async move {
        let cancel = Arc::clone(&retry.flag);
        let mut pass_scores: Vec<f64> = Vec::new();
        let mut union: Vec<EvalFinding> = Vec::new();
        let mut last_sid: Option<String> = None;
        for pass in 0..passes {
            if is_cancelled(&cancel) {
                break;
            }
            let out_path = output_path(&eval_id_bg, iter_num, &format!("val{index}-retry{pass}"));
            let prompt = build_validation_prompt(
                &base.validation,
                &criteria,
                &out_path,
                retry_diff.as_deref(),
            );
            let outcome = run_agent_capture(
                &ctx_bg.manager,
                &ws,
                &run_user,
                &provider,
                &worktree,
                &prompt,
                &out_path,
                VALIDATION_TIMEOUT,
                true,
                &cancel,
                LiveSlot::Validation {
                    repo: &ctx_bg.skill_evals_store,
                    iter_id: &iter_id_bg,
                    index,
                    base: base.clone(),
                },
            )
            .await;
            last_sid = outcome.session_id.clone().or(last_sid);
            record_validation_pass(&outcome, &mut pass_scores, &mut union);
        }

        if is_cancelled(&cancel) {
            return;
        }
        let final_state = finish_validation(base, last_sid, pass_scores, union, passes);
        let _ = ctx_bg
            .skill_evals_store
            .set_iter_agent_at(&iter_id_bg, index, &final_state)
            .await;

        let rescored = crate::eval_score::publish_validation_retry(
            &ctx_bg,
            &eval_id_bg,
            &iter_id_bg,
            previous_scoring,
        );
        let result = crate::eval_score::cancellable(&cancel, rescored).await;
        let (status, error) = match result {
            Ok(()) => (SkillEvalStatus::Done, None),
            Err(e) => (SkillEvalStatus::Error, Some(e.to_string())),
        };
        let _ = ctx_bg
            .skill_evals_store
            .finish_running(&eval_id_bg, status, error.as_deref())
            .await;
        drop(retry);
    });

    let eval = ctx
        .skill_evals_store
        .get_eval(&eval_id)
        .await
        .map_err(ApiError)?;
    Ok(Json(eval))
}

/// Routes under /api/v1 for the Skills Evaluator.
pub fn routes() -> Router<ServerCtx> {
    Router::new()
        .route(
            "/workspaces/{id}/skill-evaluations",
            post(start_eval).get(list_evals),
        )
        .route("/workspaces/{id}/skill-sources", get(list_sources))
        .route("/skill-evaluations/{id}", get(get_eval).delete(delete_eval))
        .route("/skill-evaluations/{id}/cancel", post(cancel_eval))
        .route("/skill-evaluations/{id}/promote", post(promote_skill))
        .route(
            "/skill-evaluations/{id}/promote-gate",
            get(get_promote_gate),
        )
        .route(
            "/skill-evaluations/{id}/iterations/{iter_id}/diff",
            get(impl_diff),
        )
        .route(
            "/skill-evaluations/{id}/iterations/{iter_id}/score",
            get(get_iter_score),
        )
        .route(
            "/skill-evaluations/{id}/iterations/{iter_id}/proof-pack",
            get(get_iter_proof_pack),
        )
        .route(
            "/skill-evaluations/{id}/iterations/{iter_id}/rate",
            post(rate_iteration),
        )
        .route(
            "/skill-evaluations/{id}/iterations/{iter_id}/regression",
            post(iteration_regression),
        )
        .route(
            "/skill-evaluations/{id}/iterations/{iter_id}/agents/{index}/retry",
            post(retry_validation),
        )
        .route("/settings/skill-eval", get(get_config).put(put_config))
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn cleanup_preserves_score_only_user_paths_with_a_similar_temp_prefix() {
        let pool = otto_state::db::test_pool().await;
        sqlx::query("INSERT INTO users(id,username,password_hash,is_root,created_at) VALUES('editor','editor','unused',0,'2026-10-08T00:00:00Z')").execute(&pool).await.unwrap();
        let dir = tempfile::Builder::new()
            .prefix("otto-skilleval-user-repo-")
            .tempdir()
            .unwrap();
        let ctx = ServerCtx::for_tests(&pool, dir.path()).await;
        let ws = ctx
            .workspaces
            .create("A", dir.path().to_str().unwrap(), &"editor".into())
            .await
            .unwrap();
        let mut eval = ctx
            .skill_evals_store
            .create_eval(
                &ws.id,
                "skill",
                "task",
                "fixture",
                1,
                &serde_json::json!({}),
            )
            .await
            .unwrap();
        let mut iteration = ctx
            .skill_evals_store
            .add_iteration(&eval.id, 1, None, "skill", "body", "fixture", &[])
            .await
            .unwrap();
        let sentinel = dir.path().join("user-work.txt");
        std::fs::write(&sentinel, "keep this work").unwrap();
        iteration.worktree_path = Some(dir.path().to_string_lossy().into_owned());
        eval.iterations = vec![iteration];
        eval.mode = "score_only".into();
        eval.config = serde_json::json!({"target":{"kind":"path"}});
        remove_eval_worktrees(dir.path().to_str().unwrap(), &eval).await;
        assert_eq!(std::fs::read_to_string(sentinel).unwrap(), "keep this work");
    }

    #[test]
    fn retry_admission_cancellation_and_cleanup_keep_attempt_identity() {
        let registry = otto_core::cancel::new_cancel_registry();
        let first = RetryLease::claim(&registry, "eval", "iteration:0").unwrap();
        assert!(RetryLease::claim(&registry, "eval", "iteration:0").is_err());
        let other = RetryLease::claim(&registry, "other", "iteration:0").unwrap();
        signal_cancel(&registry, "eval");
        assert!(is_cancelled(&first.flag));
        assert!(!is_cancelled(&other.flag));
        // A replaced registry entry must survive an older attempt dropping.
        let replacement = Arc::new(AtomicBool::new(false));
        registry
            .lock()
            .unwrap()
            .insert(first.key.clone(), replacement.clone());
        let key = first.key.clone();
        drop(first);
        assert!(Arc::ptr_eq(
            registry.lock().unwrap().get(&key).unwrap(),
            &replacement
        ));
        drop(other);
        assert!(!registry
            .lock()
            .unwrap()
            .contains_key("other:retry:iteration:0"));
    }

    #[tokio::test]
    async fn promotion_ignores_cached_pass_when_live_proof_is_missing_or_foreign() {
        let pool = otto_state::db::test_pool().await;
        sqlx::query("INSERT INTO users(id,username,password_hash,is_root,created_at) VALUES('editor','editor','unused',0,'2026-10-08T00:00:00Z')").execute(&pool).await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        let ctx = ServerCtx::for_tests(&pool, dir.path()).await;
        let ws = ctx
            .workspaces
            .create("A", dir.path().to_str().unwrap(), &"editor".into())
            .await
            .unwrap();
        let mut eval = ctx
            .skill_evals_store
            .create_eval(
                &ws.id,
                "skill",
                "task",
                "fixture",
                1,
                &serde_json::json!({}),
            )
            .await
            .unwrap();
        let mut iteration = ctx
            .skill_evals_store
            .add_iteration(&eval.id, 1, None, "skill", "body", "fixture", &[])
            .await
            .unwrap();
        eval.status = SkillEvalStatus::Done;
        iteration.status = "done".into();
        iteration.scoring = Some(EvalScore {
            composite: 100.0,
            proof_status: "passed".into(),
            ..Default::default()
        });
        iteration.proof_pack_id = Some("missing".into());
        assert!(!iteration_gate(&ctx, &eval, &iteration).await.allowed);
        let pack = ctx
            .proof_repo
            .create_pack(
                &ws.id,
                otto_core::proof::WorkItemKind::Task,
                "different-iteration",
                "foreign",
                "editor",
                None,
            )
            .await
            .unwrap();
        iteration.proof_pack_id = Some(pack.id);
        assert!(!iteration_gate(&ctx, &eval, &iteration).await.allowed);
    }

    #[tokio::test]
    async fn golden_task_from_another_workspace_is_rejected_before_creating_eval() {
        let pool = otto_state::db::test_pool().await;
        sqlx::query("INSERT INTO users(id,username,password_hash,is_root,created_at) VALUES('editor','editor','unused',0,'2026-10-08T00:00:00Z')").execute(&pool).await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        let ctx = ServerCtx::for_tests(&pool, dir.path()).await;
        let editor = otto_state::UsersRepo::new(pool.clone())
            .get(&"editor".into())
            .await
            .unwrap();
        let a = ctx
            .workspaces
            .create("A", dir.path().to_str().unwrap(), &editor.id)
            .await
            .unwrap();
        let b = ctx
            .workspaces
            .create("B", dir.path().to_str().unwrap(), &editor.id)
            .await
            .unwrap();
        let golden = ctx
            .golden_tasks_store
            .create(
                &b.id,
                &b.id,
                &otto_state::GoldenTaskInput {
                    name: "private".into(),
                    prompt: "Other workspace prompt".into(),
                    enabled: true,
                    ..Default::default()
                },
                "manual",
                None,
                None,
                &editor.id,
            )
            .await
            .unwrap();
        let req: StartSkillEvalReq = serde_json::from_value(serde_json::json!({
            "source":{"kind":"library","reference":""}, "task":"", "impl_cli":"", "validations":[],
            "iterations":1,"mode":"score_only","golden_task_id":golden.id
        }))
        .unwrap();
        let result = start_eval(
            AxPath(a.id.clone()),
            State(ctx.clone()),
            CurrentUser(editor),
            Json(req),
        )
        .await;
        assert!(
            matches!(result, Err(ApiError(Error::Invalid(_)))),
            "cross-workspace task must fail at admission"
        );
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM skill_evals WHERE workspace_id = ?")
                .bind(&a.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(count, 0, "rejection must not leave a running evaluation");
    }

    #[test]
    fn validator_expansion_is_bounded_before_launch() {
        let mut req: StartSkillEvalReq = serde_json::from_value(serde_json::json!({
            "source":{"kind":"library","reference":"skill"}, "task":"test", "impl_cli":"codex",
            "validations":[{"name":"review", "criteria":"correctness", "providers":["codex"]}],
            "iterations":10, "validator_passes":3
        }))
        .unwrap();
        assert_eq!(eval_work_units(&req).unwrap(), 50);
        req.validations[0].providers = (0..16).map(|i| format!("provider-{i}")).collect();
        assert_eq!(eval_work_units(&req).unwrap(), 500);
        req.validations[0].providers.push("provider-16".into());
        assert!(eval_work_units(&req).is_err());
        req.validations[0].providers = vec!["codex".into(); 1000];
        assert!(eval_work_units(&req).is_err());
        req.validations[0].providers = vec![" ".into()];
        assert!(eval_work_units(&req).is_err());
    }

    use super::*;

    #[test]
    fn sanitize_makes_safe_names() {
        assert_eq!(
            sanitize_name("golang-feature-implementation"),
            "golang-feature-implementation"
        );
        assert_eq!(sanitize_name("a/b c.d"), "a-b-c-d");
        assert_eq!(sanitize_name("///"), "skill");
    }

    #[test]
    fn score_subtracts_by_severity() {
        let none: Vec<EvalFinding> = vec![];
        assert_eq!(score_findings(&none), (true, 100.0));
        let f = vec![
            EvalFinding {
                severity: "fail".into(),
                issue: "x".into(),
                suggestion: "y".into(),
                location: None,
            },
            EvalFinding {
                severity: "warn".into(),
                issue: "x".into(),
                suggestion: "y".into(),
                location: None,
            },
        ];
        let (passed, score) = score_findings(&f);
        assert!(!passed);
        assert_eq!(score, 67.0);
    }

    #[test]
    fn invalid_validator_output_is_distinct_from_a_clean_verdict() {
        assert!(parse_findings("[]").unwrap().is_empty());
        for text in [
            "",
            "not json",
            "[",
            "[{\"issue\":\"unfinished",
            "[{}]",
            "[null]",
            "{\"error\":\"failed\",\"details\":[]}",
        ] {
            assert!(
                parse_findings(text).is_none(),
                "invalid output became a clean verdict: {text:?}"
            );
        }
    }

    #[test]
    fn parse_findings_tolerant() {
        let raw = "ok:\n```json\n[{\"severity\":\"warn\",\"issue\":\"no ctx\",\"suggestion\":\"add ctx\",\"location\":\"a.go:3\"}]\n```";
        let f = parse_findings(raw).unwrap();
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].issue, "no ctx");
        assert_eq!(f[0].suggestion, "add ctx");
        // Aliases: body/message → issue, fix → suggestion.
        let f2 = parse_findings("[{\"body\":\"b\",\"fix\":\"do x\"}]").unwrap();
        assert_eq!(f2[0].issue, "b");
        assert_eq!(f2[0].suggestion, "do x");
        assert!(parse_findings("not json").is_none());
    }

    #[test]
    fn parse_improvement_object() {
        let raw = "here:\n{\"base_iter\":1,\"skill\":\"---\\nname: x\\n---\\nbody\",\"summary\":\"tightened\"}";
        let imp = parse_improvement(raw).unwrap();
        assert_eq!(imp.base_iter, Some(1));
        assert!(imp.skill.contains("name: x"));
        assert_eq!(imp.summary, "tightened");
    }

    #[test]
    fn diff_marks_changes() {
        // simple_diff now emits GNU unified-diff format: `-b` (removed), `+B`
        // (added), ` a` (single-space context), with `@@` hunk headers.
        let d = simple_diff("a\nb\nc\n", "a\nB\nc\n");
        assert!(d.contains("-b"), "removed line marked: {d}");
        assert!(d.contains("+B"), "added line marked: {d}");
        assert!(d.contains(" a"), "context line preserved: {d}");
        assert!(d.contains("@@"), "unified-diff hunk header present: {d}");
    }

    #[test]
    fn diff_trims_common_ends_and_caps_the_lcs() {
        // Prefix/suffix trimming must keep the line numbers absolute.
        let old: String = (0..50).map(|i| format!("l{i}\n")).collect();
        let new = old.replace("l25\n", "L25\n");
        let d = simple_diff(&old, &new);
        assert!(d.contains("-l25") && d.contains("+L25"), "{d}");
        assert!(d.contains("@@ -23,7 +23,7 @@"), "{d}");
        // Past the cell cap the middle degrades to delete + insert, no blow-up.
        let big_a: String = (0..3000).map(|i| format!("a{i}\n")).collect();
        let big_b: String = (0..3000).map(|i| format!("b{i}\n")).collect();
        let d = simple_diff(&big_a, &big_b);
        assert_eq!(d.lines().filter(|l| l.starts_with("-a")).count(), 3000);
        assert_eq!(d.lines().filter(|l| l.starts_with("+b")).count(), 3000);
        assert_eq!(simple_diff("same\n", "same\n"), "");
    }

    #[test]
    fn best_iter_prefers_high_score_then_latest() {
        let h = vec![
            (1, "x".into(), 80.0),
            (2, "y".into(), 80.0),
            (3, "z".into(), 70.0),
        ];
        assert_eq!(best_iter(&h), 2);
    }
}

#[cfg(test)]
#[path = "skill_eval_output_tests.rs"]
mod output_tests;

#[cfg(test)]
#[path = "skill_eval_recovery_tests.rs"]
mod recovery_tests;
