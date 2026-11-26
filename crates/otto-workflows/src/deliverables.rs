//! What a step put outside Otto (engine.md §6.3, E9). Delivery truth comes
//! only from what the tools answered — the Otto outward calls a step's thread
//! actually made (`TranscriptDigest.actions`) or the counts a built-in node
//! posted itself — never from the agent's own words: a reply that says
//! "posted" with no ok call behind it is not a deliverable.

use otto_core::agent_runtime::{OutwardAction, OutwardOutcome};
use otto_core::workflows::{Deliverable, DeliverableState};

/// The deliverable label and its verb for an outward tool, in either catalog
/// spelling (`otto.comment_pr` / `otto_comment_pr` / `comment_pr`).
pub fn label_for(tool: &str) -> Option<(&'static str, &'static str)> {
    let name = tool
        .strip_prefix("otto.")
        .or_else(|| tool.strip_prefix("otto_"))
        .unwrap_or(tool);
    Some(match name {
        "comment_pr" => ("PR comments", "posted"),
        "create_pr" => ("Pull request", "created"),
        "merge_pr" => ("PR merge", "merged"),
        "comment_issue" => ("Jira comment", "posted"),
        "create_confluence_page" | "update_confluence_page" => ("Confluence page", "saved"),
        "comment_confluence_page" => ("Confluence comment", "posted"),
        "slack_post_message" | "send_message" => ("Slack message", "sent"),
        _ => return None,
    })
}

/// One deliverable from counts: all ok → done ("5 posted"); all failed →
/// failed; only waiting → pending approval; anything mixed → partial
/// ("3 of 4 posted; 1 failed: …"). `failed` holds each failure's message
/// (the first is shown); `pending` counts calls waiting for an approval.
pub fn tally(label: &str, verb: &str, ok: usize, failed: &[String], pending: usize) -> Deliverable {
    let total = ok + failed.len() + pending;
    let state = match (ok, failed.len(), pending) {
        (_, 0, 0) if ok > 0 => DeliverableState::Done,
        (0, _, 0) if !failed.is_empty() => DeliverableState::Failed,
        (0, 0, _) if pending > 0 => DeliverableState::PendingApproval,
        (0, 0, 0) => DeliverableState::Unknown,
        _ => DeliverableState::Partial,
    };
    let mut parts = vec![if ok == total {
        format!("{ok} {verb}")
    } else {
        format!("{ok} of {total} {verb}")
    }];
    if let Some(first) = failed.first() {
        let first = first.trim();
        parts.push(if first.is_empty() {
            format!("{} failed", failed.len())
        } else {
            format!("{} failed: {first}", failed.len())
        });
    }
    if pending > 0 {
        parts.push(format!("{pending} waiting for approval"));
    }
    Deliverable {
        label: label.to_string(),
        state,
        detail: Some(parts.join("; ")),
        url: None,
    }
}

