//! `prepare_context` workflow node: app-side Jira ticket fetch into
//! `jira-<KEY>.md` (the run's context dir), with an optional analysis-agent
//! phase layered on top (mirrors `agent_prompt` exactly when `params.prompt`
//! is set).
//!
//! Three pure/async building blocks, exercised by the `execute_node` arm in
//! `workflow_engine.rs`:
//! - [`extract_jira_key`] — find the ticket the step should act on.
//! - [`resolve_jira_account`] — pick which configured Jira account to fetch with.
//! - [`render_issue_md`] — turn a fetched [`otto_issues::IssueFull`] into the
//!   markdown written to `jira-<KEY>.md`.

use otto_core::Id;
use serde_json::Value;

use crate::state::ServerCtx;

/// Resolve the Jira key a `prepare_context` step should fetch, in order:
/// `params.key` → `input.jira_ticket` (both trusted verbatim — the caller
/// already knows the exact key) → the first Jira-key-shaped token found by
/// scanning `input.prompt`, then `input.msg` (free text — must be scanned).
pub(crate) fn extract_jira_key(params: &Value, input: &Value) -> Option<String> {
    if let Some(k) = params.get("key").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) {
        return Some(k.to_string());
    }
    if let Some(k) = input
        .get("jira_ticket")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Some(k.to_string());
    }
    for field in ["prompt", "msg"] {
        if let Some(text) = input.get(field).and_then(Value::as_str) {
            if let Some(k) = scan_jira_key(text) {
                return Some(k);
            }
        }
    }
    None
}

/// Hand-rolled scanner (no regex dependency): the first
/// `[A-Z][A-Z0-9]{1,9}-[0-9]{1,7}` token in `text` with a non-alphanumeric
/// character (or a string edge) on both sides.
fn scan_jira_key(text: &str) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let is_alnum = |c: char| c.is_ascii_alphanumeric();
    for i in 0..n {
        if !chars[i].is_ascii_uppercase() {
            continue;
        }
        let boundary_before = i == 0 || !is_alnum(chars[i - 1]);
        if !boundary_before {
            continue;
        }
        // Consume [A-Z0-9]{1,9} after the leading letter — bounded, and the
        // char class itself stops the run at the first char that doesn't fit
        // (e.g. the '-', a lowercase letter, or punctuation), so no
        // backtracking is needed: whatever `j` lands on after this loop is
        // where a dash would have to be for this candidate to match.
        let mut j = i + 1;
        let max_j = (i + 1 + 9).min(n);
        while j < max_j && (chars[j].is_ascii_uppercase() || chars[j].is_ascii_digit()) {
            j += 1;
        }
        if j <= i + 1 {
            // Needs at least 1 char after the leading letter (min total 2).
            continue;
        }
        if j >= n || chars[j] != '-' {
            continue;
        }
        let mut d = j + 1;
        let max_d = (j + 1 + 7).min(n);
        while d < max_d && chars[d].is_ascii_digit() {
            d += 1;
        }
        let digit_len = d - (j + 1);
        if digit_len == 0 {
            continue;
        }
        let boundary_after = d == n || !is_alnum(chars[d]);
        if !boundary_after {
            continue;
        }
        return Some(chars[i..d].iter().collect());
    }
    None
}

/// Resolve which Jira account a `prepare_context` fetch should use: an
/// explicit `params.account_id` wins; else the run user's own Jira account
/// (first one); else any Jira account configured on the daemon (single-user
/// / admin-configured setups). `Err` carries a human-readable reason.
pub(crate) async fn resolve_jira_account(
    ctx: &ServerCtx,
    run_user: &Id,
    account_id: Option<&str>,
) -> std::result::Result<otto_core::domain::IssueAccount, String> {
    if let Some(id) = account_id.map(str::trim).filter(|s| !s.is_empty()) {
        return ctx
            .issues_store
            .get_account(&id.to_string())
            .await
            .map_err(|e| e.to_string());
    }
    if let Ok(accounts) = ctx.issues_store.list_accounts(run_user).await {
        if let Some(a) = accounts
            .into_iter()
            .find(|a| matches!(a.provider, otto_core::domain::IssueProviderKind::Jira))
        {
            return Ok(a);
        }
    }
    let all = ctx.issues_store.list_all_accounts().await.map_err(|e| e.to_string())?;
    all.into_iter()
        .find(|a| matches!(a.provider, otto_core::domain::IssueProviderKind::Jira))
        .ok_or_else(|| "no Jira account configured".to_string())
}

