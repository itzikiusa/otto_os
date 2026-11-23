//! Intent-aware workspace memory recall & compact brief formatting for
//! `CONTEXT.md` materialization (E-03).
//!
//! Filters workspace memories to active, caller-visible rows matching optional
//! scope/tag criteria, ranks them with recency + salience + intent-keyword
//! signals (matching `otto-memory`'s re-ranker), and renders a tight,
//! byte-budgeted (`<= 2 KiB` default), provenance-cited
//! `## Relevant Workspace Memory` section. Returns an empty string when no
//! memories qualify so callers produce zero change to `CONTEXT.md`.

use std::cmp::Ordering;
use std::path::Path;

use chrono::{DateTime, Utc};
use otto_state::memory::{ListFilter, Memory, Scope};
use otto_state::MemoriesRepo;

/// Default hard byte cap for the rendered `## Relevant Workspace Memory` section.
pub const MAX_MEMORY_BRIEF_BYTES: usize = 2048;

/// Default recency half-life in days (mirrors `otto-memory::service`).
pub const DEFAULT_RECENCY_HALF_LIFE_DAYS: f32 = 30.0;

const BRIEF_HEADING: &str = "## Relevant Workspace Memory\n\n";
const MAX_EXCERPT_BYTES: usize = 240;
const FETCH_LIMIT: i64 = 200;

/// Filter & ranking options for assembling a workspace memory brief.
#[derive(Clone, Debug, Default)]
pub struct MemoryBriefFilter {
    /// Restrict to a specific memory scope (`Workspace`, `Story`, `Entity`).
    pub scope: Option<Scope>,
    /// Restrict to (and boost) a specific story id.
    pub story_id: Option<String>,
    /// Keep memories carrying at least one of these tags (case-insensitive).
    pub tags: Vec<String>,
    /// Optional intent keywords (repo name, branch name, or user task terms)
    /// used to boost matching memories above general workspace memories.
    pub query: Option<String>,
    /// Authenticated non-root viewer id; hides other users' `private` memories.
    /// `None` = root/internal caller (sees all visibilities).
    pub viewer: Option<String>,
    /// Recency decay half-life in days (defaults to [`DEFAULT_RECENCY_HALF_LIFE_DAYS`]).
    pub recency_half_life_days: Option<f32>,
    /// Hard byte cap for the rendered section (defaults to [`MAX_MEMORY_BRIEF_BYTES`]).
    pub max_bytes: Option<usize>,
}

/// Best-effort loader: queries active, caller-visible memories for `workspace_id`
/// and renders them via [`format_memory_brief`]. DB errors are logged and yield
/// an empty string so memory recall never blocks context provisioning.
pub async fn load_workspace_memory_brief(
    repo: &MemoriesRepo,
    workspace_id: &str,
    filter: &MemoryBriefFilter,
) -> String {
    let list_filter = ListFilter {
        story_id: filter.story_id.clone(),
        include_inactive: false,
        limit: FETCH_LIMIT,
        viewer: filter.viewer.clone(),
        ..Default::default()
    };
    match repo.list(workspace_id, &list_filter).await {
        Ok(memories) => format_memory_brief(&memories, filter),
        Err(e) => {
            tracing::warn!(
                workspace_id = %workspace_id,
                error = %e,
                "load workspace memories for context brief failed"
            );
            String::new()
        }
    }
}

/// Derive lightweight intent keywords from a workspace's `cwd` (repo directory
/// name + active git branch from `.git/HEAD`, pure file read) and workspace name.
pub fn intent_query_for_cwd(cwd: &str, workspace_name: &str) -> Option<String> {
    let mut terms: Vec<String> = Vec::new();
    let ws = workspace_name.trim();
    if !ws.is_empty() {
        terms.push(ws.to_string());
    }
    let cwd_path = Path::new(cwd);
    if let Some(dir_name) = cwd_path.file_name().and_then(|s| s.to_str()) {
        let dir_name = dir_name.trim();
        if !dir_name.is_empty() && !dir_name.eq_ignore_ascii_case(ws) {
            terms.push(dir_name.to_string());
        }
    }
    if let Some(branch) = read_git_branch(cwd_path) {
        if !matches!(branch.as_str(), "main" | "master" | "HEAD") {
            terms.push(branch);
        }
    }
    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" "))
    }
}

