//! The compact record of a thread the engine keeps after the thread is
//! deleted (`TranscriptDigest`, ≤ 8 KiB) and its newest timeline items.

use otto_core::agent_runtime::{clip, DigestItem, OutwardAction, OutwardOutcome, TranscriptDigest};

use super::*;

const FILES_MAX: usize = 50;
const ANSWER_MAX: usize = 4 * 1024;
const ERRORS_MAX: usize = 5;
const ACTIONS_MAX: usize = 20;
const ITEM_TEXT_MAX: usize = 300;
const MESSAGE_MAX: usize = 200;

/// Otto MCP tools that reach outside Otto (both catalog spellings).
const OUTWARD: &[&str] = &[
    "comment_pr",
    "create_pr",
    "merge_pr",
    "comment_issue",
    "create_issue",
    "create_confluence_page",
    "update_confluence_page",
    "comment_confluence_page",
    "slack_post_message",
    "send_message",
];
const TARGET_KEYS: &[&str] = &["pr_number", "page_id", "issue_key", "url", "path", "id"];

/// `mcp__otto__otto_comment_pr` / `otto.comment_pr` / `otto_comment_pr` → `comment_pr`.
fn otto_tool(name: &str) -> Option<&str> {
    let at = name.rfind("otto_").map(|i| i + 5);
    let at = at.max(name.rfind("otto.").map(|i| i + 5))?;
    let tool = &name[at..];
    OUTWARD.contains(&tool).then_some(tool)
}

fn action(item: &TurnItem) -> Option<OutwardAction> {
    let TurnItemKind::DynamicTool {
        tool_name,
        input,
        output,
        ..
    } = &item.kind
    else {
        return None;
    };
    let tool = otto_tool(tool_name)?;
    let target = TARGET_KEYS.iter().find_map(|k| {
        input.get(*k).and_then(|v| match v {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
    });
    let out = output.as_deref().unwrap_or_default();
    let outcome = if out.to_lowercase().contains("pending approval") {
        OutwardOutcome::PendingApproval
    } else if item.status == TurnItemStatus::Failed || item.error.is_some() {
        OutwardOutcome::Failed
    } else {
        OutwardOutcome::Ok
    };
    let message = (outcome != OutwardOutcome::Ok)
        .then(|| clip(item.error.as_deref().unwrap_or(out).trim(), MESSAGE_MAX))
        .filter(|m| !m.is_empty());
    Some(OutwardAction {
        tool: tool.to_string(),
        target,
        outcome,
        message,
    })
}

impl AgentRuntime {
    /// The thread's digest: turns, tool calls, files changed, the last answer,
    /// errors and the outward actions its agent took.
    pub async fn digest(&self, id: &Id) -> Result<TranscriptDigest> {
        let live = self.live(id).await?;
        let st = live.lock();
        let mut items: Vec<&TurnItem> = st.proj.items.values().collect();
        items.sort_by_key(|i| (i.started_at, i.ordinal));
        let mut d = TranscriptDigest {
            turns: st.proj.turns.len() as u32,
            started_at: st.proj.turns.values().filter_map(|t| t.started_at).min(),
            ended_at: st.proj.turns.values().filter_map(|t| t.completed_at).max(),
            ..TranscriptDigest::default()
        };
        for i in &items {
            match &i.kind {
                TurnItemKind::CommandExecution { .. }
                | TurnItemKind::DynamicTool { .. }
                | TurnItemKind::WebSearch { .. }
                | TurnItemKind::Subagent { .. } => d.tool_calls += 1,
                TurnItemKind::FileChange { file_name, .. } => {
                    d.tool_calls += 1;
                    if d.files_changed.len() < FILES_MAX && !d.files_changed.contains(file_name) {
                        d.files_changed.push(file_name.clone());
                    }
                }
                TurnItemKind::Error { failure } => d.errors.push(clip(&failure.message, 300)),
                TurnItemKind::AssistantMessage { text, .. }
                    if i.parent_item_id.is_none() && !text.trim().is_empty() =>
                {
                    d.last_answer_excerpt = clip(text.trim(), ANSWER_MAX);
                }
                _ => {}
            }
            if let Some(a) = action(i) {
                d.actions.push(a);
            }
        }
        let cut = d.errors.len().saturating_sub(ERRORS_MAX);
        d.errors.drain(..cut);
        let cut = d.actions.len().saturating_sub(ACTIONS_MAX);
        d.actions.drain(..cut);
        Ok(d)
    }

    /// The newest `n` timeline items, oldest first: what was said and done.
    pub async fn digest_items(&self, id: &Id, n: usize) -> Result<Vec<DigestItem>> {
        let live = self.live(id).await?;
        let st = live.lock();
        let mut items: Vec<&TurnItem> = st.proj.items.values().collect();
        items.sort_by_key(|i| (i.started_at, i.ordinal));
        let mut out: Vec<DigestItem> = items
            .iter()
            .filter_map(|i| {
                let (role, text, tool) = match &i.kind {
                    TurnItemKind::UserMessage { text, .. } => ("user", text.clone(), None),
                    TurnItemKind::AssistantMessage { text, .. } => {
                        ("assistant", text.clone(), None)
                    }
                    TurnItemKind::CommandExecution { input, .. } => {
                        ("tool", input.clone(), Some("command".to_string()))
                    }
                    TurnItemKind::DynamicTool {
                        tool_name, output, ..
                    } => (
                        "tool",
                        output.clone().unwrap_or_default(),
                        Some(tool_name.clone()),
                    ),
                    TurnItemKind::FileChange { file_name, .. } => {
                        ("tool", file_name.clone(), Some("file_change".to_string()))
                    }
                    TurnItemKind::Error { failure } => ("error", failure.message.clone(), None),
                    _ => return None,
                };
                Some(DigestItem {
                    role: role.to_string(),
                    text: clip(text.trim(), ITEM_TEXT_MAX),
                    tool,
                })
            })
            .collect();
        let cut = out.len().saturating_sub(n);
        out.drain(..cut);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::otto_tool;

    #[test]
    fn otto_outward_tools_are_read_in_every_spelling() {
        assert_eq!(otto_tool("mcp__otto__otto_comment_pr"), Some("comment_pr"));
        assert_eq!(
            otto_tool("otto.create_confluence_page"),
            Some("create_confluence_page")
        );
        assert_eq!(
            otto_tool("otto_slack_post_message"),
            Some("slack_post_message")
        );
        assert_eq!(
            otto_tool("mcp__otto__otto_get_pr"),
            None,
            "a read is no action"
        );
        assert_eq!(otto_tool("Bash"), None);
    }
}
