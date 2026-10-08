use super::*;
use otto_core::domain::{GoalLoopConfig, GoalLoopDefinition, GoalLoopLimits, GoalLoopStatus};

#[tokio::test]
async fn run_goal_loop_only_accepts_successful_terminal_outcome() {
    let dir = tempfile::tempdir().unwrap();
    let pool = crate::test_support::mem_pool().await;
    let ctx = ServerCtx::for_tests(&pool, dir.path()).await;
    sqlx::query("INSERT INTO workspaces (id, name, root_path, created_at) VALUES ('goal-ws', 'Goal', '/tmp', '2026-10-08')").execute(&pool).await.unwrap();
    let goal = ctx
        .goal_loops_repo
        .create(otto_state::NewGoalLoop {
            workspace_id: "goal-ws".into(),
            name: "Terminal outcome".into(),
            repo_path: dir.path().to_string_lossy().into_owned(),
            definition: GoalLoopDefinition {
                title: "Fixture".into(),
                summary: String::new(),
                objectives: vec![],
                acceptance_criteria: vec![],
                constraints: vec![],
                out_of_scope: vec![],
                success_signal: String::new(),
            },
            limits: GoalLoopLimits::default(),
            config: GoalLoopConfig::default(),
            created_by: "root".into(),
        })
        .await
        .unwrap();
    for status in [
        GoalLoopStatus::Succeeded,
        GoalLoopStatus::Failed,
        GoalLoopStatus::Stopped,
        GoalLoopStatus::Exhausted,
    ] {
        sqlx::query("UPDATE goal_loops SET status = ?, branch = 'preserved-work' WHERE id = ?")
            .bind(status.as_str())
            .bind(&goal.id)
            .execute(&pool)
            .await
            .unwrap();
        let result = poll_goal_loop(&ctx, &goal.id).await;
        assert_eq!(
            result.is_ok(),
            status == GoalLoopStatus::Succeeded,
            "{status:?} must not be treated as successful execution"
        );
        if let Err(error) = result {
            assert!(error.to_string().contains(status.as_str()), "{error}");
        }
    }
}