/// Filter, recency-rank, and render `memories` into a byte-budgeted
/// `## Relevant Workspace Memory` markdown section with provenance citations.
/// Returns `""` when no memories qualify (strict no-op).
pub fn format_memory_brief(memories: &[Memory], filter: &MemoryBriefFilter) -> String {
    format_memory_brief_at(memories, filter, Utc::now())
}

/// Deterministic variant of [`format_memory_brief`] with an explicit clock for tests.
pub fn format_memory_brief_at(
    memories: &[Memory],
    filter: &MemoryBriefFilter,
    now: DateTime<Utc>,
) -> String {
    let max_bytes = filter.max_bytes.unwrap_or(MAX_MEMORY_BRIEF_BYTES);
    if memories.is_empty() || max_bytes <= BRIEF_HEADING.len() + 4 {
        return String::new();
    }

    let half_life = filter
        .recency_half_life_days
        .unwrap_or(DEFAULT_RECENCY_HALF_LIFE_DAYS)
        .max(0.1);
    let query_terms = tokenize_query(filter.query.as_deref());

    let mut ranked: Vec<(&Memory, f32)> = memories
        .iter()
        .filter(|m| is_eligible(m, filter))
        .map(|m| (m, score_memory(m, filter, &query_terms, half_life, now)))
        .collect();

    if ranked.is_empty() {
        return String::new();
    }

    ranked.sort_by(|(ma, sa), (mb, sb)| {
        sb.partial_cmp(sa)
            .unwrap_or(Ordering::Equal)
            .then_with(|| mb.updated_at.cmp(&ma.updated_at))
            .then_with(|| ma.id.cmp(&mb.id))
    });

    let mut out = String::with_capacity(max_bytes.min(MAX_MEMORY_BRIEF_BYTES));
    out.push_str(BRIEF_HEADING);
    let mut count = 0usize;

    for (m, _score) in ranked {
        let bullet = format_bullet(m);
        if bullet.is_empty() {
            continue;
        }
        let sep = if count > 0 { "\n" } else { "" };
        if out.len() + sep.len() + bullet.len() <= max_bytes {
            out.push_str(sep);
            out.push_str(&bullet);
            count += 1;
            continue;
        }
        let remaining = max_bytes.saturating_sub(out.len() + sep.len());
        if remaining >= 24 {
            let truncated = truncate_utf8_ellipsis(&bullet, remaining);
            if !truncated.is_empty() {
                out.push_str(sep);
                out.push_str(&truncated);
                count += 1;
            }
        }
        break;
    }

    if count == 0 {
        String::new()
    } else {
        out
    }
}

fn is_eligible(m: &Memory, filter: &MemoryBriefFilter) -> bool {
    if !m.active || m.forgotten_at.is_some() {
        return false;
    }
    if let Some(viewer) = filter.viewer.as_deref() {
        if m.visibility != "shared" && m.created_by != viewer {
            return false;
        }
    }
    if let Some(scope) = filter.scope {
        if m.scope != scope {
            return false;
        }
    }
    if let Some(sid) = filter.story_id.as_deref() {
        if m.story_id.as_deref() != Some(sid) {
            return false;
        }
    }
    if !filter.tags.is_empty()
        && !filter
            .tags
            .iter()
            .any(|tag| m.tags.iter().any(|mt| mt.eq_ignore_ascii_case(tag)))
    {
        return false;
    }
    true
}

fn score_memory(
    m: &Memory,
    filter: &MemoryBriefFilter,
    query_terms: &[String],
    half_life: f32,
    now: DateTime<Utc>,
) -> f32 {
    let recency_days = elapsed_days(&m.updated_at, &m.created_at, now);
    let recency = 0.5_f32.powf(recency_days / half_life);
    let usage = (1.0 + m.access_count.max(0) as f32).ln();
    let quality = m.confidence.clamp(0.0, 1.0) * m.salience.clamp(0.0, 1.0);

    let intent_boost = if query_terms.is_empty() {
        0.0
    } else {
        let haystack = format!(
            "{} {} {} {}",
            m.title.to_lowercase(),
            m.body.to_lowercase(),
            m.tags.join(" ").to_lowercase(),
            m.entities.join(" ").to_lowercase()
        );
        let matched = query_terms
            .iter()
            .filter(|t| haystack.contains(t.as_str()))
            .count();
        1.5 * (matched as f32 / query_terms.len() as f32)
    };

    let scope_boost =
        if filter.story_id.is_some() && filter.story_id.as_deref() == m.story_id.as_deref() {
            0.15
        } else if m.scope == Scope::Workspace {
            0.10
        } else {
            0.0
        };

    let base = 1.0 + intent_boost;
    base * (1.0 + 0.3 * recency + 0.05 * usage + 0.2 * quality) + scope_boost
}

