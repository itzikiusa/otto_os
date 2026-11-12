//! Legacy-source mirror — keeps the graph's copy of every Product story
//! design attachment and Canvas scene in step with its source WITHOUT moving
//! or modifying the source (the old rows, files and routes keep working
//! exactly as before).
//!
//! Sources:
//!   - `product_attachments` of kind `design` | `mockup`, plus every image
//!     (`image/*`) and model (`model/gltf-binary`, `model/gltf+json`)
//!     attachment → one artifact each, linked to its story (`implements`);
//!     Blender outputs (`meta.derived_from`) also get a `derived_from` link to
//!     the artifact mirrored from their source attachment.
//!   - `canvas_scenes` → one `otto-canvas` whiteboard artifact each (the scene
//!     document made self-contained with its image bytes), linked to its story.
//!
//! [`sync_one`] mirrors ONE source: create the artifact, or commit a `sync`
//! version when the source changed (history kept), or — when a person
//! adopted the mirror (`crate::mirror`) — a side version
//! `variant/sync-<ts>/1` that never moves the head. A source that has
//! disappeared ARCHIVES its (unadopted) mirror; nothing is ever deleted.
//! `crate::sync` calls it live, debounced per source; [`run`] is the
//! catch-up pass over every source (daemon start, `POST /design/admin/import`,
//! a lagged event receiver).
//!
//! Idempotency: `design_artifacts(source_kind, source_id)` is unique, and the
//! mirror remembers each source's `updated_at` + content sha in
//! `meta.imported_from`. An unchanged source is skipped without reading its
//! bytes. A process-wide lock serializes every pass and single-source sync.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::OnceLock;

use chrono::{DateTime, Utc};
use otto_core::{Error, Result};
use serde_json::{json, Value};
use sqlx::Row;
use tokio::sync::Mutex;

use crate::blobs;
use crate::format;
use crate::mirror;
use crate::service::{Author, CreateInput, DesignService, SaveOpts};
use crate::sync::SourceRef;
use crate::types::{DesignArtifact, ImportReport, UpdateArtifactReq};

pub const SOURCE_ATTACHMENT: &str = "product_attachment";
pub const SOURCE_SCENE: &str = "canvas_scene";
/// Where product attachments live under the data dir (`product_media.rs`).
pub const ATTACH_ROOT: &str = "product/attachments";
const MAX_REPORTED_ERRORS: usize = 50;
/// Legacy attachment `meta_json` keys worth carrying into the graph row.
const LEGACY_META_KEYS: &[&str] = &["group", "format", "derived_from", "session_id", "run_id"];
const ATTACHMENT_COLS: &str = "id, story_id, workspace_id, filename, mime, storage_path, kind, \
                               source, meta_json, created_by, created_at, updated_at";

fn import_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Is a legacy attachment row design content the Hall should mirror?
pub fn is_design_attachment(kind: &str, mime: &str) -> bool {
    matches!(kind, "design" | "mockup")
        || mime.starts_with("image/")
        || matches!(mime, "model/gltf-binary" | "model/gltf+json")
}

/// `"AI screen.html"` → `"AI screen"` (falls back to the whole name).
pub fn title_from_filename(name: &str) -> String {
    let stem = Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(name);
    if stem.trim().is_empty() {
        "Untitled".to_string()
    } else {
        stem.to_string()
    }
}

