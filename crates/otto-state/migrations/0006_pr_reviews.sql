CREATE TABLE pr_reviews (
    id         TEXT PRIMARY KEY,
    repo_id    TEXT NOT NULL,
    pr_number  INTEGER NOT NULL,
    status     TEXT NOT NULL DEFAULT 'running',
    error      TEXT,
    created_at TEXT NOT NULL
);
CREATE INDEX idx_pr_reviews_repo_pr ON pr_reviews(repo_id, pr_number);
CREATE TABLE pr_review_comments (
    id         TEXT PRIMARY KEY,
    review_id  TEXT NOT NULL REFERENCES pr_reviews(id) ON DELETE CASCADE,
    path       TEXT,
    line       INTEGER,
    severity   TEXT NOT NULL DEFAULT 'info',
    body       TEXT NOT NULL,
    state      TEXT NOT NULL DEFAULT 'draft',
    posted     INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL
);
CREATE INDEX idx_pr_review_comments_review ON pr_review_comments(review_id);
