-- Design Hall — the artifact graph (Phase 0; docs/features/design-hall.md).
--
-- ADDITIVE ONLY: six new tables + their indexes. No existing table is altered,
-- no existing row is rewritten. Legacy design data (`product_attachments` of
-- kind design|mockup + images/models, and `canvas_scenes`) stays exactly where
-- it is; the idempotent import job in `otto-design` (`import.rs`) mirrors those
-- rows into this graph and records the original id in `source_kind`/`source_id`
-- (+ `meta_json.imported_from`), so every old route keeps working unchanged.
--
-- Content never lives in these tables: every version points at a blob in the
-- content-addressed store `<data>/design/blobs/<sha256>`, and the editable
-- working copy is `<data>/design/<artifact>/work/`. Rows hold metadata only.
--
-- No foreign-key clauses (the canvas/product convention): child rows are
-- deleted explicitly by the repo, and the saved-state archive can restore the
-- tables in any order.
--
-- The FTS5 search index (`design_search_fts`) is created at RUNTIME by
-- `otto_design::store::Store::ensure_fts`, like `memories_fts` / `vault_fts`:
-- FTS5 availability depends on the linked SQLite, and a migration that cannot
-- create it would brick daemon boot instead of degrading to LIKE search.

-- A named collection of artifacts (the unit the Lobby browses), optionally
-- bound to an epic (product_stories.id) and/or a swarm project.
CREATE TABLE IF NOT EXISTS design_projects (
    id                TEXT PRIMARY KEY,
    workspace_id      TEXT NOT NULL,              -- provenance + RBAC axis
    name              TEXT NOT NULL,
    description       TEXT NOT NULL DEFAULT '',
    epic_story_id     TEXT,                       -- product_stories.id (epic)
    swarm_project_id  TEXT,                       -- swarm_projects.id
    brand_kit_id      TEXT,                       -- design_artifacts.id (studio 'brand')
    cover_artifact_id TEXT,                       -- design_artifacts.id
    archived          INTEGER NOT NULL DEFAULT 0,
    meta_json         TEXT NOT NULL DEFAULT '{}',
    created_by        TEXT NOT NULL,              -- users.id
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_design_projects_ws ON design_projects(workspace_id, updated_at);
CREATE INDEX IF NOT EXISTS idx_design_projects_epic ON design_projects(epic_story_id);

-- One design artifact (a site, a frame board, a 3D scene, a whiteboard, a
-- brand kit, an uploaded image/model …). `head_version_id` is the newest
-- committed version; `approved_version_id` is what follow-approved consumers
-- render. `source_kind`/`source_id` identify an imported legacy row.
CREATE TABLE IF NOT EXISTS design_artifacts (
    id                  TEXT PRIMARY KEY,
    project_id          TEXT,                     -- design_projects.id; NULL = unfiled
    workspace_id        TEXT NOT NULL,
    studio              TEXT NOT NULL,            -- frames|graphics|site|3d|whiteboard|brand|spatial
    format              TEXT NOT NULL,            -- html|mermaid|excalidraw|d2|scene3d|otto-canvas|otto-site|…
    mime                TEXT NOT NULL,
    title               TEXT NOT NULL,
    status              TEXT NOT NULL DEFAULT 'draft', -- draft|review|approved|shipped|archived
    head_version_id     TEXT,
    approved_version_id TEXT,
    tags_json           TEXT NOT NULL DEFAULT '[]',
    thumb_blob          TEXT,                     -- sha256 of a PNG in the blob store
    meta_json           TEXT NOT NULL DEFAULT '{}',
    source_kind         TEXT,                     -- 'product_attachment' | 'canvas_scene' | NULL
    source_id           TEXT,
    created_by          TEXT NOT NULL,            -- users.id (the authenticated principal)
    created_by_kind     TEXT NOT NULL DEFAULT 'user', -- user|agent|system
    created_session_id  TEXT,                     -- agent session that created it, if any
    created_at          TEXT NOT NULL,
    updated_at          TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_design_artifacts_project ON design_artifacts(project_id, updated_at);
CREATE INDEX IF NOT EXISTS idx_design_artifacts_ws ON design_artifacts(workspace_id, updated_at);
CREATE INDEX IF NOT EXISTS idx_design_artifacts_studio ON design_artifacts(studio, status);
-- Idempotency key for the legacy import (one graph row per source row).
CREATE UNIQUE INDEX IF NOT EXISTS idx_design_artifacts_source
    ON design_artifacts(source_kind, source_id) WHERE source_kind IS NOT NULL;

-- Immutable version snapshots. `seq` is 1-based per artifact; `branch` is
-- 'main' or 'variant/<run>/<k>' (Phase 1 variants). `kind` records why the
-- version exists (autosave|named|agent|import|sync|restore) and drives the
-- opt-in retention planner — nothing is ever pruned automatically.
CREATE TABLE IF NOT EXISTS design_versions (
    id                TEXT PRIMARY KEY,
    artifact_id       TEXT NOT NULL,
    seq               INTEGER NOT NULL,
    parent_version_id TEXT,
    branch            TEXT NOT NULL DEFAULT 'main',
    blob_sha256       TEXT NOT NULL,
    size_bytes        INTEGER NOT NULL,
    kind              TEXT NOT NULL DEFAULT 'autosave',
    author_kind       TEXT NOT NULL,              -- user|agent|system
    author_id         TEXT NOT NULL,
    session_id        TEXT,
    message           TEXT NOT NULL DEFAULT '',
    provenance_json   TEXT NOT NULL DEFAULT '{}',
    created_at        TEXT NOT NULL,
    UNIQUE (artifact_id, seq)
);
CREATE INDEX IF NOT EXISTS idx_design_versions_blob ON design_versions(blob_sha256);

-- Typed, version-aware links. `origin='extracted'` rows are rebuilt from the
-- document's `otto://design/…` URIs on every save; `origin='explicit'` rows
-- are made in the UI / by agents. Node columns use '' (not NULL) so the unique
-- index dedups. `broken=1` marks a dangling target (badge, never a crash).
CREATE TABLE IF NOT EXISTS design_links (
    id                TEXT PRIMARY KEY,
    src_artifact_id   TEXT NOT NULL,
    src_version_id    TEXT,
    src_node          TEXT NOT NULL DEFAULT '',
    dst_kind          TEXT NOT NULL,              -- artifact|story|session|swarm_project|vault_note|pr|url|attachment|publish
    dst_id            TEXT NOT NULL,
    dst_node          TEXT NOT NULL DEFAULT '',
    rel               TEXT NOT NULL,              -- embeds|uses_component|uses_tokens|describes|derived_from|references|implements|…
    policy            TEXT NOT NULL DEFAULT 'follow_approved', -- follow_approved|follow_latest|pinned
    pinned_version_id TEXT,
    origin            TEXT NOT NULL,              -- explicit|extracted
    broken            INTEGER NOT NULL DEFAULT 0,
    meta_json         TEXT NOT NULL DEFAULT '{}',
    created_by        TEXT NOT NULL,
    created_at        TEXT NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_design_links_uniq
    ON design_links(src_artifact_id, origin, rel, dst_kind, dst_id, src_node, dst_node);
CREATE INDEX IF NOT EXISTS idx_design_links_dst ON design_links(dst_kind, dst_id);

-- Append-only learning signals (captured from day one; nothing learns from
-- them in Phase 0). Payloads are bounded summaries + version references,
-- never raw document content.
CREATE TABLE IF NOT EXISTS design_signals (
    id           TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL,
    artifact_id  TEXT NOT NULL,
    version_id   TEXT,
    kind         TEXT NOT NULL,                   -- variant_chosen|variant_rejected|edit_after_draft|review_comment|…
    actor_kind   TEXT NOT NULL,                   -- user|agent|system
    actor_id     TEXT NOT NULL,
    session_id   TEXT,
    payload_json TEXT NOT NULL DEFAULT '{}',
    created_at   TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_design_signals_ws ON design_signals(workspace_id, created_at);
CREATE INDEX IF NOT EXISTS idx_design_signals_artifact ON design_signals(artifact_id, created_at);
CREATE INDEX IF NOT EXISTS idx_design_signals_kind ON design_signals(kind, created_at);

-- Publishes pin the exact version set they embed (`pinned_set_json`), so a
-- republish is reproducible. Written by Phase 1 export/publish; the Phase 0
-- retention planner already treats every version named here as protected.
CREATE TABLE IF NOT EXISTS design_publishes (
    id              TEXT PRIMARY KEY,
    artifact_id     TEXT NOT NULL,
    version_id      TEXT NOT NULL,
    target          TEXT NOT NULL,                -- zip|local|artifact|gh_pages|netlify|cloudflare
    url             TEXT,
    pinned_set_json TEXT NOT NULL DEFAULT '[]',   -- [{artifact_id, version_id}]
    created_by      TEXT NOT NULL,
    created_at      TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_design_publishes_artifact ON design_publishes(artifact_id, created_at);

-- RBAC: the new `design` feature is granted wherever `canvas` is granted today
-- (Design Hall absorbs Canvas). Additive INSERT OR IGNORE — no existing grant
-- row is changed, and an explicit `design` row (none exist yet) always wins.
INSERT OR IGNORE INTO user_feature_grants (user_id, feature, capability)
    SELECT user_id, 'design', capability FROM user_feature_grants WHERE feature = 'canvas';
