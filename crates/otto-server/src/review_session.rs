//! Run a PR-review agent as a real, openable [`SessionManager`] session so the
//! user can watch it live and type into it to unblock it.
//!
//! Each agent is spawned as a normal agent session (tagged `meta.source =
//! "review"`), the review prompt is injected into its PTY (like the channel
//! bridge does), and it is told to write its findings to a temp file we then
//! read. Provider-agnostic: codex/agy write no transcript, so the file is the
//! reliable capture path; claude's JSONL transcript is a fallback.
//!
//! Resilience: each agent is independent — one that never starts, errors, or
//! gets stuck does NOT abort the others. While it runs we persist its live
//! state (running → waiting → done/error) so the UI's poll surfaces progress;
//! "waiting" means it looks blocked on input and the user should Open it.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use otto_core::api::CreateSessionReq;
use otto_core::domain::{ReviewAgentState, ReviewFinding, SessionKind, User, Workspace};
use otto_sessions::SessionManager;
use otto_state::ReviewsRepo;
use tokio::sync::Mutex;

// Generous: several CLIs cold-start concurrently for one review, so claude can
// take >30s to draw its TUI; injecting before it's ready loses the prompt.
const TUI_STARTUP_WAIT: Duration = Duration::from_secs(40);
const TUI_POLL: Duration = Duration::from_millis(250);
const TUI_SETTLE: Duration = Duration::from_millis(600);
pub const PASTE_TO_ENTER: Duration = Duration::from_millis(250);
// After submitting, confirm the agent actually started (output advanced); if
// not, re-send Enter once — a freshly-spawned CLI under load can drop the first.
const DISPATCH_WAIT: Duration = Duration::from_secs(6);
const DISPATCH_POLL: Duration = Duration::from_millis(250);
pub const FINDINGS_POLL: Duration = Duration::from_millis(1000);
/// After this much silence with no findings yet, assume the agent may be
/// blocked on a prompt the guard couldn't auto-accept and flag it "waiting".
pub const WAITING_IDLE: Duration = Duration::from_secs(45);

/// Absolute temp path an agent writes its findings JSON to (unique per run).
pub fn findings_path(review_id: &str, agent_index: usize) -> PathBuf {
    let dir = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(dir).join(format!("otto-review-{review_id}-{agent_index}.json"))
}

/// Absolute temp path the (already-built) prompt for one agent is saved to, so
/// a per-agent Retry can re-run exactly that agent without rebuilding it.
pub fn prompt_path(review_id: &str, agent_index: usize) -> PathBuf {
    let dir = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(dir).join(format!("otto-review-{review_id}-{agent_index}.prompt"))
}

/// Append the "write findings to this file" instruction to a reviewer prompt.
pub fn augment_prompt(base_prompt: &str, findings_path: &str) -> String {
    format!(
        "{base_prompt}\n\n---\nWhen you have finished reviewing, write your findings as a JSON \
         array (the exact schema described above) to this absolute file path, overwriting any \
         existing content:\n\n{findings_path}\n\nWrite ONLY the JSON array to that file (no prose, \
         no markdown fence). Writing the file is the last thing you do."
    )
}

#[derive(serde::Deserialize)]
struct RawFinding {
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    line: Option<u32>,
    #[serde(default = "default_severity")]
    severity: String,
    #[serde(default)]
    body: String,
}

fn default_severity() -> String {
    "info".to_string()
}

