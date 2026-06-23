CREATE TABLE product_refinement_messages (
    id            TEXT PRIMARY KEY,
    thread_id     TEXT NOT NULL,
    role          TEXT NOT NULL,
    body          TEXT NOT NULL,
    meta_json     TEXT,
    created_at    TEXT NOT NULL
);
CREATE INDEX idx_refine_messages_thread ON product_refinement_messages(thread_id, created_at);
