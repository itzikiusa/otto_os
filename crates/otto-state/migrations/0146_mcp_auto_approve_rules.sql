-- MCP auto-approve rules: an explicit, opt-in policy that lets a MUTATING
-- `otto.*` tool call run WITHOUT a per-call human approval.
--
-- A rule names WHAT it covers (one tool, or every mutating tool of a catalog
-- category) and WHERE it applies (global / one workspace / one agent session).
-- No rule ⇒ the call is approval-gated exactly as before (off by default).
-- Every auto-approved call is still audited in `mcp_call_log` with
-- decision='auto_approved' and decision_reason naming the rule.
--
-- Guardrail: a tool the daemon classifies as IRREVERSIBLE (merge a PR, kubectl
-- ops, produce to a live queue/topic, send an arbitrary HTTP request, hard
-- deletes) is never covered by a category rule; a per-tool rule covers one only
-- with `allow_irreversible = 1` (the second explicit toggle). Enforced by the
-- daemon on write and on every call.
CREATE TABLE IF NOT EXISTS mcp_auto_approve_rules (
    id                 TEXT PRIMARY KEY,
    name               TEXT NOT NULL,
    enabled            INTEGER NOT NULL DEFAULT 1,
    scope              TEXT NOT NULL,     -- global | workspace | session
    workspace_id       TEXT REFERENCES workspaces(id) ON DELETE CASCADE, -- scope=workspace
    session_id         TEXT REFERENCES sessions(id) ON DELETE CASCADE,   -- scope=session
    target_kind        TEXT NOT NULL,     -- tool | category
    target             TEXT NOT NULL,     -- bare tool name (no `otto.`) | category label
    allow_irreversible INTEGER NOT NULL DEFAULT 0,
    note               TEXT,
    created_by         TEXT NOT NULL REFERENCES users(id),
    created_at         TEXT NOT NULL,
    updated_at         TEXT NOT NULL
);
-- One rule per (scope, place, target): a duplicate is a 409, not a second row.
CREATE UNIQUE INDEX IF NOT EXISTS idx_mcp_auto_approve_unique ON mcp_auto_approve_rules(
    scope, IFNULL(workspace_id, ''), IFNULL(session_id, ''), target_kind, target
);

-- Carry over the per-tool "Ask before each call" switch: each tool in the
-- legacy `mcp_approval_exempt_tools` setting becomes a GLOBAL per-tool rule.
-- That switch was an explicit per-tool decision, so the imported rule keeps
-- covering the tool even when it is irreversible (allow_irreversible = 1) —
-- behaviour is unchanged. The setting row is left in place (no longer read).
INSERT OR IGNORE INTO mcp_auto_approve_rules (
    id, name, enabled, scope, workspace_id, session_id, target_kind, target,
    allow_irreversible, note, created_by, created_at, updated_at
)
SELECT
    lower(hex(randomblob(16))),
    'otto.' || t.bare,
    1, 'global', NULL, NULL, 'tool', t.bare, 1,
    'Imported from the per-tool "Ask before each call" switch',
    u.id,
    strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
    strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
FROM (
    SELECT DISTINCT CASE WHEN trim(j.value) LIKE 'otto.%' THEN substr(trim(j.value), 6)
                         ELSE trim(j.value) END AS bare
    FROM settings s, json_each(s.value_json) j
    WHERE s.key = 'mcp_approval_exempt_tools'
      AND json_valid(s.value_json)
      AND json_type(s.value_json) = 'array'
      AND j.type = 'text'
) t,
(SELECT id FROM users ORDER BY is_root DESC, created_at LIMIT 1) u
WHERE t.bare <> '';