fn elapsed_days(updated_at: &str, created_at: &str, now: DateTime<Utc>) -> f32 {
    let parsed = DateTime::parse_from_rfc3339(updated_at.trim())
        .or_else(|_| DateTime::parse_from_rfc3339(created_at.trim()))
        .ok()
        .map(|dt| dt.with_timezone(&Utc));
    let Some(ts) = parsed else {
        return 0.0;
    };
    let secs = (now - ts).num_seconds().max(0) as f64;
    (secs / 86_400.0) as f32
}

fn tokenize_query(query: Option<&str>) -> Vec<String> {
    let Some(q) = query else {
        return Vec::new();
    };
    let mut terms: Vec<String> = q
        .split(|c: char| !c.is_alphanumeric())
        .map(|t| t.trim().to_lowercase())
        .filter(|t| t.len() >= 2)
        .collect();
    terms.sort();
    terms.dedup();
    terms
}

fn format_bullet(m: &Memory) -> String {
    let date = short_date(&m.updated_at)
        .or_else(|| short_date(&m.created_at))
        .unwrap_or("unknown");
    let title = defang_inline(&m.title);
    let body = truncate_utf8_ellipsis(&defang_inline(&m.body), MAX_EXCERPT_BYTES);
    let content = match (title.is_empty(), body.is_empty()) {
        (false, false) => format!("**{title}** — {body}"),
        (false, true) => format!("**{title}**"),
        (true, false) => body,
        (true, true) => return String::new(),
    };
    let source = provenance_label(m);
    let suffix = source
        .map(|s| format!(" (source: {s})"))
        .unwrap_or_default();
    format!(
        "- [{} · {} · {date}] {content}{suffix}",
        defang_inline(&m.kind),
        m.scope.as_str(),
    )
}

fn provenance_label(m: &Memory) -> Option<String> {
    if let Some(sr) = m
        .source_ref
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Some(defang_inline(sr));
    }
    if let Some(r) = m.refs.first() {
        let rf = r.reference.trim();
        if !rf.is_empty() {
            return Some(defang_inline(rf));
        }
    }
    let sk = m.source_kind.trim();
    if !sk.is_empty() {
        return Some(defang_inline(sk));
    }
    None
}

fn short_date(ts: &str) -> Option<&str> {
    let s = ts.trim();
    let prefix = s.get(0..10)?;
    let b = prefix.as_bytes();
    if b[4] == b'-'
        && b[7] == b'-'
        && b[0..4].iter().all(u8::is_ascii_digit)
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[8..10].iter().all(u8::is_ascii_digit)
    {
        Some(prefix)
    } else {
        None
    }
}

