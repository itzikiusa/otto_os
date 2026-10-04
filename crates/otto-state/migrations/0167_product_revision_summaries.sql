-- Thin revision metadata avoids reading/parsing historical source bodies on
-- every Product history poll. The full versions remain authoritative.
CREATE TABLE product_revision_summaries (
    id TEXT PRIMARY KEY REFERENCES product_story_versions(id) ON DELETE CASCADE,
    story_id TEXT NOT NULL,
    version_no INTEGER NOT NULL,
    kind TEXT NOT NULL,
    title TEXT NOT NULL,
    raw_json TEXT,
    change_notes TEXT,
    created_by TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX product_revision_summaries_story ON product_revision_summaries(story_id, version_no DESC);
INSERT INTO product_revision_summaries (id, story_id, version_no, kind, title, raw_json, change_notes, created_by, created_at)
SELECT id, story_id, version_no, kind, title, CASE WHEN json_valid(raw_json) THEN CASE WHEN json_type(raw_json, '$.version') IN ('integer', 'real') THEN json_object('version', json_extract(raw_json, '$.version')) END END, change_notes, created_by, created_at FROM product_story_versions;

CREATE TRIGGER product_revision_summary_insert AFTER INSERT ON product_story_versions
BEGIN
    INSERT INTO product_revision_summaries (id, story_id, version_no, kind, title, raw_json, change_notes, created_by, created_at) VALUES (NEW.id, NEW.story_id, NEW.version_no, NEW.kind, NEW.title, CASE WHEN json_valid(NEW.raw_json) THEN CASE WHEN json_type(NEW.raw_json, '$.version') IN ('integer', 'real') THEN json_object('version', json_extract(NEW.raw_json, '$.version')) END END, NEW.change_notes, NEW.created_by, NEW.created_at);
END;
CREATE TRIGGER product_revision_summary_update AFTER UPDATE OF story_id, version_no, kind, title, raw_json, change_notes, created_by, created_at ON product_story_versions
BEGIN
    UPDATE product_revision_summaries SET
        story_id = NEW.story_id, version_no = NEW.version_no, kind = NEW.kind,
        title = NEW.title, raw_json = CASE WHEN json_valid(NEW.raw_json) THEN CASE WHEN json_type(NEW.raw_json, '$.version') IN ('integer', 'real') THEN json_object('version', json_extract(NEW.raw_json, '$.version')) END END,
        change_notes = NEW.change_notes, created_by = NEW.created_by, created_at = NEW.created_at
    WHERE id = NEW.id;
END;
