-- Product story analysis & refinement
CREATE TABLE product_stories (
    id            TEXT PRIMARY KEY,
    workspace_id  TEXT NOT NULL,
    source_kind   TEXT NOT NULL,            -- 'jira' | 'confluence'
    account_id    TEXT NOT NULL,
    source_key    TEXT NOT NULL,            -- Jira key | Confluence pageId
    title         TEXT NOT NULL,
    url           TEXT NOT NULL,
    issue_type    TEXT,
    stage         TEXT NOT NULL DEFAULT 'imported',
    cwd           TEXT,
    watch_enabled INTEGER NOT NULL DEFAULT 0,
    watch_cadence_min INTEGER NOT NULL DEFAULT 15,
    watch_cursor  TEXT,
    confluence_tests_page_id TEXT,
    confluence_tests_url     TEXT,
    created_by    TEXT NOT NULL,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);
CREATE INDEX idx_product_stories_ws ON product_stories(workspace_id);
CREATE INDEX idx_product_stories_watch ON product_stories(watch_enabled);

CREATE TABLE product_story_versions (
    id           TEXT PRIMARY KEY,
    story_id     TEXT NOT NULL,
    version_no   INTEGER NOT NULL,
    kind         TEXT NOT NULL,             -- 'source' | 'suggested' | 'published'
    title        TEXT NOT NULL,
    body_md      TEXT NOT NULL,
    raw_json     TEXT,
    change_notes TEXT,
    created_by   TEXT NOT NULL,
    created_at   TEXT NOT NULL
);
CREATE INDEX idx_product_versions_story ON product_story_versions(story_id);

CREATE TABLE product_analyses (
    id                TEXT PRIMARY KEY,
    story_id          TEXT NOT NULL,
    source_version_id TEXT,
    status            TEXT NOT NULL,        -- 'running'|'done'|'error'|'partial'
    summary           TEXT NOT NULL DEFAULT '',
    created_by        TEXT NOT NULL,
    created_at        TEXT NOT NULL,
    finished_at       TEXT
);
CREATE INDEX idx_product_analyses_story ON product_analyses(story_id);

CREATE TABLE product_analysis_agents (
    id           TEXT PRIMARY KEY,
    analysis_id  TEXT NOT NULL,
    name         TEXT NOT NULL,
    skill        TEXT NOT NULL,
    provider     TEXT NOT NULL,
    model        TEXT NOT NULL DEFAULT '',
    status       TEXT NOT NULL,            -- 'pending'|'running'|'done'|'error'
    findings_json TEXT,
    error        TEXT,
    started_at   TEXT,
    finished_at  TEXT
);
CREATE INDEX idx_product_analysis_agents_run ON product_analysis_agents(analysis_id);

CREATE TABLE product_questions (
    id          TEXT PRIMARY KEY,
    story_id    TEXT NOT NULL,
    analysis_id TEXT,
    text        TEXT NOT NULL,
    rationale   TEXT NOT NULL DEFAULT '',
    category    TEXT NOT NULL DEFAULT 'other',
    status      TEXT NOT NULL DEFAULT 'open', -- 'open'|'posted'|'answered'|'discarded'
    answer      TEXT,
    posted_ref  TEXT,
    created_by  TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);
CREATE INDEX idx_product_questions_story ON product_questions(story_id);

CREATE TABLE product_notes (
    id         TEXT PRIMARY KEY,
    story_id   TEXT NOT NULL,
    section    TEXT,
    body       TEXT NOT NULL,
    author_id  TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX idx_product_notes_story ON product_notes(story_id);

CREATE TABLE product_events (
    id         TEXT PRIMARY KEY,
    story_id   TEXT NOT NULL,
    section    TEXT NOT NULL,             -- source|analysis|questions|notes|rewrite|tests|publish|inject|watch
    kind       TEXT NOT NULL,
    summary    TEXT NOT NULL,
    actor_id   TEXT,
    meta_json  TEXT,
    created_at TEXT NOT NULL
);
CREATE INDEX idx_product_events_story ON product_events(story_id, section);

CREATE TABLE product_testcase_runs (
    id                TEXT PRIMARY KEY,
    story_id          TEXT NOT NULL,
    status            TEXT NOT NULL DEFAULT 'draft', -- 'draft'|'approved'|'published'
    confluence_page_id TEXT,
    confluence_url    TEXT,
    created_by        TEXT NOT NULL,
    created_at        TEXT NOT NULL
);
CREATE INDEX idx_product_tcruns_story ON product_testcase_runs(story_id);

CREATE TABLE product_testcases (
    id          TEXT PRIMARY KEY,
    run_id      TEXT NOT NULL,
    story_id    TEXT NOT NULL,
    title       TEXT NOT NULL,
    category    TEXT NOT NULL,            -- happy|validation|error|edge
    priority    TEXT NOT NULL DEFAULT 'medium',
    steps_json  TEXT NOT NULL,           -- {preconditions[],steps[],expected}
    status      TEXT NOT NULL DEFAULT 'draft', -- draft|approved|changes_requested|rejected
    review_note TEXT,
    order_idx   INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);
CREATE INDEX idx_product_testcases_run ON product_testcases(run_id);

CREATE TABLE product_learnings (
    id            TEXT PRIMARY KEY,
    workspace_id  TEXT NOT NULL,
    kind          TEXT NOT NULL,          -- 'pattern' | 'avoid'
    title         TEXT NOT NULL,
    body          TEXT NOT NULL,
    tags          TEXT NOT NULL DEFAULT '',
    refs_json     TEXT NOT NULL DEFAULT '[]',
    source_story_id TEXT,
    active        INTEGER NOT NULL DEFAULT 1,
    created_by    TEXT NOT NULL,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);
CREATE INDEX idx_product_learnings_ws ON product_learnings(workspace_id, active);
