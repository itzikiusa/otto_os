-- Feature: Product epic tree — one level of story nesting + folders.
-- Conventions: TEXT ULID ids, RFC3339 TEXT timestamps.
--
-- `tree_kind` is Otto's TREE ROLE ('story' | 'epic' | 'doc'), deliberately
-- distinct from `source_kind` (jira/confluence/draft), Jira's `issue_type`
-- (which may itself say "Epic") and `product_story_versions.kind`. A row is
-- shown as an epic when `tree_kind = 'epic'` OR it has children, so a Jira story
-- linked to a swarm project roots its tree without Otto rewriting the row.
--
-- One level only: a child (parent_id NOT NULL) never has children — the server
-- rejects a `parent_id` that points at a row which itself has a parent. `folder`
-- ('' | 'Design' | 'PO' …) gives the visual sub-hierarchy inside an epic without
-- recursive queries. Deleting a parent re-parents its children to top level.
ALTER TABLE product_stories ADD COLUMN parent_id TEXT;                          -- NULL = top level
ALTER TABLE product_stories ADD COLUMN tree_kind TEXT NOT NULL DEFAULT 'story'; -- 'story' | 'epic' | 'doc'
ALTER TABLE product_stories ADD COLUMN folder    TEXT NOT NULL DEFAULT '';      -- '' | 'Design' | 'PO' …
CREATE INDEX idx_product_stories_parent ON product_stories(parent_id);
