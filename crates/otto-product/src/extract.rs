//! Deterministic extractors: turn structured product artifacts (answered
//! questions, learnings, analysis summaries, story versions) into atomic
//! `NewMemory` records. Pure functions — no I/O, no LLM.

use otto_state::memory::{NewMemory, Scope};
use otto_state::{ProductAnalysis, ProductLearning, ProductQuestion, ProductStoryVersion};

fn truncate(s: &str, n: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let t: String = s.chars().take(n.saturating_sub(1)).collect();
        format!("{t}…")
    }
}

fn parse_tags(raw: &str) -> Vec<String> {
    if let Ok(v) = serde_json::from_str::<Vec<String>>(raw) {
        return v;
    }
    raw.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// An answered clarifying question → a `qa` memory (skips unanswered).
pub fn from_answered_question(story_id: &str, q: &ProductQuestion) -> Option<NewMemory> {
    let answer = q.answer.as_deref().unwrap_or("").trim();
    if q.status != "answered" || answer.is_empty() {
        return None;
    }
    Some(NewMemory {
        collection: "product".into(),
        record_type: "item".into(),
        scope: Scope::Story,
        story_id: Some(story_id.into()),
        kind: "qa".into(),
        title: truncate(&q.text, 80),
        body: format!("Q: {}\nA: {answer}", q.text.trim()),
        entities: vec![],
        tags: vec![],
        source_kind: "question".into(),
        source_ref: Some(q.id.clone()),
        refs: vec![],
        confidence: Some(0.9),
        salience: Some(0.6),
    })
}

/// A workspace learning → a `learning` memory.
pub fn from_learning(l: &ProductLearning) -> NewMemory {
    NewMemory {
        collection: "product".into(),
        record_type: "item".into(),
        scope: Scope::Workspace,
        story_id: l.source_story_id.clone(),
        kind: "learning".into(),
        title: truncate(&l.title, 80),
        body: l.body.clone(),
        entities: vec![],
        tags: parse_tags(&l.tags),
        source_kind: "learning".into(),
        source_ref: Some(l.id.clone()),
        refs: vec![],
        confidence: Some(0.8),
        salience: Some(0.6),
    }
}

/// Classify a summary bullet into a memory `kind` by its leading marker.
fn classify_line(line: &str) -> &'static str {
    let l = line.to_lowercase();
    if l.starts_with("decision") || l.starts_with("decided") {
        "decision"
    } else if l.starts_with("constraint")
        || l.starts_with("must ")
        || l.starts_with("never ")
        || l.starts_with("cannot ")
    {
        "constraint"
    } else if l.starts_with("requirement") || l.starts_with("require") {
        "requirement"
    } else {
        "fact"
    }
}

/// An analysis summary → one memory per meaningful bullet, kind-classified.
pub fn from_analysis_summary(story_id: &str, a: &ProductAnalysis) -> Vec<NewMemory> {
    a.summary
        .lines()
        .filter_map(|line| {
            let t = line
                .trim()
                .trim_start_matches(['-', '*', '•', '#', ' '])
                .trim();
            if t.chars().count() < 8 {
                return None;
            }
            let kind = classify_line(t);
            Some(NewMemory {
                collection: "product".into(),
                record_type: "item".into(),
                scope: Scope::Story,
                story_id: Some(story_id.into()),
                kind: kind.into(),
                title: truncate(t, 80),
                body: t.to_string(),
                entities: vec![],
                tags: vec![],
                source_kind: "analysis".into(),
                source_ref: Some(a.id.clone()),
                refs: vec![],
                confidence: Some(0.7),
                salience: Some(0.5),
            })
        })
        .collect()
}

/// A story version → a `summary` memory (a recap of the version body).
pub fn from_version(story_id: &str, v: &ProductStoryVersion) -> Option<NewMemory> {
    let body = v.body_md.trim();
    if body.is_empty() {
        return None;
    }
    Some(NewMemory {
        collection: "product".into(),
        record_type: "item".into(),
        scope: Scope::Story,
        story_id: Some(story_id.into()),
        kind: "summary".into(),
        title: truncate(&v.title, 80),
        body: truncate(body, 1200),
        entities: vec![],
        tags: vec![],
        source_kind: "version".into(),
        source_ref: Some(v.id.clone()),
        refs: vec![],
        confidence: Some(0.6),
        salience: Some(0.4),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn question(status: &str, answer: Option<&str>) -> ProductQuestion {
        ProductQuestion {
            id: "q1".into(),
            story_id: "s1".into(),
            analysis_id: None,
            text: "Which currencies are supported?".into(),
            rationale: "scope".into(),
            category: "scope".into(),
            status: status.into(),
            answer: answer.map(|s| s.into()),
            posted_ref: None,
            created_by: "u1".into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn answered_question_becomes_qa() {
        let m = from_answered_question("s1", &question("answered", Some("USD only at launch"))).unwrap();
        assert_eq!(m.kind, "qa");
        assert_eq!(m.scope, Scope::Story);
        assert!(m.body.contains("USD only"));
        assert_eq!(m.source_kind, "question");
    }

    #[test]
    fn unanswered_question_is_skipped() {
        assert!(from_answered_question("s1", &question("open", None)).is_none());
        assert!(from_answered_question("s1", &question("answered", Some("   "))).is_none());
    }

    #[test]
    fn analysis_summary_classifies_bullets() {
        let a = ProductAnalysis {
            id: "a1".into(),
            story_id: "s1".into(),
            source_version_id: None,
            status: "done".into(),
            summary: "- Decision: use webhooks not polling\n- Constraint: withdrawals over 2000 need manual review\n- The rate limit is 100/min\n- ok".into(),
            created_by: "u1".into(),
            created_at: Utc::now(),
            finished_at: None,
        };
        let ms = from_analysis_summary("s1", &a);
        // "ok" (< 8 chars) is dropped → 3 memories
        assert_eq!(ms.len(), 3);
        assert!(ms.iter().any(|m| m.kind == "decision"));
        assert!(ms.iter().any(|m| m.kind == "constraint"));
        assert!(ms.iter().any(|m| m.kind == "fact"));
    }
}
