//! `DesignService` — the design graph's behaviour on top of [`Store`] and the
//! [`BlobStore`]: the commit pipeline (validate → blob → guarded version
//! commit → working-copy mirror → link + search reindex → learning signals →
//! live events), approvals with change propagation, explicit links, signals,
//! and the opt-in retention prune.
//!
//! Cheap to construct (a pool handle, two paths, an optional event sender) —
//! the server builds one per request from its context; nothing here is
//! process-global except the import lock (see `import.rs`).

use std::collections::HashMap;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use otto_core::event::Event;
use otto_core::{new_id, Error, Id, Result};
use serde_json::{json, Value};
use sqlx::SqlitePool;
use tokio::sync::broadcast;

use crate::blobs::{self, BlobStore};
use crate::extract::{self, Target};
use crate::format::{self, Encoding};
use crate::graph;
use crate::retention;
use crate::store::{NewArtifactRow, NewLink, NewSignal, NewVersion, Store};
use crate::types::*;
use crate::uri::{default_policy, VersionSel};

/// Sub-directory of the daemon data dir that holds the graph's files.
pub const DESIGN_DIR: &str = "design";
/// A human save within this window after an agent version on the same
/// artifact is captured as an `edit_after_draft` signal.
pub const EDIT_AFTER_DRAFT_WINDOW_SECS: i64 = 2 * 60 * 60;
/// Bounded JSON payloads (signals, provenance, link meta, artifact meta).
pub const MAX_SIGNAL_PAYLOAD_BYTES: usize = 8 * 1024;
pub const MAX_PROVENANCE_BYTES: usize = 16 * 1024;
pub const MAX_META_BYTES: usize = 32 * 1024;
const MAX_JSON_NESTING: usize = 8;
/// Cap on `content` inlined into `GET …/{id}?content=true` (agent reads).
pub const MAX_DETAIL_CONTENT: usize = 256 * 1024;
/// Thumbnail cap (PNG).
pub const MAX_THUMB_BYTES: usize = 2 * 1024 * 1024;
/// Render-subgraph bound for cycle / depth checks.
const ADJACENCY_CAP: usize = 2_000;
const MAX_TITLE_CHARS: usize = 300;
const MAX_TAGS: usize = 32;
const MAX_TAG_CHARS: usize = 64;

/// Who a write is attributed to.
#[derive(Debug, Clone)]
pub struct Author {
    /// `user` | `agent` | `system`.
    pub kind: String,
    /// The user id (or a `system` tag such as `import`).
    pub id: String,
    pub session_id: Option<Id>,
}

impl Author {
    pub fn user(id: &str) -> Self {
        Self {
            kind: "user".into(),
            id: id.into(),
            session_id: None,
        }
    }

    pub fn system(tag: &str) -> Self {
        Self {
            kind: "system".into(),
            id: tag.into(),
            session_id: None,
        }
    }

    /// From an API request: `author_kind` ∈ {user, agent} (default user); the
    /// id is always the authenticated principal.
    pub fn from_request(user_id: &str, kind: Option<&str>, session_id: Option<Id>) -> Result<Self> {
        let kind = kind.unwrap_or("user");
        if !matches!(kind, "user" | "agent") {
            return Err(Error::Invalid(format!(
                "author_kind must be user|agent, not {kind:?}"
            )));
        }
        Ok(Self {
            kind: kind.into(),
            id: user_id.into(),
            session_id,
        })
    }
}

/// How a commit behaves.
#[derive(Debug, Clone)]
pub struct SaveOpts {
    /// See [`Store::commit_version`]: `None` unconditional, `Some("")` expects
    /// no head, `Some(id)` expects that head (else 409).
    pub base: Option<String>,
    /// A `VERSION_KINDS` value.
    pub kind: String,
    pub author: Author,
    pub message: String,
    pub provenance: Value,
    /// Commit even when the bytes equal the head (a named commit re-labels).
    pub force: bool,
    /// Run the cheap format checks (imports mirror legacy bytes verbatim).
    pub validate: bool,
    /// The `change` of the emitted `design_artifact_updated`.
    pub change: &'static str,
}

/// Everything `create_artifact` needs.
pub struct CreateInput {
    pub workspace_id: Id,
    pub project_id: Option<Id>,
    pub studio: Option<String>,
    pub format: String,
    pub title: String,
    pub tags: Vec<String>,
    pub meta: Value,
    /// `None` → fork source bytes, else the format's empty document.
    pub content: Option<Vec<u8>>,
    pub story_id: Option<Id>,
    pub derived_from: Option<ForkFrom>,
    /// The authenticated principal (users.id) the row is attributed to.
    pub created_by: Id,
    pub author: Author,
    pub message: Option<String>,
    /// Legacy mirror key (`product_attachment` | `canvas_scene`, id).
    pub source: Option<(String, Id)>,
    pub created_at: Option<DateTime<Utc>>,
    /// First version's kind (default `named`; imports pass `import`).
    pub version_kind: Option<String>,
    pub validate: bool,
}

#[derive(Clone)]
pub struct DesignService {
    store: Store,
    blobs: BlobStore,
    data_dir: PathBuf,
    events: Option<broadcast::Sender<Event>>,
}

// ---------------------------------------------------------------------------
// Pure helpers (unit-tested)
// ---------------------------------------------------------------------------

/// Trimmed, non-empty, capped title.
pub fn clean_title(t: &str) -> Result<String> {
    let t = t.trim();
    if t.is_empty() {
        return Err(Error::Invalid("title must not be empty".into()));
    }
    Ok(t.chars().take(MAX_TITLE_CHARS).collect())
}

/// Trimmed, deduped, capped tags.
pub fn clean_tags(tags: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    tags.into_iter()
        .map(|t| t.trim().chars().take(MAX_TAG_CHARS).collect::<String>())
        .filter(|t| !t.is_empty() && seen.insert(t.to_lowercase()))
        .take(MAX_TAGS)
        .collect()
}

fn json_depth(v: &Value) -> usize {
    match v {
        Value::Object(m) => 1 + m.values().map(json_depth).max().unwrap_or(0),
        Value::Array(a) => 1 + a.iter().map(json_depth).max().unwrap_or(0),
        _ => 0,
    }
}

/// Enforce a bounded JSON object payload (`null` → `{}`).
pub fn bound_json(v: Value, max_bytes: usize, what: &str) -> Result<Value> {
    let v = if v.is_null() { json!({}) } else { v };
    if !v.is_object() {
        return Err(Error::Invalid(format!("{what} must be a JSON object")));
    }
    if json_depth(&v) > MAX_JSON_NESTING {
        return Err(Error::Invalid(format!(
            "{what} nests deeper than {MAX_JSON_NESTING} levels"
        )));
    }
    let n = v.to_string().len();
    if n > max_bytes {
        return Err(Error::PayloadTooLarge(format!(
            "{what} is {n} bytes (cap {max_bytes})"
        )));
    }
    Ok(v)
}

/// Shallow-merge `patch` into `meta` (`null` values delete keys).
pub fn merge_meta(meta: &mut Value, patch: Value) -> Result<()> {
    let Value::Object(p) = patch else {
        return Err(Error::Invalid("meta must be a JSON object".into()));
    };
    if !meta.is_object() {
        *meta = json!({});
    }
    if let Value::Object(m) = meta {
        for (k, v) in p {
            if v.is_null() {
                m.remove(&k);
            } else {
                m.insert(k, v);
            }
        }
    }
    Ok(())
}

