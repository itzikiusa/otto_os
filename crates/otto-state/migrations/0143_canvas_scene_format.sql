-- SD-22: the scene lists read `json_extract(doc_json, '$.format')` per row,
-- which parses every scene's whole document (an Excalidraw scene with images
-- is megabytes) on every list. Keep the format in its own column instead.
--
-- Triggers (not application code) maintain it, so every writer stays correct:
-- the repo, the Design Hall importer, raw test SQL and archive restores. The
-- update trigger fires only when the document actually changed, so a title /
-- thumbnail edit never re-parses it. `json_valid` guards a malformed doc
-- (json_extract would raise and fail the write).
ALTER TABLE canvas_scenes ADD COLUMN format TEXT;

UPDATE canvas_scenes
SET format = json_extract(doc_json, '$.format')
WHERE json_valid(doc_json);

CREATE TRIGGER canvas_scenes_format_insert AFTER INSERT ON canvas_scenes
BEGIN
  UPDATE canvas_scenes
  SET format = CASE WHEN json_valid(NEW.doc_json) THEN json_extract(NEW.doc_json, '$.format') END
  WHERE id = NEW.id;
END;

CREATE TRIGGER canvas_scenes_format_update AFTER UPDATE OF doc_json ON canvas_scenes
WHEN NEW.doc_json IS NOT OLD.doc_json
BEGIN
  UPDATE canvas_scenes
  SET format = CASE WHEN json_valid(NEW.doc_json) THEN json_extract(NEW.doc_json, '$.format') END
  WHERE id = NEW.id;
END;
