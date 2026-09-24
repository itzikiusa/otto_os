//! Variant branches (proposal §4.2 "Generate / Variants"): a variants run
//! commits each agent draft as a version on `variant/<run>/<k>` WITHOUT moving
//! the artifact's head, so main never sees an unchosen direction. Accepting
//! one fast-forwards main (a new main version carrying the variant's bytes,
//! guarded on the head the run started from) and records the learning signals
//! — `variant_accepted` for the pick, `variant_rejected` for its siblings.
//!
//! Branch naming + parsing are pure (unit-tested); the rest is thin glue over
//! [`Store::insert_side_version`](crate::store::Store::insert_side_version) and
//! the normal commit pipeline.

use otto_core::{Error, Id, Result};
use serde::Serialize;
use serde_json::{json, Value};

use crate::format;
use crate::service::{bound_json, Author, DesignService, SaveOpts, MAX_PROVENANCE_BYTES};
use crate::store::{NewSignal, NewVersion};
use crate::types::*;

/// Branch prefix of every variant version.
pub const VARIANT_PREFIX: &str = "variant/";
/// Hard cap on variants per run (cost bound).
pub const MAX_VARIANTS: usize = 4;

/// `variant/<run>/<k>` (k is 1-based).
pub fn branch_name(run_id: &str, k: usize) -> String {
    format!("{VARIANT_PREFIX}{run_id}/{k}")
}

/// The prefix every branch of one run shares: `variant/<run>/`.
pub fn run_prefix(run_id: &str) -> String {
    format!("{VARIANT_PREFIX}{run_id}/")
}

/// `variant/<run>/<k>` → `(run, k)`; anything else (incl. `main`) → `None`.
pub fn parse_branch(branch: &str) -> Option<(String, usize)> {
    let rest = branch.strip_prefix(VARIANT_PREFIX)?;
    let (run, k) = rest.rsplit_once('/')?;
    if run.is_empty() || run.contains('/') {
        return None;
    }
    let k: usize = k.parse().ok()?;
    (k >= 1).then(|| (run.to_string(), k))
}

/// What accepting a variant did.
#[derive(Debug, Clone, Serialize)]
pub struct AcceptOutcome {
    /// The new MAIN version (head) carrying the variant's bytes.
    pub saved: SaveResult,
    pub run_id: String,
    /// The accepted variant version.
    pub accepted: DesignVersion,
    /// The run's other variants (now `variant_rejected`).
    pub rejected: Vec<DesignVersion>,
}