/// Decode `content` (UTF-8) or `content_b64`; neither → `None`.
pub fn decode_content(
    content: Option<String>,
    content_b64: Option<String>,
) -> Result<Option<Vec<u8>>> {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    match (content, content_b64) {
        (Some(_), Some(_)) => Err(Error::Invalid(
            "send either `content` or `content_b64`, not both".into(),
        )),
        (Some(c), None) => Ok(Some(c.into_bytes())),
        (None, Some(b)) => STANDARD
            .decode(b.trim())
            .map(Some)
            .map_err(|e| Error::Invalid(format!("invalid base64: {e}"))),
        (None, None) => Ok(None),
    }
}

impl DesignService {
    pub fn new(
        pool: SqlitePool,
        data_dir: impl Into<PathBuf>,
        events: Option<broadcast::Sender<Event>>,
    ) -> Self {
        let data_dir = data_dir.into();
        let blobs = BlobStore::new(data_dir.join(DESIGN_DIR).join("blobs"));
        Self {
            store: Store::new(pool),
            blobs,
            data_dir,
            events,
        }
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn blobs(&self) -> &BlobStore {
        &self.blobs
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// `<data>/design`.
    pub fn root(&self) -> PathBuf {
        self.data_dir.join(DESIGN_DIR)
    }

    fn emit(&self, ev: Event) {
        if let Some(tx) = &self.events {
            let _ = tx.send(ev);
        }
    }

    /// The editable working copy (`<data>/design/<id>/work/<file>`) — text and
    /// JSON formats only; binaries are never edited in place.
    pub fn work_file(&self, a: &DesignArtifact) -> Option<PathBuf> {
        let spec = format::spec(&a.format)?;
        if spec.encoding == Encoding::Binary {
            return None;
        }
        let dir = otto_core::paths::confine_join(&self.root(), &a.id)?;
        Some(dir.join("work").join(spec.file_name))
    }

    // -- reads ----------------------------------------------------------------

    /// A version's bytes (the version must belong to `a`).
    pub async fn version_bytes(&self, v: &DesignVersion) -> Result<Vec<u8>> {
        self.blobs.get(&v.blob_sha256).await
    }

    /// Head version + its bytes. `NotFound` when the artifact has no version.
    pub async fn head_content(&self, a: &DesignArtifact) -> Result<(DesignVersion, Vec<u8>)> {
        let hid = a.head_version_id.as_deref().ok_or_else(|| {
            Error::NotFound(format!("design artifact {} has no content yet", a.id))
        })?;
        let v = self
            .store
            .get_version(hid)
            .await?
            .ok_or_else(|| Error::NotFound(format!("design version {hid}")))?;
        let bytes = self.version_bytes(&v).await?;
        Ok((v, bytes))
    }

    /// What a named commit without content snapshots: the working copy (an
    /// agent may have edited it in place), else the head bytes.
    pub async fn working_bytes(&self, a: &DesignArtifact) -> Result<Vec<u8>> {
        if let Some(p) = self.work_file(a) {
            if let Ok(b) = tokio::fs::read(&p).await {
                return Ok(b);
            }
        }
        Ok(self.head_content(a).await?.1)
    }

    /// Resolve `v` (a version id, `v12`, or `12`) within artifact `a`.
    pub async fn resolve_version(&self, a: &DesignArtifact, v: &str) -> Result<DesignVersion> {
        let seq = v.strip_prefix('v').unwrap_or(v).parse::<i64>().ok();
        let found = match seq {
            Some(n) => self.store.get_version_by_seq(&a.id, n).await?,
            None => self
                .store
                .get_version(v)
                .await?
                .filter(|x| x.artifact_id == a.id),
        };
        found.ok_or_else(|| Error::NotFound(format!("version {v} of design artifact {}", a.id)))
    }

    pub async fn detail(&self, a: DesignArtifact) -> Result<ArtifactDetail> {
        let head = match &a.head_version_id {
            Some(h) => self.store.get_version(h).await?,
            None => None,
        };
        let approved = match &a.approved_version_id {
            Some(h) => self.store.get_version(h).await?,
            None => None,
        };
        let (links_out, links_in) = self.store.link_counts(&a.id).await?;
        let work_path = self.work_file(&a).map(|p| p.display().to_string());
        let thumbnail_path = a
            .thumb_blob
            .as_deref()
            .filter(|s| blobs::is_sha(s))
            .map(|s| self.blobs.root().join(s).display().to_string());
        Ok(ArtifactDetail {
            artifact: a,
            head,
            approved,
            links_out,
            links_in,
            work_path,
            thumbnail_path,
            content: None,
            content_version_id: None,
            content_truncated: false,
        })
    }

    /// Fill `d.content` from version `version` (id / `v12` / `12`; default the
    /// head) — UTF-8 formats only, capped at [`MAX_DETAIL_CONTENT`].
    pub async fn with_content(
        &self,
        mut d: ArtifactDetail,
        version: Option<&str>,
    ) -> Result<ArtifactDetail> {
        let v = match version {
            Some(v) => Some(self.resolve_version(&d.artifact, v).await?),
            None => d.head.clone(),
        };
        let Some(v) = v else {
            return Ok(d);
        };
        let binary =
            format::spec(&d.artifact.format).is_none_or(|s| s.encoding == Encoding::Binary);
        if !binary {
            let bytes = self.version_bytes(&v).await?;
            let text = String::from_utf8_lossy(&bytes);
            if text.len() > MAX_DETAIL_CONTENT {
                let mut end = MAX_DETAIL_CONTENT;
                while !text.is_char_boundary(end) {
                    end -= 1;
                }
                d.content = Some(text[..end].to_string());
                d.content_truncated = true;
            } else {
                d.content = Some(text.into_owned());
            }
        }
        d.content_version_id = Some(v.id);
        Ok(d)
    }

    // -- create / commit --------------------------------------------------------

    /// Create an artifact and commit its first version (plus the optional
    /// `implements` story link and `derived_from` fork link).
    pub async fn create_artifact(&self, input: CreateInput) -> Result<SaveResult> {
        let spec = format::spec(&input.format)
            .ok_or_else(|| Error::Invalid(format!("unknown design format {:?}", input.format)))?;
        let studio = input
            .studio
            .clone()
            .unwrap_or_else(|| spec.studio.to_string());
        if !format::valid_studio(&studio) {
            return Err(Error::Invalid(format!("unknown studio {studio:?}")));
        }
        let title = clean_title(&input.title)?;
        let tags = clean_tags(input.tags);
        let meta = bound_json(input.meta, MAX_META_BYTES, "meta")?;
        if let Some(p) = &input.project_id {
            let proj = self
                .store
                .get_project(p)
                .await?
                .ok_or_else(|| Error::NotFound(format!("design project {p}")))?;
            if proj.workspace_id != input.workspace_id {
                return Err(Error::Invalid(
                    "the project belongs to another workspace".into(),
                ));
            }
        }

        // Content: explicit bytes, else the fork source's version, else the
        // format's empty document.
        let mut fork: Option<(DesignArtifact, DesignVersion)> = None;
        if let Some(f) = &input.derived_from {
            let src = self.store.require_artifact(&f.artifact_id).await?;
            let vid = f
                .version_id
                .clone()
                .or_else(|| src.approved_version_id.clone())
                .or_else(|| src.head_version_id.clone())
                .ok_or_else(|| Error::Invalid("the fork source has no version yet".into()))?;
            let v = self
                .store
                .get_version(&vid)
                .await?
                .filter(|v| v.artifact_id == src.id)
                .ok_or_else(|| Error::NotFound(format!("version {vid} of {}", src.id)))?;
            if input.content.is_none() && src.format != spec.name {
                return Err(Error::Invalid(format!(
                    "a fork keeps its source's format ({}), not {}",
                    src.format, spec.name
                )));
            }
            fork = Some((src, v));
        }
        let bytes = match (input.content, &fork) {
            (Some(c), _) => c,
            (None, Some((_, v))) => self.blobs.get(&v.blob_sha256).await?,
            (None, None) => format::default_content(spec)
                .ok_or_else(|| Error::Invalid(format!("{} artifacts need content", spec.name)))?,
        };
        if input.validate {
            format::validate(spec, &bytes)?;
        }

        let id = new_id();
        let (source_kind, source_id) = match input.source {
            Some((k, i)) => (Some(k), Some(i)),
            None => (None, None),
        };
        self.store
            .insert_artifact(&NewArtifactRow {
                id: id.clone(),
                project_id: input.project_id.clone(),
                workspace_id: input.workspace_id.clone(),
                studio,
                format: spec.name.to_string(),
                mime: spec.mime.to_string(),
                title,
                status: "draft".into(),
                tags,
                meta,
                source_kind,
                source_id,
                created_by: input.created_by.clone(),
                created_by_kind: input.author.kind.clone(),
                created_session_id: input.author.session_id.clone(),
                created_at: input.created_at,
            })
            .await?;
        let artifact = self.store.require_artifact(&id).await?;

        let provenance = match &fork {
            Some((src, v)) => {
                json!({ "fork": { "artifact_id": src.id, "version_id": v.id, "seq": v.seq } })
            }
            None => json!({}),
        };
        let kind = input
            .version_kind
            .clone()
            .unwrap_or_else(|| "named".to_string());
        let message = input.message.clone().unwrap_or_else(|| match &fork {
            Some((src, v)) => format!("Forked from {} v{}", src.title, v.seq),
            None => "Created".to_string(),
        });
        let saved = self
            .commit_bytes(
                &artifact,
                bytes,
                SaveOpts {
                    base: Some(String::new()),
                    kind,
                    author: input.author.clone(),
                    message,
                    provenance,
                    force: true,
                    validate: false,
                    change: "created",
                },
            )
            .await;
        let mut saved = match saved {
            Ok(s) => s,
            Err(e) => {
                // Never leave a version-less shell behind (it holds no user
                // data yet — the bytes never landed).
                let _ = self.store.delete_artifact(&id).await;
                return Err(e);
            }
        };

        // Explicit links made at creation time.
        let mut touched = false;
        if let Some(story) = &input.story_id {
            if self.store.story_workspace(story).await?.is_some() {
                let _ = self
                    .store
                    .insert_link(&explicit_link(
                        &id,
                        "story",
                        story,
                        "implements",
                        "follow_latest",
                        None,
                        &input.created_by,
                    ))
                    .await;
                touched = true;
            }
        }
        if let Some((src, v)) = &fork {
            let _ = self
                .store
                .insert_link(&explicit_link(
                    &id,
                    "artifact",
                    &src.id,
                    "derived_from",
                    "pinned",
                    Some(v.id.clone()),
                    &input.created_by,
                ))
                .await;
            touched = true;
        }
        if touched {
            saved.artifact = self.store.require_artifact(&id).await?;
            self.refresh_search(&saved.artifact).await;
        }
        Ok(saved)
    }

    /// The commit pipeline. Unchanged bytes (same sha as the head) write
    /// nothing unless `opts.force` — `created: false` then.
    pub async fn commit_bytes(
        &self,
        artifact: &DesignArtifact,
        bytes: Vec<u8>,
        opts: SaveOpts,
    ) -> Result<SaveResult> {
        let spec = format::spec(&artifact.format).ok_or_else(|| {
            Error::Invalid(format!("unknown design format {:?}", artifact.format))
        })?;
        if opts.validate {
            format::validate(spec, &bytes)?;
        }
        if !one_of(VERSION_KINDS, &opts.kind) {
            return Err(Error::Invalid(format!(
                "unknown version kind {:?}",
                opts.kind
            )));
        }
        if !one_of(AUTHOR_KINDS, &opts.author.kind) {
            return Err(Error::Invalid(format!(
                "unknown author kind {:?}",
                opts.author.kind
            )));
        }
        let provenance = bound_json(opts.provenance.clone(), MAX_PROVENANCE_BYTES, "provenance")?;
        let message: String = opts.message.trim().chars().take(2_000).collect();

        let prev_head = match &artifact.head_version_id {
            Some(h) => self.store.get_version(h).await?,
            None => None,
        };
        let sha = blobs::sha256_hex(&bytes);
        if !opts.force {
            if let Some(h) = &prev_head {
                if h.blob_sha256 == sha {
                    if let Some(b) = &opts.base {
                        if b != &h.id {
                            return Err(Error::Conflict(format!(
                                "design artifact {} changed: head is {}; reload before saving",
                                artifact.id, h.id
                            )));
                        }
                    }
                    return Ok(SaveResult {
                        artifact: artifact.clone(),
                        version: h.clone(),
                        created: false,
                        links: LinkReport::default(),
                    });
                }
            }
        }

        let stored = self.blobs.put(&bytes).await?;
        let version = self
            .store
            .commit_version(
                NewVersion {
                    artifact_id: artifact.id.clone(),
                    blob_sha256: stored,
                    size_bytes: bytes.len() as i64,
                    kind: opts.kind.clone(),
                    branch: "main".into(),
                    author_kind: opts.author.kind.clone(),
                    author_id: opts.author.id.clone(),
                    session_id: opts.author.session_id.clone(),
                    message,
                    provenance,
                },
                opts.base.as_deref(),
            )
            .await?;

        // Mirror into the working copy (best-effort: the blob is the truth).
        if let Some(path) = self.work_file(artifact) {
            if let Some(parent) = path.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
            if let Err(e) = tokio::fs::write(&path, &bytes).await {
                tracing::warn!(artifact = %artifact.id, "design working copy not written: {e}");
            }
        }

        let artifact = self.store.require_artifact(&artifact.id).await?;
        let links = match self.reindex(&artifact, &version, &bytes).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(artifact = %artifact.id, "design link/search reindex failed: {e}");
                LinkReport::default()
            }
        };

        // Learning signal: a human save right after an agent draft.
        if let Some(prev) = &prev_head {
            let fresh =
                (Utc::now() - prev.created_at).num_seconds() <= EDIT_AFTER_DRAFT_WINDOW_SECS;
            if prev.author_kind == "agent" && opts.author.kind == "user" && fresh {
                let old = self.blobs.get(&prev.blob_sha256).await.unwrap_or_default();
                let summary = crate::diff::summarize(spec.encoding, &old, &bytes);
                let _ = self
                    .record_signal_row(NewSignal {
                        workspace_id: artifact.workspace_id.clone(),
                        artifact_id: artifact.id.clone(),
                        version_id: Some(version.id.clone()),
                        kind: "edit_after_draft".into(),
                        actor_kind: "user".into(),
                        actor_id: opts.author.id.clone(),
                        session_id: None,
                        payload: json!({
                            "agent_version_id": prev.id,
                            "agent_session_id": prev.session_id,
                            "user_version_id": version.id,
                            "format": artifact.format,
                            "summary": summary,
                        }),
                    })
                    .await;
            }
        }

        self.emit(Event::DesignArtifactUpdated {
            workspace_id: artifact.workspace_id.clone(),
            artifact_id: artifact.id.clone(),
            format: artifact.format.clone(),
            change: opts.change.to_string(),
            version_id: Some(version.id.clone()),
            content: format::event_content(spec, &bytes),
        });
        self.notify_consumers(
            &artifact,
            Some(&version.id),
            "target_updated",
            Some("follow_latest"),
        )
        .await;
        Ok(SaveResult {
            artifact,
            version,
            created: true,
            links,
        })
    }

