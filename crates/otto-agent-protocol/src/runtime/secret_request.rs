//! Private secret requests (T3 Code #15907, #30257): `otto.request_secret`
//! lets a thread's agent ask its owner for a token or password in a private
//! card instead of the chat. The pending request lives only in memory, the
//! value goes only into the calling agent's tool result, and the thread masks
//! it in every later event (`AgentRuntime::redact_value`), so it reaches
//! neither the timeline nor the database. The `secret_request_updated` event
//! (owner only) carries the prompt, never the value.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use otto_core::event::Event;
use otto_core::{Error, Id, Result};
use serde_json::{json, Value};
use tokio::sync::{broadcast, oneshot};

use super::AgentRuntime;

/// How long a request waits for the person (inside the governed run budget).
const WAIT: Duration = Duration::from_secs(170);
const PROMPT_MAX: usize = 300;
const NAME_MAX: usize = 60;
/// A secret longer than this is refused (a token, not a file).
const VALUE_MAX: usize = 8 * 1024;

struct Pending {
    owner: Id,
    session_id: Id,
    workspace_id: Id,
    prompt: String,
    name: Option<String>,
    /// Taken by the answer; the request itself goes when the call ends.
    tx: Option<oneshot::Sender<Option<String>>>,
}

static PENDING: LazyLock<Mutex<HashMap<Id, Pending>>> = LazyLock::new(Default::default);

fn pending() -> std::sync::MutexGuard<'static, HashMap<Id, Pending>> {
    PENDING.lock().unwrap_or_else(|e| e.into_inner())
}

fn clip(s: &str, max: usize) -> String {
    s.trim().chars().take(max).collect()
}

/// Removes the request (and its card) however the call ends.
struct Settle<'a> {
    events: &'a broadcast::Sender<Event>,
    id: Id,
}

impl Drop for Settle<'_> {
    fn drop(&mut self) {
        if let Some(p) = pending().remove(&self.id) {
            let _ = self.events.send(Event::SecretRequestUpdated {
                user_id: p.owner,
                workspace_id: p.workspace_id,
                session_id: p.session_id,
                request_id: self.id.clone(),
                pending: false,
                prompt: String::new(),
                name: None,
            });
        }
    }
}

impl AgentRuntime {
    /// `otto.request_secret`, called by the agent of thread `session`;
    /// `events` carries the owner's card.
    pub async fn request_secret(
        &self,
        events: &broadcast::Sender<Event>,
        args: &Value,
        session: Option<&Id>,
    ) -> Result<Value> {
        let session_id = session.ok_or_else(|| {
            Error::Invalid("otto.request_secret is for an agent working in an Otto thread".into())
        })?;
        let thread = self
            .get(session_id)
            .await
            .map_err(|_| Error::Invalid("otto.request_secret works in Otto Threads only".into()))?;
        let prompt = clip(
            args.get("prompt").and_then(Value::as_str).unwrap_or(""),
            PROMPT_MAX,
        );
        if prompt.is_empty() {
            return Err(Error::Invalid(
                "`prompt` is required: say what the secret is for".into(),
            ));
        }
        let name = args
            .get("name")
            .and_then(Value::as_str)
            .map(|n| clip(n, NAME_MAX))
            .filter(|n| !n.is_empty());
        let id = otto_core::new_id();
        let (tx, rx) = oneshot::channel();
        pending().insert(
            id.clone(),
            Pending {
                owner: thread.created_by.clone(),
                session_id: session_id.clone(),
                workspace_id: thread.workspace_id.clone(),
                prompt: prompt.clone(),
                name: name.clone(),
                tx: Some(tx),
            },
        );
        let _settle = Settle {
            events,
            id: id.clone(),
        };
        let _ = events.send(Event::SecretRequestUpdated {
            user_id: thread.created_by.clone(),
            workspace_id: thread.workspace_id.clone(),
            session_id: session_id.clone(),
            request_id: id,
            pending: true,
            prompt,
            name,
        });
        Ok(match tokio::time::timeout(WAIT, rx).await {
            Ok(Ok(Some(value))) => {
                self.redact_value(session_id, &value).await?;
                json!({"status": "provided", "value": value,
                       "note": "Use it for the task only; don't repeat it in your replies."})
            }
            Ok(Ok(None)) => {
                json!({"status": "declined", "note": "The person declined to give it."})
            }
            _ => json!({"status": "no_answer",
                        "note": "The person did not answer in time; ask again only if the task still needs it."}),
        })
    }
}

/// `owner`'s unanswered requests (no values exist here), so a reloaded
/// window shows its cards again.
pub fn pending_secret_requests(owner: &Id) -> Vec<Value> {
    pending()
        .iter()
        .filter(|(_, p)| &p.owner == owner && p.tx.is_some())
        .map(|(id, p)| {
            json!({"request_id": id, "session_id": p.session_id, "workspace_id": p.workspace_id,
                   "prompt": p.prompt, "name": p.name})
        })
        .collect()
}

/// Answer request `id` with `value` (`None` declines). Only its owner
/// (`user`), signed in as themselves (`real_user`), may; **NotFound** once
/// settled or for anyone else.
pub fn answer_secret_request(
    id: &Id,
    user: &Id,
    real_user: &Id,
    value: Option<String>,
) -> Result<()> {
    if value.as_ref().is_some_and(|v| v.len() > VALUE_MAX) {
        return Err(Error::Invalid("the secret is too long".into()));
    }
    let tx = pending()
        .get_mut(id)
        .filter(|p| &p.owner == user && real_user == user)
        .and_then(|p| p.tx.take())
        .ok_or_else(|| Error::NotFound("secret request".into()))?;
    let _ = tx.send(value);
    Ok(())
}