/// The `direction` label a variants run stamped into a version's provenance
/// (`provenance.assist.direction`), if any.
pub fn direction_of(v: &DesignVersion) -> Option<String> {
    v.provenance
        .get("assist")
        .and_then(|a| a.get("direction"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

impl DesignService {
    /// Commit a variant draft on `variant/<run>/<k>` (head untouched, working
    /// copy untouched, no link re-index — links follow main only). The caller
    /// validates deep formats (e.g. scene3d) first; the cheap checks run here.
    #[allow(clippy::too_many_arguments)]
    pub async fn commit_variant(
        &self,
        a: &DesignArtifact,
        bytes: Vec<u8>,
        run_id: &str,
        k: usize,
        base_version_id: Option<&str>,
        author: Author,
        message: String,
        provenance: Value,
    ) -> Result<DesignVersion> {
        let spec = format::spec(&a.format)
            .ok_or_else(|| Error::Invalid(format!("unknown design format {:?}", a.format)))?;
        format::validate(spec, &bytes)?;
        if k == 0 || k > MAX_VARIANTS {
            return Err(Error::Invalid(format!(
                "variant index must be 1..={MAX_VARIANTS}"
            )));
        }
        if !one_of(AUTHOR_KINDS, &author.kind) {
            return Err(Error::Invalid(format!(
                "unknown author kind {:?}",
                author.kind
            )));
        }
        let provenance = bound_json(provenance, MAX_PROVENANCE_BYTES, "provenance")?;
        let message: String = message.trim().chars().take(2_000).collect();
        let sha = self.blobs().put(&bytes).await?;
        self.store()
            .insert_side_version(
                NewVersion {
                    artifact_id: a.id.clone(),
                    blob_sha256: sha,
                    size_bytes: bytes.len() as i64,
                    kind: "agent".into(),
                    branch: branch_name(run_id, k),
                    author_kind: author.kind.clone(),
                    author_id: author.id.clone(),
                    session_id: author.session_id.clone(),
                    message,
                    provenance,
                },
                base_version_id,
            )
            .await
    }

    /// Every version of one variants run, `k` ascending.
    pub async fn variant_versions(
        &self,
        artifact_id: &str,
        run_id: &str,
    ) -> Result<Vec<DesignVersion>> {
        let mut out = self
            .store()
            .versions_on_branch_prefix(artifact_id, &run_prefix(run_id))
            .await?;
        out.sort_by_key(|v| {
            parse_branch(&v.branch)
                .map(|(_, k)| k)
                .unwrap_or(usize::MAX)
        });
        Ok(out)
    }

    /// The run ids that already had a variant accepted (from the signals).
    pub async fn accepted_runs(&self, artifact_id: &str) -> Result<Vec<String>> {
        let sigs = self
            .store()
            .list_signals(
                None,
                Some(artifact_id),
                Some("variant_accepted"),
                None,
                1_000,
            )
            .await?;
        Ok(sigs
            .iter()
            .filter_map(|s| s.payload.get("run_id").and_then(Value::as_str))
            .map(str::to_string)
            .collect())
    }

    /// Accept variant `variant` (a version id, `v12` or `12`): fast-forward
    /// main to its bytes and record the learning signals. Refused (409) when
    /// the run already had a variant accepted, or — unless `force` — when main
    /// moved since the run started (the variant was drawn on an older head).
    pub async fn accept_variant(
        &self,
        artifact_id: &str,
        variant: &str,
        actor: &Author,
        force: bool,
    ) -> Result<AcceptOutcome> {
        let a = self.store().require_artifact(artifact_id).await?;
        let v = self.resolve_version(&a, variant).await?;
        let (run_id, k) = parse_branch(&v.branch).ok_or_else(|| {
            Error::Invalid(format!(
                "version {} is not a variant (branch {})",
                v.id, v.branch
            ))
        })?;
        if self
            .accepted_runs(&a.id)
            .await?
            .iter()
            .any(|r| r == &run_id)
        {
            return Err(Error::Conflict(format!(
                "a variant of run {run_id} was already accepted"
            )));
        }
        let head = a.head_version_id.clone();
        if !force && head != v.parent_version_id {
            return Err(Error::Conflict(format!(
                "main moved since the variants were generated (head {}, variants drawn on {}); \
                 re-run the variants or accept with force",
                head.as_deref().unwrap_or("(none)"),
                v.parent_version_id.as_deref().unwrap_or("(none)")
            )));
        }
        let bytes = self.blobs().get(&v.blob_sha256).await?;
        let mut provenance = if v.provenance.is_object() {
            v.provenance.clone()
        } else {
            json!({})
        };
        provenance["accepted_variant"] = json!({
            "version_id": v.id, "run_id": run_id, "k": k, "seq": v.seq,
            "accepted_by": actor.id,
        });
        // A variant's provenance is already bounded; the extra key is tiny, but
        // never let it fail the accept — fall back to the minimal record.
        let provenance = bound_json(provenance, MAX_PROVENANCE_BYTES, "provenance").unwrap_or_else(
            |_| json!({ "accepted_variant": { "version_id": v.id, "run_id": run_id, "k": k } }),
        );
        let first_line = v.message.lines().next().unwrap_or("").trim();
        let message = if first_line.is_empty() {
            format!("Accepted variant {k}")
        } else {
            format!("Accepted variant {k}: {first_line}")
        };
        let saved = self
            .commit_bytes(
                &a,
                bytes,
                SaveOpts {
                    base: Some(head.clone().unwrap_or_default()),
                    kind: "agent".into(),
                    // The bytes are the agent's draft; the human decided. The
                    // next human edit is then an `edit_after_draft` against it.
                    author: Author {
                        kind: "agent".into(),
                        id: actor.id.clone(),
                        session_id: v.session_id.clone(),
                    },
                    message,
                    provenance,
                    force: true,
                    validate: true,
                    change: "content",
                },
            )
            .await?;

        let rejected: Vec<DesignVersion> = self
            .variant_versions(&a.id, &run_id)
            .await?
            .into_iter()
            .filter(|x| x.id != v.id)
            .collect();
        let _ = self
            .record_signal_row(NewSignal {
                workspace_id: a.workspace_id.clone(),
                artifact_id: a.id.clone(),
                version_id: Some(v.id.clone()),
                kind: "variant_accepted".into(),
                actor_kind: actor.kind.clone(),
                actor_id: actor.id.clone(),
                session_id: v.session_id.clone(),
                payload: json!({
                    "run_id": run_id,
                    "k": k,
                    "direction": direction_of(&v),
                    "format": a.format,
                    "main_version_id": saved.version.id,
                    "rejected_version_ids": rejected.iter().map(|x| x.id.clone()).collect::<Vec<Id>>(),
                }),
            })
            .await;
        for r in &rejected {
            let _ = self
                .record_signal_row(NewSignal {
                    workspace_id: a.workspace_id.clone(),
                    artifact_id: a.id.clone(),
                    version_id: Some(r.id.clone()),
                    kind: "variant_rejected".into(),
                    actor_kind: actor.kind.clone(),
                    actor_id: actor.id.clone(),
                    session_id: r.session_id.clone(),
                    payload: json!({
                        "run_id": run_id,
                        "k": parse_branch(&r.branch).map(|(_, k)| k),
                        "direction": direction_of(r),
                        "format": a.format,
                        "chosen_version_id": v.id,
                    }),
                })
                .await;
        }
        Ok(AcceptOutcome {
            saved,
            run_id,
            accepted: v,
            rejected,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::CreateInput;
    use otto_core::event::Event;
    use tokio::sync::broadcast;

    #[test]
    fn branch_names_round_trip_and_reject_main() {
        assert_eq!(branch_name("r1", 2), "variant/r1/2");
        assert_eq!(run_prefix("r1"), "variant/r1/");
        assert_eq!(parse_branch("variant/r1/2"), Some(("r1".into(), 2)));
        assert_eq!(parse_branch("main"), None);
        assert_eq!(parse_branch("variant//2"), None);
        assert_eq!(parse_branch("variant/r1/0"), None);
        assert_eq!(parse_branch("variant/r1/x"), None);
        assert_eq!(parse_branch("variant/a/b/3"), None);
    }

    async fn svc() -> (DesignService, tempfile::TempDir, broadcast::Receiver<Event>) {
        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = broadcast::channel(256);
        let s = DesignService::new(otto_state::db::test_pool().await, dir.path(), Some(tx));
        (s, dir, rx)
    }

    fn input(content: &str) -> CreateInput {
        CreateInput {
            workspace_id: "w1".into(),
            project_id: None,
            studio: None,
            format: "html".into(),
            title: "Hero".into(),
            tags: vec![],
            meta: json!({}),
            content: Some(content.as_bytes().to_vec()),
            story_id: None,
            derived_from: None,
            created_by: "u1".into(),
            author: Author::user("u1"),
            message: None,
            source: None,
            created_at: None,
            version_kind: None,
            validate: true,
        }
    }

    fn agent(session: &str) -> Author {
        Author {
            kind: "agent".into(),
            id: "u1".into(),
            session_id: Some(session.into()),
        }
    }

    #[tokio::test]
    async fn variants_never_move_head_and_accept_fast_forwards_main() {
        let (s, _dir, _rx) = svc().await;
        let a = s
            .create_artifact(input("<h1>v1</h1>"))
            .await
            .unwrap()
            .artifact;
        let base = a.head_version_id.clone();
        let wf = s.work_file(&a).unwrap();
        let mut ids = Vec::new();
        for (k, dir) in [(1usize, "defaults"), (2, "explore"), (3, "calm")] {
            let v = s
                .commit_variant(
                    &a,
                    format!("<h1>variant {k}</h1>").into_bytes(),
                    "run1",
                    k,
                    base.as_deref(),
                    agent(&format!("s{k}")),
                    format!("Draft {k}"),
                    json!({ "assist": { "direction": dir } }),
                )
                .await
                .unwrap();
            assert_eq!(v.branch, format!("variant/run1/{k}"));
            assert_eq!(v.parent_version_id, base);
            ids.push(v.id);
        }
        // Head + working copy untouched by variant commits.
        let now = s.store().require_artifact(&a.id).await.unwrap();
        assert_eq!(now.head_version_id, base);
        assert_eq!(std::fs::read_to_string(&wf).unwrap(), "<h1>v1</h1>");
        assert_eq!(s.variant_versions(&a.id, "run1").await.unwrap().len(), 3);
        // A bad index is refused before anything is written.
        assert!(s
            .commit_variant(
                &a,
                b"<p/>".to_vec(),
                "run1",
                9,
                None,
                agent("x"),
                "".into(),
                json!({})
            )
            .await
            .is_err());

        // Accept #2: main fast-forwards to its bytes, siblings are rejected.
        let out = s
            .accept_variant(&a.id, &ids[1], &Author::user("u1"), false)
            .await
            .unwrap();
        assert_eq!(out.run_id, "run1");
        assert_eq!(out.saved.version.branch, "main");
        assert_eq!(out.saved.version.parent_version_id, base);
        assert_eq!(out.rejected.len(), 2);
        let (_, bytes) = s.head_content(&out.saved.artifact).await.unwrap();
        assert_eq!(bytes, b"<h1>variant 2</h1>");
        assert_eq!(std::fs::read_to_string(&wf).unwrap(), "<h1>variant 2</h1>");
        assert_eq!(out.saved.version.provenance["accepted_variant"]["k"], 2);
        let acc = s
            .store()
            .list_signals(
                None,
                Some(a.id.as_str()),
                Some("variant_accepted"),
                None,
                10,
            )
            .await
            .unwrap();
        assert_eq!(acc.len(), 1);
        assert_eq!(acc[0].payload["direction"], "explore");
        let rej = s
            .store()
            .list_signals(
                None,
                Some(a.id.as_str()),
                Some("variant_rejected"),
                None,
                10,
            )
            .await
            .unwrap();
        assert_eq!(rej.len(), 2);

        // The same run can't be accepted twice; a main version isn't a variant.
        assert!(matches!(
            s.accept_variant(&a.id, &ids[0], &Author::user("u1"), true)
                .await,
            Err(Error::Conflict(_))
        ));
        let main_id = out.saved.version.id.clone();
        assert!(matches!(
            s.accept_variant(&a.id, &main_id, &Author::user("u1"), false)
                .await,
            Err(Error::Invalid(_))
        ));
    }

    #[tokio::test]
    async fn accept_refuses_a_moved_head_unless_forced() {
        let (s, _dir, _rx) = svc().await;
        let a = s
            .create_artifact(input("<h1>v1</h1>"))
            .await
            .unwrap()
            .artifact;
        let base = a.head_version_id.clone();
        let v = s
            .commit_variant(
                &a,
                b"<h1>alt</h1>".to_vec(),
                "r2",
                1,
                base.as_deref(),
                agent("s"),
                "Alt".into(),
                json!({}),
            )
            .await
            .unwrap();
        // A human saves on main meanwhile.
        s.commit_bytes(
            &a,
            b"<h1>human</h1>".to_vec(),
            SaveOpts {
                base: None,
                kind: "autosave".into(),
                author: Author::user("u1"),
                message: String::new(),
                provenance: json!({}),
                force: false,
                validate: true,
                change: "content",
            },
        )
        .await
        .unwrap();
        assert!(matches!(
            s.accept_variant(&a.id, &v.id, &Author::user("u1"), false)
                .await,
            Err(Error::Conflict(_))
        ));
        let out = s
            .accept_variant(&a.id, &format!("v{}", v.seq), &Author::user("u1"), true)
            .await
            .unwrap();
        let (_, bytes) = s.head_content(&out.saved.artifact).await.unwrap();
        assert_eq!(bytes, b"<h1>alt</h1>");
    }
}