    /// A NAMED commit: `content` if given, else the working copy (what an agent
    /// edited in place), else the head bytes re-labelled.
    pub async fn commit_named(
        &self,
        a: &DesignArtifact,
        content: Option<Vec<u8>>,
        base: Option<String>,
        author: Author,
        message: String,
        provenance: Value,
    ) -> Result<SaveResult> {
        if message.trim().is_empty() {
            return Err(Error::Invalid("a named version needs a message".into()));
        }
        let bytes = match content {
            Some(c) => c,
            None => {
                let from_work = match self.work_file(a) {
                    Some(p) => tokio::fs::read(&p).await.ok(),
                    None => None,
                };
                match from_work {
                    Some(b) => b,
                    None => self.head_content(a).await?.1,
                }
            }
        };
        let kind = if author.kind == "agent" {
            "agent"
        } else {
            "named"
        };
        self.commit_bytes(
            a,
            bytes,
            SaveOpts {
                base,
                kind: kind.into(),
                author,
                message,
                provenance,
                force: true,
                validate: true,
                change: "content",
            },
        )
        .await
    }

    // -- links + search index ---------------------------------------------------

    /// Rebuild `a`'s extracted links and search row from `bytes` (the version
    /// just committed). Broken references are stored (badge) and reported;
    /// render links that would close a cycle are reported and NOT stored.
    pub async fn reindex(
        &self,
        a: &DesignArtifact,
        version: &DesignVersion,
        bytes: &[u8],
    ) -> Result<LinkReport> {
        let ex = extract::extract(&a.format, bytes);
        let mut report = LinkReport::default();
        let mut links: Vec<NewLink> = Vec::new();

        // One bounded render subgraph for every candidate target.
        let candidates: Vec<String> = ex
            .refs
            .iter()
            .filter(|r| RENDER_RELS.contains(&r.rel))
            .filter_map(|r| match &r.target {
                Target::Uri(u) => Some(u.artifact_id.clone()),
                Target::LegacyId(id) => Some(id.clone()),
                Target::Malformed(_) => None,
            })
            .collect();
        let adj = if candidates.is_empty() {
            graph::Adjacency::new()
        } else {
            self.store
                .render_adjacency(&candidates, ADJACENCY_CAP)
                .await?
        };
        let mut node_cache: HashMap<String, Option<HashSet<String>>> = HashMap::new();

        for r in &ex.refs {
            let src_node = r.src_node.clone().unwrap_or_default();
            // Resolve the reference to (target artifact?, uri text, node, policy, pin).
            let (target, uri_s, dst_node, policy, mut pinned, mut broken) = match &r.target {
                Target::Malformed(raw) => {
                    report.broken.push(BrokenRef {
                        uri: raw.clone(),
                        src_node: r.src_node.clone(),
                        reason: "malformed".into(),
                    });
                    continue;
                }
                Target::LegacyId(aid) => {
                    let resolved = match self.store.get_artifact(aid).await? {
                        Some(t) => Some(t),
                        None => self.store.find_by_source("product_attachment", aid).await?,
                    };
                    match resolved {
                        Some(t) => (
                            Some(t),
                            format!("attachment:{aid}"),
                            String::new(),
                            default_policy(r.rel),
                            None,
                            None,
                        ),
                        None => {
                            // A legacy attachment not (yet) mirrored — keep the
                            // edge so "Used in" still works after the import.
                            links.push(NewLink {
                                src_artifact_id: a.id.clone(),
                                src_version_id: Some(version.id.clone()),
                                src_node,
                                dst_kind: "attachment".into(),
                                dst_id: aid.clone(),
                                dst_node: String::new(),
                                rel: r.rel.into(),
                                policy: "follow_latest".into(),
                                pinned_version_id: None,
                                origin: "extracted".into(),
                                broken: false,
                                meta: json!({}),
                                created_by: version.author_id.clone(),
                            });
                            continue;
                        }
                    }
                }
                Target::Uri(u) => {
                    let t = self.store.get_artifact(&u.artifact_id).await?;
                    let mut broken: Option<&'static str> = None;
                    let mut pinned: Option<Id> = None;
                    match &t {
                        None => broken = Some("missing_artifact"),
                        Some(t) => {
                            if let VersionSel::Seq(n) = u.version {
                                match self.store.get_version_by_seq(&t.id, n).await? {
                                    Some(v) => pinned = Some(v.id),
                                    None => broken = Some("missing_version"),
                                }
                            }
                        }
                    }
                    (
                        t,
                        u.to_uri_string(),
                        u.node.clone().unwrap_or_default(),
                        u.policy_for(r.rel),
                        pinned,
                        broken,
                    )
                }
            };
            let dst_id = match &target {
                Some(t) => t.id.clone(),
                None => match &r.target {
                    Target::Uri(u) => u.artifact_id.clone(),
                    _ => continue,
                },
            };
            if let Some(t) = &target {
                // `#node` must exist in the version a consumer would render.
                if broken.is_none() && !dst_node.is_empty() {
                    let vid = pinned
                        .clone()
                        .or_else(|| t.approved_version_id.clone())
                        .or_else(|| t.head_version_id.clone());
                    if let Some(vid) = vid {
                        if !node_cache.contains_key(&vid) {
                            let ids = match self.store.get_version(&vid).await? {
                                Some(v) => match self.blobs.get(&v.blob_sha256).await {
                                    Ok(b) => extract::node_ids(&t.format, &b),
                                    Err(_) => None,
                                },
                                None => None,
                            };
                            node_cache.insert(vid.clone(), ids);
                        }
                        if let Some(Some(ids)) = node_cache.get(&vid) {
                            if !ids.contains(&dst_node) {
                                broken = Some("missing_node");
                            }
                        }
                    }
                }
                if RENDER_RELS.contains(&r.rel) {
                    if t.id == a.id || graph::reaches(&adj, &t.id, &a.id, ADJACENCY_CAP) {
                        report.cycles.push(uri_s.clone());
                        continue;
                    }
                    if 1 + graph::depth_below(&adj, &t.id, MAX_RENDER_DEPTH) > MAX_RENDER_DEPTH {
                        report.depth_exceeded.push(uri_s.clone());
                    }
                }
                if policy == "pinned" && pinned.is_none() {
                    // An un-versioned provenance reference pins what exists now.
                    pinned = t
                        .approved_version_id
                        .clone()
                        .or_else(|| t.head_version_id.clone());
                }
            }
            if let Some(reason) = broken {
                report.broken.push(BrokenRef {
                    uri: uri_s.clone(),
                    src_node: r.src_node.clone(),
                    reason: reason.into(),
                });
            }
            links.push(NewLink {
                src_artifact_id: a.id.clone(),
                src_version_id: Some(version.id.clone()),
                src_node,
                dst_kind: "artifact".into(),
                dst_id,
                dst_node,
                rel: r.rel.into(),
                policy: policy.into(),
                pinned_version_id: pinned,
                origin: "extracted".into(),
                broken: broken.is_some(),
                meta: json!({ "uri": uri_s }),
                created_by: version.author_id.clone(),
            });
        }

        report.extracted = links.len();
        let changed = self.store.replace_extracted(&a.id, &links).await?;
        if changed {
            self.emit(Event::DesignLinkUpdated {
                workspace_id: a.workspace_id.clone(),
                artifact_id: a.id.clone(),
                link_id: None,
                target_artifact_id: None,
                target_version_id: Some(version.id.clone()),
                reason: "extracted".into(),
            });
        }
        self.index_search(a, &ex.text).await;
        Ok(report)
    }

