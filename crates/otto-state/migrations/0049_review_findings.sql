-- Persistent finding identity for the S2 verified-review loop.
-- A "finding" is a deduplicated, fingerprinted review comment tracked across
-- multiple review runs (open → fixing → resolved → regressed → declined).
-- fingerprint: sha256(normalized_path || ":" || normalized_body_prefix)
-- stored as a 64-hex-char string.
CREATE TABLE review_findings (
    id                   TEXT PRIMARY KEY,
    workspace_id         TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    repo_id              TEXT NOT NULL REFERENCES git_repos(id) ON DELETE CASCADE,
    pr_number            INTEGER NOT NULL,
    -- Stable fingerprint across runs (path + normalized body).
    fingerprint          TEXT NOT NULL,
    -- Latest known status: "open" | "fixing" | "resolved" | "regressed" | "declined"
    status               TEXT NOT NULL DEFAULT 'open',
    severity             TEXT NOT NULL DEFAULT 'info',
    path                 TEXT,
    line                 INTEGER,
    -- Canonical body text (from the first run that saw this finding).
    body                 TEXT NOT NULL,
    -- First and most-recent review run where this fingerprint appeared.
    first_seen_review_id TEXT NOT NULL REFERENCES reviews(id) ON DELETE CASCADE,
    last_seen_review_id  TEXT NOT NULL REFERENCES reviews(id) ON DELETE CASCADE,
    -- Number of consecutive runs where this appeared (confidence indicator).
    occurrence_count     INTEGER NOT NULL DEFAULT 1,
    created_at           TEXT NOT NULL,
    updated_at           TEXT NOT NULL
);

CREATE INDEX idx_review_findings_repo_pr ON review_findings(repo_id, pr_number);
CREATE INDEX idx_review_findings_fp      ON review_findings(fingerprint);
CREATE INDEX idx_review_findings_status  ON review_findings(status);

-- Per-(repo_id, pr_number) merge-readiness summary view used by the UI gate.
CREATE VIEW review_merge_readiness AS
SELECT
    repo_id,
    pr_number,
    COUNT(*) FILTER (WHERE status IN ('open', 'regressed') AND severity = 'bug')
        AS unresolved_blocker_count,
    COUNT(*) FILTER (WHERE status IN ('open', 'regressed'))
        AS unresolved_total,
    COUNT(*) FILTER (WHERE status = 'resolved')
        AS resolved_count,
    MAX(updated_at) AS last_updated
FROM review_findings
GROUP BY repo_id, pr_number;
