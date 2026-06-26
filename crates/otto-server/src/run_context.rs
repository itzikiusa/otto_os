//! Run with Otto — the Context Packet.
//!
//! Assembles the **task prompt** that drives the execute stage from the resolved
//! source + repo orientation. The *environmental* context (soul, installed skills,
//! memory, repo rules, hooks) is injected automatically at session/agent spawn by
//! the out-of-tree `PreSpawnHook` — we do not duplicate it here. This is purely the
//! "what to do" half: goal + source content + the working agreement.

use otto_core::domain::Repo;
use otto_core::run::{OttoRun, ResolvedSource};

/// The assembled packet. `prompt` is fed to `run_agent` / the goal loop; `summary`
/// is stored (truncated) on the run for transparency in the UI/timeline.
pub(crate) struct ContextPacket {
    pub prompt: String,
    pub summary: String,
}

const SUMMARY_CAP: usize = 2_000;

pub(crate) fn build_packet(run: &OttoRun, resolved: &ResolvedSource, repo: &Repo) -> ContextPacket {
    let url = resolved
        .source_url
        .as_deref()
        .map(|u| format!("\n{u}"))
        .unwrap_or_default();

    let prompt = format!(
        "You are Otto, an autonomous coding agent working a single, focused change \
         derived from a {kind} source item. You are already checked out on an \
         isolated git branch inside `{repo}` — your current directory is the \
         worktree, so all your edits stay isolated.\n\n\
         # Goal\n{goal}\n\n\
         # Source: {title}{url}\n\n{body}\n\n\
         # Working agreement\n\
         - Make the smallest correct change that satisfies the goal.\n\
         - Add or update tests where it makes sense, and run them if a test command is evident.\n\
         - COMMIT your work to the current branch when done (`git add -A && git commit -m ...`). \
         Do NOT push, open a PR, or touch any other branch — Otto handles review, proof, \
         approval and the PR draft after you finish.\n\
         - End by printing a short summary: what you changed, why, and how you verified it.\n",
        kind = run.source_kind.as_str(),
        repo = repo.name,
        goal = resolved.goal,
        title = resolved.title,
        url = url,
        body = resolved.body_md,
    );

    let mut summary = format!(
        "{goal}\n\nSource: {title} ({kind})",
        goal = resolved.goal,
        title = resolved.title,
        kind = run.source_kind.as_str(),
    );
    if summary.len() > SUMMARY_CAP {
        let mut end = SUMMARY_CAP;
        while end > 0 && !summary.is_char_boundary(end) {
            end -= 1;
        }
        summary.truncate(end);
    }

    ContextPacket { prompt, summary }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use otto_core::run::{RunMode, RunOrigin, RunStatus, SourceKind};

    fn repo() -> Repo {
        Repo {
            id: "r1".into(),
            workspace_id: "w1".into(),
            name: "widgets".into(),
            path: "/tmp/widgets".into(),
            remote_url: None,
            provider: None,
            git_account_id: None,
            created_at: Utc::now(),
        }
    }

    fn run() -> OttoRun {
        OttoRun {
            id: "x".into(),
            workspace_id: "w1".into(),
            title: "t".into(),
            source_kind: SourceKind::Finding,
            source_ref: "f1".into(),
            source_url: None,
            goal: "g".into(),
            mode: RunMode::SingleAgent,
            provider: "claude".into(),
            repo_id: Some("r1".into()),
            repo_path: Some("/tmp/widgets".into()),
            base_branch: Some("main".into()),
            branch: None,
            worktree_path: None,
            base_commit: None,
            status: RunStatus::BuildingContext,
            error: None,
            origin_kind: RunOrigin::Api,
            origin_chat: None,
            origin_thread: None,
            origin_user: None,
            callback_url: None,
            goal_loop_id: None,
            review_id: None,
            proof_pack_id: None,
            proof_status: None,
            risk_score: None,
            findings_total: 0,
            findings_blocking: 0,
            pr_draft_json: None,
            pr_url: None,
            auto_open_pr: false,
            approval_decision: None,
            approved_by: None,
            approved_at: None,
            result_summary: None,
            context_summary: None,
            created_by: "root".into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn packet_includes_goal_body_and_commit_instruction() {
        let resolved = ResolvedSource {
            title: "Fix login".into(),
            body_md: "the bug is here".into(),
            goal: "Fix the login bug".into(),
            source_url: Some("https://example.com/x".into()),
            repo_hint: None,
            metadata: serde_json::json!({}),
        };
        let p = build_packet(&run(), &resolved, &repo());
        assert!(p.prompt.contains("Fix the login bug"));
        assert!(p.prompt.contains("the bug is here"));
        assert!(p.prompt.contains("git commit"));
        assert!(p.prompt.contains("Do NOT push"));
        assert!(p.summary.contains("finding"));
    }
}
