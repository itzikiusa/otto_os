CREATE TABLE product_attachments (
    id            TEXT PRIMARY KEY,
    story_id      TEXT NOT NULL,
    workspace_id  TEXT NOT NULL,
    filename      TEXT NOT NULL,          -- original filename (sanitized for display)
    mime          TEXT NOT NULL,          -- detected/declared content type
    size_bytes    INTEGER NOT NULL,
    sha256        TEXT,                   -- content hash (dedup / integrity); nullable, left NULL in MVP (no sha2 dep)
    storage_path  TEXT NOT NULL,          -- path relative to data_dir
    kind          TEXT NOT NULL DEFAULT 'file',  -- 'file' | 'mockup' | 'image'
    source        TEXT NOT NULL DEFAULT 'user',  -- 'user' | 'agent'
    meta_json     TEXT,                   -- optional: agent run id, mockup format, etc.
    created_by    TEXT NOT NULL,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);
CREATE INDEX idx_product_attachments_story ON product_attachments(story_id, kind);
