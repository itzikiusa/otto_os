-- Mission Control / work graph: a unified, traceable view of every agentic
-- activity. Items are a PROJECTION of the authoritative module rows (sessions,
-- swarm, goal loops, workflows, reviews, product stories, PRs, channel
-- triggers), upserted by the natural key (workspace_id, kind, source_id) from
-- the daemon event bus + a backfill sweep. Edges link items; events are the
-- append-only audit trail; artifacts are evidence/trace; approvals are the
-- human gate. All workspace-scoped, cascading from `workspaces`.

CREATE TABLE work_items (
    id              TEXT PRIMARY KEY,
    workspace_id    TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    -- session | swarm | goal_loop | workflow | review | product_story | pr | external_trigger
    kind            TEXT NOT NULL,
    -- natural key of the upstream row this item projects (session id, swarm id,
    -- loop id, workflow-run id, review id, story id, "repo:pr_number", trigger id)
    source_id       TEXT NOT NULL,
    title           TEXT NOT NULL,
    goal            TEXT,
    -- normalized lifecycle: pending|running|waiting|blocked|succeeded|failed|cancelled|done
    status          TEXT NOT NULL,
    -- who/what started it (user id, or "agent"/"integration"/"system")
    owner           TEXT,
    -- the actor class that started it: user|agent|system|integration
    owner_kind      TEXT NOT NULL DEFAULT 'system',
    repo_id         TEXT,
    branch          TEXT,
    cost_so_far     REAL NOT NULL DEFAULT 0,
    -- the "policy" axis: low|medium|high|critical
    risk_level      TEXT NOT NULL DEFAULT 'low',
    result_summary  TEXT,
    -- "what context was used" (prompt/soul/skills/PR/changed-files, best-effort)
    context_summary TEXT,
    -- work_item id of the initiator, when known (also mirrored as a spawned edge)
    started_by_id   TEXT,
    last_event_at   TEXT,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    UNIQUE(workspace_id, kind, source_id)
);
CREATE INDEX idx_work_items_ws        ON work_items(workspace_id, updated_at DESC);
CREATE INDEX idx_work_items_ws_kind   ON work_items(workspace_id, kind, updated_at DESC);
CREATE INDEX idx_work_items_ws_status ON work_items(workspace_id, status);

CREATE TABLE work_edges (
    id           TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    from_item_id TEXT NOT NULL REFERENCES work_items(id) ON DELETE CASCADE,
    to_item_id   TEXT NOT NULL REFERENCES work_items(id) ON DELETE CASCADE,
    -- spawned | depends_on | fixes | reviews | verifies | blocks | belongs_to
    relation     TEXT NOT NULL,
    created_at   TEXT NOT NULL,
    UNIQUE(from_item_id, to_item_id, relation)
);
CREATE INDEX idx_work_edges_from ON work_edges(from_item_id);
CREATE INDEX idx_work_edges_to   ON work_edges(to_item_id);

CREATE TABLE work_events (
    id           TEXT PRIMARY KEY,
    work_item_id TEXT NOT NULL REFERENCES work_items(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL,
    ts           TEXT NOT NULL,
    -- user | agent | system | integration
    actor        TEXT NOT NULL,
    -- created|status_changed|tool_call|context|progress|result|approval_requested|approval_decided|artifact_added|edge_added|note
    event_type   TEXT NOT NULL,
    payload_json TEXT NOT NULL DEFAULT '{}',
    created_at   TEXT NOT NULL
);
CREATE INDEX idx_work_events_item ON work_events(work_item_id, ts DESC);

CREATE TABLE work_artifacts (
    id           TEXT PRIMARY KEY,
    work_item_id TEXT NOT NULL REFERENCES work_items(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL,
    -- diff|commit|pr|test_run|report|file|link|finding|session
    kind         TEXT NOT NULL,
    title        TEXT NOT NULL,
    ref          TEXT,
    payload_json TEXT NOT NULL DEFAULT '{}',
    created_at   TEXT NOT NULL,
    UNIQUE(work_item_id, kind, ref)
);
CREATE INDEX idx_work_artifacts_item ON work_artifacts(work_item_id);

CREATE TABLE work_approvals (
    id            TEXT PRIMARY KEY,
    work_item_id  TEXT NOT NULL REFERENCES work_items(id) ON DELETE CASCADE,
    workspace_id  TEXT NOT NULL,
    -- pending | approved | rejected
    status        TEXT NOT NULL,
    reason        TEXT,
    requested_by  TEXT NOT NULL,
    requested_at  TEXT NOT NULL,
    decided_by    TEXT,
    decided_at    TEXT,
    decision_note TEXT
);
CREATE INDEX idx_work_approvals_item ON work_approvals(work_item_id);
CREATE INDEX idx_work_approvals_ws   ON work_approvals(workspace_id, status);
