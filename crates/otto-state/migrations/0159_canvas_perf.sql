-- Perf wave (F4): canvas version history without per-open JSON parsing, and
-- Excalidraw images stored once per content hash instead of once per version.
-- Additive only.

-- `format` / `size` are filled once at snapshot time, so listing a scene's
-- history reads two plain columns instead of json-parsing up to 30 full
-- documents (each up to 25 MB) on every open.
ALTER TABLE canvas_scene_versions ADD COLUMN format TEXT;
ALTER TABLE canvas_scene_versions ADD COLUMN size INTEGER;
UPDATE canvas_scene_versions
   SET size   = length(doc_json),
       format = CASE WHEN json_valid(doc_json) THEN json_extract(doc_json, '$.format') END;

-- Content-addressed Excalidraw files (pasted images). A version's doc keeps
-- `dataURL: "otto-canvas-file:<sha256>"` in place of the base64 payload; the
-- bytes live here exactly once however many versions reference them.
CREATE TABLE IF NOT EXISTS canvas_files (
    sha256      TEXT PRIMARY KEY,
    mime        TEXT NOT NULL,
    data_url    TEXT NOT NULL,
    size        INTEGER NOT NULL,
    created_at  TEXT NOT NULL
);

-- Which version references which file — the GC root set after a prune.
CREATE TABLE IF NOT EXISTS canvas_version_files (
    version_id  TEXT NOT NULL,
    sha256      TEXT NOT NULL,
    PRIMARY KEY (version_id, sha256)
);
CREATE INDEX IF NOT EXISTS idx_canvas_version_files_sha ON canvas_version_files(sha256);