    /// Write `a`'s search row (title, tags, extracted text, linked story keys
    /// + titles, project name).
    async fn index_search(&self, a: &DesignArtifact, body: &str) {
        let mut story = String::new();
        for s in self.store.story_ids_for(&a.id).await.unwrap_or_default() {
            if let Ok(Some(label)) = self.store.story_label(&s).await {
                story.push_str(&label);
                story.push(' ');
            }
        }
        let project = match &a.project_id {
            Some(p) => self
                .store
                .get_project(p)
                .await
                .ok()
                .flatten()
                .map(|p| p.name)
                .unwrap_or_default(),
            None => String::new(),
        };
        self.store
            .fts_index(
                &a.id,
                &a.title,
                &a.tags.join(" "),
                body,
                story.trim(),
                &project,
            )
            .await;
    }

    /// Re-index search after a metadata change (re-extracts the head's text).
    pub async fn refresh_search(&self, a: &DesignArtifact) {
        let text = match self.head_content(a).await {
            Ok((_, bytes)) => extract::extract(&a.format, &bytes).text,
            Err(_) => String::new(),
        };
        self.index_search(a, &text).await;
    }

    /// Tell every consumer following `target` (with `policy`, or any policy
    /// when `None`) that it moved.
    async fn notify_consumers(
        &self,
        target: &DesignArtifact,
        version_id: Option<&str>,
        reason: &str,
        policy: Option<&str>,
    ) {
        let Ok(links) = self.store.links_in(&target.id).await else {
            return;
        };
        let mut ws_cache: HashMap<String, Option<Id>> = HashMap::new();
        for l in links
            .into_iter()
            .filter(|l| policy.is_none_or(|p| l.policy == p))
        {
            if !ws_cache.contains_key(&l.src_artifact_id) {
                let ws = self
                    .store
                    .get_artifact(&l.src_artifact_id)
                    .await
                    .ok()
                    .flatten()
                    .map(|a| a.workspace_id);
                ws_cache.insert(l.src_artifact_id.clone(), ws);
            }
            let Some(Some(ws)) = ws_cache.get(&l.src_artifact_id).cloned() else {
                continue;
            };
            self.emit(Event::DesignLinkUpdated {
                workspace_id: ws,
                artifact_id: l.src_artifact_id.clone(),
                link_id: Some(l.id.clone()),
                target_artifact_id: Some(target.id.clone()),
                target_version_id: version_id.map(str::to_string),
                reason: reason.to_string(),
            });
        }
    }

