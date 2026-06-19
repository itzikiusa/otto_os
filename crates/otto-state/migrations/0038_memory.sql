-- Memory layer: a workspace-scoped knowledge store with keyword + vector recall.
-- One comprehensive schema for the whole feature core (items + chunks, vectors,
-- and the lightweight link graph). Keyword search uses LIKE (always available);
-- vectors live in a companion BLOB table searched in-process.

CREATE TABLE memories (
  id              TEXT PRIMARY KEY,
  workspace_id    TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
  collection      TEXT NOT NULL DEFAULT 'product',   -- product | code | docs | confluence | platform-map
  record_type     TEXT NOT NULL DEFAULT 'item',      -- item (distilled) | chunk (raw)
  scope           TEXT NOT NULL,                      -- workspace | story | entity
  story_id        TEXT,
  kind            TEXT NOT NULL,                      -- fact|decision|requirement|constraint|qa|learning|summary|entity|snapshot|glossary|chunk
  title           TEXT NOT NULL,
  body            TEXT NOT NULL,
  entities_json   TEXT NOT NULL DEFAULT '[]',
  tags_json       TEXT NOT NULL DEFAULT '[]',
  source_kind     TEXT NOT NULL,
  source_ref      TEXT,
  refs_json       TEXT NOT NULL DEFAULT '[]',
  confidence      REAL NOT NULL DEFAULT 0.7,
  salience        REAL NOT NULL DEFAULT 0.5,
  content_hash    TEXT NOT NULL,
  active          INTEGER NOT NULL DEFAULT 1,
  superseded_by   TEXT,
  version         INTEGER NOT NULL DEFAULT 1,
  created_by      TEXT NOT NULL,
  created_at      TEXT NOT NULL,
  updated_at      TEXT NOT NULL,
  last_accessed_at TEXT,
  access_count    INTEGER NOT NULL DEFAULT 0,
  expires_at      TEXT
);
CREATE INDEX idx_memories_ws     ON memories(workspace_id, active);
CREATE INDEX idx_memories_story  ON memories(story_id, active);
CREATE INDEX idx_memories_kind   ON memories(workspace_id, collection, kind, active);
CREATE INDEX idx_memories_source ON memories(workspace_id, source_kind, source_ref);
CREATE UNIQUE INDEX idx_memories_dedup
  ON memories(workspace_id, collection, scope, IFNULL(story_id,''), content_hash);

CREATE TABLE memory_vectors (
  memory_id   TEXT PRIMARY KEY REFERENCES memories(id) ON DELETE CASCADE,
  model_id    TEXT NOT NULL,
  dim         INTEGER NOT NULL,
  embedding   BLOB NOT NULL,                          -- little-endian f32[dim]
  embedded_at TEXT NOT NULL
);

CREATE TABLE memory_links (
  src_id     TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
  dst_id     TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
  rel        TEXT NOT NULL,                           -- relates_to|supersedes|derived_from|about_entity|duplicates|blocks
  weight     REAL NOT NULL DEFAULT 1.0,
  certainty  TEXT,                                    -- extracted|inferred|ambiguous (graphify-style)
  created_at TEXT NOT NULL,
  PRIMARY KEY (src_id, dst_id, rel)
);
CREATE INDEX idx_memory_links_dst ON memory_links(dst_id);
