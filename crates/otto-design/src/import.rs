//! Idempotent legacy import — mirrors existing design data into the graph
//! WITHOUT moving or modifying it (the old rows, files and routes keep
//! working exactly as before).
//!
//! Sources:
//!   - `product_attachments` of kind `design` | `mockup`, plus every image
//!     (`image/*`) and model (`model/gltf-binary`, `model/gltf+json`)
//!     attachment → one artifact each, linked to its story (`implements`);
//!     Blender outputs (`meta.derived_from`) also get a `derived_from` link to
//!     the artifact mirrored from their source attachment.
//!   - `canvas_scenes` → one `otto-canvas` whiteboard artifact each (the scene
//!     document kept verbatim), linked to its story when it has one.
//!
//! Idempotency: `design_artifacts(source_kind, source_id)` is unique, and the
//! import remembers each source's `updated_at` + content sha in
//! `meta.imported_from`. An unchanged source is skipped without reading its
//! bytes; a changed one gets a new `sync` version (history kept — nothing is
//! overwritten). A source row that disappears leaves its artifact untouched.
//!
//! Runs at daemon startup (cheap: one indexed query per table plus a map
//! lookup per row) and on demand via `POST /design/admin/import`. A process-
//! wide lock serializes concurrent runs.

use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

use chrono::{DateTime, Utc};
use otto_core::{Error, Result};
use serde_json::{json, Value};
use sqlx::Row;
use tokio::sync::Mutex;

use crate::blobs;
use crate::format;
use crate::service::{Author, CreateInput, DesignService, SaveOpts};
use crate::types::{DesignArtifact, ImportReport};

pub const SOURCE_ATTACHMENT: &str = "product_attachment";
pub const SOURCE_SCENE: &str = "canvas_scene";
/// Where product attachments live under the data dir (`product_media.rs`).
pub const ATTACH_ROOT: &str = "product/attachments";
const MAX_REPORTED_ERRORS: usize = 50;
/// Legacy attachment `meta_json` keys worth carrying into the graph row.
const LEGACY_META_KEYS: &[&str] = &["group", "format", "derived_from", "session_id", "run_id"];

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
    let mut next = a.clone();
    if !next.meta.is_object() {
        next.meta = json!({});
    }
    if !next.meta["imported_from"].is_object() {
        next.meta["imported_from"] = json!({});
    }
    next.meta["imported_from"]["source_updated_at"] = json!(updated_at);
    next.meta["imported_from"]["source_sha256"] = json!(sha);
    svc.store().write_artifact_meta(&next).await?;
    Ok(())
}

/// Commit a `sync` version from a changed legacy source (history is kept;
/// an unchanged sha writes nothing).
async fn sync_version(
    svc: &DesignService,
    a: &DesignArtifact,
    bytes: Vec<u8>,
    source_kind: &str,
    source_id: &str,
    message: &str,
) -> Result<()> {
    svc.commit_bytes(
        a,
        bytes,
        SaveOpts {
            base: None,
            kind: "sync".into(),
            author: Author::system("import"),
            message: message.into(),
            provenance: json!({ "imported_from": { "kind": source_kind, "id": source_id } }),
            force: false,
            validate: false,
            change: "content",
        },
    )
    .await?;
    Ok(())
}

/// Run the import once. Per-row failures are reported, never fatal.
pub async fn run(svc: &DesignService) -> Result<ImportReport> {
    let _guard = import_lock().lock().await;
    let mut report = ImportReport::default();
    let index = svc.store().source_index().await?;
    import_attachments(svc, &index, &mut report).await?;
    import_scenes(svc, &index, &mut report).await?;
    Ok(report)
}