    // -- approve / metadata / delete ---------------------------------------------

    /// Move `approved_version_id` (default: head) — only humans approve (the
    /// route is Editor-gated; no MCP tool writes it). Follow-approved
    /// consumers get `design_link_updated { reason: "target_approved" }`.
    pub async fn approve(
        &self,
        artifact_id: &str,
        version_id: Option<&str>,
        actor: &Author,
    ) -> Result<DesignArtifact> {
        let a = self.store.require_artifact(artifact_id).await?;
        let vid = match version_id {
            Some(v) => v.to_string(),
            None => a
                .head_version_id
                .clone()
                .ok_or_else(|| Error::Invalid("the artifact has no version to approve".into()))?,
        };
        let v = self.resolve_version(&a, &vid).await?;
        self.store.set_approved(&a.id, &v.id, "approved").await?;
        let updated = self.store.require_artifact(&a.id).await?;
        let _ = self
            .record_signal_row(NewSignal {
                workspace_id: a.workspace_id.clone(),
                artifact_id: a.id.clone(),
                version_id: Some(v.id.clone()),
                kind: "status_change".into(),
                actor_kind: actor.kind.clone(),
                actor_id: actor.id.clone(),
                session_id: actor.session_id.clone(),
                payload: json!({ "from": a.status, "to": "approved", "seq": v.seq }),
            })
            .await;
        self.emit(Event::DesignArtifactUpdated {
            workspace_id: updated.workspace_id.clone(),
            artifact_id: updated.id.clone(),
            format: updated.format.clone(),
            change: "approved".into(),
            version_id: Some(v.id.clone()),
            content: None,
        });
        self.notify_consumers(
            &updated,
            Some(&v.id),
            "target_approved",
            Some("follow_approved"),
        )
        .await;
        Ok(updated)
    }

    /// Apply a metadata patch (never content). Status transitions record a
    /// `status_change` / `shipped` signal.
    pub async fn update_meta(
        &self,
        artifact_id: &str,
        req: UpdateArtifactReq,
        actor: &Author,
    ) -> Result<DesignArtifact> {
        let mut a = self.store.require_artifact(artifact_id).await?;
        let prev_status = a.status.clone();
        if let Some(t) = req.title {
            a.title = clean_title(&t)?;
        }
        if let Some(p) = req.project_id {
            if p.is_empty() {
                a.project_id = None;
            } else {
                let proj = self
                    .store
                    .get_project(&p)
                    .await?
                    .ok_or_else(|| Error::NotFound(format!("design project {p}")))?;
                if proj.workspace_id != a.workspace_id {
                    return Err(Error::Invalid(
                        "the project belongs to another workspace".into(),
                    ));
                }
                a.project_id = Some(p);
            }
        }
        if let Some(s) = req.studio {
            if !format::valid_studio(&s) {
                return Err(Error::Invalid(format!("unknown studio {s:?}")));
            }
            a.studio = s;
        }
        if let Some(s) = req.status {
            if !one_of(STATUSES, &s) {
                return Err(Error::Invalid(format!("unknown status {s:?}")));
            }
            if s == "approved" && a.approved_version_id.is_none() {
                return Err(Error::Invalid(
                    "approve a version first (POST …/approve)".into(),
                ));
            }
            a.status = s;
        }
        if let Some(tags) = req.tags {
            a.tags = clean_tags(tags);
        }
        if let Some(m) = req.meta {
            merge_meta(&mut a.meta, m)?;
            a.meta = bound_json(a.meta.clone(), MAX_META_BYTES, "meta")?;
        }
        if let Some(b64) = req.thumb_b64 {
            let bytes = decode_content(None, Some(b64))?.unwrap_or_default();
            if bytes.len() > MAX_THUMB_BYTES {
                return Err(Error::PayloadTooLarge("thumbnail exceeds 2 MB".into()));
            }
            let png = format::spec("png").expect("png is a known format");
            format::validate(png, &bytes)?;
            a.thumb_blob = Some(self.blobs.put(&bytes).await?);
        }
        let updated = self.store.write_artifact_meta(&a).await?;
        if updated.status != prev_status {
            let (kind, payload) = if updated.status == "shipped" {
                (
                    "shipped",
                    json!({
                        "from": prev_status,
                        "version_id": updated.approved_version_id.clone().or(updated.head_version_id.clone()),
                    }),
                )
            } else {
                (
                    "status_change",
                    json!({ "from": prev_status, "to": updated.status }),
                )
            };
            let _ = self
                .record_signal_row(NewSignal {
                    workspace_id: updated.workspace_id.clone(),
                    artifact_id: updated.id.clone(),
                    version_id: updated
                        .approved_version_id
                        .clone()
                        .or(updated.head_version_id.clone()),
                    kind: kind.into(),
                    actor_kind: actor.kind.clone(),
                    actor_id: actor.id.clone(),
                    session_id: actor.session_id.clone(),
                    payload,
                })
                .await;
        }
        self.refresh_search(&updated).await;
        let change = if updated.status == "archived" && prev_status != "archived" {
            "archived"
        } else {
            "meta"
        };
        self.emit(Event::DesignArtifactUpdated {
            workspace_id: updated.workspace_id.clone(),
            artifact_id: updated.id.clone(),
            format: updated.format.clone(),
            change: change.into(),
            version_id: None,
            content: None,
        });
        Ok(updated)
    }

