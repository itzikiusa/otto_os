//! Mirror a session's transcript artifacts into its Mission Control work item
//! (`work_artifacts`), once per `(kind, ref)` (architecture-3-03r). Used by
//! the artifacts route and the live tailer. Silent on any failure — Mission
//! Control is a projection.

use otto_core::domain::Session;
use otto_core::Id;
use otto_transcript::{Artifact, ArtifactKind};

use crate::state::ServerCtx;

/// The session's Mission Control work item, if the projector made one.
async fn session_work_item(ctx: &ServerCtx, session: &Session) -> Option<otto_state::WorkItem> {
    ctx.workgraph
        .repo()
        .find_by_source(
            &session.workspace_id,
            otto_state::WorkKind::Session,
            &session.id,
        )
        .await
        .ok()
        .flatten()
}

/// The work-graph `(kind, ref)` an artifact registers under — the
/// `work_artifacts` unique key within one item.
fn work_artifact_key(a: &Artifact) -> (otto_state::ArtifactKind, Option<String>) {
    let kind = match a.kind {
        ArtifactKind::Pr => otto_state::ArtifactKind::Pr,
        ArtifactKind::Report => otto_state::ArtifactKind::Report,
        ArtifactKind::Url => otto_state::ArtifactKind::Link,
        ArtifactKind::File | ArtifactKind::Image => otto_state::ArtifactKind::File,
    };
    (kind, a.path.clone().or_else(|| a.url.clone()))
}

async fn insert_work_artifact(
    ctx: &ServerCtx,
    session: &Session,
    item_id: &Id,
    a: &Artifact,
    (kind, reference): (otto_state::ArtifactKind, Option<String>),
) {
    let _ = ctx
        .workgraph
        .add_artifact(otto_state::NewArtifact {
            work_item_id: item_id.clone(),
            workspace_id: session.workspace_id.clone(),
            kind,
            title: a.label.clone(),
            reference,
            payload: serde_json::to_value(a).unwrap_or(serde_json::Value::Null),
        })
        .await;
}

/// Mirror an artifact into `work_artifacts` for the session's work item (once
/// per `(kind, ref)`). Silent on any failure — Mission Control is a projection.
pub(crate) async fn register_work_artifact(ctx: &ServerCtx, session: &Session, a: &Artifact) {
    let Some(item) = session_work_item(ctx, session).await else {
        return;
    };
    let key = work_artifact_key(a);
    // One indexed existence probe; on a probe error fall through to the insert
    // (`ON CONFLICT DO NOTHING` keeps it idempotent).
    if let Ok(true) = ctx
        .workgraph
        .repo()
        .has_artifact(&item.id, key.0, key.1.as_deref())
        .await
    {
        return;
    }
    insert_work_artifact(ctx, session, &item.id, a, key).await;
}

/// [`register_work_artifact`] for a whole list: resolves the work item once,
/// reads the attached `(kind, ref)` set once (no payloads), and inserts only
/// the artifacts that are missing.
pub(crate) async fn register_work_artifacts(ctx: &ServerCtx, session: &Session, arts: &[Artifact]) {
    if arts.is_empty() {
        return;
    }
    let Some(item) = session_work_item(ctx, session).await else {
        return;
    };
    let Ok(mut known) = ctx.workgraph.repo().artifact_keys(&item.id).await else {
        return;
    };
    for a in arts {
        let key = work_artifact_key(a);
        if known.insert((key.0.as_str().to_string(), key.1.clone())) {
            insert_work_artifact(ctx, session, &item.id, a, key).await;
        }
    }
}
