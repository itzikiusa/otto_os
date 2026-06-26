//! Repo rules → Context Engine bridge.
//!
//! Repo rules (DB, `repo_rules` table) are the single source of truth. Whenever
//! they change we render all *enabled* rules for a workspace into the
//! per-workspace `WorkspaceContextConfig.repo_rules_md` field — a machine-managed
//! block kept SEPARATE from the user-owned `extra_context_md` (so a user context
//! edit never clobbers it; the `PUT /context` handler preserves it). The
//! Provisioner then injects that block into every future agent session's
//! instruction file (`build_block` renders a `## Repo Rules (from code review)`
//! section). Net effect: a lesson learned in review is enforced as context on
//! every subsequent agent run — review feeds the Context Engine.

use otto_core::finding::RepoRule;
use otto_core::Result;

use crate::state::ServerCtx;

/// Render the enabled repo rules into a compact markdown block injected into
/// agent instruction files. Empty when there are no enabled rules.
pub fn render_repo_rules_block(rules: &[RepoRule]) -> String {
    let mut out = String::new();
    for r in rules.iter().filter(|r| r.enabled) {
        let mut head = format!("- **{}**", r.title.trim());
        let mut tags: Vec<String> = Vec::new();
        if let Some(c) = r.category.as_deref().filter(|s| !s.trim().is_empty()) {
            tags.push(c.trim().to_string());
        }
        if let Some(s) = r.severity.as_deref().filter(|s| !s.trim().is_empty()) {
            tags.push(s.trim().to_string());
        }
        if let Some(g) = r.glob.as_deref().filter(|s| !s.trim().is_empty()) {
            tags.push(format!("scope: {}", g.trim()));
        }
        if !tags.is_empty() {
            head.push_str(&format!(" _({})_", tags.join(", ")));
        }
        out.push_str(&head);
        out.push('\n');
        for line in r.body.trim().lines() {
            out.push_str("  ");
            out.push_str(line);
            out.push('\n');
        }
    }
    out.trim_end().to_string()
}

/// Re-render the workspace's enabled repo rules into its `repo_rules_md` context
/// field (DB → Context Engine). Idempotent: always renders the full current set,
/// so it converges regardless of the order of concurrent rule mutations.
pub async fn apply_repo_rules_to_context(ctx: &ServerCtx, workspace_id: &str) -> Result<()> {
    let rules = ctx.repo_rules_store.list_enabled(workspace_id).await?;
    let block = render_repo_rules_block(&rules);
    let ws = ctx.workspaces.get(&workspace_id.to_string()).await?;
    let mut cfg = otto_context::config::from_settings(&ws.settings);
    cfg.repo_rules_md = block;
    let merged = otto_context::config::write_into_settings(&ws.settings, &cfg);
    ctx.workspaces
        .update(&workspace_id.to_string(), None, None, Some(&merged), None)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(title: &str, body: &str, enabled: bool) -> RepoRule {
        RepoRule {
            id: "r".into(),
            workspace_id: "ws".into(),
            title: title.into(),
            body: body.into(),
            category: Some("security".into()),
            severity: Some("high".into()),
            glob: None,
            source_finding_id: None,
            enabled,
            created_by: "u".into(),
            created_at: "now".into(),
            updated_at: "now".into(),
        }
    }

    #[test]
    fn renders_enabled_rules_only() {
        let rules = vec![
            rule("Never build SQL with format!", "Use parameterized queries.", true),
            rule("Disabled rule", "should not appear", false),
        ];
        let block = render_repo_rules_block(&rules);
        assert!(block.contains("Never build SQL with format!"));
        assert!(block.contains("Use parameterized queries."));
        assert!(block.contains("security, high"));
        assert!(!block.contains("Disabled rule"));
    }

    #[test]
    fn empty_when_no_enabled() {
        assert_eq!(render_repo_rules_block(&[]), "");
        assert_eq!(render_repo_rules_block(&[rule("x", "y", false)]), "");
    }
}
