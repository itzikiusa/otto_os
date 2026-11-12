//! Managed summarizer lifecycle. The session is published before waiting for its
//! result, and its PTY is released even when publication, cancellation or the
//! absolute deadline ends the attempt.

use std::{future::Future, path::PathBuf, time::Duration};
use tokio::sync::oneshot;

/// Largest summarizer brief pasted into the terminal as is; longer ones go
/// through [`Attempt::instructions_path`].
pub use crate::handoff::PASTE_INLINE_MAX;

pub struct Attempt {
    dir: tempfile::TempDir,
    pub provider: String,
    pub meta: serde_json::Value,
}

impl Attempt {
    pub fn new(provider: &str, model: &str) -> Result<Self, String> {
        let dir = tempfile::Builder::new()
            .prefix("otto-summary-")
            .tempdir()
            .map_err(|e| e.to_string())?;
        let provider = if provider.trim().is_empty() {
            "claude"
        } else {
            provider.trim()
        };
        // Confined + no project settings, like every reviewer (see
        // `session::review_session_meta`): it runs in the same checkout.
        let mut meta = serde_json::json!({
            "source": "review_summarizer",
            "read_only": true,
            "project_settings": false,
        });
        if !model.trim().is_empty() {
            meta["model"] = model.trim().into();
        }
        Ok(Self {
            dir,
            provider: provider.into(),
            meta,
        })
    }
    pub fn result_path(&self) -> PathBuf {
        self.dir.path().join("result.json")
    }
    pub fn temporary_path(&self) -> PathBuf {
        self.dir.path().join("result.json.tmp")
    }
    pub fn instructions_path(&self) -> PathBuf {
        self.dir.path().join("instructions.md")
    }
    /// The text typed into the summarizer's terminal. A large brief (every raw
    /// claim plus the run's context — 100+ KB on a big PR) is written to
    /// [`Self::instructions_path`] and only a short pointer is pasted: a
    /// terminal paste that size can outlast the PTY write deadline on a busy
    /// daemon, and then the summarizer never starts. If the file cannot be
    /// written the brief is pasted whole, as before. Same hand-off as the
    /// reviewers ([`crate::handoff::typed_prompt`]); the brief's file lives in
    /// this attempt's own directory.
    pub fn prompt(&self, prompt: &str) -> String {
        let handoff = format!("Return the final draft comments as a JSON array. Each item must include a string body. \
Write the COMPLETE JSON array to the temporary file {} first, then atomically rename it to {}. \
Do not write directly to the final file or publish it before all comments are complete. \
The final file is the completion signal; a chat reply alone does not complete this task.",
            self.temporary_path().display(), self.result_path().display());
        crate::handoff::typed_prompt(
            &format!("{prompt}\n\n{handoff}"),
            &self.instructions_path(),
            &format!(" Every raw claim it lists needs its own disposition.\n\n{handoff}"),
        )
    }
}

/// The handoff file is complete when it parses through the same reader as
/// the comments themselves ([`crate::engine::draft_comments_from_json`]).
pub fn valid_result(text: &str) -> bool {
    crate::engine::draft_comments_from_json(text).is_ok()
}

#[cfg(test)]
async fn read_result(path: &std::path::Path) -> Option<String> {
    tokio::fs::read_to_string(path)
        .await
        .ok()
        .filter(|text| valid_result(text))
}

