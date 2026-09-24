-- Otto Assistant (docs/contracts/api.md "Otto Assistant"). Additive only: new
-- tables, no change to any existing one. Every assistant row is per user
-- (`owner_user_id`); the daemon never shows one user's rows to another.

-- One thread = one resumable CLI session (claude / codex). `session_id` is the
-- CURRENT backing session (a provider switch starts a new one); `space_slot`
-- 1..4 pins the thread to a floating-bar space (unique per user).
CREATE TABLE assistant_threads (
    id              TEXT PRIMARY KEY,
    owner_user_id   TEXT NOT NULL,
    space_slot      INTEGER CHECK (space_slot IS NULL OR space_slot BETWEEN 1 AND 4),
    title           TEXT NOT NULL DEFAULT '',
    provider        TEXT NOT NULL DEFAULT 'claude',
    model           TEXT,
    account_id      TEXT,
    route_pinned    INTEGER NOT NULL DEFAULT 0,
    session_id      TEXT,
    incognito       INTEGER NOT NULL DEFAULT 0,
    failover_choice TEXT NOT NULL DEFAULT 'ask',            -- ask | switch | stay
    last_turn_at    TEXT,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);
CREATE UNIQUE INDEX idx_assistant_threads_slot
    ON assistant_threads(owner_user_id, space_slot) WHERE space_slot IS NOT NULL;
CREATE INDEX idx_assistant_threads_owner ON assistant_threads(owner_user_id, updated_at);
CREATE INDEX idx_assistant_threads_session ON assistant_threads(session_id);

-- The turn INDEX (search, hand-offs, phone push). The canonical reply text
-- stays in the provider transcript; `source_ref` is the transcript turn id an
-- indexed reply came from (idempotent re-index).
CREATE TABLE assistant_turns (
    id               TEXT PRIMARY KEY,
    thread_id        TEXT NOT NULL REFERENCES assistant_threads(id) ON DELETE CASCADE,
    role             TEXT NOT NULL,                          -- user | assistant | system
    kind             TEXT NOT NULL DEFAULT 'message',
    text             TEXT NOT NULL DEFAULT '',
    provider         TEXT,
    model            TEXT,
    route_reason     TEXT,
    session_id       TEXT,
    attachments_json TEXT NOT NULL DEFAULT '[]',
    data_json        TEXT,
    source_ref       TEXT,
    created_at       TEXT NOT NULL
);
CREATE INDEX idx_assistant_turns_thread ON assistant_turns(thread_id, id);
CREATE UNIQUE INDEX idx_assistant_turns_source
    ON assistant_turns(thread_id, source_ref) WHERE source_ref IS NOT NULL;

-- Files dropped on the bar / pasted / shared from the phone; the bytes live in
-- `<assistant cwd>/inbox/`, this row is the reference a turn attaches.
CREATE TABLE assistant_attachments (
    id            TEXT PRIMARY KEY,
    thread_id     TEXT NOT NULL REFERENCES assistant_threads(id) ON DELETE CASCADE,
    owner_user_id TEXT NOT NULL,
    name          TEXT NOT NULL,
    path          TEXT NOT NULL,
    mime          TEXT NOT NULL DEFAULT 'application/octet-stream',
    size          INTEGER NOT NULL DEFAULT 0,
    created_at    TEXT NOT NULL
);
CREATE INDEX idx_assistant_attachments_thread ON assistant_attachments(thread_id);

-- Tasks, reminders and the needs-you queue (state `needs_you` + `needs_you_json`).
-- `schedule_json` is a cadence spec (`{cadence:"once", run_at}`) for reminders;
-- `schedule_id` names a backing Personal Agent schedule when there is one.
CREATE TABLE assistant_tasks (
    id             TEXT PRIMARY KEY,
    owner_user_id  TEXT NOT NULL,
    thread_id      TEXT REFERENCES assistant_threads(id) ON DELETE CASCADE,
    kind           TEXT NOT NULL,
    state          TEXT NOT NULL DEFAULT 'queued',
    title          TEXT NOT NULL DEFAULT '',
    detail         TEXT NOT NULL DEFAULT '',
    origin         TEXT NOT NULL DEFAULT 'thread',
    run_at         TEXT,
    timezone       TEXT NOT NULL DEFAULT '',
    schedule_json  TEXT,
    schedule_id    TEXT,
    agent_id       TEXT,
    agent_run_id   TEXT,
    needs_you_json TEXT,
    result_json    TEXT,
    created_at     TEXT NOT NULL,
    updated_at     TEXT NOT NULL,
    finished_at    TEXT
);
CREATE INDEX idx_assistant_tasks_owner_state ON assistant_tasks(owner_user_id, state);
CREATE INDEX idx_assistant_tasks_due ON assistant_tasks(kind, state, run_at);
CREATE INDEX idx_assistant_tasks_thread ON assistant_tasks(thread_id);

-- Per-user router settings + the last detected usage limit per provider.
CREATE TABLE assistant_routing (
    owner_user_id   TEXT PRIMARY KEY,
    rules_json      TEXT NOT NULL DEFAULT '{}',             -- {targets, extra_keywords}
    auto_failover   INTEGER NOT NULL DEFAULT 0,
    memory_approval INTEGER NOT NULL DEFAULT 0,
    limits_json     TEXT NOT NULL DEFAULT '[]',
    updated_at      TEXT NOT NULL
);

-- Agent identities + grants (the Permissions / identity work is a later phase;
-- today only the assistant's "always allow" destination+tool grants use them).
CREATE TABLE agent_principals (
    id            TEXT PRIMARY KEY,
    kind          TEXT NOT NULL,                             -- assistant | personal_agent | swarm_agent
    owner_user_id TEXT NOT NULL,
    ref_id        TEXT NOT NULL DEFAULT '',                  -- e.g. personal_agents.id; '' for the assistant
    name          TEXT NOT NULL,
    avatar        TEXT NOT NULL DEFAULT '',
    token_hash    TEXT,
    created_at    TEXT NOT NULL,
    UNIQUE (kind, owner_user_id, ref_id)
);

CREATE TABLE agent_grants (
    principal_id  TEXT NOT NULL REFERENCES agent_principals(id) ON DELETE CASCADE,
    resource_kind TEXT NOT NULL,                             -- e.g. tool_destination
    resource      TEXT NOT NULL,
    mode          TEXT NOT NULL CHECK (mode IN ('allow', 'ask', 'deny')),
    created_at    TEXT NOT NULL,
    PRIMARY KEY (principal_id, resource_kind, resource)
);
