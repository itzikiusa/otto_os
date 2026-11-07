//! Opt-in reuse for read-only context discovery. Never used for checkout,
//! deterministic gates, review, fixes, notifications, or publishing.
use super::super::*;
use super::NodeCall;

const CACHE_KEY: &str = "review_context_key";

/// Every declared path is a complete dependency of this context step. Missing
/// paths, dirty trees, changed ticket/request/config, or unknown lineage are a
/// cache miss. Authors must opt in explicitly; arbitrary agent prompts are not
/// assumed to be pure.
pub(super) async fn key(call: &NodeCall<'_>) -> Option<Value> {
    let paths = call.node.params.get("review_context_paths")?.as_array()?;
    if paths.is_empty() || paths.len() > 32 {
        return None;
    }
    let paths: Vec<&str> = paths.iter().map(Value::as_str).collect::<Option<_>>()?;
    if paths.iter().any(|p| {
        p.is_empty()
            || p.starts_with('/')
            || p.starts_with(':')
            || p.split('/').any(|c| c == "..")
            || p.contains('\0')
    }) {
        return None;
    }
    let repo = WorkflowsRepo::new(call.ctx.pool.clone());
    let current = repo.get_run(&call.env.run_id).await.ok()?;
    let mut request = current.input.as_object()?.clone();
    for k in [
        otto_workflows::rereview::PREVIOUS_RUN_KEY,
        "channel",
        "chat",
        "thread",
        "origin_workspace_id",
        "user",
    ] {
        request.remove(k);
    }
    let dependencies = tracked_dependencies(&call.env.files.repos(), &paths).await?;
    Some(
        json!({"version":current.workflow_version,"params":call.node.params,"request":request,
        "jira":call.env.files.read_jira_md(),"dependencies":dependencies}),
    )
}

async fn tracked_dependencies(
    entries: &[crate::workflow_context::RepoEntry],
    paths: &[&str],
) -> Option<Value> {
    let mut dependencies = Vec::new();
    for entry in entries {
        let git = otto_git::LocalGit::new(entry.worktree.as_ref()?);
        if entry.error.is_some()
            || !git
                .run(&["status", "--porcelain", "--untracked-files=normal"])
                .await
                .ok()?
                .trim()
                .is_empty()
        {
            return None;
        }
        let mut blobs = Vec::new();
        for path in paths {
            let listing = git.run(&["ls-tree", "-r", "HEAD", "--", path]).await.ok()?;
            if listing.trim().is_empty() {
                return None;
            }
            blobs.push(listing);
        }
        dependencies
            .push(json!({"repo":entry.repo_id.as_ref()?, "base":entry.base, "blobs":blobs}));
    }
    if dependencies.is_empty() {
        return None;
    }
    Some(json!(dependencies))
}

pub(super) async fn previous_reply(call: &NodeCall<'_>, key: &Value) -> Option<String> {
    let previous_id = call
        .env
        .run_input
        .get(otto_workflows::rereview::PREVIOUS_RUN_KEY)?
        .as_str()?;
    let repo = WorkflowsRepo::new(call.ctx.pool.clone());
    let previous = repo.get_run(&previous_id.to_string()).await.ok()?;
    let current = repo.get_run(&call.env.run_id).await.ok()?;
    if previous.workspace_id != current.workspace_id || previous.workflow_id != current.workflow_id
    {
        return None;
    }
    let node = previous
        .nodes
        .iter()
        .find(|n| n.node_id == call.node.id && n.status == NodeStatus::Success)?;
    let output = node.output.as_ref()?;
    if output.get(CACHE_KEY)? != key {
        return None;
    }
    let reply = output.get("reply")?.as_str()?;
    Some(reply.to_string())
}

pub(super) fn attach(output: &mut serde_json::Map<String, Value>, key: Option<Value>) {
    if let Some(key) = key {
        output.insert(CACHE_KEY.into(), key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn context_cache_tracks_declared_blobs_and_rejects_dirty_or_missing_inputs() {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            let result = otto_git::local::hardened_std_command()
                .current_dir(dir.path())
                .args([
                    "-c",
                    "user.name=Test",
                    "-c",
                    "user.email=test@example.invalid",
                ])
                .args(args)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
        };
        git(&["init"]);
        std::fs::write(dir.path().join("context.md"), "architecture").unwrap();
        std::fs::write(dir.path().join("code.rs"), "old behavior").unwrap();
        git(&["add", "."]);
        git(&["commit", "-m", "fixture"]);
        let entries = vec![crate::workflow_context::RepoEntry {
            repo: "repo".into(),
            repo_id: Some("repo".into()),
            repo_name: None,
            kind: "worktree".into(),
            name: "fixture".into(),
            source: None,
            worktree: Some(dir.path().to_string_lossy().into()),
            base: Some("base".into()),
            error: None,
        }];
        let key = tracked_dependencies(&entries, &["context.md"])
            .await
            .unwrap();
        std::fs::write(dir.path().join("code.rs"), "fixed behavior").unwrap();
        assert!(
            tracked_dependencies(&entries, &["context.md"])
                .await
                .is_none(),
            "dirty checkout cannot reuse"
        );
        git(&["add", "."]);
        git(&["commit", "-m", "fix"]);
        assert_eq!(
            tracked_dependencies(&entries, &["context.md"])
                .await
                .unwrap(),
            key,
            "unrelated code fix reuses declared context"
        );
        assert!(
            tracked_dependencies(&entries, &["context.md", "missing.md"])
                .await
                .is_none()
        );
        std::fs::write(dir.path().join("context.md"), "changed architecture").unwrap();
        git(&["add", "."]);
        git(&["commit", "-m", "context change"]);
        assert_ne!(
            tracked_dependencies(&entries, &["context.md"])
                .await
                .unwrap(),
            key
        );
    }
}
