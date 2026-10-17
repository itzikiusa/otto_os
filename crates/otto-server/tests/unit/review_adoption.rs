use super::review_adoptable;

fn review(status: &str, agents: &[&str]) -> otto_core::domain::Review {
    let agents: Vec<_> = agents
        .iter()
        .map(|s| {
            serde_json::json!({"name": "r", "provider": "claude", "model": "",
                "status": s, "note": "", "comment_count": 0})
        })
        .collect();
    serde_json::from_value(serde_json::json!({
        "id": "rv", "repo_id": "repo", "pr_number": 0, "status": status,
        "agents": agents, "comments": [], "created_at": "2026-10-16T00:00:00Z"
    }))
    .unwrap()
}

#[test]
fn only_complete_or_running_reviews_are_adopted() {
    assert!(review_adoptable(&review("done", &["done", "done"])));
    assert!(!review_adoptable(&review("done", &["error", "done"])));
    assert!(review_adoptable(&review(
        "running",
        &["running", "pending"]
    )));
    assert!(!review_adoptable(&review("error", &["done", "done"])));
    let mut fell_back = review("done", &["done", "done"]);
    fell_back.summary_fallback = Some(true);
    assert!(!review_adoptable(&fell_back), "summary fell back");
}
