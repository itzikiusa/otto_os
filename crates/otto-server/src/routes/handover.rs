//! Agent handover: pass one agent's working context into another agent in the
//! same workspace — either a freshly spawned CLI or an already-running one — so
//! work a previous agent started can continue elsewhere.
//!
//! Endpoints:
//!  - `POST /api/v1/sessions/{id}/handover`       — deliver the handover
//!  - `POST /api/v1/sessions/{id}/handover/brief` — generate the brief for review
//!
//! Brief sources, best-effort and in order: the source agent's claude JSONL
//! transcript (or live PTY scrollback for other CLIs), plus the repo's git state
//! (branch / changed files / recent commits). That digest, weighted toward the
//! user's focus note, is summarized by the headless-claude planner. On any
//! summarizer failure it degrades to the raw digest rather than dropping context.
//!
//! Progress is visible live: the target carries `meta.handover_pending` while the
//! brief is prepared (cleared via `SessionMetaUpdated` when done), and a `Notice`
//! toast fires on completion/failure.

use std::path::PathBuf;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::Json;
use otto_core::api::{
    CreateSessionReq, HandoverBriefReq, HandoverBriefResp, HandoverReq, HandoverTarget,
};
use otto_core::domain::{Session, SessionKind, WorkspaceRole};
use otto_core::event::Event;
use otto_core::{Error, Id};
use otto_git::LocalGit;
use otto_sessions::SessionManager;

use crate::auth::CurrentUser;
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

/// Upper bound on the raw context digest we feed the summarizer (chars). We keep
/// the *tail* (most recent work) when trimming.
const CONTEXT_CAP: usize = 24_000;
/// Total wait for the summarizer turn. A cold claude spawn alone can take
/// ~25-30s before it starts answering, so this is deliberately generous.
const SUMMARY_TIMEOUT: Duration = Duration::from_secs(120);

// ---------------------------------------------------------------------------
// Endpoint: deliver a handover
// ---------------------------------------------------------------------------

/// `POST /sessions/{id}/handover`
pub async fn handover_session(
    Path(source_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<HandoverReq>,
) -> ApiResult<Json<Session>> {
    let source = ctx.manager.get(&source_id).await.map_err(ApiError)?;
    require_agent(&source)?;
    crate::auth::require_ws_role(&ctx, &user, &source.workspace_id, WorkspaceRole::Editor).await?;

    // Resolve the target: spawn a new agent, or claim an existing one. Both end
    // up carrying `handover_from` (breadcrumb) and `handover_pending` (badge).
    let target = match &req.target {
        HandoverTarget::NewAgent { provider } => {
            let provider = provider.trim();
            if provider.is_empty() {
                return Err(ApiError(Error::Invalid("provider must not be empty".into())));
            }
            let workspace = ctx
                .workspaces
                .get(&source.workspace_id)
                .await
                .map_err(ApiError)?;
            let title = req
                .title
                .as_ref()
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| format!("Handover from {}", source.provider));
            let create_req = CreateSessionReq {
                kind: SessionKind::Agent,
                provider: Some(provider.to_string()),
                title: Some(title),
                cwd: Some(source.cwd.clone()),
                connection_id: None,
                meta: Some(serde_json::json!({
                    "handover_from": source.id,
                    "handover_pending": true,
                })),
            };
            ctx.manager
                .create(&workspace, &user.id, create_req, None)
                .await
                .map_err(ApiError)?
        }
        HandoverTarget::ExistingSession { session_id } => {
            let existing = ctx.manager.get(session_id).await.map_err(ApiError)?;
            require_agent(&existing)?;
            if existing.id == source.id {
                return Err(ApiError(Error::Invalid(
                    "cannot hand a session over to itself".into(),
                )));
            }
            if existing.workspace_id != source.workspace_id {
                return Err(ApiError(Error::Invalid(
                    "target session is in a different workspace".into(),
                )));
            }
            // Records the breadcrumb + pending badge and broadcasts the change.
            ctx.manager
                .update_meta(
                    session_id,
                    serde_json::json!({
                        "handover_from": source.id,
                        "handover_pending": true,
                    }),
                )
                .await
                .map_err(ApiError)?
        }
    };

    let params = HandoverParams {
        source_id: source.id.clone(),
        focus: req.focus.unwrap_or_default(),
        brief: req.brief.filter(|b| !b.trim().is_empty()),
        include_git: req.include_git.unwrap_or(true),
        fast: req.fast.unwrap_or(false),
        archive_source: req.archive_source.unwrap_or(false),
    };
    spawn_handover_worker(ctx.clone(), source, target.clone(), params);

    Ok(Json(target))
}

