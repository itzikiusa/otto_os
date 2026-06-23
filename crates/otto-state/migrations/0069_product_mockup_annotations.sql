CREATE TABLE product_mockup_annotations (
    id             TEXT PRIMARY KEY,
    attachment_id  TEXT NOT NULL,         -- the mockup attachment being annotated
    story_id       TEXT NOT NULL,
    workspace_id   TEXT NOT NULL,
    x_pct          REAL NOT NULL,         -- 0..1 relative to rendered mockup box
    y_pct          REAL NOT NULL,
    body           TEXT NOT NULL,
    resolved       INTEGER NOT NULL DEFAULT 0,
    author_id      TEXT NOT NULL,
    created_at     TEXT NOT NULL,
    updated_at     TEXT NOT NULL
);
CREATE INDEX idx_mockup_annotations_att ON product_mockup_annotations(attachment_id);