/// Render a fetched issue (+ its comments/links/attachments) to markdown —
/// the body written to `jira-<KEY>.md`.
pub(crate) fn render_issue_md(issue: &otto_issues::IssueFull) -> String {
    let mut md = String::new();
    md.push_str(&format!("# {}: {}\n\n", issue.key, issue.summary));
    md.push_str(&format!("- URL: {}\n", issue.url));
    md.push_str(&format!("- Status: {}\n", issue.status));
    md.push_str(&format!("- Type: {}\n", issue.issue_type));
    if let Some(a) = &issue.assignee {
        md.push_str(&format!("- Assignee: {}\n", a.display_name));
    }
    if let Some(r) = &issue.reporter {
        md.push_str(&format!("- Reporter: {}\n", r.display_name));
    }
    if let Some(p) = &issue.priority {
        md.push_str(&format!("- Priority: {p}\n"));
    }
    if !issue.labels.is_empty() {
        md.push_str(&format!("- Labels: {}\n", issue.labels.join(", ")));
    }
    if let Some(e) = &issue.estimate {
        md.push_str(&format!("- Estimate: {e}\n"));
    }
    md.push('\n');

    md.push_str("## Description\n\n");
    if issue.description_md.trim().is_empty() {
        md.push_str("_No description._\n\n");
    } else {
        md.push_str(&issue.description_md);
        md.push_str("\n\n");
    }

    let custom_fields: Vec<&otto_issues::JiraField> =
        issue.fields.iter().filter(|f| !f.value.trim().is_empty()).collect();
    if !custom_fields.is_empty() {
        md.push_str("## Fields\n\n");
        for f in custom_fields {
            md.push_str(&format!("- **{}**: {}\n", f.name, f.value));
        }
        md.push('\n');
    }

    if !issue.comments.is_empty() {
        md.push_str("## Comments\n\n");
        for c in &issue.comments {
            md.push_str(&format!("**{}** ({}):\n{}\n\n", c.author, c.created, c.body_md));
        }
    }

    if !issue.links.is_empty() {
        md.push_str("## Links\n\n");
        for l in &issue.links {
            md.push_str(&format!(
                "- {} {} — {} ({}, {})\n",
                l.rel, l.key, l.summary, l.issue_type, l.status
            ));
        }
        md.push('\n');
    }

    if !issue.attachments.is_empty() {
        md.push_str("## Attachments\n\n");
        for a in &issue.attachments {
            md.push_str(&format!("- {} ({}, {} bytes) — {}\n", a.filename, a.mime, a.size, a.author));
        }
        md.push('\n');
    }

    md
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn key_extraction_order_and_shape() {
        assert_eq!(extract_jira_key(&json!({"key":"AB-1"}), &json!({})), Some("AB-1".into()));
        assert_eq!(extract_jira_key(&json!({}), &json!({"jira_ticket":"PROJ-9"})), Some("PROJ-9".into()));
        assert_eq!(extract_jira_key(&json!({}), &json!({"prompt":"please do PROJ-123 now"})), Some("PROJ-123".into()));
        assert_eq!(extract_jira_key(&json!({}), &json!({"msg":"see K2X-77."})), Some("K2X-77".into()));
        assert_eq!(extract_jira_key(&json!({}), &json!({"prompt":"lowercase ab-1 or A-2 or X9"})), None);
        assert_eq!(extract_jira_key(&json!({}), &json!({})), None);
    }

    /// Order: params.key wins over input.jira_ticket, which wins over a
    /// scanned prompt/msg token.
    #[test]
    fn key_extraction_precedence() {
        assert_eq!(
            extract_jira_key(&json!({"key":"AB-1"}), &json!({"jira_ticket": "CD-2", "prompt": "do EF-3"})),
            Some("AB-1".into())
        );
        assert_eq!(
            extract_jira_key(&json!({}), &json!({"jira_ticket": "CD-2", "prompt": "do EF-3"})),
            Some("CD-2".into())
        );
        assert_eq!(
            extract_jira_key(&json!({}), &json!({"prompt": "do EF-3", "msg": "see GH-4"})),
            Some("EF-3".into()),
            "prompt scanned before msg"
        );
    }

    fn sample_issue_full() -> otto_issues::IssueFull {
        otto_issues::IssueFull {
            key: "AB-1".into(),
            id: "10001".into(),
            summary: "Sample issue summary".into(),
            status: "In Progress".into(),
            issue_type: "Story".into(),
            url: "https://example.atlassian.net/browse/AB-1".into(),
            description_md: "This is the **description**.".into(),
            assignee: Some(otto_issues::JiraUser {
                account_id: "u1".into(),
                display_name: "Alice Assignee".into(),
                avatar_url: None,
            }),
            reporter: Some(otto_issues::JiraUser {
                account_id: "u2".into(),
                display_name: "Bob Reporter".into(),
                avatar_url: None,
            }),
            priority: Some("High".into()),
            labels: vec!["backend".into(), "urgent".into()],
            fields: vec![otto_issues::JiraField {
                key: "customfield_10016".into(),
                name: "Story Points".into(),
                value: "5".into(),
            }],
            comments: vec![
                otto_issues::IssueComment {
                    id: "c1".into(),
                    author: "Carol Commenter".into(),
                    body_md: "First comment body.".into(),
                    created: "2026-06-01T10:00:00Z".into(),
                },
                otto_issues::IssueComment {
                    id: "c2".into(),
                    author: "Dave Commenter".into(),
                    body_md: "Second comment body.".into(),
                    created: "2026-06-02T11:00:00Z".into(),
                },
            ],
            history: vec![],
            attachments: vec![otto_issues::JiraAttachment {
                id: "a1".into(),
                filename: "diagram.png".into(),
                mime: "image/png".into(),
                size: 1024,
                created: "2026-06-01T09:00:00Z".into(),
                author: "Alice Assignee".into(),
            }],
            links: vec![otto_issues::JiraLink {
                rel: "blocks".into(),
                key: "AB-2".into(),
                summary: "Blocked issue".into(),
                status: "To Do".into(),
                issue_type: "Bug".into(),
            }],
            estimate: Some("5 pts".into()),
        }
    }

    #[test]
    fn issue_md_renders_everything() {
        let issue = sample_issue_full();
        let md = render_issue_md(&issue);
        assert!(md.contains(&issue.key) && md.contains(&issue.summary) && md.contains(&issue.url));
        assert!(md.contains(&issue.description_md));
        for c in &issue.comments {
            assert!(md.contains(&c.author) && md.contains(&c.body_md));
        }
        assert!(md.contains("Comments"));
    }

    #[test]
    fn issue_md_header_and_optional_sections() {
        let issue = sample_issue_full();
        let md = render_issue_md(&issue);
        assert!(md.contains("In Progress") && md.contains("Story"));
        assert!(md.contains("Alice Assignee") && md.contains("Bob Reporter"));
        assert!(md.contains("High"));
        assert!(md.contains("backend") && md.contains("urgent"));
        assert!(md.contains("5 pts"));
        assert!(md.contains("## Fields") && md.contains("Story Points"));
        assert!(md.contains("## Links") && md.contains("AB-2"));
        assert!(md.contains("## Attachments") && md.contains("diagram.png"));
    }

    #[test]
    fn issue_md_skips_empty_optional_sections() {
        let mut issue = sample_issue_full();
        issue.fields.clear();
        issue.links.clear();
        issue.attachments.clear();
        issue.comments.clear();
        issue.description_md.clear();
        let md = render_issue_md(&issue);
        assert!(!md.contains("## Fields"));
        assert!(!md.contains("## Links"));
        assert!(!md.contains("## Attachments"));
        assert!(!md.contains("## Comments"));
        assert!(md.contains("_No description._"));
    }
}
