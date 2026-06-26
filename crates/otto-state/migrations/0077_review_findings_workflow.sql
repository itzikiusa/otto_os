-- Review Findings Workflow: turn fingerprinted review findings into a tracked
-- security/code-quality workflow that closes the loop with evidence.
--
-- Two axes coexist on `review_findings`, with DISJOINT writers (no dual-write):
--   * `status` (this migration repurposes the dormant 0049 column) — the human
--     WORKFLOW disposition: open|accepted|false_positive|fixed|verified|waived.
--     Written ONLY by the action endpoints.
--   * `state` (0054) — the engine DETECTION lifecycle (open|fixing|resolved|
--     regressed|declined). Written ONLY by the review engine (upsert/resolve).
-- The dormant `status` column was only ever set to 'open', so no backfill is
-- needed — every existing row is already a valid new-vocab value ('open').

-- --- Enrich review_findings with the workflow fields (all 11 required + gates) ---
ALTER TABLE review_findings ADD COLUMN title                   TEXT    NOT NULL DEFAULT '';
ALTER TABLE review_findings ADD COLUMN evidence                TEXT    NOT NULL DEFAULT '';
ALTER TABLE review_findings ADD COLUMN agent_reasoning_summary TEXT    NOT NULL DEFAULT '';
ALTER TABLE review_findings ADD COLUMN suggested_fix           TEXT;
ALTER TABLE review_findings ADD COLUMN line_end                INTEGER;
ALTER TABLE review_findings ADD COLUMN linked_commit           TEXT;
ALTER TABLE review_findings ADD COLUMN linked_test             TEXT;
ALTER TABLE review_findings ADD COLUMN reviewer                TEXT    NOT NULL DEFAULT '';
ALTER TABLE review_findings ADD COLUMN produced_by_agent       TEXT;
ALTER TABLE review_findings ADD COLUMN requires_human_approval INTEGER NOT NULL DEFAULT 0;
ALTER TABLE review_findings ADD COLUMN approved_by             TEXT;
ALTER TABLE review_findings ADD COLUMN approved_at             TEXT;
ALTER TABLE review_findings ADD COLUMN approval_decision       TEXT;   -- 'approved' | 'rejected' | NULL
ALTER TABLE review_findings ADD COLUMN jira_key                TEXT;
ALTER TABLE review_findings ADD COLUMN jira_url                TEXT;
ALTER TABLE review_findings ADD COLUMN repo_rule_id            TEXT;

-- Filter findings by workflow status quickly (the board's default grouping).
CREATE INDEX IF NOT EXISTS idx_review_findings_workflow_status
    ON review_findings(workspace_id, status);

-- --- Rewrite the merge-readiness view to the new workflow `status` vocabulary ---
-- (The 0049 view filtered on the OLD lifecycle values, which the new vocab no
-- longer produces — leaving it would make the gate silently report all clean.)
DROP VIEW IF EXISTS review_merge_readiness;
CREATE VIEW review_merge_readiness AS
SELECT
    repo_id,
    pr_number,
    COUNT(*) FILTER (WHERE status IN ('open', 'accepted', 'fixed')
                       AND severity IN ('critical', 'high'))
        AS unresolved_blocker_count,
    COUNT(*) FILTER (WHERE status IN ('open', 'accepted', 'fixed'))
        AS unresolved_total,
    COUNT(*) FILTER (WHERE status IN ('verified', 'false_positive', 'waived'))
        AS resolved_count,
    MAX(updated_at) AS last_updated
FROM review_findings
GROUP BY repo_id, pr_number;

-- --- finding_events: the loop-closing audit trail (one row per action/transition) ---
CREATE TABLE finding_events (
    id           TEXT PRIMARY KEY,
    finding_id   TEXT NOT NULL REFERENCES review_findings(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL,
    -- created|status_changed|accepted|fix_requested|fix_applied|verify_started|
    -- verified|verify_failed|false_positive|waived|approval_required|approved|
    -- rejected|jira_created|repo_rule_added|regression_test_added|regressed|comment
    kind         TEXT NOT NULL,
    actor        TEXT NOT NULL,            -- user id or agent name
    from_status  TEXT,
    to_status    TEXT,
    detail_json  TEXT,                     -- {session_id?,commit?,test?,jira_key?,evidence?,note?}
    created_at   TEXT NOT NULL
);
CREATE INDEX idx_finding_events_finding ON finding_events(finding_id, created_at);

-- --- repo_rules: durable lessons fed into the Context Engine (materialized into
-- future agent sessions' instruction files). DB is the single source of truth. ---
CREATE TABLE repo_rules (
    id                TEXT PRIMARY KEY,
    workspace_id      TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    title             TEXT NOT NULL,
    body              TEXT NOT NULL,       -- the rule text injected into CLAUDE.md/AGENTS.md
    category          TEXT,
    severity          TEXT,
    glob              TEXT,                -- optional path scope hint
    source_finding_id TEXT,
    enabled           INTEGER NOT NULL DEFAULT 1,
    created_by        TEXT NOT NULL,
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL
);
CREATE INDEX idx_repo_rules_ws ON repo_rules(workspace_id, enabled);

-- --- review_proof_packs: persisted evidence-bundle snapshots (export/share/audit).
-- (Namespaced `review_` to avoid colliding with a parallel `feat/proof-packs`
-- branch's generic ProofPack.) review_id stored as plain TEXT (no FK: SQLite FK
-- enforcement is off here and the reviews table name is ambiguous across migrations). ---
CREATE TABLE review_proof_packs (
    id           TEXT PRIMARY KEY,
    review_id    TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    format       TEXT NOT NULL,           -- 'markdown' | 'json'
    content      TEXT NOT NULL,
    summary_json TEXT NOT NULL,           -- counts by status/severity at export time
    created_by   TEXT NOT NULL,
    created_at   TEXT NOT NULL
);
CREATE INDEX idx_review_proof_packs_review ON review_proof_packs(review_id, created_at);
