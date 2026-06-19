//! E2E: structured product artifacts → extractors → memory → recall.

use chrono::Utc;

use otto_memory::{MemoryQuery, MemoryService, RecallOpts, SearchMode};
use otto_product::extract;
use otto_state::{ProductAnalysis, ProductQuestion};

async fn seeded_pool() -> sqlx::SqlitePool {
    let pool = otto_state::db::test_pool().await;
    sqlx::query("INSERT INTO workspaces (id, name, root_path, created_at) VALUES ('ws1','WS','/tmp/ws','2026-06-19T00:00:00+00:00')")
        .execute(&pool)
        .await
        .unwrap();
    pool
}

fn answered_question() -> ProductQuestion {
    ProductQuestion {
        id: "q1".into(),
        story_id: "s1".into(),
        analysis_id: None,
        text: "Which currencies are supported at launch?".into(),
        rationale: "scope".into(),
        category: "scope".into(),
        status: "answered".into(),
        answer: Some("USD only at launch; EUR is a fast-follow.".into()),
        posted_ref: None,
        created_by: "u1".into(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn analysis() -> ProductAnalysis {
    ProductAnalysis {
        id: "a1".into(),
        story_id: "s1".into(),
        source_version_id: None,
        status: "done".into(),
        summary: "- Decision: use webhooks instead of polling for settlement events\n- Constraint: withdrawals over 2000 USD require manual compliance review".into(),
        created_by: "u1".into(),
        created_at: Utc::now(),
        finished_at: None,
    }
}

#[tokio::test]
async fn product_artifacts_become_searchable_memories() {
    let pool = seeded_pool().await;
    let mem = MemoryService::with_defaults(pool);
    let (ws, by) = ("ws1", "u1");

    let mut items = Vec::new();
    if let Some(m) = extract::from_answered_question("s1", &answered_question()) {
        items.push(m);
    }
    items.extend(extract::from_analysis_summary("s1", &analysis()));
    assert!(items.len() >= 3, "expected qa + decision + constraint");
    mem.save(ws, by, items).await.unwrap();

    // Recall the answered question.
    let h1 = mem
        .search(
            ws,
            MemoryQuery {
                text: Some("currencies supported launch".into()),
                story_id: Some("s1".into()),
                mode: SearchMode::Hybrid,
                k: 5,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(h1.iter().any(|h| h.memory.body.contains("USD only")), "qa not recalled");

    // Recall the decision.
    let h2 = mem
        .search(
            ws,
            MemoryQuery {
                text: Some("webhooks settlement".into()),
                story_id: Some("s1".into()),
                mode: SearchMode::Hybrid,
                k: 5,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(h2.iter().any(|h| h.memory.kind == "decision"), "decision not recalled");
}

#[tokio::test]
async fn recall_brief_groups_story_memories() {
    let pool = seeded_pool().await;
    let mem = MemoryService::with_defaults(pool);
    let (ws, by) = ("ws1", "u1");

    let mut items = vec![extract::from_answered_question("s1", &answered_question()).unwrap()];
    items.extend(extract::from_analysis_summary("s1", &analysis()));
    mem.save(ws, by, items).await.unwrap();

    let brief = mem
        .recall_brief(
            ws,
            "s1",
            RecallOpts {
                focus: None,
                token_budget: 1000,
                kinds: vec![],
            },
        )
        .await
        .unwrap();

    assert!(!brief.sections.is_empty(), "brief should have sections");
    let headings: Vec<&str> = brief.sections.iter().map(|s| s.heading.as_str()).collect();
    assert!(
        headings.iter().any(|h| h.contains("Constraints") || h.contains("Decisions") || h.contains("Answered")),
        "expected grouped headings, got {headings:?}"
    );
    assert!(brief.token_estimate <= 1000);
}