fn parse_ts(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

fn push_err(report: &mut ImportReport, msg: String) {
    tracing::warn!("design import: {msg}");
    if report.errors.len() < MAX_REPORTED_ERRORS {
        report.errors.push(msg);
    }
}

/// Read a legacy attachment file, confined (after symlink resolution) under
/// the attachments root — the same containment `product_media` enforces.
async fn read_attachment(
    data_dir: &Path,
    storage_path: &str,
) -> std::result::Result<Vec<u8>, String> {
    let root = data_dir.join(ATTACH_ROOT);
    let full = otto_core::paths::confine_join(data_dir, storage_path)
        .ok_or_else(|| "path escapes the data dir".to_string())?;
    let canon = tokio::fs::canonicalize(&full)
        .await
        .map_err(|e| format!("file unavailable: {e}"))?;
    let root_c = tokio::fs::canonicalize(&root)
        .await
        .map_err(|e| format!("attachments root unavailable: {e}"))?;
    if !canon.starts_with(&root_c) {
        return Err("path is outside the attachments root".into());
    }
    let md = tokio::fs::metadata(&canon)
        .await
        .map_err(|e| format!("stat: {e}"))?;
    if md.len() as usize > format::MAX_CONTENT_BYTES {
        return Err(format!("file is {} bytes (over the design cap)", md.len()));
    }
    tokio::fs::read(&canon)
        .await
        .map_err(|e| format!("read: {e}"))
}

/// Record the source's latest `updated_at` + sha on the mirror row.
async fn stamp_source(
    svc: &DesignService,
    a: &DesignArtifact,
    updated_at: &str,
    sha: &str,
) -> Result<()> {
    let lock = crate::service::commit_lock(svc.root(), &a.id).await?;
    let _publication = lock.lock().await;
    // Import inspected its source before this boundary. Preserve all edits
    // acknowledged since that snapshot, including nested import metadata.
    let mut next = svc.store().require_artifact(&a.id).await?;
    if !next.meta.is_object() {
        next.meta = json!({});
    }
    if !next.meta["imported_from"].is_object() {
        next.meta["imported_from"] = json!({});
    }
    next.meta["imported_from"]["source_updated_at"] = json!(updated_at);
    next.meta["imported_from"]["source_sha256"] = json!(sha);
    if a.source_kind.as_deref() == Some(SOURCE_SCENE) {
        next.meta["imported_from"]["portable_images"] = json!(true);
    }
    svc.store().write_artifact_meta(&next).await?;
    Ok(())
}

/// A changed source reached an existing mirror: a `sync` head for a mirror
/// (guarded on the head it has — nothing can have edited it, the write
/// routes refuse mirrors), a side version `variant/sync-<ts>/1` for an
/// adopted one (the Hall owns its head now; nothing is overwritten), nothing
/// for an unchanged sha. Then the source stamp moves.
#[allow(clippy::too_many_arguments)]
async fn sync_existing(
    svc: &DesignService,
    mirror_id: &str,
    bytes: Vec<u8>,
    sha: &str,
    updated_at: &str,
    source_kind: &str,
    source_id: &str,
    report: &mut ImportReport,
) -> Result<()> {
    // Fresh: the index snapshot may predate an adopt.
    let a = svc.store().require_artifact(mirror_id).await?;
    let label = match source_kind {
        SOURCE_SCENE => "the Canvas scene",
        _ => "the product attachment",
    };
    let provenance = json!({ "imported_from": { "kind": source_kind, "id": source_id } });
    if a.meta["imported_from"]["source_sha256"].as_str() == Some(sha) {
        report.unchanged += 1;
    } else if mirror::is_adopted(&a) {
        let run = format!("sync-{}", Utc::now().format("%Y%m%dT%H%M%S%3fZ"));
        let side = svc
            .commit_side(
                &a,
                bytes,
                crate::variants::branch_name(&run, 1),
                "sync",
                a.head_version_id.as_deref(),
                Author::system("import"),
                format!("Changed in {label} since you adopted it"),
                provenance,
            )
            .await;
        if let Err(e) = side {
            push_err(
                report,
                format!("{source_kind} {source_id}: sync failed: {e}"),
            );
            return Ok(());
        }
        report.side_versions += 1;
    } else {
        let synced = svc
            .commit_bytes(
                &a,
                bytes,
                SaveOpts {
                    base: a.head_version_id.clone(),
                    kind: "sync".into(),
                    author: Author::system("import"),
                    message: format!("Synced from {label}"),
                    provenance,
                    force: false,
                    validate: false,
                    change: "content",
                },
            )
            .await;
        if let Err(e) = synced {
            push_err(
                report,
                format!("{source_kind} {source_id}: sync failed: {e}"),
            );
            return Ok(());
        }
        report.synced += 1;
    }
    let fresh = svc.store().require_artifact(&a.id).await?;
    stamp_source(svc, &fresh, updated_at, sha).await
}

/// The source row is gone: archive its mirror (never delete it) and flag
/// `meta.source_missing`. An adopted mirror is the Hall's own design now —
/// it only gets the flag.
async fn archive_vanished(
    svc: &DesignService,
    mirror_id: &str,
    report: &mut ImportReport,
) -> Result<()> {
    let a = svc.store().require_artifact(mirror_id).await?;
    let archive = !mirror::is_adopted(&a) && a.status != "archived";
    if !archive && a.meta.get("source_missing") == Some(&json!(true)) {
        return Ok(());
    }
    svc.update_meta(
        &a.id,
        UpdateArtifactReq {
            status: if archive {
                Some("archived".to_string())
            } else {
                None
            },
            meta: Some(json!({ "source_missing": true })),
            ..Default::default()
        },
        &Author::system("import"),
    )
    .await?;
    if archive {
        report.archived += 1;
    }
    Ok(())
}

/// Mirror ONE legacy source now (create / sync / side version / archive).
/// The returned report counts what happened to it.
pub async fn sync_one(svc: &DesignService, src: &SourceRef) -> Result<ImportReport> {
    let _guard = import_lock().lock().await;
    let mut report = ImportReport::default();
    let existing = svc.store().find_by_source(src.kind(), src.id()).await?;
    match src {
        SourceRef::Attachment(id) => {
            let row = sqlx::query(sqlx::AssertSqlSafe(format!(
                "SELECT {ATTACHMENT_COLS} FROM product_attachments WHERE id = ?"
            )))
            .bind(id)
            .fetch_optional(svc.store().pool())
            .await
            .map_err(|e| Error::Internal(format!("design sync: read attachment: {e}")))?;
            let Some(row) = row.as_ref().map(AttachmentRow::from_row) else {
                if let Some(a) = &existing {
                    archive_vanished(svc, &a.id, &mut report).await?;
                }
                return Ok(report);
            };
            let mut derived = Vec::new();
            sync_attachment(svc, &row, existing.as_ref(), &mut derived, &mut report).await?;
            link_derived(svc, derived, &mut report).await?;
        }
        SourceRef::Scene(id) => {
            let updated_at: Option<String> =
                sqlx::query_scalar("SELECT updated_at FROM canvas_scenes WHERE id = ?")
                    .bind(id)
                    .fetch_optional(svc.store().pool())
                    .await
                    .map_err(|e| Error::Internal(format!("design sync: read scene: {e}")))?;
            match updated_at {
                Some(at) => sync_scene(svc, id, &at, existing.as_ref(), &mut report).await?,
                None => {
                    if let Some(a) = &existing {
                        archive_vanished(svc, &a.id, &mut report).await?;
                    }
                }
            }
        }
    }
    for e in &report.errors {
        tracing::warn!(kind = src.kind(), id = src.id(), "design sync: {e}");
    }
    Ok(report)
}

/// The catch-up pass over every source. Per-row failures are reported,
/// never fatal.
pub async fn run(svc: &DesignService) -> Result<ImportReport> {
    let _guard = import_lock().lock().await;
    let mut report = ImportReport::default();
    let index = svc.store().source_index().await?;
    let attachments = import_attachments(svc, &index, &mut report).await?;
    let scenes = import_scenes(svc, &index, &mut report).await?;
    // Mirrors whose source row is gone (deleted while the daemon was down,
    // or a missed event) are archived.
    for ((kind, id), a) in &index {
        let gone = match kind.as_str() {
            SOURCE_ATTACHMENT => !attachments.contains(id),
            SOURCE_SCENE => !scenes.contains(id),
            _ => false,
        };
        if gone {
            if let Err(e) = archive_vanished(svc, &a.id, &mut report).await {
                push_err(&mut report, format!("{kind} {id}: archive failed: {e}"));
            }
        }
    }
    Ok(report)
}

/// One `product_attachments` row ([`ATTACHMENT_COLS`]).
struct AttachmentRow {
    id: String,
    story_id: String,
    workspace_id: String,
    filename: String,
    mime: String,
    storage_path: String,
    kind: String,
    source: String,
    meta_json: Option<String>,
    created_by: String,
    created_at: String,
    updated_at: String,
}

impl AttachmentRow {
    fn from_row(r: &sqlx::sqlite::SqliteRow) -> Self {
        Self {
            id: r.get("id"),
            story_id: r.get("story_id"),
            workspace_id: r.get("workspace_id"),
            filename: r.get("filename"),
            mime: r.get("mime"),
            storage_path: r.get("storage_path"),
            kind: r.get("kind"),
            source: r.get("source"),
            meta_json: r.get("meta_json"),
            created_by: r.get("created_by"),
            created_at: r.get("created_at"),
            updated_at: r.get("updated_at"),
        }
    }
}

/// Every attachment, mirrored; returns the ids of ALL attachment rows (the
/// vanished-source check needs to know which still exist).
async fn import_attachments(
    svc: &DesignService,
    index: &HashMap<(String, String), DesignArtifact>,
    report: &mut ImportReport,
) -> Result<HashSet<String>> {
    let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {ATTACHMENT_COLS} FROM product_attachments ORDER BY created_at"
    )))
    .fetch_all(svc.store().pool())
    .await
    .map_err(|e| Error::Internal(format!("design import: list attachments: {e}")))?;

    // (new artifact id, the attachment it was rendered from) — resolved after
    // every attachment is mirrored.
    let mut derived: Vec<(String, String)> = Vec::new();
    let mut seen = HashSet::new();
    for r in &rows {
        let row = AttachmentRow::from_row(r);
        seen.insert(row.id.clone());
        let existing = index.get(&(SOURCE_ATTACHMENT.to_string(), row.id.clone()));
        if let Err(e) = sync_attachment(svc, &row, existing, &mut derived, report).await {
            push_err(report, format!("attachment {}: {e}", row.id));
        }
    }
    link_derived(svc, derived, report).await?;
    Ok(seen)
}

