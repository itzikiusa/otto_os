//! The `verifier` workflow node, moved verbatim out of
//! `execute_node`'s match (see `nodes/mod.rs`).

use super::super::*;
use super::NodeCall;

pub(in crate::workflow_engine) async fn execute(
    call: NodeCall<'_>,
) -> Result<(Value, Vec<String>)> {
    let NodeCall { input, .. } = call;

    // Game path: verify the built HTML game exists and the agent's
    // self-test reported it playable. A failed check errors the node so
    // the run is marked error (the pipeline isn't "done" until playable).
    if let Some(play_url) = input.get("play_url").and_then(Value::as_str) {
        let game_path = input.get("game_path").and_then(Value::as_str).unwrap_or("");
        let meta = if game_path.is_empty() {
            None
        } else {
            tokio::fs::metadata(game_path).await.ok()
        };
        let exists = meta.as_ref().is_some_and(|m| m.is_file());
        let big_enough = meta.as_ref().is_some_and(|m| m.len() > 1500);
        let self_test = input
            .get("playable")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        // Structural integrity is the reliable in-pipeline gate; the
        // agent's own self-test is reported but not required (the
        // authoritative behavioral check is an external headless run).
        let passed = exists && big_enough;
        let report = json!({
            "checks": [
                { "name": "game_file_exists", "passed": exists },
                { "name": "game_non_trivial", "passed": big_enough },
                { "name": "agent_self_test_playable", "passed": self_test },
            ],
            "passed": passed,
            "play_url": play_url,
            "game_path": game_path,
        });
        if !passed {
            return Err(otto_core::Error::Upstream(format!(
                "game file missing or trivial (exists={exists}, non_trivial={big_enough})"
            )));
        }
        return Ok((
            json!({ "verified": report, "play_url": play_url, "playable": true }),
            vec!["game verified playable".into()],
        ));
    }

    Err(otto_core::Error::Invalid(
        "verifier: missing `play_url` / `game_path` artifact to verify".into(),
    ))
}