    /// HARD delete (explicit `?hard=true`, workspace Admin). Versions' blobs
    /// stay in the store (only the opt-in prune GC removes blobs); consumers'
    /// links are kept and flagged broken.
    pub async fn hard_delete(&self, artifact_id: &str) -> Result<()> {
        let a = self.store.require_artifact(artifact_id).await?;
        self.store.delete_artifact(&a.id).await?;
        self.notify_consumers(&a, None, "target_deleted", None)
            .await;
        self.emit(Event::DesignArtifactUpdated {
            workspace_id: a.workspace_id.clone(),
            artifact_id: a.id.clone(),
            format: a.format.clone(),
            change: "deleted".into(),
            version_id: None,
            content: None,
        });
        Ok(())
    }

    // -- explicit links -----------------------------------------------------------

    pub async fn create_link(
        &self,
        src: &DesignArtifact,
        req: CreateLinkReq,
        actor: &Author,
    ) -> Result<DesignLink> {
        if !one_of(RELS, &req.rel) {
            return Err(Error::Invalid(format!("unknown rel {:?}", req.rel)));
        }
        if !one_of(DST_KINDS, &req.dst_kind) {
            return Err(Error::Invalid(format!(
                "unknown dst_kind {:?}",
                req.dst_kind
            )));
        }
        let dst_id = req.dst_id.trim().to_string();
        if dst_id.is_empty() || dst_id.len() > 2_048 {
            return Err(Error::Invalid("dst_id must be 1..=2048 chars".into()));
        }
        let src_node = req.src_node.unwrap_or_default();
        let dst_node = req.dst_node.unwrap_or_default();
        if src_node.len() > 128 || dst_node.len() > 128 {
            return Err(Error::Invalid("node ids are capped at 128 chars".into()));
        }
        let policy = req
            .policy
            .unwrap_or_else(|| default_policy(&req.rel).to_string());
        if !one_of(POLICIES, &policy) {
            return Err(Error::Invalid(format!("unknown policy {policy:?}")));
        }
        let mut pinned = req.pinned_version_id;
        match req.dst_kind.as_str() {
            "artifact" => {
                let t = self.store.require_artifact(&dst_id).await?;
                if let Some(p) = &pinned {
                    self.resolve_version(&t, p).await?;
                } else if policy == "pinned" {
                    pinned = t.approved_version_id.clone().or(t.head_version_id.clone());
                }
                if RENDER_RELS.contains(&req.rel.as_str()) {
                    let adj = self
                        .store
                        .render_adjacency(&[t.id.clone()], ADJACENCY_CAP)
                        .await?;
                    if t.id == src.id || graph::reaches(&adj, &t.id, &src.id, ADJACENCY_CAP) {
                        return Err(Error::Conflict(format!(
                            "linking {} {} {} would create a render cycle",
                            src.id, req.rel, t.id
                        )));
                    }
                }
            }
            "story" => {
                if self.store.story_workspace(&dst_id).await?.is_none() {
                    return Err(Error::NotFound(format!("product story {dst_id}")));
                }
            }
            "url" => {
                if !(dst_id.starts_with("https://") || dst_id.starts_with("http://")) {
                    return Err(Error::Invalid("url links must be http(s)".into()));
                }
            }
            _ => {}
        }
        let meta = bound_json(
            req.meta.unwrap_or(Value::Null),
            MAX_SIGNAL_PAYLOAD_BYTES,
            "link meta",
        )?;
        let link = self
            .store
            .insert_link(&NewLink {
                src_artifact_id: src.id.clone(),
                src_version_id: None,
                src_node,
                dst_kind: req.dst_kind.clone(),
                dst_id: dst_id.clone(),
                dst_node,
                rel: req.rel.clone(),
                policy,
                pinned_version_id: pinned,
                origin: "explicit".into(),
                broken: false,
                meta,
                created_by: actor.id.clone(),
            })
            .await?;
        self.emit(Event::DesignLinkUpdated {
            workspace_id: src.workspace_id.clone(),
            artifact_id: src.id.clone(),
            link_id: Some(link.id.clone()),
            target_artifact_id: (link.dst_kind == "artifact").then(|| link.dst_id.clone()),
            target_version_id: link.pinned_version_id.clone(),
            reason: "created".into(),
        });
        if link.dst_kind == "story" {
            self.refresh_search(src).await;
        }
        Ok(link)
    }

    pub async fn delete_link(&self, src: &DesignArtifact, link_id: &str) -> Result<()> {
        let l = self
            .store
            .get_link(link_id)
            .await?
            .filter(|l| l.src_artifact_id == src.id)
            .ok_or_else(|| Error::NotFound(format!("design link {link_id}")))?;
        if l.origin == "extracted" {
            return Err(Error::Conflict(
                "this link is extracted from the document — edit the document to remove the reference"
                    .into(),
            ));
        }
        self.store.delete_link(&l.id).await?;
        self.emit(Event::DesignLinkUpdated {
            workspace_id: src.workspace_id.clone(),
            artifact_id: src.id.clone(),
            link_id: Some(l.id.clone()),
            target_artifact_id: (l.dst_kind == "artifact").then(|| l.dst_id.clone()),
            target_version_id: None,
            reason: "deleted".into(),
        });
        if l.dst_kind == "story" {
            self.refresh_search(src).await;
        }
        Ok(())
    }

    // -- signals --------------------------------------------------------------------

    /// Record a signal from the API (bounded payload, known kind).
    pub async fn record_signal(
        &self,
        a: &DesignArtifact,
        req: SignalReq,
        actor: &Author,
    ) -> Result<DesignSignal> {
        if !one_of(SIGNAL_KINDS, &req.kind) {
            return Err(Error::Invalid(format!(
                "unknown signal kind {:?}",
                req.kind
            )));
        }
        let actor_kind = req.actor_kind.unwrap_or_else(|| actor.kind.clone());
        if !one_of(AUTHOR_KINDS, &actor_kind) {
            return Err(Error::Invalid(format!("unknown actor_kind {actor_kind:?}")));
        }
        if let Some(v) = &req.version_id {
            self.resolve_version(a, v).await?;
        }
        let payload = bound_json(
            req.payload.unwrap_or(Value::Null),
            MAX_SIGNAL_PAYLOAD_BYTES,
            "signal payload",
        )?;
        self.record_signal_row(NewSignal {
            workspace_id: a.workspace_id.clone(),
            artifact_id: a.id.clone(),
            version_id: req.version_id,
            kind: req.kind,
            actor_kind,
            actor_id: actor.id.clone(),
            session_id: req.session_id.or(actor.session_id.clone()),
            payload,
        })
        .await
    }

    async fn record_signal_row(&self, s: NewSignal) -> Result<DesignSignal> {
        let sig = self.store.insert_signal(s).await?;
        self.emit(Event::DesignLearningUpdate {
            workspace_id: sig.workspace_id.clone(),
            kind: sig.kind.clone(),
            signal_id: Some(sig.id.clone()),
            artifact_id: Some(sig.artifact_id.clone()),
        });
        Ok(sig)
    }