/// `ready` is delivered by the managed runner's synchronous on_ready callback.
/// Publication is awaited here, never spawned, so completion cannot race a
/// detached DB write. The deadline includes startup AND publication.
pub async fn drive<T, C, P, PF, S, SF>(
    turn: T,
    mut ready: oneshot::Receiver<String>,
    budget: Duration,
    cancelled: C,
    publish: P,
    stop: S,
) -> Result<String, String>
where
    T: Future<Output = Result<String, String>>,
    C: Future<Output = ()>,
    P: FnOnce(String) -> PF,
    PF: Future<Output = Result<(), String>>,
    S: FnOnce(String) -> SF,
    SF: Future<Output = ()>,
{
    let mut session_id = None;
    let mut publish = Some(publish);
    let result = {
        tokio::pin!(turn);
        let work = async {
            loop {
                tokio::select! {
                    biased;
                    Ok(sid) = &mut ready, if session_id.is_none() => {
                        session_id = Some(sid.clone());
                        publish.take().expect("publish once")(sid).await?;
                    }
                    result = &mut turn => {
                        // A canned/immediately failing runner can send ready and
                        // return in the same poll. Publish before returning it too.
                        if session_id.is_none() {
                            if let Ok(sid) = ready.try_recv() {
                                session_id = Some(sid.clone());
                                publish.take().expect("publish once")(sid).await?;
                            }
                        }
                        break result;
                    }
                }
            }
        };
        tokio::select! {
            biased;
            _ = cancelled => Err("review cancelled".into()),
            result = tokio::time::timeout(budget, work) => result
                .unwrap_or_else(|_| Err("summarizer absolute deadline exceeded".into())),
        }
    }; // Drop the turn (including its live hold) BEFORE releasing the PTY.
       // Cancellation can win the select immediately after the callback fired.
    if session_id.is_none() {
        session_id = ready.try_recv().ok();
    }
    if let Some(sid) = session_id {
        stop(sid).await;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    /// The 2026-11-12 bo_common_ui failure: a ~100 KB brief pasted into the
    /// terminal never drained, the summarizer never started and the review
    /// ended in error. A big brief must reach the agent as a file.
    #[test]
    fn a_large_brief_is_handed_over_as_a_file_and_a_small_one_inline() {
        let attempt = Attempt::new("claude", "").unwrap();
        let small = attempt.prompt("Summarize these 2 claims.");
        assert!(small.starts_with("Summarize these 2 claims."));
        assert!(!attempt.instructions_path().exists());

        let brief = format!("BRIEF-START {}", "claim text ".repeat(10_000));
        let typed = attempt.prompt(&brief);
        assert!(
            typed.len() < 2048,
            "pasted text stays short: {}",
            typed.len()
        );
        let path = attempt.instructions_path();
        assert!(typed.contains(&path.display().to_string()));
        assert!(typed.contains(&attempt.result_path().display().to_string()));
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.starts_with("BRIEF-START"));
        assert!(written.contains(&attempt.result_path().display().to_string()));
    }

    #[tokio::test]
    async fn publishes_pending_sessions_and_cleans_up_for_every_provider() {
        for provider in ["", "claude", "codex", "agy"] {
            let spec = Attempt::new(provider, " chosen-model ").unwrap();
            assert_eq!(
                spec.provider,
                if provider.is_empty() {
                    "claude"
                } else {
                    provider
                }
            );
            assert_eq!(spec.meta["model"], "chosen-model");
            let events = Arc::new(Mutex::new(Vec::new()));
            let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
            let (finish_tx, finish_rx) = tokio::sync::oneshot::channel();
            let published = events.clone();
            let stopped = events.clone();
            let turn = async {
                ready_tx.send("session".to_string()).unwrap();
                finish_rx.await.unwrap();
                Ok("[]".to_string())
            };
            let result = drive(
                turn,
                ready_rx,
                Duration::from_secs(1),
                std::future::pending(),
                |sid| async move {
                    published.lock().unwrap().push(format!("published {sid}"));
                    finish_tx.send(()).unwrap();
                    Ok(())
                },
                |sid| async move {
                    stopped.lock().unwrap().push(format!("stopped {sid}"));
                },
            )
            .await;
            assert_eq!(result.unwrap(), "[]");
            assert_eq!(
                *events.lock().unwrap(),
                ["published session", "stopped session"]
            );
        }
    }

    #[tokio::test]
    async fn timeout_cancel_and_errors_release_the_published_session() {
        for mode in ["timeout", "cancel", "error", "publish-error"] {
            let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
            let stopped = Arc::new(Mutex::new(Vec::new()));
            let stop = stopped.clone();
            let turn = async {
                ready_tx.send("session".to_string()).unwrap();
                if mode == "error" {
                    return Err("provider failed".into());
                }
                std::future::pending().await
            };
            let cancel = async {
                if mode != "cancel" {
                    std::future::pending::<()>().await;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            };
            let result = drive(
                turn,
                ready_rx,
                Duration::from_millis(20),
                cancel,
                |_| async {
                    if mode == "publish-error" {
                        Err("DB failed".into())
                    } else {
                        Ok(())
                    }
                },
                |sid| async move {
                    stop.lock().unwrap().push(sid);
                },
            )
            .await;
            assert!(result.is_err(), "{mode}");
            assert_eq!(*stopped.lock().unwrap(), ["session"], "{mode}");
        }
    }

    #[tokio::test]
    async fn retry_results_are_isolated_and_partial_json_is_not_complete() {
        let first = Attempt::new("codex", "").unwrap();
        let retry = Attempt::new("codex", "").unwrap();
        assert_ne!(first.result_path(), retry.result_path());
        std::fs::write(first.result_path(), r#"[{"body":"old"}]"#).unwrap();
        assert!(read_result(&retry.result_path()).await.is_none());
        std::fs::write(retry.result_path(), r#"[{"body":"partial"}"#).unwrap();
        assert!(read_result(&retry.result_path()).await.is_none());
        std::fs::write(retry.temporary_path(), r#"[{"body":"fresh"}]"#).unwrap();
        std::fs::rename(retry.temporary_path(), retry.result_path()).unwrap();
        assert_eq!(
            read_result(&retry.result_path()).await.unwrap(),
            r#"[{"body":"fresh"}]"#
        );
        assert!(!valid_result(r#"[{"title":"no body"}]"#));
        assert!(!valid_result(r#"{"body":"not an array"}"#));
        assert!(valid_result("[]"));
        // Contract: the exact shape the verifier prompt asks for (a verified
        // comment with source_ids + evidence, a suppressed note, and the
        // mandatory overall score) must pass the handoff check AND yield the
        // comments. Two readers disagreeing here cost a release (2026-11-09).
        let example = r#"[
            {"path":"a.go","line":3,"severity":"warn","body":"x","disposition":"verified",
             "evidence":"a.go:3 traced","source_ids":["r0-0"]},
            {"body":"claim y","disposition":"not_relevant","evidence":"b.go:9","source_ids":["r1-0"]},
            {"lens_score":6,"score_reason":"one should-fix"}
        ]"#;
        assert!(valid_result(example));
        let id = otto_core::Id::from("r");
        assert_eq!(crate::engine::parse_draft_comments(&id, example).len(), 2);
        assert_eq!(
            crate::session::parse_lens_score(example).map(|s| s.0),
            Some(6)
        );
        assert!(valid_result(
            r#"[{"body":"x"},{"lens_score":7,"score_reason":"one should-fix"}]"#
        ));
        assert!(valid_result(
            r#"[{"lens_score":10,"score_reason":"clean"}]"#
        ));
        assert!(!valid_result(r#"[{"lens_score":7},{"title":"no body"}]"#));
    }

    /// Contract: every rejection word the verifier PROMPT offers is accepted
    /// by the completeness CHECK (as a suppressed note, never a PR comment).
    /// go_marketing #249 (2026-11-10) failed: the prompt offered `unverified`,
    /// the check knew only `not_relevant|duplicate`, so a finished review with
    /// 7 verified comments was marked incomplete and its workflow scored 0.
    #[test]
    fn contract_every_disposition_the_prompt_offers_passes_the_check() {
        let cfg = crate::engine::default_review_config("claude").summarizer;
        let prompt = crate::prompts::summarizer_prompt("ctx", &cfg, &[]);
        let offered = prompt
            .split("disposition=")
            .nth(2)
            .and_then(|rest| rest.split(',').next())
            .expect("prompt lists the rejection dispositions");
        assert_eq!(offered, "not_relevant|unverified|duplicate");
        for d in offered.split('|') {
            let c: crate::engine::DraftComment = serde_json::from_value(serde_json::json!(
                {"body":"claim","disposition":d,"evidence":"x.go:1 unreachable"}
            ))
            .unwrap();
            assert!(c.verification_complete(), "{d} rejected by the check");
            assert!(c.suppressed(), "{d} must never be published");
        }
    }

    /// The go_marketing #249 handoff, shape for shape (texts anonymised):
    /// 8 batches with 1,0,0,1,3,1,4,1 raw claims, one merged comment, an
    /// `unverified` rejection, two `not_relevant` notes and the score.
    #[test]
    fn contract_real_go_marketing_handoff_counts_as_complete() {
        let v = |ids: &[&str], d: &str| {
            serde_json::json!({"path":"a.go","line":1,"severity":"warn","body":"b",
                "evidence":"a.go:1 traced","disposition":d,"source_ids":ids})
        };
        let payload = serde_json::json!([
            v(&["batch:1:claim:1", "batch:4:claim:1"], "verified"),
            v(&["batch:6:claim:1"], "verified"),
            v(&["batch:5:claim:1"], "verified"),
            v(&["batch:5:claim:2"], "verified"),
            v(&["batch:7:claim:2"], "verified"),
            v(&["batch:7:claim:4"], "verified"),
            v(&["batch:7:claim:3"], "verified"),
            v(&["batch:7:claim:1"], "unverified"),
            v(&["batch:5:claim:3"], "not_relevant"),
            v(&["batch:8:claim:1"], "not_relevant"),
            {"lens_score":7,"score_reason":"should-fix only"}
        ])
        .to_string();
        assert!(valid_result(&payload));
        let comments = crate::engine::draft_comments_from_json(&payload).unwrap();
        assert!(crate::engine::verifier_covers_claims(
            &comments,
            &[1, 0, 0, 1, 3, 1, 4, 1]
        ));
        assert_eq!(comments.iter().filter(|c| !c.suppressed()).count(), 7);
    }
}
