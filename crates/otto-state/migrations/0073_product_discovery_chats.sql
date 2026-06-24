-- Discovery Chat: a lightweight, interactive conversation with an agent attached
-- to a product story (works even on an empty/Untitled draft) to help with early
-- discovery and research BEFORE a story is written. Distinct from the heavyweight
-- swarm discovery runs (0068) and from per-version refinement threads (0070/0071).
CREATE TABLE product_discovery_chats (
    id            TEXT PRIMARY KEY,
    story_id      TEXT NOT NULL,
    workspace_id  TEXT NOT NULL,
    cwd           TEXT NOT NULL,
    title         TEXT NOT NULL,
    status        TEXT NOT NULL DEFAULT 'active',  -- active | archived
    model         TEXT,
    created_by    TEXT NOT NULL,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);
CREATE INDEX idx_discovery_chats_story ON product_discovery_chats(story_id, created_at);

CREATE TABLE product_discovery_chat_messages (
    id            TEXT PRIMARY KEY,
    chat_id       TEXT NOT NULL,
    role          TEXT NOT NULL,        -- user | agent
    body          TEXT NOT NULL,        -- markdown (agent prose; user text)
    actions_json  TEXT,                 -- agent: JSON array of proposed actions
    meta_json     TEXT,                 -- user: the assembled context bundle (audit)
    created_at    TEXT NOT NULL
);
CREATE INDEX idx_discovery_chat_msgs ON product_discovery_chat_messages(chat_id, created_at);