    // -- retention (opt-in) -----------------------------------------------------------

    /// Plan (and with `apply`, execute) the retention policy. Never runs on its
    /// own. On apply, blobs no version/thumbnail references any more are GC'd.
    pub async fn prune(
        &self,
        artifact_id: Option<&str>,
        apply: bool,
        window_secs: i64,
    ) -> Result<PruneReport> {
        let ids = match artifact_id {
            Some(id) => vec![self.store.require_artifact(id).await?.id],
            None => self.store.all_artifact_ids().await?,
        };
        let mut report = PruneReport {
            applied: apply,
            artifacts_scanned: ids.len(),
            ..Default::default()
        };
        for id in ids {
            let Some(a) = self.store.get_artifact(&id).await? else {
                continue;
            };
            let infos = self.store.version_infos(&a.id).await?;
            let protected = self.store.protected_versions(&a).await?;
            let doomed = retention::plan(&infos, &protected, window_secs);
            if doomed.is_empty() {
                continue;
            }
            if apply {
                let candidate_blobs = self.store.delete_versions(&doomed).await?;
                for sha in candidate_blobs {
                    if !self.store.blob_in_use(&sha).await? {
                        self.blobs.remove(&sha).await?;
                        report.blobs.push(sha);
                    }
                }
            }
            report.versions.extend(doomed);
        }
        Ok(report)
    }
}

