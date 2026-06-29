//! Start a workflow from a structured chat message (Slack/Telegram/webhook).
//!
//! The recognized shape (field labels are case-insensitive; `Goals:` may be a
//! bullet list or an inline comma list):
//!
//! ```text
//! @otto
//! Action: Workflow
//! Name: <workflow name>
//! Msg: please do x y z, follow all relevant rules
//! Jira ticket: GS-1111
//! Working Directory: ~/path
//! Relevant Info: ~/a, ~/b
//! Goals:
//!   - 100% test coverage
//!   - under 2 minutes runtime
//! ```
//!
//! [`parse_workflow_command`] is pure + unit-tested; [`WorkflowChatTriggerImpl`]
//! resolves the workflow by **Name** within the message's workspace and starts a
//! run whose input carries every parsed field (so the first node — e.g. a
//! "prepare relevant info" agent — can gather the Jira ticket / working dir /
//! relevant paths into a context file and pass it downstream).

use std::collections::HashMap;

use async_trait::async_trait;
use otto_channels::workflow_trigger::{WorkflowChatAck, WorkflowChatTrigger};
use otto_state::WorkflowsRepo;
use serde_json::json;

use crate::state::ServerCtx;

/// A parsed `Action: Workflow` command.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowCommand {
    pub name: String,
    pub msg: String,
    pub jira_ticket: Option<String>,
    pub working_directory: Option<String>,
    pub relevant_info: Vec<String>,
    pub goals: Vec<String>,
    pub raw: String,
}

/// Parse a structured workflow command. Returns `None` unless the text declares
/// `Action: Workflow` and carries a non-empty `Name:`.
pub fn parse_workflow_command(text: &str) -> Option<WorkflowCommand> {
    if !text.to_lowercase().contains("action:") {
        return None;
    }
    let mut fields: HashMap<String, String> = HashMap::new();
    let mut goals: Vec<String> = vec![];
    let mut current_label: Option<String> = None;

    for raw_line in text.lines() {
        let line = raw_line.trim();
        // Bullet line under the Goals label → a goal item.
        let bullet = line
            .strip_prefix('-')
            .or_else(|| line.strip_prefix('*'))
            .or_else(|| line.strip_prefix('•'));
        if let Some(rest) = bullet {
            if current_label.as_deref() == Some("goals") {
                let g = rest.trim();
                if !g.is_empty() {
                    goals.push(g.to_string());
                }
                continue;
            }
        }
        // Label: value
        if let Some((label, val)) = line.split_once(':') {
            let key = label.trim().to_lowercase();
            let is_labelish =
                !key.is_empty() && key.len() <= 24 && key.chars().all(|c| c.is_alphabetic() || c == ' ' || c == '_');
            if is_labelish {
                let v = val.trim().to_string();
                current_label = Some(key.clone());
                if key == "goals" {
                    for g in v.split([',', ';']) {
                        let g = g.trim();
                        if !g.is_empty() {
                            goals.push(g.to_string());
                        }
                    }
                } else {
                    fields.insert(key, v);
                }
                continue;
            }
        }
        if line.is_empty() {
            current_label = None;
        }
    }

    if fields.get("action").map(|s| s.to_lowercase()) != Some("workflow".to_string()) {
        return None;
    }
    let name = fields.get("name").cloned().unwrap_or_default();
    if name.trim().is_empty() {
        return None;
    }
    let pick = |keys: &[&str]| -> Option<String> {
        keys.iter()
            .find_map(|k| fields.get(*k).cloned())
            .filter(|s| !s.trim().is_empty())
    };
    let relevant_info = pick(&["relevant info", "relevant_info", "relevant"])
        .map(|s| {
            s.split(',')
                .map(|x| x.trim().to_string())
                .filter(|x| !x.is_empty())
                .collect()
        })
        .unwrap_or_default();

    Some(WorkflowCommand {
        name,
        msg: pick(&["msg", "message"]).unwrap_or_default(),
        jira_ticket: pick(&["jira ticket", "jira", "jira_ticket", "ticket"]),
        working_directory: pick(&["working directory", "working dir", "workdir", "cwd"]),
        relevant_info,
        goals,
        raw: text.to_string(),
    })
}

