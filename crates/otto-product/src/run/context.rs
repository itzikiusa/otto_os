//! What every product runner assembles before its agents start: the skill
//! body, the story context document the prompts point at, the work-graph
//! attribution and the folder pre-trust. One copy each — the analysis, retry,
//! rewrite, test and plan runners used to carry their own.

use std::collections::HashSet;

use otto_core::workref::WorkRef;
use otto_core::Id;
use tracing::warn;

use crate::host::ProductRunHost;

/// The body of skill `name`: the context library first, then the bundled
/// copy, else empty.
pub(super) fn skill_body<C: ProductRunHost>(ctx: &C, name: &str) -> String {
    ctx.context_library()
        .get_skill(name)
        .map(|s| s.body)
        .or_else(|| crate::skill_body(name).map(|s| s.to_string()))
        .unwrap_or_default()
}

/// Work-graph attribution for a session run on behalf of `story_id`, so its
/// cost lands on the story.
pub(super) fn story_work(story_id: &Id) -> Option<serde_json::Value> {
    serde_json::to_value(WorkRef {
        story_id: Some(story_id.clone()),
        origin: Some("product".into()),
        ..Default::default()
    })
    .ok()
}

/// The enriched story context document (body, Jira details, learnings,
/// `focus`). When it cannot be built, the bare story body under its title.
/// `tag` prefixes the warning (`product_run(rewrite)`, …).
pub(super) async fn story_context_md<C: ProductRunHost>(
    ctx: &C,
    story_id: &Id,
    title: &str,
    focus: Option<&str>,
    tag: &str,
) -> String {
    match ctx.product().build_agent_context(story_id, focus).await {
        Ok(md) => md,
        Err(e) => {
            warn!("{tag}: build_agent_context: {e}; falling back to story body");
            let body = match ctx.product_repo().latest_source_version(story_id).await {
                Ok(Some(v)) => v.body_md,
                _ => String::new(),
            };
            format!("# {title}\n\n## Story\n\n{body}\n")
        }
    }
}

/// Append the story's answered questions and its latest analysis summary to
/// a context document (the rewrite, test and plan writers need both).
pub(super) async fn append_answers_and_summary<C: ProductRunHost>(
    ctx: &C,
    story_id: &Id,
    context_md: &mut String,
) {
    let answered: Vec<otto_state::ProductQuestion> = ctx
        .product_repo()
        .list_questions(story_id)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|q| q.status == "answered")
        .collect();
    if !answered.is_empty() {
        context_md.push_str("\n## Answered Questions\n\n");
        for q in &answered {
            context_md.push_str(&format!("**Q:** {}\n", q.text));
            if let Some(ref ans) = q.answer {
                if !ans.trim().is_empty() {
                    context_md.push_str(&format!("**A:** {}\n", ans));
                }
            }
            context_md.push('\n');
        }
    }

    let analysis_summary = ctx
        .product_repo()
        .list_analyses(story_id)
        .await
        .unwrap_or_default()
        .into_iter()
        .max_by_key(|a| a.created_at)
        .map(|a| a.summary)
        .unwrap_or_default();
    if !analysis_summary.trim().is_empty() {
        context_md.push_str("\n## Analysis Summary\n\n");
        context_md.push_str(&analysis_summary);
        context_md.push('\n');
    }
}

/// Pre-trust every distinct provider on `cwd` (mirrors review) so no session
/// stalls on the interactive "trust this folder?" prompt and times out.
pub(super) fn trust_providers<'a>(providers: impl IntoIterator<Item = &'a str>, cwd: &str) {
    let mut trusted = HashSet::<&str>::new();
    for provider in providers {
        if trusted.insert(provider) {
            otto_sessions::trust::ensure_trusted(provider, cwd);
        }
    }
}
