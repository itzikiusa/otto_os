-- Session ↔ Canvas scene references: lets a Canvas scene be attached to one or
-- more agent sessions so it shows up in that session's Canvas panel (and the
-- agent can create/point sessions at scenes via the MCP write tools). A scene
-- may be referenced by many sessions, and a session may reference many scenes.
CREATE TABLE canvas_scene_refs (
    scene_id     TEXT NOT NULL REFERENCES canvas_scenes(id) ON DELETE CASCADE,
    session_id   TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    created_by   TEXT NOT NULL REFERENCES users(id),
    created_at   TEXT NOT NULL,
    PRIMARY KEY (scene_id, session_id)
);

-- Lists refs for a session (the panel's primary query).
CREATE INDEX idx_canvas_refs_session ON canvas_scene_refs(session_id);