/// otto-server's implementation of the channel workflow trigger.
pub struct WorkflowChatTriggerImpl {
    pub ctx: ServerCtx,
}

#[async_trait]
impl WorkflowChatTrigger for WorkflowChatTriggerImpl {
    async fn try_start(
        &self,
        workspace_id: &str,
        channel: &str,
        chat: &str,
        thread: Option<&str>,
        user: &str,
        text: &str,
    ) -> Option<WorkflowChatAck> {
        let cmd = parse_workflow_command(text)?;
        let repo = WorkflowsRepo::new(self.ctx.pool.clone());
        let wfs = repo.list(&workspace_id.to_string()).await.ok()?;
        let wf = wfs
            .into_iter()
            .find(|w| w.name.trim().eq_ignore_ascii_case(cmd.name.trim()))?;

        let input = json!({
            "trigger": "chat",
            "channel": channel,
            "chat": chat,
            "thread": thread,
            "user": user,
            "name": cmd.name,
            "msg": cmd.msg,
            "jira_ticket": cmd.jira_ticket,
            "working_directory": cmd.working_directory,
            "relevant_info": cmd.relevant_info,
            "goals": cmd.goals,
            "raw": cmd.raw,
        });

        let run = repo.create_run(&wf.id, &wf.workspace_id, &input).await.ok()?;
        let ws = self.ctx.workspaces.get(&wf.workspace_id).await.ok()?;
        let ctx2 = self.ctx.clone();
        let run_id = run.id.clone();
        let wf2 = wf.clone();
        let input2 = input.clone();
        tokio::spawn(async move {
            crate::workflow_engine::run_workflow(ctx2, ws, wf2, run_id, input2, None, false).await;
        });

        let goals_txt = if cmd.goals.is_empty() {
            "none".to_string()
        } else {
            cmd.goals.join("; ")
        };
        Some(WorkflowChatAck {
            reply: format!(
                "🚀 Started workflow **{}** (run `{}`). Working through the steps now — goals: {}.",
                wf.name, run.id, goals_txt
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_command() {
        let text = "@otto\n\
                    Action: Workflow\n\
                    Name: Implement Feature\n\
                    Msg: please do x y z, follow all relevant rules\n\
                    Jira ticket: GS-1111\n\
                    Working Directory: ~/repo\n\
                    Relevant Info: ~/a, ~/b\n\
                    Goals:\n\
                    - 100% test coverage\n\
                    - under 2 minutes runtime\n";
        let cmd = parse_workflow_command(text).expect("should parse");
        assert_eq!(cmd.name, "Implement Feature");
        assert_eq!(cmd.msg, "please do x y z, follow all relevant rules");
        assert_eq!(cmd.jira_ticket.as_deref(), Some("GS-1111"));
        assert_eq!(cmd.working_directory.as_deref(), Some("~/repo"));
        assert_eq!(cmd.relevant_info, vec!["~/a", "~/b"]);
        assert_eq!(cmd.goals, vec!["100% test coverage", "under 2 minutes runtime"]);
    }

    #[test]
    fn inline_goals_and_aliases() {
        let text = "Action: Workflow\nName: Tests\nJira: GS-9\nGoals: a, b; c\n";
        let cmd = parse_workflow_command(text).unwrap();
        assert_eq!(cmd.jira_ticket.as_deref(), Some("GS-9"));
        assert_eq!(cmd.goals, vec!["a", "b", "c"]);
    }

    #[test]
    fn requires_action_workflow_and_name() {
        assert!(parse_workflow_command("just a normal message").is_none());
        assert!(parse_workflow_command("Action: Swarm\nName: x").is_none());
        assert!(parse_workflow_command("Action: Workflow\nMsg: no name").is_none());
    }
}
