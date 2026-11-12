//! Mirrored artifacts — the Hall's copy of a Canvas scene or a Product story
//! attachment (`source_kind` / `source_id` set by `import`).
//!
//! Their source stays the truth: live sync (`sync`) moves the head whenever
//! the scene / attachment changes, so an edit made here would be overwritten
//! by the next sync. The write routes therefore refuse a mirrored artifact
//! with **409 `code: "mirrored"`** until a person adopts it
//! (`POST /design/artifacts/{id}/adopt`, which sets `meta.adopted = true`).
//! From then on the Hall owns the head and a changed source only ever lands
//! as a side version (`variant/sync-<ts>/1`) — nothing is overwritten.

use chrono::Utc;
use otto_core::api::Problem;
use otto_core::event::Event;
use otto_core::{Error, Result};
use serde_json::{json, Value};

use crate::import::{SOURCE_ATTACHMENT, SOURCE_SCENE};
use crate::service::{commit_lock, Author, DesignService};
use crate::store::NewSignal;
use crate::types::DesignArtifact;

/// The stable `Problem.code` of a write refused on a mirrored artifact.
pub const MIRRORED_CODE: &str = "mirrored";
/// Marks the `Conflict` message [`problem`] maps to [`MIRRORED_CODE`].
const MIRRORED_TAG: &str = "[mirrored] ";
/// `meta` keys only `POST …/adopt` writes (a PATCH can neither set nor clear them).
const ADOPTION_KEYS: &[&str] = &["adopted", "adopted_at", "adopted_by"];

/// Has a person adopted this mirror (`meta.adopted == true`)?
pub fn is_adopted(a: &DesignArtifact) -> bool {
    a.meta.get("adopted").and_then(Value::as_bool) == Some(true)
}

/// Read-only here: mirrored from a legacy source and not adopted.
pub fn is_mirrored(a: &DesignArtifact) -> bool {
    a.source_kind.is_some() && !is_adopted(a)
}

fn source_label(kind: Option<&str>) -> &'static str {
    match kind {
        Some(SOURCE_SCENE) => "a Canvas scene",
        Some(SOURCE_ATTACHMENT) => "a story attachment",
        _ => "a legacy design",
    }
}

/// The 409 every content-writing route answers for a mirrored artifact.
pub fn mirrored_error(a: &DesignArtifact) -> Error {
    Error::Conflict(format!(
        "{MIRRORED_TAG}design {} mirrors {} and is read-only in Design Hall; \
         edit it where it lives, or adopt it (POST /design/artifacts/{}/adopt) to edit it here",
        a.id,
        source_label(a.source_kind.as_deref()),
        a.id
    ))
}

/// `Ok` unless `a` is mirrored (then [`mirrored_error`]).
pub fn ensure_editable(a: &DesignArtifact) -> Result<()> {
    if is_mirrored(a) {
        Err(mirrored_error(a))
    } else {
        Ok(())
    }
}

/// The JSON error body of `e`: the usual `{code, message}`, except a
/// [`mirrored_error`] answers `code: "mirrored"` (and its message untagged).
pub fn problem(e: &Error) -> Problem {
    if let Error::Conflict(m) = e {
        if let Some(rest) = m.strip_prefix(MIRRORED_TAG) {
            return Problem {
                code: MIRRORED_CODE.to_string(),
                message: rest.to_string(),
            };
        }
    }
    Problem {
        code: e.code().to_string(),
        message: e.to_string(),
    }
}

/// Undo whatever a metadata PATCH did to the adoption keys (`before` is the
/// meta as stored, `after` the merged result).
pub(crate) fn keep_adoption(before: &Value, after: &mut Value) {
    let Value::Object(m) = after else {
        return;
    };
    for k in ADOPTION_KEYS {
        match before.get(*k) {
            Some(v) => {
                m.insert((*k).to_string(), v.clone());
            }
            None => {
                m.remove(*k);
            }
        }
    }
}

