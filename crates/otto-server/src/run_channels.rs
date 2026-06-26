//! Channel ↔ Run with Otto glue: a `/run <ref>` (or "run with otto …") message
//! launches the one-button pipeline, and an `approve`/`reject` reply resolves an
//! awaiting run's gate. Injected into the channel `Bridge` as a `RunTrigger`.
//!
//! Authorization (design §20.3): the bridge already enforces the integration's
//! `allowed_users` allowlist *before* this trigger runs, so any message reaching
//! here is from an authorized chat user; the action executes as the daemon root
//! user (the established channel-trust model). The Slack/Telegram user id is
//! recorded as the approver.

use async_trait::async_trait;
use otto_channels::{RunAck, RunTrigger};
use otto_core::run::{ApproveRunReq, LaunchRunReq, RunOrigin};

use crate::run_service::{self, LaunchOrigin};
use crate::state::ServerCtx;

pub struct ChannelRunTrigger {
    ctx: ServerCtx,
    root_user_id: String,
}

impl ChannelRunTrigger {
    pub fn new(ctx: ServerCtx, root_user_id: String) -> Self {
        Self { ctx, root_user_id }
    }
}

/// `"approve"` / `"reject"` from a chat reply (tolerant of a few synonyms and the
/// `/run approve|reject` form).
fn approval_decision(lower: &str) -> Option<&'static str> {
    let s = lower.trim().trim_end_matches(['.', '!']);
    match s {
        "approve" | "approved" | "yes" | "lgtm" | "/run approve" | "/approve" => Some("approve"),
        "reject" | "rejected" | "no" | "/run reject" | "/reject" => Some("reject"),
        _ => None,
    }
}

/// Extract the argument of a `/run …` command or a "run with otto …" mention.
/// Returns `None` if the message isn't a run command.
fn run_arg(text: &str) -> Option<String> {
    let t = text.trim();
    if let Some(rest) = t.strip_prefix("/run") {
        // "/run", "/run <ref>" (but NOT "/run approve|reject" — handled earlier).
        return Some(rest.trim().to_string());
    }
    let lower = t.to_ascii_lowercase();
    if let Some(idx) = lower.find("run with otto") {
        let after = &t[idx + "run with otto".len()..];
        return Some(after.trim().trim_start_matches(':').trim().to_string());
    }
    None
}

#[async_trait]
impl RunTrigger for ChannelRunTrigger {
    async fn handle(
        &self,
        workspace_id: &str,
        channel: &str,
        chat: &str,
        thread: Option<&str>,
        user: &str,
        text: &str,
    ) -> Option<RunAck> {
        let lower = text.trim().to_ascii_lowercase();

        // 1. Approval/rejection of an awaiting run bound to this thread.
        if let Some(decision) = approval_decision(&lower) {
            match self
                .ctx
                .runs
                .find_awaiting_for_thread(&workspace_id.to_string(), chat, thread)
                .await
            {
                Ok(Some(run)) => {
                    let req = ApproveRunReq {
                        decision: decision.to_string(),
                        note: None,
                    };
                    return Some(match run_service::approve(&self.ctx, &run.id, &req, user).await {
                        Ok(updated) => {
                            let emoji = if decision == "approve" { "✅" } else { "🛑" };
                            RunAck {
                                reply: format!(
                                    "{emoji} *{}* — {}.",
                                    run.title,
                                    updated.status.as_str()
                                ),
                            }
                        }
                        Err(e) => RunAck {
                            reply: format!("Couldn't {decision} the run: {e}"),
                        },
                    });
                }
                // No awaiting run here → not for us; let the message route normally.
                _ => return None,
            }
        }

        // 2. Launch via `/run <ref>` or "run with otto …".
        let Some(arg) = run_arg(text) else {
            return None;
        };
        if arg.is_empty() {
            return Some(RunAck {
                reply: "Usage: `/run <Jira key | GitHub/Confluence URL | finding:/story:/test:/report:<id>>` \
                        — or `/run <describe what you want>`."
                    .to_string(),
            });
        }
        let run_origin = if channel == "telegram" {
            RunOrigin::Telegram
        } else {
            RunOrigin::Slack
        };
        let req = LaunchRunReq {
            source_ref: Some(arg.clone()),
            seed_text: Some(arg),
            ..Default::default()
        };
        let origin = LaunchOrigin {
            kind_chat: Some(chat.to_string()),
            thread: thread.map(str::to_string),
            user: Some(user.to_string()),
            callback_url: None,
        };
        Some(
            match run_service::launch(
                &self.ctx,
                &workspace_id.to_string(),
                &self.root_user_id,
                run_origin,
                origin,
                req,
            )
            .await
            {
                Ok(run) => RunAck {
                    reply: format!(
                        "🧪 *Run with Otto* started for *{}* — I'll post progress here, \
                         then ask you to approve before drafting a PR.",
                        run.title
                    ),
                },
                Err(e) => RunAck {
                    reply: format!("Couldn't start the run: {e}"),
                },
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_decision_parsing() {
        assert_eq!(approval_decision("approve"), Some("approve"));
        assert_eq!(approval_decision("Approved."), None); // case handled by caller (lower)
        assert_eq!(approval_decision("approved"), Some("approve"));
        assert_eq!(approval_decision("reject!"), Some("reject"));
        assert_eq!(approval_decision("/run reject"), Some("reject"));
        assert_eq!(approval_decision("ship it"), None);
    }

    #[test]
    fn run_arg_extraction() {
        assert_eq!(run_arg("/run PROJ-1"), Some("PROJ-1".to_string()));
        assert_eq!(run_arg("/run"), Some(String::new()));
        assert_eq!(
            run_arg("hey, run with otto: fix the login bug"),
            Some("fix the login bug".to_string())
        );
        assert_eq!(run_arg("just chatting"), None);
    }
}