/// Mirror one attachment row (`existing`: its mirror, if any).
async fn sync_attachment(
    svc: &DesignService,
    r: &AttachmentRow,
    existing: Option<&DesignArtifact>,
    derived: &mut Vec<(String, String)>,
    report: &mut ImportReport,
) -> Result<()> {
    if !is_design_attachment(&r.kind, &r.mime) {
        return Ok(());
    }
    report.attachments_scanned += 1;
    let id = &r.id;
    let Some(spec) = format::from_mime(&r.mime) else {
        report.skipped += 1;
        return Ok(());
    };
    if let Some(a) = existing {
        if a.meta["imported_from"]["source_updated_at"].as_str() == Some(r.updated_at.as_str()) {
            report.unchanged += 1;
            return Ok(());
        }
    }
    let bytes = match read_attachment(svc.data_dir(), &r.storage_path).await {
        Ok(b) => b,
        Err(e) => {
            push_err(report, format!("attachment {id}: {e}"));
            report.skipped += 1;
            return Ok(());
        }
    };
    let sha = blobs::sha256_hex(&bytes);

    if let Some(a) = existing {
        return sync_existing(
            svc,
            &a.id,
            bytes,
            &sha,
            &r.updated_at,
            SOURCE_ATTACHMENT,
            id,
            report,
        )
        .await;
    }

    let legacy: Value = r
        .meta_json
        .as_deref()
        .and_then(|m| serde_json::from_str(m).ok())
        .unwrap_or_else(|| json!({}));
    let mut legacy_meta = serde_json::Map::new();
    for k in LEGACY_META_KEYS {
        if let Some(v) = legacy.get(*k).filter(|v| v.is_string() || v.is_number()) {
            legacy_meta.insert((*k).to_string(), v.clone());
        }
    }
    let tags: Vec<String> = legacy
        .get("group")
        .and_then(Value::as_str)
        .map(|g| vec![g.to_string()])
        .unwrap_or_default();
    let derived_from = legacy
        .get("derived_from")
        .and_then(Value::as_str)
        .map(str::to_string);
    let meta = json!({
        "imported_from": {
            "kind": SOURCE_ATTACHMENT,
            "id": id,
            "story_id": r.story_id,
            "filename": r.filename,
            "attachment_kind": r.kind,
            "source": r.source,
            "source_sha256": sha,
            "source_updated_at": r.updated_at,
        },
        "legacy_meta": Value::Object(legacy_meta),
    });
    let input = CreateInput {
        workspace_id: r.workspace_id.clone(),
        project_id: None,
        studio: None,
        format: spec.name.to_string(),
        title: title_from_filename(&r.filename),
        tags,
        meta,
        content: Some(bytes),
        story_id: Some(r.story_id.clone()),
        derived_from: None,
        created_by: r.created_by.clone(),
        author: Author::system("import"),
        message: Some("Imported from a product story attachment".into()),
        source: Some((SOURCE_ATTACHMENT.to_string(), id.clone())),
        created_at: parse_ts(&r.created_at),
        version_kind: Some("import".into()),
        validate: false,
    };
    match svc.create_artifact(input).await {
        Ok(saved) => {
            report.created += 1;
            report.links_created += svc
                .store()
                .story_ids_for(&saved.artifact.id)
                .await
                .map(|v| v.len())
                .unwrap_or(0);
            if let Some(d) = derived_from {
                derived.push((saved.artifact.id.clone(), d));
            }
        }
        // A concurrent run mirrored it first — idempotent by the unique key.
        Err(Error::Conflict(_)) => report.unchanged += 1,
        Err(e) => push_err(report, format!("attachment {id}: {e}")),
    }
    Ok(())
}

