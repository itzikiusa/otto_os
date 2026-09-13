-- This is a derived index policy flag; original Vault files are untouched.
ALTER TABLE vault_notes ADD COLUMN content_index_status TEXT NOT NULL DEFAULT 'full'
    CHECK(content_index_status IN ('full', 'size_limited'));
