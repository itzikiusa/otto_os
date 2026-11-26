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
/// The serialized digest's bound, in bytes.
const DIGEST_MAX: usize = 8 * 1024;
/// The answer excerpt shrinks to this before anything else goes.
const ANSWER_FLOOR: usize = 1024;

/// `s` without the secrets `otto_core::redact` recognises.
fn redact(s: &str) -> String {
    otto_core::redact::redact_text(s).value
}

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
    // The gateway's verdict (`{"decision": …, "executed": false}`), also as
    // the escaped text of an MCP content block.
    let flat: String = out
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '\\')
        .collect::<String>()
        .to_lowercase();
    let outcome = if flat.contains(r#""decision":"pending_approval""#)
        || out.to_lowercase().contains("pending approval")
    {
        OutwardOutcome::PendingApproval
    } else if item.status == TurnItemStatus::Failed
        || item.error.is_some()
        || flat.contains(r#""decision":"denied""#)
        || flat.contains(r#""executed":false"#)
    {
        OutwardOutcome::Failed
    } else {
        OutwardOutcome::Ok
    };
    let message = (outcome != OutwardOutcome::Ok)
        .then(|| {
            clip(
                &redact(item.error.as_deref().unwrap_or(out).trim()),
                MESSAGE_MAX,
            )
        })
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
        d.last_answer_excerpt = redact(&d.last_answer_excerpt);
        for e in &mut d.errors {
            *e = redact(e);
        }
        fit(&mut d);
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
                // Clipped before the redaction (an item can be a whole
                // log), with room for a secret that straddles the clip.
                let text = redact(&clip(text.trim(), 4 * ITEM_TEXT_MAX));
                Some(DigestItem {
                    role: role.to_string(),
                    text: clip(&text, ITEM_TEXT_MAX),
                    tool,
                })
            })
            .collect();
        let cut = out.len().saturating_sub(n);
        out.drain(..cut);
        Ok(out)
    }
}

/// Hold the digest to [`DIGEST_MAX`] serialized bytes: the answer shrinks to
/// [`ANSWER_FLOOR`] first, then files, the oldest errors and actions go.
fn fit(d: &mut TranscriptDigest) {
    let size = |d: &TranscriptDigest| serde_json::to_vec(d).map_or(0, |v| v.len());
    while size(d) > DIGEST_MAX {
        let n = d.last_answer_excerpt.chars().count();
        if n > ANSWER_FLOOR {
            d.last_answer_excerpt = clip(&d.last_answer_excerpt, (n / 2).max(ANSWER_FLOOR));
            continue;
        }
        if d.files_changed.pop().is_some() {
            continue;
        }
        if !d.errors.is_empty() {
            d.errors.remove(0);
        } else if !d.actions.is_empty() {
            d.actions.remove(0);
        } else if n > 0 {
            d.last_answer_excerpt = clip(&d.last_answer_excerpt, n / 2);
        } else {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str, output: &str) -> TurnItem {
        TurnItem {
            id: "i1".into(),
            turn_id: Some("t1".into()),
            ordinal: 1,
            status: TurnItemStatus::Completed,
            title: None,
            started_at: None,
            completed_at: None,
            updated_at: chrono::Utc::now(),
            error: None,
            parent_item_id: None,
            detail_omitted: false,
            kind: TurnItemKind::DynamicTool {
                image_files: Vec::new(),
                tool_name: name.into(),
                input: serde_json::json!({"pr_number": 12}),
                output: Some(output.into()),
                images: Vec::new(),
            },
        }
    }

    /// The governed gateway answers a call it held or refused with a JSON
    /// verdict, as is or as the text of an MCP content block: never `ok`.
    #[test]
    fn a_held_or_refused_outward_call_is_not_ok() {
        let outcome = |out: &str| {
            action(&tool("mcp__otto__otto_merge_pr", out))
                .unwrap()
                .outcome
        };
        let pending = r#"{"decision":"pending_approval","executed":false,"approval_id":"a1"}"#;
        assert_eq!(outcome(pending), OutwardOutcome::PendingApproval);
        let wrapped = serde_json::json!([{"type": "text", "text": pending}]).to_string();
        assert_eq!(outcome(&wrapped), OutwardOutcome::PendingApproval);
        let denied = r#"{"decision": "denied", "executed": false, "reason": "human denied"}"#;
        assert_eq!(outcome(denied), OutwardOutcome::Failed);
        assert_eq!(
            outcome(r#"{"decision":"dry_run","executed":false}"#),
            OutwardOutcome::Failed
        );
        assert_eq!(
            outcome(r#"{"ok":true,"url":"https://x/pr/12"}"#),
            OutwardOutcome::Ok
        );
    }

    /// Fifty long paths, five long errors, twenty actions and a long answer
    /// still serialize within 8 KiB, and a token in the answer is gone.
    #[test]
    fn the_digest_stays_within_its_bound_and_holds_no_secret() {
        let long = "é".repeat(4000);
        let mut d = TranscriptDigest {
            files_changed: (0..FILES_MAX)
                .map(|i| format!("{i}/{}", "p".repeat(200)))
                .collect(),
            last_answer_excerpt: clip(&long, ANSWER_MAX),
            errors: vec!["e".repeat(300); ERRORS_MAX],
            actions: (0..ACTIONS_MAX)
                .map(|_| action(&tool("otto.comment_pr", &"x".repeat(400))).unwrap())
                .collect(),
            ..TranscriptDigest::default()
        };
        fit(&mut d);
        assert!(serde_json::to_vec(&d).unwrap().len() <= DIGEST_MAX);
        assert!(!d.last_answer_excerpt.is_empty() && !d.actions.is_empty());
        let token = "ghp_0123456789abcdefghijABCDEFGHIJ012345";
        assert!(!redact(&format!("pushed with {token}")).contains(token));
    }

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