impl DesignService {
    /// "Edit here": the Hall takes over a mirrored artifact. Sets
    /// `meta.adopted` (+ `adopted_at` / `adopted_by`), records an `adopted`
    /// signal and emits `design_artifact_updated {change: "meta"}`. Idempotent
    /// (an adopted artifact is returned as is); 400 for an artifact that
    /// mirrors nothing.
    pub async fn adopt(&self, artifact_id: &str, actor: &Author) -> Result<DesignArtifact> {
        let lock = commit_lock(self.root(), artifact_id).await?;
        let publication = lock.lock().await;
        let mut a = self.store().require_artifact(artifact_id).await?;
        if a.source_kind.is_none() {
            return Err(Error::Invalid(format!(
                "design artifact {} is not mirrored from Canvas or a story; it is already editable",
                a.id
            )));
        }
        if is_adopted(&a) {
            return Ok(a);
        }
        if !a.meta.is_object() {
            a.meta = json!({});
        }
        a.meta["adopted"] = json!(true);
        a.meta["adopted_at"] = json!(Utc::now().to_rfc3339());
        a.meta["adopted_by"] = json!(actor.id);
        let updated = self.store().write_artifact_meta(&a).await?;
        drop(publication);
        let _ = self
            .record_signal_row(NewSignal {
                workspace_id: updated.workspace_id.clone(),
                artifact_id: updated.id.clone(),
                version_id: updated.head_version_id.clone(),
                kind: "adopted".into(),
                actor_kind: actor.kind.clone(),
                actor_id: actor.id.clone(),
                session_id: actor.session_id.clone(),
                payload: json!({
                    "source_kind": updated.source_kind,
                    "source_id": updated.source_id,
                }),
            })
            .await;
        self.emit(Event::DesignArtifactUpdated {
            workspace_id: updated.workspace_id.clone(),
            artifact_id: updated.id.clone(),
            format: updated.format.clone(),
            change: "meta".into(),
            version_id: None,
            content: None,
        });
        Ok(updated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn artifact(source: Option<&str>, meta: Value) -> DesignArtifact {
        serde_json::from_value(json!({
            "id": "A", "project_id": null, "workspace_id": "w1", "studio": "whiteboard",
            "format": "otto-canvas", "mime": "application/json", "title": "Board",
            "status": "draft", "head_version_id": "v1", "head_seq": 1,
            "approved_version_id": null, "tags": [], "thumb_blob": null, "meta": meta,
            "source_kind": source, "source_id": source.map(|_| "sc1"),
            "created_by": "u1", "created_by_kind": "user", "created_session_id": null,
            "created_at": "2026-11-12T00:00:00Z", "updated_at": "2026-11-12T00:00:00Z"
        }))
        .expect("artifact fixture")
    }

    #[test]
    fn only_unadopted_mirrors_are_read_only() {
        assert!(ensure_editable(&artifact(None, json!({}))).is_ok());
        let mirrored = artifact(Some(SOURCE_SCENE), json!({}));
        assert!(is_mirrored(&mirrored));
        let err = ensure_editable(&mirrored).expect_err("mirrored is refused");
        assert!(matches!(err, Error::Conflict(_)));
        let p = problem(&err);
        assert_eq!(p.code, MIRRORED_CODE);
        assert!(p.message.contains("a Canvas scene"), "{}", p.message);
        assert!(!p.message.contains(MIRRORED_TAG));
        let adopted = artifact(Some(SOURCE_ATTACHMENT), json!({"adopted": true}));
        assert!(ensure_editable(&adopted).is_ok());
        // Any other error keeps its usual code.
        assert_eq!(problem(&Error::Conflict("x".into())).code, "conflict");
        assert_eq!(problem(&Error::NotFound("x".into())).code, "not_found");
    }

    #[test]
    fn a_patch_cannot_touch_the_adoption_keys() {
        let before = json!({"adopted": true, "adopted_by": "u1", "x": 1});
        let mut after = json!({"adopted": false, "x": 2, "adopted_at": "now"});
        keep_adoption(&before, &mut after);
        assert_eq!(after, json!({"adopted": true, "adopted_by": "u1", "x": 2}));
    }
}