/// Extract the JSON array of findings from arbitrary agent output (tolerates
/// ```` ```json ```` fences + surrounding prose). Returns `[]` on any failure.
pub fn parse_findings(text: &str) -> Vec<ReviewFinding> {
    let stripped = text
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    let start = stripped.find('[').unwrap_or(0);
    let end = stripped.rfind(']').map(|i| i + 1).unwrap_or(stripped.len());
    if start >= end {
        return Vec::new();
    }
    serde_json::from_str::<Vec<RawFinding>>(&stripped[start..end])
        .map(|raw| {
            raw.into_iter()
                .map(|r| ReviewFinding {
                    path: r.path,
                    line: r.line,
                    severity: r.severity,
                    body: r.body,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Outcome of one review agent run (fed to the summarizer).
pub struct AgentRunResult {
    pub findings: Vec<ReviewFinding>,
    pub errored: bool,
}

/// Shared, persisted live state for all agents in a review.
pub type SharedStates = Arc<Mutex<Vec<ReviewAgentState>>>;

/// Spawn `provider` as a live session in the repo, inject the (augmented)
/// review prompt, and wait until it writes its findings file (or `timeout`
/// elapses / it exits). Updates + persists this agent's state throughout so the
/// UI shows live progress; archives the session when done.
#[allow(clippy::too_many_arguments)]
pub async fn run_agent_session(
    manager: &Arc<SessionManager>,
    reviews: &ReviewsRepo,
    states: &SharedStates,
    ws: &Workspace,
    user: &User,
    provider: &str,
    // Working directory for the agent (the repo path — where the diff lives).
    cwd: &str,
    review_id: &str,
    agent_index: usize,
    base_prompt: &str,
    timeout: Duration,
) -> AgentRunResult {
    let path = findings_path(review_id, agent_index);
    let _ = std::fs::remove_file(&path); // clear any stale file
    let prompt = augment_prompt(base_prompt, &path.to_string_lossy());

    // Helper to mutate this agent's state then persist. `$f` is a closure
    // `|s: &mut ReviewAgentState| { ... }` applied to the live row.
    //
    // Persistence writes ONLY this agent's own array element (via
    // `set_agent_at`), never the whole array. Review agents run concurrently and
    // each persists its own state; rewriting the full array would let a stale
    // snapshot (taken before this agent flipped to "running") commit after a
    // fresher one and revert other rows to "pending" with no session_id —
    // making live agents show as PENDING and look as though the run is
    // capped/stuck at a couple of agents.
    macro_rules! update {
        ($f:expr) => {{
            let row = {
                let mut g = states.lock().await;
                g.get_mut(agent_index).map(|s| {
                    ($f)(s);
                    s.clone()
                })
            };
            if let Some(row) = row {
                let _ = reviews
                    .set_agent_at(&review_id.to_string(), agent_index, &row)
                    .await;
            }
        }};
    }

    let meta = serde_json::json!({
        "source": "review",
        "review_id": review_id,
        "agent_index": agent_index,
    });
    let req = CreateSessionReq {
        kind: SessionKind::Agent,
        provider: Some(provider.to_string()),
        title: None,
        cwd: Some(cwd.to_string()),
        connection_id: None,
        meta: Some(meta),
    };
    let session = match manager.create(ws, &user.id, req, None).await {
        Ok(s) => s,
        Err(e) => {
            update!(|s: &mut ReviewAgentState| {
                s.status = "error".into();
                s.note = format!("could not start: {e}").chars().take(120).collect();
            });
            return AgentRunResult { findings: Vec::new(), errored: true };
        }
    };
    let sid = session.id.clone();
    update!(|s: &mut ReviewAgentState| {
        s.status = "running".into();
        s.session_id = Some(sid.clone());
    });

    // Inject the prompt once the TUI has drawn + settled, then confirm it
    // dispatched (re-sending Enter once if the first submit was dropped).
    if wait_for_tui(manager, &sid).await {
        let _ = manager.input(&sid, &bracketed_paste(&prompt)).await;
        tokio::time::sleep(PASTE_TO_ENTER).await;
        let before = manager.live_handle(&sid).map(|h| h.last_output_at());
        let _ = manager.input(&sid, b"\r").await;
        if !dispatched(manager, &sid, before).await {
            let _ = manager.input(&sid, b"\r").await;
        }
    }

    // Watch for the findings file (or claude transcript), flagging "waiting"
    // when it goes quiet without producing anything.
    let deadline = Instant::now() + timeout;
    let mut flagged_waiting = false;
    let result = loop {
        if let Ok(text) = std::fs::read_to_string(&path) {
            let findings = parse_findings(&text);
            let _ = std::fs::remove_file(&path);
            break AgentRunResult { findings, errored: false };
        }
        if provider == "claude" {
            if let Some(psid) = session.provider_session_id.as_deref() {
                let jsonl = otto_orchestrator::claude_pty::session_jsonl_path(cwd, psid);
                if let Ok(raw) = std::fs::read_to_string(&jsonl) {
                    if let Some(turn) = otto_orchestrator::claude_pty::completed_turn_text(&raw) {
                        let findings = parse_findings(&turn);
                        if !findings.is_empty() {
                            break AgentRunResult { findings, errored: false };
                        }
                    }
                }
            }
        }

        match manager.live_handle(&sid) {
            Some(handle) => {
                if handle.on_exit().borrow().is_some() {
                    update!(|s: &mut ReviewAgentState| {
                        s.status = "error".into();
                        s.note = "session exited before writing findings".into();
                    });
                    break AgentRunResult { findings: Vec::new(), errored: true };
                }
                // Idle-with-no-findings → likely waiting for input.
                let idle = handle.last_output_at().elapsed();
                if idle >= WAITING_IDLE && !flagged_waiting {
                    flagged_waiting = true;
                    update!(|s: &mut ReviewAgentState| {
                        s.status = "waiting".into();
                        s.note = "looks blocked on input — Open it to respond".into();
                    });
                } else if idle < WAITING_IDLE && flagged_waiting {
                    flagged_waiting = false;
                    update!(|s: &mut ReviewAgentState| {
                        s.status = "running".into();
                        s.note = String::new();
                    });
                }
            }
            None => {
                update!(|s: &mut ReviewAgentState| {
                    s.status = "error".into();
                    s.note = "session is no longer live".into();
                });
                break AgentRunResult { findings: Vec::new(), errored: true };
            }
        }

        if Instant::now() >= deadline {
            update!(|s: &mut ReviewAgentState| {
                s.status = "error".into();
                s.note = "timed out (grace period elapsed)".into();
            });
            break AgentRunResult { findings: Vec::new(), errored: true };
        }
        tokio::time::sleep(FINDINGS_POLL).await;
    };

    // Record findings. We intentionally do NOT archive/kill the session: it
    // stays live (hidden from the main grid via meta.source="review") so the
    // user can still open + inspect its terminal after it finishes.
    if !result.errored {
        let findings = result.findings.clone();
        let count = findings.len();
        update!(|s: &mut ReviewAgentState| {
            s.status = "done".into();
            s.note = format!("{count} finding{}", if count == 1 { "" } else { "s" });
            s.comment_count = count as u32;
            s.findings = findings.clone();
        });
    }
    result
}

pub fn bracketed_paste(text: &str) -> Vec<u8> {
    let mut v = Vec::with_capacity(text.len() + 16);
    v.extend_from_slice(b"\x1b[200~");
    v.extend_from_slice(text.as_bytes());
    v.extend_from_slice(b"\x1b[201~");
    v
}

pub async fn wait_for_tui(manager: &Arc<SessionManager>, sid: &otto_core::Id) -> bool {
    let deadline = Instant::now() + TUI_STARTUP_WAIT;
    loop {
        let Some(handle) = manager.live_handle(sid) else {
            return false;
        };
        if handle.on_exit().borrow().is_some() {
            return false;
        }
        if !handle.scrollback(1).is_empty() && handle.last_output_at().elapsed() >= TUI_SETTLE {
            return true;
        }
        if Instant::now() >= deadline {
            return true;
        }
        tokio::time::sleep(TUI_POLL).await;
    }
}

/// True if the session produced fresh output after `before` within
/// [`DISPATCH_WAIT`] — i.e. the submitted prompt was accepted and the agent
/// started working.
pub async fn dispatched(
    manager: &Arc<SessionManager>,
    sid: &otto_core::Id,
    before: Option<std::time::Instant>,
) -> bool {
    let Some(before) = before else { return false };
    let deadline = Instant::now() + DISPATCH_WAIT;
    loop {
        match manager.live_handle(sid) {
            Some(h) if h.last_output_at() > before => return true,
            None => return false,
            _ => {}
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(DISPATCH_POLL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn findings_path_unique_per_agent() {
        assert_ne!(findings_path("r", 0), findings_path("r", 1));
        assert!(findings_path("r", 2).to_string_lossy().ends_with("otto-review-r-2.json"));
    }

    #[test]
    fn augment_prompt_includes_path_and_base() {
        let out = augment_prompt("Review it.", "/tmp/x.json");
        assert!(out.contains("Review it."));
        assert!(out.contains("/tmp/x.json"));
        assert!(out.to_lowercase().contains("json array"));
    }

    #[test]
    fn parse_findings_tolerates_fences_prose_and_garbage() {
        let raw = "ok:\n```json\n[{\"path\":\"a.rs\",\"line\":3,\"severity\":\"bug\",\"body\":\"x\"}]\n```";
        let f = parse_findings(raw);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].path.as_deref(), Some("a.rs"));
        assert_eq!(f[0].severity, "bug");

        assert_eq!(parse_findings("[{\"body\":\"n\"}]")[0].severity, "info");
        assert!(parse_findings("not json").is_empty());
        assert!(parse_findings("").is_empty());
    }
}
