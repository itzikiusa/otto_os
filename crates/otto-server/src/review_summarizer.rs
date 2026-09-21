//! Managed summarizer lifecycle. The session is published before waiting for its
//! result, and its PTY is released even when publication, cancellation or the
//! absolute deadline ends the attempt.

use std::{future::Future, path::PathBuf, time::Duration};
use tokio::sync::oneshot;

pub(crate) struct Attempt {
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
        let mut meta = serde_json::json!({ "source": "review_summarizer" });
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
    pub fn prompt(&self, prompt: &str) -> String {
        format!("{prompt}\n\nReturn the final draft comments as a JSON array. Each item must include a string body. \
Write the COMPLETE JSON array to the temporary file {} first, then atomically rename it to {}. \
Do not write directly to the final file or publish it before all comments are complete. \
The final file is the completion signal; a chat reply alone does not complete this task.",
            self.temporary_path().display(), self.result_path().display())
    }
}

pub(crate) fn valid_result(text: &str) -> bool {
    serde_json::from_str::<Vec<crate::modules::DraftComment>>(text).is_ok()
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
pub(crate) async fn drive<T, C, P, PF, S, SF>(
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
    }
}