/// An explicit link row made by the server itself (create-time story / fork).
fn explicit_link(
    src: &str,
    dst_kind: &str,
    dst_id: &str,
    rel: &str,
    policy: &str,
    pinned: Option<Id>,
    created_by: &str,
) -> NewLink {
    NewLink {
        src_artifact_id: src.into(),
        src_version_id: None,
        src_node: String::new(),
        dst_kind: dst_kind.into(),
        dst_id: dst_id.into(),
        dst_node: String::new(),
        rel: rel.into(),
        policy: policy.into(),
        pinned_version_id: pinned,
        origin: "explicit".into(),
        broken: false,
        meta: json!({}),
        created_by: created_by.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn svc() -> (DesignService, tempfile::TempDir, broadcast::Receiver<Event>) {
        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = broadcast::channel(256);
        let s = DesignService::new(otto_state::db::test_pool().await, dir.path(), Some(tx));
        assert!(s.store().ensure_fts().await);
        (s, dir, rx)
    }

    fn input(format: &str, title: &str, content: Option<&str>) -> CreateInput {
        CreateInput {
            workspace_id: "w1".into(),
            project_id: None,
            studio: None,
            format: format.into(),
            title: title.into(),
            tags: vec![],
            meta: json!({}),
            content: content.map(|c| c.as_bytes().to_vec()),
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

    fn opts(base: Option<&str>, author: Author) -> SaveOpts {
        SaveOpts {
            base: base.map(str::to_string),
            kind: "autosave".into(),
            author,
            message: String::new(),
            provenance: json!({}),
            force: false,
            validate: true,
            change: "content",
        }
    }

    fn drain(rx: &mut broadcast::Receiver<Event>) -> Vec<Event> {
        let mut out = Vec::new();
        while let Ok(e) = rx.try_recv() {
            out.push(e);
        }
        out
    }

    #[test]
    fn pure_helpers_bound_and_clean_inputs() {
        assert!(clean_title("  ").is_err());
        assert_eq!(
            clean_title(&"x".repeat(400)).unwrap().len(),
            MAX_TITLE_CHARS
        );
        assert_eq!(
            clean_tags(vec![
                " Hero ".into(),
                "hero".into(),
                "".into(),
                "cta".into()
            ]),
            vec!["Hero".to_string(), "cta".to_string()]
        );
        assert!(bound_json(json!([1]), 100, "x").is_err());
        assert!(matches!(
            bound_json(json!({"k": "y".repeat(200)}), 100, "x"),
            Err(Error::PayloadTooLarge(_))
        ));
        let deep = json!({"a":{"b":{"c":{"d":{"e":{"f":{"g":{"h":{"i":1}}}}}}}}});
        assert!(bound_json(deep, 10_000, "x").is_err());
        assert_eq!(bound_json(Value::Null, 10, "x").unwrap(), json!({}));
        let mut m = json!({"a": 1, "b": 2});
        merge_meta(&mut m, json!({"b": null, "c": 3})).unwrap();
        assert_eq!(m, json!({"a": 1, "c": 3}));
        assert!(decode_content(Some("x".into()), Some("eA==".into())).is_err());
        assert_eq!(
            decode_content(None, Some("aGk=".into())).unwrap().unwrap(),
            b"hi"
        );
    }

    #[tokio::test]
    async fn create_save_dedup_conflict_and_working_copy() {
        let (s, _dir, mut rx) = svc().await;
        let created = s
            .create_artifact(input("html", "Landing", Some("<h1>Hi</h1>")))
            .await
            .unwrap();
        assert!(created.created);
        assert_eq!(created.version.seq, 1);
        assert_eq!(created.artifact.studio, "frames");
        let wf = s.work_file(&created.artifact).unwrap();
        assert_eq!(std::fs::read_to_string(&wf).unwrap(), "<h1>Hi</h1>");
        let evs = drain(&mut rx);
        assert!(evs.iter().any(|e| matches!(e,
            Event::DesignArtifactUpdated { change, content: Some(c), .. } if change == "created" && c == "<h1>Hi</h1>")));

        let a = created.artifact;
        // Same bytes → no new version.
        let same = s
            .commit_bytes(
                &a,
                b"<h1>Hi</h1>".to_vec(),
                opts(Some(&created.version.id), Author::user("u1")),
            )
            .await
            .unwrap();
        assert!(!same.created);
        assert_eq!(same.version.id, created.version.id);
        // New bytes on the right base → v2.
        let v2 = s
            .commit_bytes(
                &a,
                b"<h1>Hello</h1>".to_vec(),
                opts(Some(&created.version.id), Author::user("u1")),
            )
            .await
            .unwrap();
        assert_eq!(v2.version.seq, 2);
        // Stale base → 409, head unchanged.
        let stale = s
            .commit_bytes(
                &v2.artifact,
                b"<h1>Nope</h1>".to_vec(),
                opts(Some(&created.version.id), Author::user("u1")),
            )
            .await;
        assert!(matches!(stale, Err(Error::Conflict(_))), "{stale:?}");
        let (head, bytes) = s.head_content(&v2.artifact).await.unwrap();
        assert_eq!(head.id, v2.version.id);
        assert_eq!(bytes, b"<h1>Hello</h1>");
        // Invalid bytes for a JSON format are refused before any write.
        let board = s
            .create_artifact(input("excalidraw", "Board", None))
            .await
            .unwrap();
        assert!(s
            .commit_bytes(
                &board.artifact,
                b"not json".to_vec(),
                opts(None, Author::user("u1"))
            )
            .await
            .is_err());
        assert_eq!(s.resolve_version(&v2.artifact, "v1").await.unwrap().seq, 1);
    }

    #[tokio::test]
    async fn links_extract_on_save_report_broken_and_reject_cycles() {
        let (s, _dir, mut rx) = svc().await;
        let card = s
            .create_artifact(input("scene3d", "Card", None))
            .await
            .unwrap()
            .artifact;
        let site_doc = json!({
            "type": "otto-site",
            "pages": [{ "id": "home", "sections": [
                { "id": "hero", "props": { "src": format!("otto://design/{}@approved", card.id) } },
                { "id": "faq", "props": { "href": "otto://design/MISSING01" } },
                { "id": "bad", "props": { "href": format!("otto://design/{}@v9", card.id) } }
            ]}]
        });
        let site = s
            .create_artifact(input(
                "otto-site",
                "Launch site",
                Some(&site_doc.to_string()),
            ))
            .await
            .unwrap();
        assert_eq!(site.links.extracted, 3);
        let reasons: Vec<&str> = site
            .links
            .broken
            .iter()
            .map(|b| b.reason.as_str())
            .collect();
        assert!(reasons.contains(&"missing_artifact"), "{reasons:?}");
        assert!(reasons.contains(&"missing_version"), "{reasons:?}");
        let out = s.store().links_out(&site.artifact.id).await.unwrap();
        let embed = out.iter().find(|l| l.rel == "embeds").unwrap();
        assert_eq!(embed.dst_id, card.id);
        assert_eq!(embed.policy, "follow_approved");
        assert_eq!(embed.src_node.as_deref(), Some("hero"));
        assert!(out.iter().filter(|l| l.broken).count() == 2);
        assert!(drain(&mut rx).iter().any(
            |e| matches!(e, Event::DesignLinkUpdated { reason, .. } if reason == "extracted")
        ));

        // Card embedding the site back would close a cycle → reported, not stored.
        let back = json!({ "type": "otto-scene3d", "version": 1, "objects": [],
            "meta": { "src": format!("otto://design/{}", site.artifact.id) } });
        let saved = s
            .commit_bytes(
                &card,
                back.to_string().into_bytes(),
                opts(None, Author::user("u1")),
            )
            .await
            .unwrap();
        assert_eq!(saved.links.cycles.len(), 1);
        assert!(s.store().links_out(&card.id).await.unwrap().is_empty());
        // Explicit render link that closes the cycle is a 409.
        let err = s
            .create_link(
                &card,
                CreateLinkReq {
                    rel: "embeds".into(),
                    dst_kind: "artifact".into(),
                    dst_id: site.artifact.id.clone(),
                    dst_node: None,
                    src_node: None,
                    policy: None,
                    pinned_version_id: None,
                    meta: None,
                },
                &Author::user("u1"),
            )
            .await;
        assert!(matches!(err, Err(Error::Conflict(_))), "{err:?}");
    }

    #[tokio::test]
    async fn approve_notifies_follow_approved_consumers_and_signals_flow() {
        let (s, _dir, mut rx) = svc().await;
        let card = s
            .create_artifact(input("scene3d", "Card", None))
            .await
            .unwrap()
            .artifact;
        let page = format!("<img src=\"otto://design/{}\">", card.id);
        let site = s
            .create_artifact(input("html", "Page", Some(&page)))
            .await
            .unwrap()
            .artifact;
        drain(&mut rx);
        let approved = s
            .approve(&card.id, None, &Author::user("u1"))
            .await
            .unwrap();
        assert_eq!(approved.status, "approved");
        assert_eq!(approved.approved_version_id, approved.head_version_id);
        let evs = drain(&mut rx);
        assert!(evs.iter().any(|e| matches!(e,
            Event::DesignLinkUpdated { artifact_id, reason, .. } if artifact_id == &site.id && reason == "target_approved")));
        assert!(evs.iter().any(
            |e| matches!(e, Event::DesignLearningUpdate { kind, .. } if kind == "status_change")
        ));

        // Agent draft then a human edit → edit_after_draft.
        let agent = Author {
            kind: "agent".into(),
            id: "u1".into(),
            session_id: Some("sess1".into()),
        };
        let draft = s
            .commit_bytes(&site, b"<h1>Agent</h1>".to_vec(), opts(None, agent))
            .await
            .unwrap();
        s.commit_bytes(
            &draft.artifact,
            b"<h1>Human</h1>\n<p>x</p>".to_vec(),
            opts(None, Author::user("u1")),
        )
        .await
        .unwrap();
        let sigs = s
            .store()
            .list_signals(
                None,
                Some(site.id.as_str()),
                Some("edit_after_draft"),
                None,
                10,
            )
            .await
            .unwrap();
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].payload["agent_session_id"], "sess1");
        assert!(sigs[0].payload["summary"]["lines_added"].as_u64().unwrap() >= 1);

        // Shipping via PATCH records `shipped`.
        s.update_meta(
            &card.id,
            UpdateArtifactReq {
                status: Some("shipped".into()),
                ..Default::default()
            },
            &Author::user("u1"),
        )
        .await
        .unwrap();
        let shipped = s
            .store()
            .list_signals(None, Some(card.id.as_str()), Some("shipped"), None, 10)
            .await
            .unwrap();
        assert_eq!(shipped.len(), 1);
        // Bad signal kinds / oversized payloads are refused.
        let bad = s
            .record_signal(
                &card,
                SignalReq {
                    artifact_id: card.id.clone(),
                    kind: "nope".into(),
                    version_id: None,
                    actor_kind: None,
                    session_id: None,
                    payload: None,
                },
                &Author::user("u1"),
            )
            .await;
        assert!(bad.is_err());
    }

    #[tokio::test]
    async fn fork_story_link_search_and_opt_in_prune() {
        let (s, _dir, _rx) = svc().await;
        let src = s
            .create_artifact(input(
                "mermaid",
                "Checkout flow",
                Some("flowchart TD\n Pay --> Done\n"),
            ))
            .await
            .unwrap()
            .artifact;
        let mut fork_in = input("mermaid", "Checkout v2", None);
        fork_in.derived_from = Some(ForkFrom {
            artifact_id: src.id.clone(),
            version_id: None,
        });
        let fork = s.create_artifact(fork_in).await.unwrap();
        let (_, bytes) = s.head_content(&fork.artifact).await.unwrap();
        assert!(String::from_utf8(bytes).unwrap().contains("Pay --> Done"));
        let out = s.store().links_out(&fork.artifact.id).await.unwrap();
        assert!(out.iter().any(|l| l.rel == "derived_from"
            && l.policy == "pinned"
            && l.pinned_version_id.is_some()));

        let f = crate::store::ArtifactFilter::default();
        let hits = s.store().search("checkout", &f).await.unwrap();
        assert_eq!(hits.len(), 2);

        // Autosaves inside one window: prune plans all but the last; nothing
        // is removed until `apply`.
        let mut a = src.clone();
        for body in [
            "flowchart TD\n A\n",
            "flowchart TD\n B\n",
            "flowchart TD\n C\n",
        ] {
            a = s
                .commit_bytes(&a, body.as_bytes().to_vec(), opts(None, Author::user("u1")))
                .await
                .unwrap()
                .artifact;
        }
        let dry = s.prune(Some(&src.id), false, 600).await.unwrap();
        assert!(!dry.applied);
        // v1 (named, fork-pinned) and v4 (head, last of the window) stay.
        assert_eq!(dry.versions.len(), 2, "{dry:?}");
        assert_eq!(
            s.store()
                .list_versions(&src.id, None, 0, 0)
                .await
                .unwrap()
                .len(),
            4
        );
        let done = s.prune(Some(&src.id), true, 600).await.unwrap();
        assert_eq!(done.versions, dry.versions);
        assert_eq!(
            s.store()
                .list_versions(&src.id, None, 0, 0)
                .await
                .unwrap()
                .len(),
            2
        );
        // The fork pinned v1 of the source — protected, still there.
        assert!(s
            .store()
            .get_version_by_seq(&src.id, 1)
            .await
            .unwrap()
            .is_some());
    }
}