async fn import_attachments(
    svc: &DesignService,
    index: &HashMap<(String, String), DesignArtifact>,
    report: &mut ImportReport,
) -> Result<()> {
    let rows = sqlx::query(
        "SELECT id, story_id, workspace_id, filename, mime, storage_path, kind, source,
                meta_json, created_by, created_at, updated_at
         FROM product_attachments ORDER BY created_at",
    )
    .fetch_all(svc.store().pool())
    .await
    .map_err(|e| Error::Internal(format!("design import: list attachments: {e}")))?;

    // (new artifact id, the attachment it was rendered from) — resolved after
    // every attachment is mirrored.
    let mut derived: Vec<(String, String)> = Vec::new();
    for r in &rows {
        let kind: String = r.get("kind");
        let mime: String = r.get("mime");
        if !is_design_attachment(&kind, &mime) {
            continue;
        }
        report.attachments_scanned += 1;
        let id: String = r.get("id");
        let Some(spec) = format::from_mime(&mime) else {
            report.skipped += 1;
            continue;
        };
        let updated_at: String = r.get("updated_at");
        let existing = index.get(&(SOURCE_ATTACHMENT.to_string(), id.clone()));
        if let Some(a) = existing {
            if a.meta["imported_from"]["source_updated_at"].as_str() == Some(updated_at.as_str()) {
                report.unchanged += 1;
                continue;
            }
        }
        let storage_path: String = r.get("storage_path");
        let bytes = match read_attachment(svc.data_dir(), &storage_path).await {
            Ok(b) => b,
            Err(e) => {
                push_err(report, format!("attachment {id}: {e}"));
                report.skipped += 1;
                continue;
            }
        };
        let sha = blobs::sha256_hex(&bytes);

        if let Some(a) = existing {
            if a.meta["imported_from"]["source_sha256"].as_str() != Some(sha.as_str()) {
                if let Err(e) = sync_version(
                    svc,
                    a,
                    bytes,
                    SOURCE_ATTACHMENT,
                    &id,
                    "Synced from the product attachment",
                )
                .await
                {
                    push_err(report, format!("attachment {id}: sync failed: {e}"));
                    continue;
                }
                report.synced += 1;
            } else {
                report.unchanged += 1;
            }
            let fresh = svc.store().require_artifact(&a.id).await?;
            stamp_source(svc, &fresh, &updated_at, &sha).await?;
            continue;
        }

        let story_id: String = r.get("story_id");
        let workspace_id: String = r.get("workspace_id");
        let filename: String = r.get("filename");
        let source: String = r.get("source");
        let created_by: String = r.get("created_by");
        let created_at: String = r.get("created_at");
        let legacy: Value = r
            .get::<Option<String>, _>("meta_json")
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
                "story_id": story_id,
                "filename": filename,
                "attachment_kind": kind,
                "source": source,
                "source_sha256": sha,
                "source_updated_at": updated_at,
            },
            "legacy_meta": Value::Object(legacy_meta),
        });
        let input = CreateInput {
            workspace_id,
            project_id: None,
            studio: None,
            format: spec.name.to_string(),
            title: title_from_filename(&filename),
            tags,
            meta,
            content: Some(bytes),
            story_id: Some(story_id),
            derived_from: None,
            created_by,
            author: Author::system("import"),
            message: Some("Imported from a product story attachment".into()),
            source: Some((SOURCE_ATTACHMENT.to_string(), id.clone())),
            created_at: parse_ts(&created_at),
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
    }

    // Blender renders / exports → `derived_from` their source scene (pinned to
    // the version mirrored now).
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

async fn import_scenes(
    svc: &DesignService,
    index: &HashMap<(String, String), DesignArtifact>,
    report: &mut ImportReport,
) -> Result<()> {
    let rows = sqlx::query(
        "SELECT id, workspace_id, story_id, title, doc_json, section, created_by,
                created_at, updated_at
         FROM canvas_scenes ORDER BY created_at",
    )
    .fetch_all(svc.store().pool())
    .await
    .map_err(|e| Error::Internal(format!("design import: list canvas scenes: {e}")))?;

    for r in &rows {
        report.scenes_scanned += 1;
        let id: String = r.get("id");
        let updated_at: String = r.get("updated_at");
        let existing = index.get(&(SOURCE_SCENE.to_string(), id.clone()));
        if let Some(a) = existing {
            if a.meta["imported_from"]["source_updated_at"].as_str() == Some(updated_at.as_str()) {
                report.unchanged += 1;
                continue;
            }
        }
        let doc: String = r.get("doc_json");
        let bytes = doc.clone().into_bytes();
        if bytes.len() > format::MAX_CONTENT_BYTES {
            push_err(
                report,
                format!("canvas scene {id}: document over the design cap"),
            );
            report.skipped += 1;
            continue;
        }
        let sha = blobs::sha256_hex(&bytes);

        if let Some(a) = existing {
            if a.meta["imported_from"]["source_sha256"].as_str() != Some(sha.as_str()) {
                if let Err(e) = sync_version(
                    svc,
                    a,
                    bytes,
                    SOURCE_SCENE,
                    &id,
                    "Synced from the Canvas scene",
                )
                .await
                {
                    push_err(report, format!("canvas scene {id}: sync failed: {e}"));
                    continue;
                }
                report.synced += 1;
            } else {
                report.unchanged += 1;
            }
            let fresh = svc.store().require_artifact(&a.id).await?;
            stamp_source(svc, &fresh, &updated_at, &sha).await?;
            continue;
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
        let section: Option<String> = r.get("section");
        let story_id: Option<String> = r.get("story_id");
        let title: String = r.get("title");
        let created_at: String = r.get("created_at");
        let meta = json!({
            "imported_from": {
                "kind": SOURCE_SCENE,
                "id": id,
                "story_id": story_id,
                "section": section,
                "canvas_format": canvas_format,
                "source_sha256": sha,
                "source_updated_at": updated_at,
            }
        });
        let input = CreateInput {
            workspace_id: r.get("workspace_id"),
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
            created_by: r.get("created_by"),
            author: Author::system("import"),
            message: Some("Imported from Canvas".into()),
            source: Some((SOURCE_SCENE.to_string(), id.clone())),
            created_at: parse_ts(&created_at),
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
    }
    Ok(())
}

/// Daemon-startup hook: create the FTS index (backfilling it when it is new),
/// then run the import. Logs; never fails the boot.
pub async fn startup(svc: DesignService) {
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
            errors = r.errors.len(),
            "design: legacy import done"
        ),
        Err(e) => tracing::warn!("design: legacy import failed: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn seed_story(pool: &sqlx::SqlitePool, id: &str, ws: &str) {
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
        pool: &sqlx::SqlitePool,
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

    async fn seed_scene(pool: &sqlx::SqlitePool, id: &str, story: Option<&str>, doc: &str) {
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
}
