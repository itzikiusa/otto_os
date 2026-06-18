CREATE TABLE product_transcripts (
    id         TEXT PRIMARY KEY,
    story_id   TEXT NOT NULL,
    title      TEXT NOT NULL DEFAULT '',
    body       TEXT NOT NULL,
    created_by TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX idx_product_transcripts_story ON product_transcripts(story_id);
