-- Review Findings Workflow: turn fingerprinted review findings into a tracked
-- security/code-quality workflow that closes the loop with evidence.
--
-- This migration also FIXES a latent bug: migrations 0049/0054 declared
-- `review_findings` with foreign keys to `reviews(id)` AND `git_repos(id)` — but
-- those tables don't exist (the real names are `pr_reviews` and `repos`). With
-- SQLite foreign-key enforcement ON (the sqlx default) EVERY insert into
-- `review_findings` fails ("no such table"), and even ALTER/RENAME of the table
-- re-validates the bad FKs and errors. The bug was dormant only because the upsert
-- had no call sites until this feature. We rebuild the table (empty in practice)
-- with the FKs corrected to the REAL tables + the workflow columns inline.
--
-- To avoid re-validating the OLD table's bad FKs, we build a NEW table (FKs to
-- real tables only), copy, DROP the old one, then RENAME the new one into place.
--
-- Two axes coexist on the rebuilt table, with DISJOINT writers (no dual-write):
--   * `status` — the human WORKFLOW disposition (open|accepted|false_positive|
--     fixed|verified|waived). Written ONLY by the action endpoints.
--   * `state`  — the engine DETECTION lifecycle (open|fixing|resolved|regressed|
--     declined). Written ONLY by the review engine (upsert/resolve).

-- Drop the view first so the table swap below doesn't rewrite it.
DROP VIEW IF EXISTS review_merge_readiness;

-- 1. Build the corrected table under a temp name (FKs → real tables only).
CREATE TABLE review_findings_v2 (
    id                   TEXT PRIMARY KEY,
    workspace_id         TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    repo_id              TEXT NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
    pr_number            INTEGER NOT NULL,
    fingerprint          TEXT NOT NULL,
    -- WORKFLOW disposition (the required 6 values).
    status               TEXT NOT NULL DEFAULT 'open',
    severity             TEXT NOT NULL DEFAULT 'info',
    path                 TEXT,
    line                 INTEGER,
    body                 TEXT NOT NULL DEFAULT '',
    -- review FKs corrected: pr_reviews, not the non-existent `reviews`.
    first_seen_review_id TEXT REFERENCES pr_reviews(id) ON DELETE CASCADE,
    last_seen_review_id  TEXT REFERENCES pr_reviews(id) ON DELETE CASCADE,
    occurrence_count     INTEGER NOT NULL DEFAULT 1,
    created_at           TEXT NOT NULL,
    updated_at           TEXT NOT NULL,
    -- engine DETECTION lifecycle (0054).
    state                TEXT NOT NULL DEFAULT 'open',
    fix_session_id       TEXT,
    fix_commit           TEXT,
    first_seen_run       TEXT,
    last_seen_run        TEXT,
    category             TEXT,
    review_id            TEXT REFERENCES pr_reviews(id) ON DELETE CASCADE,
    -- workflow fields (the 11 required + gates/artifacts).
    title                    TEXT    NOT NULL DEFAULT '',
    evidence                 TEXT    NOT NULL DEFAULT '',
    agent_reasoning_summary  TEXT    NOT NULL DEFAULT '',
    suggested_fix            TEXT,
    line_end                 INTEGER,
    linked_commit            TEXT,
    linked_test              TEXT,
    reviewer                 TEXT    NOT NULL DEFAULT '',
    produced_by_agent        TEXT,
    requires_human_approval  INTEGER NOT NULL DEFAULT 0,
    approved_by              TEXT,
    approved_at              TEXT,
    approval_decision        TEXT,
    jira_key                 TEXT,
    jira_url                 TEXT,
    repo_rule_id             TEXT
);

-- Copy any pre-existing rows (none in practice — the upsert was never called).
INSERT INTO review_findings_v2
    (id, workspace_id, repo_id, pr_number, fingerprint, status, severity, path, line, body,
     first_seen_review_id, last_seen_review_id, occurrence_count, created_at, updated_at,
     state, fix_session_id, fix_commit, first_seen_run, last_seen_run, category, review_id)
SELECT
    id, workspace_id, repo_id, pr_number, fingerprint, status, severity, path, line, body,
    first_seen_review_id, last_seen_review_id, occurrence_count, created_at, updated_at,
    state, fix_session_id, fix_commit, first_seen_run, last_seen_run, category, review_id
FROM review_findings;

DROP TABLE review_findings;
ALTER TABLE review_findings_v2 RENAME TO review_findings;

-- Recreate indexes (0049/0054 set + the new workflow-status index).
CREATE INDEX idx_review_findings_repo_pr  ON review_findings(repo_id, pr_number);
CREATE INDEX idx_review_findings_fp       ON review_findings(fingerprint);
CREATE INDEX idx_review_findings_status   ON review_findings(status);
CREATE INDEX idx_review_findings_review_fp ON review_findings(review_id, fingerprint);
CREATE INDEX idx_review_findings_workflow_status ON review_findings(workspace_id, status);

-- 2. Merge-readiness view keyed on the new workflow `status` vocabulary
--    (unresolved = open|accepted|fixed; a blocker is an unresolved critical/high).
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

-- 3. finding_events: the loop-closing audit trail (one row per action/transition).
CREATE TABLE finding_events (
    id           TEXT PRIMARY KEY,
    finding_id   TEXT NOT NULL REFERENCES review_findings(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL,
    kind         TEXT NOT NULL,
    actor        TEXT NOT NULL,
    from_status  TEXT,
    to_status    TEXT,
    detail_json  TEXT,
    created_at   TEXT NOT NULL
);
CREATE INDEX idx_finding_events_finding ON finding_events(finding_id, created_at);

-- 4. repo_rules: durable lessons fed into the Context Engine.
CREATE TABLE repo_rules (
    id                TEXT PRIMARY KEY,
    workspace_id      TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    title             TEXT NOT NULL,
    body              TEXT NOT NULL,
    category          TEXT,
    severity          TEXT,
    glob              TEXT,
    source_finding_id TEXT,
    enabled           INTEGER NOT NULL DEFAULT 1,
    created_by        TEXT NOT NULL,
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL
);
CREATE INDEX idx_repo_rules_ws ON repo_rules(workspace_id, enabled);

-- 5. review_proof_packs: persisted evidence-bundle snapshots (namespaced to avoid
--    a parallel feat/proof-packs branch's generic ProofPack).
CREATE TABLE review_proof_packs (
    id           TEXT PRIMARY KEY,
    review_id    TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    format       TEXT NOT NULL,
    content      TEXT NOT NULL,
    summary_json TEXT NOT NULL,
    created_by   TEXT NOT NULL,
    created_at   TEXT NOT NULL
);
CREATE INDEX idx_review_proof_packs_review ON review_proof_packs(review_id, created_at);