/// Blender renders / exports → `derived_from` their source scene (pinned to
/// the version mirrored now).
async fn link_derived(
    svc: &DesignService,
    derived: Vec<(String, String)>,
    report: &mut ImportReport,
) -> Result<()> {
    for (artifact_id, source_attachment) in derived {
        let Some(src) = svc
            .store()
            .find_by_source(SOURCE_ATTACHMENT, &source_attachment)
            .await?
        else {
            continue;
        };
        let link = crate::store::NewLink {
            src_artifact_id: artifact_id.clone(),
            src_version_id: None,
            src_node: String::new(),
            dst_kind: "artifact".into(),
            dst_id: src.id.clone(),
            dst_node: String::new(),
            rel: "derived_from".into(),
            policy: "pinned".into(),
            pinned_version_id: src.head_version_id.clone(),
            origin: "explicit".into(),
            broken: false,
            meta: json!({ "imported_from": { "derived_from": source_attachment } }),
            created_by: "import".into(),
        };
        if svc.store().insert_link(&link).await.is_ok() {
            report.links_created += 1;
        }
    }
    Ok(())
}

/// Every scene, mirrored; returns the ids of all scene rows.
async fn import_scenes(
    svc: &DesignService,
    index: &HashMap<(String, String), DesignArtifact>,
    report: &mut ImportReport,
) -> Result<HashSet<String>> {
    // D13: list ids + stamps only; a scene's (up to 25 MB) `doc_json` is read
    // only when it changed since the last import — the boot import used to
    // load every scene document to skip nearly all of them.
    let heads = sqlx::query("SELECT id, updated_at FROM canvas_scenes ORDER BY created_at")
        .fetch_all(svc.store().pool())
        .await
        .map_err(|e| Error::Internal(format!("design import: list canvas scenes: {e}")))?;

    let mut seen = HashSet::new();
    for h in &heads {
        let id: String = h.get("id");
        let updated_at: String = h.get("updated_at");
        seen.insert(id.clone());
        let existing = index.get(&(SOURCE_SCENE.to_string(), id.clone()));
        if let Err(e) = sync_scene(svc, &id, &updated_at, existing, report).await {
            push_err(report, format!("canvas scene {id}: {e}"));
        }
    }
    Ok(seen)
}

/// Mirror one scene (`updated_at`: its row's stamp; `existing`: its mirror).
async fn sync_scene(
    svc: &DesignService,
    id: &str,
    updated_at: &str,
    existing: Option<&DesignArtifact>,
    report: &mut ImportReport,
) -> Result<()> {
    report.scenes_scanned += 1;
    if let Some(a) = existing {
        if a.meta["imported_from"]["source_updated_at"].as_str() == Some(updated_at)
            && a.meta["imported_from"]["portable_images"] == true
        {
            report.unchanged += 1;
            return Ok(());
        }
    }
    let repo = otto_state::CanvasRepo::new(svc.store().pool().clone());
    let scene = match repo.portable_scene(&id.to_string()).await {
        Ok(Some(scene)) => scene,
        Ok(None) => return Ok(()),
        Err(e) => {
            push_err(report, format!("canvas scene {id}: {e}"));
            return Ok(());
        }
    };
    let updated_at = scene.updated_at.to_rfc3339();
    let doc = scene.doc_json;
    let bytes = doc.clone().into_bytes();
    if bytes.len() > format::MAX_CONTENT_BYTES {
        push_err(
            report,
            format!("canvas scene {id}: document over the design cap"),
        );
        report.skipped += 1;
        return Ok(());
    }
    let sha = blobs::sha256_hex(&bytes);

    if let Some(a) = existing {
        return sync_existing(
            svc,
            &a.id,
            bytes,
            &sha,
            &updated_at,
            SOURCE_SCENE,
            id,
            report,
        )
        .await;
    }

    let canvas_format = serde_json::from_str::<Value>(&doc)
        .ok()
        .and_then(|v| {
            v.get("format")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| v.get("type").and_then(Value::as_str).map(str::to_string))
        })
        .unwrap_or_else(|| "mermaid".to_string());
    let section = scene.section;
    let story_id = scene.story_id;
    let title = scene.title;
    let created_at = scene.created_at;
    let meta = json!({
        "imported_from": {
            "kind": SOURCE_SCENE,
            "id": id,
            "story_id": story_id,
            "section": section,
            "canvas_format": canvas_format,
            "portable_images": true,
            "source_sha256": sha,
            "source_updated_at": updated_at,
        }
    });
    let input = CreateInput {
        workspace_id: scene.workspace_id,
        project_id: None,
        studio: Some("whiteboard".into()),
        format: "otto-canvas".into(),
        title: if title.trim().is_empty() {
            "Untitled board".into()
        } else {
            title
        },
        tags: section.into_iter().collect(),
        meta,
        content: Some(bytes),
        story_id,
        derived_from: None,
        created_by: scene.created_by,
        author: Author::system("import"),
        message: Some("Imported from Canvas".into()),
        source: Some((SOURCE_SCENE.to_string(), id.to_string())),
        created_at: Some(created_at),
        version_kind: Some("import".into()),
        validate: false,
    };
    match svc.create_artifact(input).await {
        Ok(saved) => {
            report.created += 1;
            report.links_created += svc
                .store()
                .story_ids_for(&saved.artifact.id)
                .await
                .map(|v| v.len())
                .unwrap_or(0);
        }
        Err(Error::Conflict(_)) => report.unchanged += 1,
        Err(e) => push_err(report, format!("canvas scene {id}: {e}")),
    }
    Ok(())
}

