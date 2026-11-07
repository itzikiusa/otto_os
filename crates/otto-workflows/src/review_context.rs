//! Opt-in reuse for read-only context discovery. Never used for checkout,
//! deterministic gates, review, fixes, notifications, or publishing.
use crate::context::{RepoEntry, RunContextFiles};
use otto_core::{workflows::NodeStatus, Id};
use otto_state::{DbPool, WorkflowsRepo};
use serde_json::{json, Value};

pub struct ContextInput<'a> {
    pub run_id: &'a Id,
    pub node_id: &'a str,
    pub params: &'a Value,
    pub upstream: &'a Value,
    pub files: &'a RunContextFiles,
    pub default_provider: &'a str,
    pub instructions: &'a str,
    pub previous_run_id: Option<&'a str>,
}

pub async fn recover(pool: &DbPool, input: &ContextInput<'_>) -> (Option<Value>, Option<String>) {
    let key = key(pool, input).await;
    let reply = match key.as_ref() {
        Some(key) => previous_reply(pool, input, key).await,
        None => None,
    };
    (key, reply)
}

const CACHE_KEY: &str = "review_context_key";

/// Every declared path is a complete dependency of this context step. Missing
/// paths, dirty trees, changed ticket/request/config, or unknown lineage are a
/// cache miss. Authors must opt in explicitly; arbitrary agent prompts are not
/// assumed to be pure.
async fn key(pool: &DbPool, input: &ContextInput<'_>) -> Option<Value> {
    let paths = input.params.get("review_context_paths")?.as_array()?;
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
    let repo = WorkflowsRepo::new(pool.clone());
    let current = repo.get_run(input.run_id).await.ok()?;
    let mut request = current.input.as_object()?.clone();
    for k in [
        crate::rereview::PREVIOUS_RUN_KEY,
        "channel",
        "chat",
        "thread",
        "origin_workspace_id",
        "user",
    ] {
        request.remove(k);
    }
    let upstream = context_input(input.upstream);
    let instructions = input.instructions;
    // The standard agent preamble exposes every earlier handoff, not just
    // the immediate input. Include those too; a changed contract brief must
    // invalidate an otherwise unchanged docs-only context result.
    let mut handoffs = Vec::new();
    for name in input.files.list_step_mds() {
        let body = input.files.read_named(&name)?;
        handoffs.push((name, body));
    }
    let dependencies = tracked_dependencies(&input.files.repos(), &paths).await?;
    // Store only a digest: handoff files may contain earlier cache metadata,
    // and embedding that again would recursively inflate every next output.
    use sha2::{Digest, Sha256};
    let inputs = json!({"version":current.workflow_version,"params":input.params,"request":request,
        "jira":input.files.read_jira_md(),"dependencies":dependencies,
        "upstream":upstream,"handoffs":handoffs,"instructions":instructions,"default_provider":input.default_provider});
    Some(json!({"sha256":hex::encode(Sha256::digest(serde_json::to_vec(&inputs).ok()?))}))
}

/// Runtime paths and session identity do not change the context's meaning.
/// Preserve every substantive upstream value (including a checkout/contract
/// summary); unknown dependencies must miss rather than reuse stale advice.
fn context_input(input: &Value) -> Value {
    let mut input = input.clone();
    if let Some(map) = input.as_object_mut() {
        for key in [
            "working_directory",
            "worktree",
            "session_id",
            "context_dir",
            "run_context_dir",
            crate::rereview::PREVIOUS_RUN_KEY,
        ] {
            map.remove(key);
        }
    }
    input
}

async fn tracked_dependencies(entries: &[RepoEntry], paths: &[&str]) -> Option<Value> {
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

async fn previous_reply(pool: &DbPool, input: &ContextInput<'_>, key: &Value) -> Option<String> {
    let previous_id = input.previous_run_id?;
    let repo = WorkflowsRepo::new(pool.clone());
    let previous = repo.get_run(&previous_id.to_string()).await.ok()?;
    let current = repo.get_run(input.run_id).await.ok()?;
    if previous.workspace_id != current.workspace_id || previous.workflow_id != current.workflow_id
    {
        return None;
    }
    let node = previous
        .nodes
        .iter()
        .find(|n| n.node_id == input.node_id && n.status == NodeStatus::Success)?;
    let output = node.output.as_ref()?;
    if output.get(CACHE_KEY)? != key {
        return None;
    }
    let reply = output.get("reply")?.as_str()?;
    Some(reply.to_string())
}

pub fn attach(output: &mut serde_json::Map<String, Value>, key: Option<Value>) {
    if let Some(key) = key {
        output.insert(CACHE_KEY.into(), key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn context_input_preserves_changed_upstream_contracts() {
        let before =
            json!({"reply":"contract accepts USD", "working_directory":"/old", "session_id":"old"});
        let moved =
            json!({"reply":"contract accepts USD", "working_directory":"/new", "session_id":"new"});
        let changed =
            json!({"reply":"contract accepts EUR", "working_directory":"/new", "session_id":"new"});
        assert_eq!(context_input(&before), context_input(&moved));
        assert_ne!(context_input(&before), context_input(&changed));
    }

    #[tokio::test]
    async fn context_cache_tracks_declared_blobs_and_rejects_dirty_or_missing_inputs() {
        let dir = tempfile::tempdir().unwrap();
        let git = async |args: &[&str]| {
            let result = tokio::process::Command::from(otto_git::local::hardened_std_command())
                .current_dir(dir.path())
                .args([
                    "-c",
                    "user.name=Test",
                    "-c",
                    "user.email=test@example.invalid",
                ])
                .args(args)
                .output()
                .await
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
        };
        git(&["init"]).await;
        std::fs::write(dir.path().join("context.md"), "architecture").unwrap();
        std::fs::write(dir.path().join("code.rs"), "old behavior").unwrap();
        git(&["add", "."]).await;
        git(&["commit", "-m", "fixture"]).await;
        let entries = vec![RepoEntry {
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
        git(&["add", "."]).await;
        git(&["commit", "-m", "fix"]).await;
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
        git(&["add", "."]).await;
        git(&["commit", "-m", "context change"]).await;
        assert_ne!(
            tracked_dependencies(&entries, &["context.md"])
                .await
                .unwrap(),
            key
        );
    }
}
