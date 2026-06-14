-- Add agents_json column to store per-agent live state during a review run.
ALTER TABLE pr_reviews ADD COLUMN agents_json TEXT NOT NULL DEFAULT '[]';