/// Defang untrusted text so code fences or newlines cannot break out of the
/// memory bullet or inject markdown headings into `CONTEXT.md`.
fn defang_inline(s: &str) -> String {
    s.replace('`', "ʼ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Truncate `s` to at most `max_bytes` on a valid UTF-8 char boundary,
/// appending `"…"` when shortened. Guaranteed `out.len() <= max_bytes`.
fn truncate_utf8_ellipsis(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    const ELLIPSIS: &str = "…";
    if max_bytes <= ELLIPSIS.len() {
        let mut end = max_bytes.min(s.len());
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        return s[..end].to_string();
    }
    let mut end = max_bytes - ELLIPSIS.len();
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = s[..end].trim_end().to_string();
    out.push_str(ELLIPSIS);
    out
}

/// Read the current branch name from `<cwd>/.git/HEAD` without spawning `git`.
fn read_git_branch(cwd: &Path) -> Option<String> {
    let git_entry = cwd.join(".git");
    let head_path = if git_entry.is_dir() {
        git_entry.join("HEAD")
    } else if git_entry.is_file() {
        // Worktree `.git` file: `gitdir: <path>`
        let raw = std::fs::read_to_string(&git_entry).ok()?;
        let rel = raw.trim().strip_prefix("gitdir:")?.trim();
        let dir = Path::new(rel);
        if dir.is_absolute() {
            dir.join("HEAD")
        } else {
            cwd.join(dir).join("HEAD")
        }
    } else {
        return None;
    };
    let head = std::fs::read_to_string(head_path).ok()?;
    let branch = head
        .trim()
        .strip_prefix("ref: refs/heads/")
        .map(str::trim)
        .filter(|b| !b.is_empty())?;
    Some(branch.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_state::memory::MemoryRef;

    fn make_memory(
        id: &str,
        kind: &str,
        scope: Scope,
        title: &str,
        body: &str,
        updated_at: &str,
    ) -> Memory {
        Memory {
            id: id.into(),
            workspace_id: "ws1".into(),
            collection: "product".into(),
            record_type: "item".into(),
            scope,
            story_id: None,
            kind: kind.into(),
            title: title.into(),
            body: body.into(),
            entities: vec![],
            tags: vec![],
            source_kind: "manual".into(),
            source_ref: None,
            refs: vec![],
            confidence: 0.9,
            salience: 0.9,
            content_hash: id.into(),
            active: true,
            superseded_by: None,
            version: 1,
            created_by: "alice".into(),
            created_at: updated_at.into(),
            updated_at: updated_at.into(),
            last_accessed_at: None,
            access_count: 0,
            expires_at: None,
            visibility: "shared".into(),
            state: "accepted".into(),
            provenance_json: None,
            forgotten_at: None,
            undo_token: None,
        }
    }

    #[test]
    fn empty_and_filtered_memories_are_strict_noop() {
        let now = DateTime::parse_from_rfc3339("2026-06-15T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            format_memory_brief_at(&[], &MemoryBriefFilter::default(), now),
            ""
        );

        let mut inactive = make_memory(
            "1",
            "fact",
            Scope::Workspace,
            "Inactive",
            "should be skipped",
            "2026-06-14T12:00:00Z",
        );
        inactive.active = false;

        let mut other_private = make_memory(
            "2",
            "fact",
            Scope::Workspace,
            "Secret",
            "bob private note",
            "2026-06-14T12:00:00Z",
        );
        other_private.visibility = "private".into();
        other_private.created_by = "bob".into();

        let out = format_memory_brief_at(
            &[inactive, other_private],
            &MemoryBriefFilter {
                viewer: Some("alice".into()),
                ..Default::default()
            },
            now,
        );
        assert_eq!(out, "");
    }

    #[test]
    fn byte_budget_truncates_cleanly_at_utf8_boundary() {
        let now = DateTime::parse_from_rfc3339("2026-06-15T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let mut memories = Vec::new();
        for i in 0..40 {
            let mut m = make_memory(
                &format!("m{i:02}"),
                "decision",
                Scope::Workspace,
                &format!("قرار هندسي {i} 🚀"),
                &"שלום עולם 日本語メモリテスト café résumé ".repeat(6),
                "2026-06-14T12:00:00Z",
            );
            m.source_ref = Some(format!("ADR-{i}"));
            memories.push(m);
        }

        let brief = format_memory_brief_at(&memories, &MemoryBriefFilter::default(), now);
        assert!(brief.starts_with("## Relevant Workspace Memory\n\n"));
        assert!(
            brief.len() <= MAX_MEMORY_BRIEF_BYTES,
            "brief exceeded 2 KiB cap: {} bytes",
            brief.len()
        );
        assert!(std::str::from_utf8(brief.as_bytes()).is_ok());

        // Also test a tight custom cap that forces mid-bullet truncation across multi-byte UTF-8.
        let tight = format_memory_brief_at(
            &memories[..1],
            &MemoryBriefFilter {
                max_bytes: Some(95),
                ..Default::default()
            },
            now,
        );
        assert!(!tight.is_empty());
        assert!(tight.len() <= 95, "tight brief was {} bytes", tight.len());
        assert!(tight.ends_with('…'));
    }

    #[test]
    fn recency_weighting_and_scope_tag_filtering() {
        let now = DateTime::parse_from_rfc3339("2026-06-15T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let mut old_ws = make_memory(
            "old",
            "fact",
            Scope::Workspace,
            "Legacy cache TTL",
            "Use 60s cache TTL for payments",
            "2024-01-01T00:00:00Z",
        );
        old_ws.tags = vec!["payments".into()];

        let mut fresh_ws = make_memory(
            "fresh",
            "decision",
            Scope::Workspace,
            "New cache TTL",
            "Use 15s cache TTL for payments",
            "2026-06-14T12:00:00Z",
        );
        fresh_ws.tags = vec!["payments".into()];

        let mut story_other = make_memory(
            "story",
            "constraint",
            Scope::Story,
            "Billing story rule",
            "Requires dual signoff",
            "2026-06-15T10:00:00Z",
        );
        story_other.tags = vec!["billing".into()];

        // Filter to Workspace scope + payments tag (case-insensitive) => fresh ranks above old.
        let brief = format_memory_brief_at(
            &[old_ws.clone(), fresh_ws.clone(), story_other.clone()],
            &MemoryBriefFilter {
                scope: Some(Scope::Workspace),
                tags: vec!["PAYMENTS".into()],
                ..Default::default()
            },
            now,
        );
        let fresh_pos = brief.find("**New cache TTL**").expect("fresh present");
        let old_pos = brief.find("**Legacy cache TTL**").expect("old present");
        assert!(
            fresh_pos < old_pos,
            "fresh memory should rank above stale memory:\n{brief}"
        );
        assert!(
            !brief.contains("Billing story rule"),
            "story-scoped / other-tag memory must be filtered out"
        );

        // Intent query boosts matching memory even across equal recency.
        let mut a = make_memory(
            "a",
            "fact",
            Scope::Workspace,
            "General note",
            "Logging uses tracing crate",
            "2026-06-14T12:00:00Z",
        );
        a.tags = vec!["infra".into()];
        let mut b = make_memory(
            "b",
            "fact",
            Scope::Workspace,
            "Checkout retries",
            "Checkout retries max 3 times with idempotency key",
            "2026-06-14T12:00:00Z",
        );
        b.tags = vec!["checkout".into()];

        let intent_brief = format_memory_brief_at(
            &[a, b],
            &MemoryBriefFilter {
                query: Some("feat/checkout-idempotency".into()),
                ..Default::default()
            },
            now,
        );
        let b_pos = intent_brief.find("**Checkout retries**").unwrap();
        let a_pos = intent_brief.find("**General note**").unwrap();
        assert!(
            b_pos < a_pos,
            "intent-matching memory should rank first:\n{intent_brief}"
        );
    }

    #[test]
    fn provenance_citations_include_kind_scope_date_and_source_ref() {
        let now = DateTime::parse_from_rfc3339("2026-06-15T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let mut with_source_ref = make_memory(
            "1",
            "decision",
            Scope::Workspace,
            "SQLite WAL",
            "Keep WAL mode on",
            "2026-06-12T08:00:00Z",
        );
        with_source_ref.source_ref = Some("docs/adr-007.md".into());

        let mut with_ref_fallback = make_memory(
            "2",
            "constraint",
            Scope::Story,
            "No SELECT *",
            "Always name columns",
            "2026-06-11T08:00:00Z",
        );
        with_ref_fallback.refs.push(MemoryRef {
            kind: "jira".into(),
            reference: "OTTO-412".into(),
            url: None,
            label: None,
        });

        let brief = format_memory_brief_at(
            &[with_source_ref, with_ref_fallback],
            &MemoryBriefFilter::default(),
            now,
        );
        assert!(brief.contains(
            "- [decision · workspace · 2026-06-12] **SQLite WAL** — Keep WAL mode on (source: docs/adr-007.md)"
        ));
        assert!(brief.contains(
            "- [constraint · story · 2026-06-11] **No SELECT *** — Always name columns (source: OTTO-412)"
        ));
    }

    #[test]
    fn intent_query_extracts_workspace_dir_and_git_branch() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        std::fs::write(
            dir.path().join(".git/HEAD"),
            "ref: refs/heads/feat/memory-recall\n",
        )
        .unwrap();
        let q = intent_query_for_cwd(&dir.path().to_string_lossy(), "otto")
            .expect("query should be derived");
        assert!(q.contains("otto"));
        assert!(q.contains("feat/memory-recall"));
    }
}
