-- Connections are a global library, not tied to a workspace. Opening one
-- creates a terminal session that attaches to a workspace only for its
-- lifetime (Session.workspace_id) — the connection itself belongs to no
-- workspace. Migrate every existing connection to the global pool.
UPDATE connections SET workspace_id = NULL;

-- Sections (the folder tree) are now a single global tree shared by both the
-- Connections page and the DB Explorer; they are listed globally regardless of
-- their `workspace_id` (kept only as the creating workspace for FK integrity).
