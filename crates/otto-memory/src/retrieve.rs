//! Pure retrieval helpers: light re-rank signals over the keyword ranking and
//! compact, byte-budgeted context-brief formatting.

use crate::types::{Memory, MemoryHit};

/// Hard default byte cap for a rendered `## Relevant Workspace Memory` brief.
pub const MAX_CONTEXT_BRIEF_BYTES: usize = 2048;

const BRIEF_HEADING: &str = "## Relevant Workspace Memory\n\n";
const MAX_EXCERPT_BYTES: usize = 240;

pub struct RerankSignals {
    pub recency_days: f32,
    pub access_count: i64,
    pub confidence: f32,
    pub salience: f32,
    pub scope_match: bool,
}

/// Apply light priors on top of the fused base score.
pub fn rerank_score(base: f32, s: &RerankSignals, half_life_days: f32) -> f32 {
    let recency = 0.5f32.powf(s.recency_days / half_life_days.max(0.1));
    let usage = (1.0 + s.access_count as f32).ln();
    let scope = if s.scope_match { 0.15 } else { 0.0 };
    base * (1.0 + 0.3 * recency + 0.05 * usage + 0.2 * (s.confidence * s.salience)) + scope
}

/// Render already-ranked [`MemoryHit`]s into a tight, provenance-cited
/// `## Relevant Workspace Memory` markdown section capped at `max_bytes`.
/// Returns an empty string when `hits` is empty or nothing fits (strict no-op).
pub fn format_hits_brief(hits: &[MemoryHit], max_bytes: usize) -> String {
    let memories: Vec<&Memory> = hits.iter().map(|h| &h.memory).collect();
    format_memories_brief(&memories, max_bytes)
}

/// Render an ordered slice of [`Memory`] references into a byte-budgeted
/// `## Relevant Workspace Memory` section with provenance citations.
pub fn format_memories_brief(memories: &[&Memory], max_bytes: usize) -> String {
    if memories.is_empty() || max_bytes <= BRIEF_HEADING.len() + 4 {
        return String::new();
    }

    let mut out = String::with_capacity(max_bytes.min(MAX_CONTEXT_BRIEF_BYTES));
    out.push_str(BRIEF_HEADING);
    let mut count = 0usize;

    for m in memories {
        let bullet = format_memory_bullet(m);
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

fn format_memory_bullet(m: &Memory) -> String {
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

/// Defang inline text so code fences, newlines, and control characters cannot
/// break markdown structure or inject instructions.
fn defang_inline(s: &str) -> String {
    s.replace('`', "ʼ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Truncate `s` to at most `max_bytes` on a UTF-8 char boundary, appending `…`
/// when truncation occurs. Guaranteed `out.len() <= max_bytes`.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{MemoryRef, Scope};

    fn sample_mem(id: &str, title: &str, body: &str, source_ref: Option<&str>) -> Memory {
        Memory {
            id: id.into(),
            workspace_id: "ws1".into(),
            collection: "product".into(),
            record_type: "item".into(),
            scope: Scope::Workspace,
            story_id: None,
            kind: "decision".into(),
            title: title.into(),
            body: body.into(),
            entities: vec![],
            tags: vec![],
            source_kind: "manual".into(),
            source_ref: source_ref.map(str::to_string),
            refs: vec![],
            confidence: 0.9,
            salience: 0.9,
            content_hash: id.into(),
            active: true,
            superseded_by: None,
            version: 1,
            created_by: "u1".into(),
            created_at: "2026-06-01T10:00:00Z".into(),
            updated_at: "2026-06-10T10:00:00Z".into(),
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
    fn empty_memories_is_strict_noop() {
        assert_eq!(format_hits_brief(&[], MAX_CONTEXT_BRIEF_BYTES), "");
        assert_eq!(format_memories_brief(&[], MAX_CONTEXT_BRIEF_BYTES), "");
    }

    #[test]
    fn renders_provenance_citations_and_ref_fallback() {
        let m1 = sample_mem(
            "1",
            "Use SQLite WAL",
            "Always enable WAL mode",
            Some("ADR-42"),
        );
        let mut m2 = sample_mem("2", "Retry backoff", "Exponential jitter", None);
        m2.refs.push(MemoryRef {
            kind: "pr".into(),
            reference: "PR#128".into(),
            url: None,
            label: None,
        });
        let md = format_memories_brief(&[&m1, &m2], MAX_CONTEXT_BRIEF_BYTES);
        assert!(md.starts_with("## Relevant Workspace Memory\n\n"));
        assert!(md.contains("- [decision · workspace · 2026-06-10] **Use SQLite WAL** — Always enable WAL mode (source: ADR-42)"));
        assert!(md.contains("(source: PR#128)"));
    }

    #[test]
    fn enforces_byte_cap_on_multibyte_utf8_boundaries() {
        let mut items = Vec::new();
        for i in 0..30 {
            items.push(sample_mem(
                &format!("m{i}"),
                &format!("قرار تصميم {i} 🚀"),
                &"שלום עולם 日本語テスト ".repeat(8),
                Some("doc/arch.md"),
            ));
        }
        let refs: Vec<&Memory> = items.iter().collect();
        let md = format_memories_brief(&refs, MAX_CONTEXT_BRIEF_BYTES);
        assert!(!md.is_empty());
        assert!(
            md.len() <= MAX_CONTEXT_BRIEF_BYTES,
            "got {} bytes",
            md.len()
        );
        assert!(std::str::from_utf8(md.as_bytes()).is_ok());
    }
}
