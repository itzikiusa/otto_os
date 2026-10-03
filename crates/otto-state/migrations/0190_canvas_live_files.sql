-- Perf wave 2 (R2): the LIVE canvas document no longer embeds Excalidraw
-- images. Every write externalizes inline `files[*].dataURL` payloads into the
-- content-addressed `canvas_files` table (0159) and keeps an
-- `otto-canvas-file:<sha256>` ref in the doc, so a 700 ms autosave stops
-- re-sending (and re-storing) a pasted screenshot. This table is the live
-- doc's half of the GC root set (`canvas_version_files` is the history half):
-- a file stays while any scene or version references it. Additive only.
CREATE TABLE IF NOT EXISTS canvas_scene_files (
    scene_id    TEXT NOT NULL,
    sha256      TEXT NOT NULL,
    PRIMARY KEY (scene_id, sha256)
);
CREATE INDEX IF NOT EXISTS idx_canvas_scene_files_sha ON canvas_scene_files(sha256);