// ---------------------------------------------------------------------------
// Endpoint: generate the brief for review
// ---------------------------------------------------------------------------

/// `POST /sessions/{id}/handover/brief` — synchronous; blocks while summarizing.
pub async fn handover_brief(
    Path(source_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<HandoverBriefReq>,
) -> ApiResult<Json<HandoverBriefResp>> {
    let source = ctx.manager.get(&source_id).await.map_err(ApiError)?;
    require_agent(&source)?;
    crate::auth::require_ws_role(&ctx, &user, &source.workspace_id, WorkspaceRole::Editor).await?;

    let (brief, fallback, had_context) = generate_brief(
        &ctx,
        &source,
        &req.focus.unwrap_or_default(),
        req.include_git.unwrap_or(true),
        req.fast.unwrap_or(false),
    )
    .await;

    Ok(Json(HandoverBriefResp {
        brief,
        fallback,
        had_context,
    }))
}

fn require_agent(session: &Session) -> Result<(), ApiError> {
    if session.kind != SessionKind::Agent {
        return Err(ApiError(Error::Invalid(
            "handover is only available for agent sessions".into(),
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Background worker: build (or accept) the brief, inject it, signal completion
// ---------------------------------------------------------------------------

struct HandoverParams {
    source_id: Id,
    focus: String,
    /// A pre-reviewed brief; when present, summarization is skipped.
    brief: Option<String>,
    include_git: bool,
    fast: bool,
    archive_source: bool,
}

fn spawn_handover_worker(ctx: ServerCtx, source: Session, target: Session, params: HandoverParams) {
    tokio::spawn(async move {
        let brief = match &params.brief {
            Some(b) => b.trim().to_string(),
            None => {
                let (brief, _fallback, _had) = generate_brief(
                    &ctx,
                    &source,
                    &params.focus,
                    params.include_git,
                    params.fast,
                )
                .await;
                brief
            }
        };
        let prompt = compose_handover_prompt(&source.provider, &brief, &params.focus);

        wait_for_ready(&ctx.manager, &target.id).await;
        let delivered = inject_handover_prompt(&ctx.manager, &target.id, &prompt).await;

        // Clear the pending badge (broadcasts SessionMetaUpdated → live UI).
        let _ = ctx
            .manager
            .update_meta(&target.id, serde_json::json!({ "handover_pending": null }))
            .await;

        if delivered {
            let _ = ctx.events.send(Event::Notice {
                level: "info".to_string(),
                title: "Handover delivered".to_string(),
                body: format!("Context handed to {}.", target.title),
            });
            if params.archive_source {
                if let Err(e) = ctx.manager.archive(&params.source_id).await {
                    tracing::warn!(session = %params.source_id, "handover: archive source failed: {e}");
                }
            }
        } else {
            let _ = ctx.events.send(Event::Notice {
                level: "error".to_string(),
                title: "Handover failed".to_string(),
                body: format!("Could not deliver the brief to {}.", target.title),
            });
        }
    });
}

// ---------------------------------------------------------------------------
// Brief generation (shared by both endpoints)
// ---------------------------------------------------------------------------

/// Gather context (transcript/scrollback + optional git) and summarize it.
/// Returns `(brief, fallback, had_context)`. `fallback` is true when the
/// summarizer was unavailable and the brief is raw context; `had_context` is
/// false when nothing at all could be recovered.
async fn generate_brief(
    ctx: &ServerCtx,
    source: &Session,
    focus: &str,
    include_git: bool,
    fast: bool,
) -> (String, bool, bool) {
    let mut parts: Vec<String> = Vec::new();
    if let Some(work) = gather_source_context(ctx, source).await {
        parts.push(work);
    }
    if include_git {
        if let Some(git) = git_digest(ctx, &source.workspace_id, &source.cwd).await {
            parts.push(format!("=== GIT STATE ===\n{git}"));
        }
    }
    if parts.is_empty() {
        return (String::new(), false, false);
    }
    let context = parts.join("\n\n");

    let model = if fast { Some("haiku") } else { None };
    match ctx
        .orchestrator
        .run_agent(&summary_prompt(&context, focus), &source.cwd, model, SUMMARY_TIMEOUT)
        .await
    {
        Ok(text) if !text.trim().is_empty() => (text.trim().to_string(), false, true),
        Ok(_) => (fallback_brief(&context), true, true),
        Err(e) => {
            tracing::warn!("handover: summary failed, using raw digest: {e}");
            (fallback_brief(&context), true, true)
        }
    }
}

/// The instruction we give the summarizer.
fn summary_prompt(context: &str, focus: &str) -> String {
    let mut prompt = String::from(
        "You are writing a concise handover brief so a DIFFERENT coding agent can take over an \
         in-progress task with full context. Below is a digest of the previous agent's work in \
         this workspace, plus the repo's git state.\n\n\
         Write a tight, skimmable brief using short markdown sections:\n\
         - **Goal**: what we are ultimately trying to achieve\n\
         - **Done**: what has already been completed or decided\n\
         - **Next**: what remains, with the immediate next step first\n\
         - **Key files & commands**: paths, functions, and commands that matter\n\
         - **Gotchas**: pitfalls, constraints, and approaches that did not work\n\n\
         Be specific (name real files/identifiers). Output ONLY the brief — no preamble, no \
         sign-off.\n",
    );
    if !focus.trim().is_empty() {
        prompt.push_str(&format!(
            "\nThe user said the next agent should prioritize: {}\nWeight the brief toward that.\n",
            focus.trim()
        ));
    }
    prompt.push_str("\n=== PREVIOUS AGENT WORK (digest) ===\n");
    prompt.push_str(context);
    prompt
}

/// When summarization is unavailable, hand over the raw recent context instead
/// of nothing — labeled so the receiving agent knows it is unsummarized.
fn fallback_brief(context: &str) -> String {
    format!(
        "(Automatic summary was unavailable — raw recent context from the previous agent \
         follows.)\n\n{}",
        tail_cap(context, 8_000)
    )
}

// ---------------------------------------------------------------------------
// Context gathering: agent transcript / scrollback
// ---------------------------------------------------------------------------

/// A readable digest of what the source agent has been doing, or `None` when we
/// can't recover anything. Prefers the claude transcript (richer, survives an
/// exited session); falls back to the live terminal scrollback for other CLIs.
async fn gather_source_context(ctx: &ServerCtx, source: &Session) -> Option<String> {
    if source.provider == "claude" {
        if let Some(sid) = source.provider_session_id.as_deref() {
            if let Some(path) = find_claude_transcript(&source.cwd, sid) {
                if let Ok(content) = tokio::fs::read_to_string(&path).await {
                    let digest = transcript_digest(&content, CONTEXT_CAP);
                    if !digest.trim().is_empty() {
                        return Some(digest);
                    }
                }
            }
        }
    }
    live_scrollback_digest(&ctx.manager, &source.id)
}

/// Locate a claude session transcript: `~/.claude/projects/<enc(cwd)>/<sid>.jsonl`.
/// The exact encoding (every non-alphanumeric → `-`) is tried first; if that
/// misses we scan every project dir for `<sid>.jsonl`, which makes a positive
/// hit reliable even when the stored cwd doesn't encode 1:1 to the dir name.
fn find_claude_transcript(cwd: &str, sid: &str) -> Option<PathBuf> {
    let home = std::env::var("HOME")
        .ok()
        .or_else(|| std::env::var("USERPROFILE").ok())?;
    let projects = PathBuf::from(home).join(".claude").join("projects");

    let enc: String = cwd
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let exact = projects.join(&enc).join(format!("{sid}.jsonl"));
    if exact.is_file() {
        return Some(exact);
    }

    let target = format!("{sid}.jsonl");
    let entries = std::fs::read_dir(&projects).ok()?;
    for entry in entries.flatten() {
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            let candidate = entry.path().join(&target);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Condense a claude JSONL transcript into `USER:/ASSISTANT:` lines with a brief
/// note of which tools each assistant turn used. Keeps the last `cap` chars.
fn transcript_digest(jsonl: &str, cap: usize) -> String {
    let mut out = String::new();
    for line in jsonl.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue; // metadata / partially-written lines
        };
        let Some(msg) = v.get("message") else { continue };
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
        if role != "user" && role != "assistant" {
            continue;
        }

        let mut text = String::new();
        let mut tools: Vec<String> = Vec::new();
        match msg.get("content") {
            Some(serde_json::Value::String(s)) => text.push_str(s),
            Some(serde_json::Value::Array(blocks)) => {
                for block in blocks {
                    match block.get("type").and_then(|t| t.as_str()) {
                        Some("text") => {
                            if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                                if !t.is_empty() {
                                    if !text.is_empty() {
                                        text.push('\n');
                                    }
                                    text.push_str(t);
                                }
                            }
                        }
                        Some("tool_use") => {
                            if let Some(n) = block.get("name").and_then(|t| t.as_str()) {
                                tools.push(n.to_string());
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }

        let text = text.trim();
        if !text.is_empty() {
            out.push_str(if role == "user" { "USER: " } else { "ASSISTANT: " });
            out.push_str(text);
            out.push('\n');
        }
        if !tools.is_empty() {
            out.push_str("  [tools: ");
            out.push_str(&tools.join(", "));
            out.push_str("]\n");
        }
    }
    tail_cap(&out, cap)
}

/// Best-effort plain text of the source agent's live terminal scrollback.
fn live_scrollback_digest(manager: &SessionManager, id: &Id) -> Option<String> {
    let handle = manager.live_handle(id)?;
    let bytes = handle.scrollback(600);
    if bytes.is_empty() {
        return None;
    }
    let text = strip_ansi(&String::from_utf8_lossy(&bytes));
    let text = text.trim();
    if text.is_empty() {
        None
    } else {
        Some(tail_cap(text, CONTEXT_CAP))
    }
}

// ---------------------------------------------------------------------------
// Context gathering: git state
// ---------------------------------------------------------------------------

/// Compact git orientation for the repo containing `cwd`: current branch,
/// changed files, and recent commit subjects. `None` when `cwd` isn't inside any
/// registered repo, or git can't be read. The summarizer runs in `cwd` so it can
/// dig into actual diffs itself — this is just the high-signal orientation.
async fn git_digest(ctx: &ServerCtx, workspace_id: &Id, cwd: &str) -> Option<String> {
    let repos = ctx.git_store.list_repos(workspace_id).await.ok()?;
    // Longest path prefix wins when nested repos share a workspace.
    let repo = repos
        .into_iter()
        .filter(|r| cwd.starts_with(&r.path))
        .max_by_key(|r| r.path.len())?;

    let git = LocalGit::new(&repo.path);
    let status = git.status().await.ok()?;
    let commits = git.log(8, 0, false).await.unwrap_or_default();

    let mut out = format!("Branch: {}", status.branch);
    if status.ahead > 0 || status.behind > 0 {
        out.push_str(&format!(" (ahead {}, behind {})", status.ahead, status.behind));
    }
    out.push('\n');

    if status.changes.is_empty() {
        out.push_str("Working tree clean.\n");
    } else {
        out.push_str(&format!("Changed files ({}):\n", status.changes.len()));
        for c in status.changes.iter().take(40) {
            out.push_str(&format!("  {} {}\n", c.kind, c.path));
        }
        if status.changes.len() > 40 {
            out.push_str(&format!("  …and {} more\n", status.changes.len() - 40));
        }
    }

    if !commits.is_empty() {
        out.push_str("Recent commits:\n");
        for c in commits.iter().take(8) {
            out.push_str(&format!("  {} {}\n", c.short_sha, c.subject));
        }
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// Prompt assembly + injection
// ---------------------------------------------------------------------------

/// The first message typed into the receiving agent.
fn compose_handover_prompt(source_provider: &str, brief: &str, focus: &str) -> String {
    let mut p = format!(
        "You are taking over an in-progress task from another agent ({source_provider}) working \
         in this same workspace. Read the handover brief below, then continue the work.\n\n"
    );
    if !brief.trim().is_empty() {
        p.push_str("=== HANDOVER BRIEF ===\n");
        p.push_str(brief.trim());
        p.push_str("\n\n");
    }
    if !focus.trim().is_empty() {
        p.push_str("=== WHAT TO FOCUS ON (from the user — prioritize this) ===\n");
        p.push_str(focus.trim());
        p.push_str("\n\n");
    }
    p.push_str(
        "Begin by briefly confirming your understanding of the current state and your plan, then \
         proceed.",
    );
    p
}

/// Wait for a freshly spawned agent's TUI to draw and go quiet before typing.
/// Mirrors the planner's settle logic; bails after a cap so we still try to
/// inject even if the CLI never produces the expected idle window.
async fn wait_for_ready(manager: &SessionManager, id: &Id) {
    const SETTLE: Duration = Duration::from_millis(500);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(handle) = manager.live_handle(id) {
            if !handle.scrollback(1).is_empty() && handle.last_output_at().elapsed() >= SETTLE {
                return;
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// Type the brief into the receiving agent as a single message. Bracketed paste
/// (`ESC[200~ … ESC[201~`) keeps the multi-line brief from submitting on its
/// first newline; a trailing `\r` then sends it. Returns whether it landed.
async fn inject_handover_prompt(manager: &SessionManager, id: &Id, prompt: &str) -> bool {
    let paste = format!("\x1b[200~{prompt}\x1b[201~");
    if let Err(e) = manager.input(id, paste.as_bytes()).await {
        tracing::warn!(session = %id, "handover: paste failed: {e}");
        return false;
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    if let Err(e) = manager.input(id, b"\r").await {
        tracing::warn!(session = %id, "handover: submit failed: {e}");
        return false;
    }
    true
}

// ---------------------------------------------------------------------------
// Text helpers
// ---------------------------------------------------------------------------

/// Keep the last `cap` chars of `s`, prefixing a marker when truncated. Splits
/// on a char boundary so multi-byte text is never cut mid-codepoint.
fn tail_cap(s: &str, cap: usize) -> String {
    if s.len() <= cap {
        return s.to_string();
    }
    let mut start = s.len() - cap;
    while start < s.len() && !s.is_char_boundary(start) {
        start += 1;
    }
    format!("…(earlier context trimmed)\n{}", &s[start..])
}

/// Strip ANSI/VT control sequences from terminal scrollback, leaving readable
/// text. Handles CSI (`ESC[…`), OSC (`ESC]…` ended by BEL or ST) and lone
/// two-char escapes; passes printable text, tabs, and newlines through.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\x1b' => match chars.next() {
                Some('[') => {
                    // CSI: params/intermediates until a final byte 0x40-0x7E.
                    for f in chars.by_ref() {
                        if ('\x40'..='\x7e').contains(&f) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    // OSC: until BEL, or ST (ESC \).
                    while let Some(f) = chars.next() {
                        if f == '\x07' {
                            break;
                        }
                        if f == '\x1b' {
                            if matches!(chars.peek(), Some('\\')) {
                                chars.next();
                            }
                            break;
                        }
                    }
                }
                _ => {} // lone/other escape: drop the escape + next char
            },
            '\r' | '\x07' => {}
            '\n' | '\t' => out.push(c),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_cap_keeps_recent_and_marks_truncation() {
        assert_eq!(tail_cap("short", 100), "short");
        let capped = tail_cap("0123456789", 4);
        assert!(capped.ends_with("6789"));
        assert!(capped.starts_with("…(earlier context trimmed)"));
    }

    #[test]
    fn tail_cap_respects_char_boundaries() {
        // 5 × 2-byte chars; cap mid-char must not panic.
        let _ = tail_cap("ααααα", 3);
    }

    #[test]
    fn strip_ansi_removes_color_and_cursor_moves() {
        let raw = "\x1b[1;32mhello\x1b[0m\r\n\x1b[2Kworld\x07";
        assert_eq!(strip_ansi(raw), "hello\nworld");
    }

    #[test]
    fn strip_ansi_removes_osc_titles() {
        let raw = "\x1b]0;window title\x07kept";
        assert_eq!(strip_ansi(raw), "kept");
    }

    #[test]
    fn transcript_digest_extracts_roles_and_tools() {
        let jsonl = concat!(
            r#"{"type":"summary","summary":"meta"}"#,
            "\n",
            r#"{"message":{"role":"user","content":[{"type":"text","text":"add a feature"}]}}"#,
            "\n",
            r#"{"message":{"role":"assistant","content":[{"type":"text","text":"on it"},{"type":"tool_use","name":"Edit"}]}}"#,
            "\n",
        );
        let d = transcript_digest(jsonl, 10_000);
        assert!(d.contains("USER: add a feature"), "got: {d}");
        assert!(d.contains("ASSISTANT: on it"), "got: {d}");
        assert!(d.contains("[tools: Edit]"), "got: {d}");
    }

    #[test]
    fn transcript_digest_handles_string_content() {
        let jsonl = r#"{"message":{"role":"user","content":"plain string"}}"#;
        assert!(transcript_digest(jsonl, 1000).contains("USER: plain string"));
    }

    #[test]
    fn compose_prompt_includes_brief_and_focus() {
        let p = compose_handover_prompt("claude", "the brief", "ship the API");
        assert!(p.contains("another agent (claude)"));
        assert!(p.contains("=== HANDOVER BRIEF ===\nthe brief"));
        assert!(p.contains("ship the API"));
    }

    #[test]
    fn compose_prompt_omits_empty_sections() {
        let p = compose_handover_prompt("codex", "", "   ");
        assert!(!p.contains("HANDOVER BRIEF"));
        assert!(!p.contains("WHAT TO FOCUS ON"));
        assert!(p.contains("taking over an in-progress task"));
    }

    #[test]
    fn summary_prompt_weaves_in_focus() {
        let p = summary_prompt("some context", "the auth layer");
        assert!(p.contains("prioritize: the auth layer"));
        assert!(p.contains("some context"));
    }

    #[test]
    fn summary_prompt_without_focus_has_no_priority_line() {
        let p = summary_prompt("ctx", "  ");
        assert!(!p.contains("prioritize:"));
    }
}