/// Daemon-startup hook: create the FTS index (backfilling it when it is new),
/// run the catch-up pass, then keep every source in step live
/// ([`crate::sync::run_live`], until the bus closes). Logs; never fails the
/// boot.
pub async fn startup(svc: DesignService) {
    // Subscribe BEFORE the catch-up pass so nothing saved meanwhile is missed.
    let live = svc.subscribe();
    let had_rows = svc
        .store()
        .all_artifact_ids()
        .await
        .map(|v| !v.is_empty())
        .unwrap_or(false);
    let fts_before: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE name = 'design_search_fts'")
            .fetch_one(svc.store().pool())
            .await
            .unwrap_or(0);
    if !svc.store().ensure_fts().await {
        tracing::warn!("design: FTS5 unavailable — design search falls back to LIKE");
    } else if fts_before == 0 && had_rows {
        // The index is new but artifacts exist: backfill it once.
        if let Ok(ids) = svc.store().all_artifact_ids().await {
            for id in ids {
                if let Ok(Some(a)) = svc.store().get_artifact(&id).await {
                    svc.refresh_search(&a).await;
                }
            }
        }
    }
    match run(&svc).await {
        Ok(r) => tracing::info!(
            created = r.created,
            synced = r.synced,
            unchanged = r.unchanged,
            skipped = r.skipped,
            archived = r.archived,
            side_versions = r.side_versions,
            errors = r.errors.len(),
            "design: legacy import done"
        ),
        Err(e) => tracing::warn!("design: legacy import failed: {e}"),
    }
    if let Some(rx) = live {
        crate::sync::run_live(svc, rx, crate::sync::DEBOUNCE).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn seed_story(pool: &otto_state::DbPool, id: &str, ws: &str) {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO product_stories
             (id, workspace_id, source_kind, account_id, source_key, title, url,
              created_by, created_at, updated_at)
             VALUES (?, ?, 'jira', 'acc', 'LOY-142', 'Rewards launch', 'https://x', 'u1', ?, ?)",
        )
        .bind(id)
        .bind(ws)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();
    }

    #[allow(clippy::too_many_arguments)]
    async fn seed_attachment(
        pool: &otto_state::DbPool,
        data_dir: &Path,
        id: &str,
        story: &str,
        kind: &str,
        mime: &str,
        body: &[u8],
        meta: Option<&str>,
    ) {
        let rel = format!("{ATTACH_ROOT}/{story}/{id}.bin");
        let full = data_dir.join(&rel);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(&full, body).unwrap();
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO product_attachments
             (id, story_id, workspace_id, filename, mime, size_bytes, sha256, storage_path,
              kind, source, meta_json, created_by, created_at, updated_at)
             VALUES (?, ?, 'w1', ?, ?, ?, NULL, ?, ?, 'agent', ?, 'u1', ?, ?)",
        )
        .bind(id)
        .bind(story)
        .bind(format!("{id}.file"))
        .bind(mime)
        .bind(body.len() as i64)
        .bind(&rel)
        .bind(kind)
        .bind(meta)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_scene(pool: &otto_state::DbPool, id: &str, story: Option<&str>, doc: &str) {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO canvas_scenes
             (id, workspace_id, story_id, title, doc_json, thumbnail, provider, section,
              created_by, created_at, updated_at)
             VALUES (?, 'w1', ?, 'Checkout flow', ?, NULL, 'claude', 'Flows', 'u1', ?, ?)",
        )
        .bind(id)
        .bind(story)
        .bind(doc)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();
    }

    #[test]
    fn classifies_design_attachments_and_titles() {
        assert!(is_design_attachment("mockup", "text/html"));
        assert!(is_design_attachment("file", "image/png"));
        assert!(is_design_attachment("file", "model/gltf-binary"));
        assert!(!is_design_attachment("file", "application/pdf"));
        assert_eq!(title_from_filename("AI screen.html"), "AI screen");
        assert_eq!(title_from_filename(".html"), ".html");
        assert_eq!(title_from_filename(""), "Untitled");
    }

    #[tokio::test]
    async fn stamp_source_preserves_metadata_approved_after_its_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let pool = otto_state::db::test_pool().await;
        let svc = DesignService::new(pool.clone(), dir.path(), None);
        assert!(svc.store().ensure_fts().await);
        seed_scene(&pool, "scene-stamp", None, &json!({
            "type":"otto-canvas", "version":1, "format":"mermaid", "source":"flowchart TD\n A --> B\n"
        }).to_string()).await;
        run(&svc).await.unwrap();
        let stale = svc
            .store()
            .find_by_source(SOURCE_SCENE, "scene-stamp")
            .await
            .unwrap()
            .unwrap();
        // The importer already read `stale`; a human commits acknowledged
        // metadata, approval and thumbnail before the import stamps its hash.
        svc.update_meta(
            &stale.id,
            crate::types::UpdateArtifactReq {
                title: Some("Human title".into()),
                tags: Some(vec!["human-tag".into()]),
                meta: Some(
                    json!({"human":true,"imported_from":{"id":"scene-stamp","custom":"preserve"}}),
                ),
                ..Default::default()
            },
            &Author::user("u1"),
        )
        .await
        .unwrap();
        svc.approve(&stale.id, None, &Author::user("u1"))
            .await
            .unwrap();
        let fresh = svc
            .set_thumbnail(&stale, b"\x89PNG\r\n\x1a\nstamp-thumb")
            .await
            .unwrap();
        stamp_source(&svc, &stale, "2026-10-05T00:00:00Z", "new-source-hash")
            .await
            .unwrap();
        let saved = svc.store().require_artifact(&stale.id).await.unwrap();
        assert_eq!(saved.title, "Human title");
        assert_eq!(saved.tags, vec!["human-tag"]);
        assert_eq!(saved.status, "approved");
        assert_eq!(saved.approved_version_id, fresh.approved_version_id);
        assert_eq!(saved.thumb_blob, fresh.thumb_blob);
        assert_eq!(saved.meta["human"], true);
        assert_eq!(saved.meta["imported_from"]["custom"], "preserve");
        assert_eq!(
            saved.meta["imported_from"]["source_sha256"],
            "new-source-hash"
        );
    }

    #[tokio::test]
    async fn import_is_idempotent_links_stories_and_syncs_changes() {
        let dir = tempfile::tempdir().unwrap();
        let pool = otto_state::db::test_pool().await;
        let svc = DesignService::new(pool.clone(), dir.path(), None);
        assert!(svc.store().ensure_fts().await);
        seed_story(&pool, "st1", "w1").await;
        let scene = serde_json::json!({
            "type": "otto-canvas", "version": 1, "format": "mermaid",
            "source": "flowchart TD\n A --> B\n"
        })
        .to_string();
        seed_attachment(
            &pool,
            dir.path(),
            "att1",
            "st1",
            "design",
            "application/vnd.otto.scene3d+json",
            br#"{"type":"otto-scene3d","version":1,"objects":[{"id":"hero","type":"gltf","attachment_id":"glb1"}]}"#,
            Some(r#"{"format":"scene3d","group":"Models"}"#),
        )
        .await;
        seed_attachment(
            &pool,
            dir.path(),
            "glb1",
            "st1",
            "design",
            "model/gltf-binary",
            b"glTF\x02\x00\x00\x00\x0c\x00\x00\x00",
            Some(r#"{"derived_from":"att1"}"#),
        )
        .await;
        seed_attachment(
            &pool,
            dir.path(),
            "doc1",
            "st1",
            "file",
            "application/pdf",
            b"%PDF-1",
            None,
        )
        .await;
        seed_scene(&pool, "sc1", Some("st1"), &scene).await;

        let r1 = run(&svc).await.unwrap();
        assert_eq!(r1.attachments_scanned, 2, "{r1:?}");
        assert_eq!(r1.scenes_scanned, 1);
        assert_eq!(r1.created, 3, "{r1:?}");
        assert!(r1.errors.is_empty(), "{r1:?}");

        let scene3d = svc
            .store()
            .find_by_source(SOURCE_ATTACHMENT, "att1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(scene3d.format, "scene3d");
        assert_eq!(scene3d.studio, "3d");
        assert_eq!(scene3d.tags, vec!["Models".to_string()]);
        assert_eq!(scene3d.meta["imported_from"]["id"], "att1");
        assert_eq!(
            svc.store().story_ids_for(&scene3d.id).await.unwrap(),
            vec!["st1".to_string()]
        );
        let glb = svc
            .store()
            .find_by_source(SOURCE_ATTACHMENT, "glb1")
            .await
            .unwrap()
            .unwrap();
        // The GLB derives from the scene (Blender output); the scene's gltf
        // object embeds the GLB (resolved after the second import pass or on
        // the next save — the attachment edge is kept meanwhile).
        let glb_out = svc.store().links_out(&glb.id).await.unwrap();
        assert!(glb_out
            .iter()
            .any(|l| l.rel == "derived_from" && l.dst_id == scene3d.id));
        let scene_out = svc.store().links_out(&scene3d.id).await.unwrap();
        assert!(scene_out
            .iter()
            .any(|l| l.rel == "embeds" && (l.dst_id == "glb1" || l.dst_id == glb.id)));
        let board = svc
            .store()
            .find_by_source(SOURCE_SCENE, "sc1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(board.format, "otto-canvas");
        assert_eq!(board.meta["imported_from"]["canvas_format"], "mermaid");

        // Second run: nothing new, nothing re-read.
        let r2 = run(&svc).await.unwrap();
        assert_eq!(r2.created, 0, "{r2:?}");
        assert_eq!(r2.unchanged, 3, "{r2:?}");

        // The legacy scene changes → a `sync` version; the old row untouched.
        let changed = serde_json::json!({
            "type": "otto-canvas", "version": 1, "format": "mermaid",
            "source": "flowchart TD\n A --> C\n"
        })
        .to_string();
        sqlx::query("UPDATE canvas_scenes SET doc_json = ?, updated_at = ? WHERE id = 'sc1'")
            .bind(&changed)
            .bind(Utc::now().to_rfc3339())
            .execute(&pool)
            .await
            .unwrap();
        let r3 = run(&svc).await.unwrap();
        assert_eq!(r3.synced, 1, "{r3:?}");
        let versions = svc
            .store()
            .list_versions(&board.id, None, 0, 0)
            .await
            .unwrap();
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].kind, "sync");
        assert_eq!(versions[1].kind, "import");
        let (_, head) = svc
            .head_content(&svc.store().require_artifact(&board.id).await.unwrap())
            .await
            .unwrap();
        assert_eq!(String::from_utf8(head).unwrap(), changed);
        // The legacy row is exactly as the user left it.
        let still: String =
            sqlx::query_scalar("SELECT doc_json FROM canvas_scenes WHERE id = 'sc1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(still, changed);
    }
    #[tokio::test]
    async fn imported_canvas_images_survive_source_deletion_and_fork() {
        let dir = tempfile::tempdir().unwrap();
        let pool = otto_state::db::test_pool().await;
        let svc = DesignService::new(pool.clone(), dir.path(), None);
        let repo = otto_state::CanvasRepo::new(pool);
        let image = "data:image/png;base64,aGVsbG8=";
        let inner = json!({"type":"excalidraw", "elements":[], "files": {
            "img": {"id":"img", "mimeType":"image/png", "dataURL":image}
        }});
        let scene = repo.create(otto_state::NewScene {
            workspace_id: "w1".into(), story_id: None, title: "Board".into(),
            doc_json: json!({"type":"otto-canvas", "version":1, "format":"excalidraw", "source":inner.to_string()}).to_string(),
            provider: "claude".into(), section: None, created_by: "u1".into(),
        }).await.unwrap();
        assert!(scene.doc_json.contains("otto-canvas-file:"));
        run(&svc).await.unwrap();
        let imported = svc
            .store()
            .find_by_source(SOURCE_SCENE, &scene.id)
            .await
            .unwrap()
            .unwrap();
        let fork = svc
            .create_artifact(CreateInput {
                workspace_id: "w1".into(),
                project_id: None,
                studio: None,
                format: "otto-canvas".into(),
                title: "Copy".into(),
                tags: vec![],
                meta: json!({}),
                content: None,
                story_id: None,
                derived_from: Some(crate::types::ForkFrom {
                    artifact_id: imported.id.clone(),
                    version_id: None,
                }),
                created_by: "u1".into(),
                author: Author::user("u1"),
                message: None,
                source: None,
                created_at: None,
                version_kind: None,
                validate: true,
            })
            .await
            .unwrap();
        repo.delete(&scene.id).await.unwrap();
        assert_eq!(repo.file_store_stats().await.unwrap().0, 0);
        for artifact in [&imported, &fork.artifact] {
            let body = svc.head_content(artifact).await.unwrap().1;
            let doc: Value = serde_json::from_slice(&body).unwrap();
            let inner: Value = serde_json::from_str(doc["source"].as_str().unwrap()).unwrap();
            assert_eq!(inner["files"]["img"]["dataURL"], image);
        }
    }

    async fn touch_scene(pool: &otto_state::DbPool, id: &str, source: &str) -> String {
        let doc = json!({"type":"otto-canvas", "version":1, "format":"mermaid", "source":source})
            .to_string();
        sqlx::query("UPDATE canvas_scenes SET doc_json = ?, updated_at = ? WHERE id = ?")
            .bind(&doc)
            .bind(Utc::now().to_rfc3339())
            .bind(id)
            .execute(pool)
            .await
            .unwrap();
        doc
    }

    fn scene_doc(source: &str) -> String {
        json!({"type":"otto-canvas", "version":1, "format":"mermaid", "source":source}).to_string()
    }

    #[tokio::test]
    async fn sync_one_mirrors_then_syncs_an_attachment() {
        let dir = tempfile::tempdir().unwrap();
        let pool = otto_state::db::test_pool().await;
        let svc = DesignService::new(pool.clone(), dir.path(), None);
        seed_story(&pool, "st1", "w1").await;
        seed_attachment(
            &pool,
            dir.path(),
            "att1",
            "st1",
            "mockup",
            "text/html",
            b"<h1>One</h1>",
            None,
        )
        .await;
        let src = SourceRef::Attachment("att1".into());
        let r = sync_one(&svc, &src).await.unwrap();
        assert_eq!(r.created, 1, "{r:?}");
        let a = svc
            .store()
            .find_by_source(SOURCE_ATTACHMENT, "att1")
            .await
            .unwrap()
            .expect("mirrored");
        assert_eq!(svc.store().story_ids_for(&a.id).await.unwrap(), vec!["st1"]);
        // Unchanged → nothing written.
        assert_eq!(sync_one(&svc, &src).await.unwrap().unchanged, 1);
        // The attachment is saved again → one `sync` head with its bytes.
        std::fs::write(
            dir.path().join(format!("{ATTACH_ROOT}/st1/att1.bin")),
            b"<h1>Two</h1>",
        )
        .unwrap();
        sqlx::query("UPDATE product_attachments SET updated_at = ? WHERE id = 'att1'")
            .bind(Utc::now().to_rfc3339())
            .execute(&pool)
            .await
            .unwrap();
        let r = sync_one(&svc, &src).await.unwrap();
        assert_eq!(r.synced, 1, "{r:?}");
        let a = svc.store().require_artifact(&a.id).await.unwrap();
        let (head, bytes) = svc.head_content(&a).await.unwrap();
        assert_eq!(head.kind, "sync");
        assert_eq!(bytes, b"<h1>Two</h1>");
    }

    #[tokio::test]
    async fn sync_one_mirrors_then_syncs_a_scene() {
        let dir = tempfile::tempdir().unwrap();
        let pool = otto_state::db::test_pool().await;
        let svc = DesignService::new(pool.clone(), dir.path(), None);
        seed_scene(&pool, "sc1", None, &scene_doc("flowchart TD\n A --> B\n")).await;
        let src = SourceRef::Scene("sc1".into());
        assert_eq!(sync_one(&svc, &src).await.unwrap().created, 1);
        let a = svc
            .store()
            .find_by_source(SOURCE_SCENE, "sc1")
            .await
            .unwrap()
            .expect("mirrored");
        assert_eq!(a.format, "otto-canvas");
        let changed = touch_scene(&pool, "sc1", "flowchart TD\n A --> C\n").await;
        let r = sync_one(&svc, &src).await.unwrap();
        assert_eq!(r.synced, 1, "{r:?}");
        let (head, bytes) = svc
            .head_content(&svc.store().require_artifact(&a.id).await.unwrap())
            .await
            .unwrap();
        assert_eq!(head.kind, "sync");
        assert_eq!(String::from_utf8(bytes).unwrap(), changed);
    }

    #[tokio::test]
    async fn a_vanished_source_archives_its_mirror_and_keeps_it() {
        let dir = tempfile::tempdir().unwrap();
        let pool = otto_state::db::test_pool().await;
        let svc = DesignService::new(pool.clone(), dir.path(), None);
        seed_story(&pool, "st1", "w1").await;
        seed_scene(&pool, "sc1", None, &scene_doc("flowchart TD\n A --> B\n")).await;
        seed_attachment(
            &pool,
            dir.path(),
            "att1",
            "st1",
            "design",
            "text/html",
            b"<p>x</p>",
            None,
        )
        .await;
        run(&svc).await.unwrap();
        let board = svc
            .store()
            .find_by_source(SOURCE_SCENE, "sc1")
            .await
            .unwrap()
            .unwrap();
        // Live: the scene is deleted → its mirror is archived, never deleted.
        sqlx::query("DELETE FROM canvas_scenes WHERE id = 'sc1'")
            .execute(&pool)
            .await
            .unwrap();
        let r = sync_one(&svc, &SourceRef::Scene("sc1".into()))
            .await
            .unwrap();
        assert_eq!(r.archived, 1, "{r:?}");
        let board = svc.store().require_artifact(&board.id).await.unwrap();
        assert_eq!(board.status, "archived");
        assert_eq!(board.meta["source_missing"], true);
        assert!(svc.head_content(&board).await.is_ok(), "history kept");
        // Catch-up: an attachment deleted while the daemon was down.
        sqlx::query("DELETE FROM product_attachments WHERE id = 'att1'")
            .execute(&pool)
            .await
            .unwrap();
        let r = run(&svc).await.unwrap();
        assert_eq!(r.archived, 1, "{r:?}");
        let att = svc
            .store()
            .find_by_source(SOURCE_ATTACHMENT, "att1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(att.status, "archived");
        // Idempotent: nothing more to archive.
        assert_eq!(run(&svc).await.unwrap().archived, 0);
    }

    #[tokio::test]
    async fn an_adopted_mirror_gets_a_side_version_not_a_new_head() {
        let dir = tempfile::tempdir().unwrap();
        let pool = otto_state::db::test_pool().await;
        let svc = DesignService::new(pool.clone(), dir.path(), None);
        seed_scene(&pool, "sc1", None, &scene_doc("flowchart TD\n A --> B\n")).await;
        let src = SourceRef::Scene("sc1".into());
        sync_one(&svc, &src).await.unwrap();
        let a = svc
            .store()
            .find_by_source(SOURCE_SCENE, "sc1")
            .await
            .unwrap()
            .unwrap();
        let adopted = svc.adopt(&a.id, &Author::user("u1")).await.unwrap();
        assert!(mirror::is_adopted(&adopted));
        let signals = svc
            .store()
            .list_signals(None, Some(a.id.as_str()), Some("adopted"), None, 10)
            .await
            .unwrap();
        assert_eq!(signals.len(), 1);
        // A Hall edit after adopting…
        let edited = scene_doc("flowchart TD\n A --> Hall\n");
        svc.commit_bytes(
            &adopted,
            edited.clone().into_bytes(),
            SaveOpts {
                base: adopted.head_version_id.clone(),
                kind: "autosave".into(),
                author: Author::user("u1"),
                message: String::new(),
                provenance: Value::Null,
                force: false,
                validate: true,
                change: "content",
            },
        )
        .await
        .unwrap();
        // …then the Canvas scene changes too: a side version, head untouched.
        let canvas = touch_scene(&pool, "sc1", "flowchart TD\n A --> Canvas\n").await;
        let r = sync_one(&svc, &src).await.unwrap();
        assert_eq!((r.synced, r.side_versions), (0, 1), "{r:?}");
        let fresh = svc.store().require_artifact(&a.id).await.unwrap();
        let (_, head) = svc.head_content(&fresh).await.unwrap();
        assert_eq!(String::from_utf8(head).unwrap(), edited);
        let side = svc
            .store()
            .versions_on_branch_prefix(&a.id, "variant/sync-")
            .await
            .unwrap();
        assert_eq!(side.len(), 1);
        assert_eq!(side[0].kind, "sync");
        assert_eq!(side[0].parent_version_id, fresh.head_version_id);
        let (run_id, k) = crate::variants::parse_branch(&side[0].branch).unwrap();
        assert!(run_id.starts_with("sync-") && k == 1, "{}", side[0].branch);
        assert_eq!(
            svc.version_bytes(&side[0]).await.unwrap(),
            canvas.into_bytes()
        );
        // Unchanged since → no second side version.
        assert_eq!(sync_one(&svc, &src).await.unwrap().side_versions, 0);
        // The source disappears: an adopted design is the Hall's own now —
        // flagged, not archived.
        sqlx::query("DELETE FROM canvas_scenes WHERE id = 'sc1'")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(sync_one(&svc, &src).await.unwrap().archived, 0);
        let fresh = svc.store().require_artifact(&a.id).await.unwrap();
        assert_ne!(fresh.status, "archived");
        assert_eq!(fresh.meta["source_missing"], true);
        assert_eq!(fresh.meta["adopted"], true);
    }
}