/// An agent step's deliverables, one per label in first-call order. A call
/// that answered "pending approval" counts as waiting until a later ok call
/// with the same label and target (the approved retry, or the approval's own
/// result) settles it. The url is the last ok call's target when it is a link.
pub fn from_actions(actions: &[OutwardAction]) -> Vec<Deliverable> {
    let mut labels: Vec<(&str, &str)> = Vec::new();
    for a in actions {
        if let Some(l) = label_for(&a.tool) {
            if !labels.contains(&l) {
                labels.push(l);
            }
        }
    }
    labels
        .into_iter()
        .map(|(label, verb)| {
            let calls: Vec<&OutwardAction> = actions
                .iter()
                .filter(|a| label_for(&a.tool).is_some_and(|(l, _)| l == label))
                .collect();
            let (mut ok, mut failed, mut pending) = (0, Vec::new(), 0);
            for (i, a) in calls.iter().enumerate() {
                match a.outcome {
                    OutwardOutcome::Ok => ok += 1,
                    OutwardOutcome::Failed => failed.push(a.message.clone().unwrap_or_default()),
                    OutwardOutcome::PendingApproval => {
                        let settled = calls[i + 1..]
                            .iter()
                            .any(|b| b.outcome == OutwardOutcome::Ok && b.target == a.target);
                        if !settled {
                            pending += 1;
                        }
                    }
                }
            }
            let mut d = tally(label, verb, ok, &failed, pending);
            d.url = calls
                .iter()
                .rev()
                .filter(|a| a.outcome == OutwardOutcome::Ok)
                .find_map(|a| a.target.clone())
                .filter(|t| t.starts_with("https://") || t.starts_with("http://"));
            d
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(
        tool: &str,
        outcome: OutwardOutcome,
        target: Option<&str>,
        msg: Option<&str>,
    ) -> OutwardAction {
        OutwardAction {
            tool: tool.into(),
            target: target.map(Into::into),
            outcome,
            message: msg.map(Into::into),
        }
    }
    use OutwardOutcome::{Failed, Ok, PendingApproval};

    #[test]
    fn both_catalog_spellings_map_and_other_tools_are_not_deliverables() {
        for t in ["otto.comment_pr", "otto_comment_pr", "comment_pr"] {
            assert_eq!(label_for(t).map(|l| l.0), Some("PR comments"), "{t}");
        }
        assert_eq!(
            label_for("otto_update_confluence_page").unwrap().0,
            "Confluence page"
        );
        assert_eq!(label_for("otto.send_message").unwrap().0, "Slack message");
        assert!(label_for("otto.list_findings").is_none());
        assert!(from_actions(&[call("otto_read_file", Ok, None, None)]).is_empty());
    }

    #[test]
    fn the_same_label_aggregates_into_one_truthful_state() {
        let five: Vec<_> = (0..5)
            .map(|_| call("otto_comment_pr", Ok, None, None))
            .collect();
        let d = &from_actions(&five)[0];
        assert_eq!(
            (d.state, d.detail.as_deref()),
            (DeliverableState::Done, Some("5 posted"))
        );

        let mixed = vec![
            call("otto_comment_pr", Ok, None, None),
            call(
                "otto.comment_pr",
                Failed,
                None,
                Some("you do not have the permission to comment"),
            ),
            call("otto_comment_pr", Ok, None, None),
            call("otto_comment_pr", Ok, None, None),
        ];
        let d = &from_actions(&mixed)[0];
        assert_eq!(d.state, DeliverableState::Partial);
        assert_eq!(
            d.detail.as_deref(),
            Some("3 of 4 posted; 1 failed: you do not have the permission to comment")
        );

        let failed = vec![call("otto_comment_issue", Failed, None, Some("403"))];
        let d = &from_actions(&failed)[0];
        assert_eq!(
            (d.state, d.label.as_str()),
            (DeliverableState::Failed, "Jira comment")
        );
        assert_eq!(d.detail.as_deref(), Some("0 of 1 posted; 1 failed: 403"));
    }

    #[test]
    fn pending_approval_counts_until_a_later_ok_on_the_same_target() {
        let page = Some("https://wiki.test/p/1");
        let waiting = vec![call(
            "otto_comment_confluence_page",
            PendingApproval,
            page,
            None,
        )];
        let d = &from_actions(&waiting)[0];
        assert_eq!(d.state, DeliverableState::PendingApproval);
        assert_eq!(
            d.detail.as_deref(),
            Some("0 of 1 posted; 1 waiting for approval")
        );

        let approved = vec![
            call("otto_comment_confluence_page", PendingApproval, page, None),
            call("otto_comment_confluence_page", Ok, page, None),
        ];
        let d = &from_actions(&approved)[0];
        assert_eq!(d.state, DeliverableState::Done);
        assert_eq!(d.url.as_deref(), page);

        // An ok on another target does not settle the waiting one.
        let other = vec![
            call("otto_comment_confluence_page", PendingApproval, page, None),
            call(
                "otto_comment_confluence_page",
                Ok,
                Some("https://wiki.test/p/2"),
                None,
            ),
        ];
        assert_eq!(from_actions(&other)[0].state, DeliverableState::Partial);
    }

    #[test]
    fn labels_keep_first_call_order_and_urls_are_only_links() {
        let calls = vec![
            call("otto_slack_post_message", Ok, Some("C123"), None),
            call("otto_create_pr", Ok, Some("https://forge.test/pr/9"), None),
            call("otto_slack_post_message", Ok, Some("C123"), None),
        ];
        let ds = from_actions(&calls);
        let labels: Vec<_> = ds.iter().map(|d| d.label.as_str()).collect();
        assert_eq!(labels, ["Slack message", "Pull request"]);
        assert_eq!(ds[0].url, None, "a channel id is not a link");
        assert_eq!(ds[1].url.as_deref(), Some("https://forge.test/pr/9"));
        assert_eq!(ds[1].detail.as_deref(), Some("1 created"));
    }

    #[test]
    fn tally_serves_built_in_nodes_too() {
        let d = tally("PR comments", "posted", 5, &[], 0);
        assert_eq!(d.detail.as_deref(), Some("5 posted"));
        let d = tally("PR comments", "posted", 0, &[], 0);
        assert_eq!(d.state, DeliverableState::Unknown);
    }
}
