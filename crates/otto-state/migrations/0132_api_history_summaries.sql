-- Compact list metadata. Original request/response bytes and replay history stay intact.
-- source_kind intentionally has no affinity: a malformed numeric legacy kind must
-- not suddenly match a text source filter that previously used json_extract.
ALTER TABLE api_history ADD COLUMN source_kind DEFAULT 'human';
ALTER TABLE api_history ADD COLUMN source_session_id TEXT;
ALTER TABLE api_history ADD COLUMN source_via TEXT;
ALTER TABLE api_history ADD COLUMN request_id TEXT;

-- One cold metadata pass; malformed legacy JSON cannot abort upgrade.
UPDATE api_history SET (source_kind, source_session_id, source_via, request_id) = (
SELECT
    COALESCE(MAX(CASE WHEN key = 0 THEN value END), 'human'),
    MAX(CASE WHEN key = 1 AND type = 'text' THEN value END),
    MAX(CASE WHEN key = 2 AND type = 'text' THEN value END),
    MAX(CASE WHEN key = 3 AND type = 'text' THEN value END)
FROM json_each(CASE WHEN json_valid(request_json)
    THEN json_extract(request_json, '$.source.kind', '$.source.session_id', '$.source.via', '$.request_id')
    ELSE '[null,null,null,null]' END)
);

CREATE TRIGGER api_history_summary_insert AFTER INSERT ON api_history BEGIN
UPDATE api_history SET (source_kind, source_session_id, source_via, request_id) = (
SELECT
    COALESCE(MAX(CASE WHEN key = 0 THEN value END), 'human'),
    MAX(CASE WHEN key = 1 AND type = 'text' THEN value END),
    MAX(CASE WHEN key = 2 AND type = 'text' THEN value END),
    MAX(CASE WHEN key = 3 AND type = 'text' THEN value END)
FROM json_each(CASE WHEN json_valid(request_json)
    THEN json_extract(request_json, '$.source.kind', '$.source.session_id', '$.source.via', '$.request_id')
    ELSE '[null,null,null,null]' END)
)
WHERE id = NEW.id;
END;

CREATE TRIGGER api_history_summary_update AFTER UPDATE OF request_json ON api_history BEGIN
UPDATE api_history SET (source_kind, source_session_id, source_via, request_id) = (
SELECT
    COALESCE(MAX(CASE WHEN key = 0 THEN value END), 'human'),
    MAX(CASE WHEN key = 1 AND type = 'text' THEN value END),
    MAX(CASE WHEN key = 2 AND type = 'text' THEN value END),
    MAX(CASE WHEN key = 3 AND type = 'text' THEN value END)
FROM json_each(CASE WHEN json_valid(request_json)
    THEN json_extract(request_json, '$.source.kind', '$.source.session_id', '$.source.via', '$.request_id')
    ELSE '[null,null,null,null]' END)
)
WHERE id = NEW.id;
END;

CREATE INDEX api_history_workspace_source_time ON api_history(workspace_id, source_kind, executed_at DESC, id DESC);
CREATE INDEX api_history_workspace_request_time ON api_history(workspace_id, request_id, executed_at DESC, id DESC);
