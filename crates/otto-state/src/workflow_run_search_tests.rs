use super::*;
use crate::WorkflowsRepo;
use otto_core::workflows::NodeRunState;
use serde_json::json;

async fn pool() -> DbPool {
    let opts = sqlx::sqlite::SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(false);
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .unwrap();
    sqlx::migrate!().run(&pool).await.unwrap();
    pool.into()
}

fn graph(step: &str) -> WorkflowGraph {
    serde_json::from_value(json!({
        "nodes": [{"id": "t", "kind": "manual_trigger"}, {"id": "r", "kind": "review_run", "name": step}],
        "edges": []
    }))
    .unwrap()
}

/// Two workflows in "ws", one in "other"; returns the repo + both workflow ids.
async fn seed(pool: &DbPool) -> (WorkflowsRepo, Id, Id) {
    let repo = WorkflowsRepo::new(pool.clone());
    let a = repo
        .create(
            &"ws".into(),
            "PR Reviewer",
            "",
            "",
            &graph("Multi-Agent PR Review"),
            &"u".into(),
        )
        .await
        .unwrap();
    let b = repo
        .create(
            &"ws".into(),
            "Nightly Docs",
            "",
            "",
            &graph("Write docs"),
            &"u".into(),
        )
        .await
        .unwrap();
    let other = repo
        .create(
            &"other".into(),
            "PR Reviewer",
            "",
            "",
            &graph("x"),
            &"u".into(),
        )
        .await
        .unwrap();
    let inputs = [
        (
            &a.id,
            json!({"trigger": "chat", "msg": "Review the payments PR please", "working_directory": "/src/sinatra-games-go", "pr": 15, "pr_branch": "feature/PAY-1"}),
        ),
        (
            &a.id,
            json!({"trigger": "chat", "msg": "Review gateway", "working_directory": "/src/sinatra-gateway-go,/src/other", "pr": "gw#7", "base": "develop"}),
        ),
        (
            &b.id,
            json!({"trigger": "schedule", "prompt": "Regenerate the 100% docs_index"}),
        ),
        (&b.id, json!({})),
    ];
    for (wf, input) in inputs {
        repo.create_run(wf, &"ws".into(), &input, None)
            .await
            .unwrap();
        // Distinct, ordered started_at values.
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    repo.create_run(
        &other.id,
        &"other".into(),
        &json!({"msg": "payments"}),
        None,
    )
    .await
    .unwrap();
    (repo, a.id, b.id)
}

fn search(q: &str) -> RunSearch {
    RunSearch {
        workspace_id: "ws".into(),
        q: Some(q.into()),
        ..Default::default()
    }
}

#[tokio::test]
async fn open_text_matches_message_repo_branch_pr_steps_and_workflow_name() {
    let pool = pool().await;
    let (_, a, _) = seed(&pool).await;
    let hit = |q: &'static str| {
        let pool = pool.clone();
        async move {
            search_runs(&pool, &search(q))
                .await
                .unwrap()
                .items
                .into_iter()
                .map(|i| i.message.unwrap_or_default())
                .collect::<Vec<_>>()
        }
    };
    assert_eq!(hit("PAYMENTS").await, vec!["Review the payments PR please"]); // case-insensitive, other ws excluded
    assert_eq!(hit("gateway-go").await, vec!["Review gateway"]); // repo path
    assert_eq!(hit("PAY-1").await, vec!["Review the payments PR please"]); // branch
    assert_eq!(hit("gw#7").await, vec!["Review gateway"]); // PR
    assert_eq!(hit("Multi-Agent").await.len(), 2); // step name (graph)
    assert_eq!(hit("nightly").await.len(), 2); // workflow name
    assert_eq!(hit("review develop").await, vec!["Review gateway"]); // every term must match
                                                                     // LIKE wildcards in the query are literal.
    assert_eq!(hit("100%").await, vec!["Regenerate the 100% docs_index"]);
    assert!(hit("10_%").await.is_empty());
    // Item fields.
    let page = search_runs(&pool, &search("payments")).await.unwrap();
    let it = &page.items[0];
    assert_eq!(it.workflow_id, a);
    assert_eq!(it.workflow_name, "PR Reviewer");
    assert_eq!(it.trigger, "chat");
    assert_eq!(it.repo.as_deref(), Some("sinatra-games-go"));
    assert_eq!(it.branch.as_deref(), Some("feature/PAY-1"));
    assert_eq!(it.pr.as_deref(), Some("15"));
    let none = search_runs(
        &pool,
        &RunSearch {
            workspace_id: "ws".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(none.items.len(), 4);
    assert_eq!(none.items[0].trigger, "manual"); // newest first; no trigger recorded
}

#[tokio::test]
async fn filters_and_keyset_pagination_cover_every_row_once() {
    let pool = pool().await;
    let (repo, a, b) = seed(&pool).await;
    let runs = repo.list_runs(&b).await.unwrap();
    repo.update_run(
        &runs[0].id,
        RunStatus::Error,
        &Vec::<NodeRunState>::new(),
        Some("boom: disk full"),
        true,
    )
    .await
    .unwrap();
    let by_wf = RunSearch {
        workspace_id: "ws".into(),
        workflow_id: Some(a.clone()),
        ..Default::default()
    };
    assert_eq!(search_runs(&pool, &by_wf).await.unwrap().items.len(), 2);
    let errs = RunSearch {
        workspace_id: "ws".into(),
        statuses: vec!["error".into()],
        ..Default::default()
    };
    let page = search_runs(&pool, &errs).await.unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].error.as_deref(), Some("boom: disk full"));
    assert!(page.items[0].duration_ms.is_some());
    assert_eq!(
        search_runs(&pool, &search("disk"))
            .await
            .unwrap()
            .items
            .len(),
        1
    ); // error text
       // Pages of 3: 3 + 1, no overlap, then no cursor.
    let mut seen = Vec::new();
    let mut cursor = None;
    loop {
        let page = search_runs(
            &pool,
            &RunSearch {
                workspace_id: "ws".into(),
                limit: Some(3),
                cursor: cursor.clone(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(page.items.len() <= 3);
        seen.extend(page.items.into_iter().map(|i| i.id));
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(seen.len(), 4);
    seen.dedup();
    assert_eq!(seen.len(), 4);
}

#[tokio::test]
async fn bounds_are_enforced() {
    let pool = pool().await;
    seed(&pool).await;
    let long = "x".repeat(MAX_Q + 1);
    assert!(search_runs(&pool, &search(&long)).await.is_err());
    assert!(search_runs(&pool, &search("a b c d e f g h i"))
        .await
        .is_err());
    let bad = RunSearch {
        workspace_id: "ws".into(),
        statuses: vec!["nope".into()],
        ..Default::default()
    };
    assert!(search_runs(&pool, &bad).await.is_err());
    let bad = RunSearch {
        workspace_id: "ws".into(),
        cursor: Some("garbage".into()),
        ..Default::default()
    };
    assert!(search_runs(&pool, &bad).await.is_err());
    let big = RunSearch {
        workspace_id: "ws".into(),
        limit: Some(10_000),
        ..Default::default()
    };
    assert_eq!(search_runs(&pool, &big).await.unwrap().items.len(), 4); // clamped, not refused
}
