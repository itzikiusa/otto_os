-- MCP profiles: named variants of the Otto MCP connection, each exposing only
-- a subset of the tools. A profile is a set of tool GROUPS (the membership
-- table lives in `otto_mcp::profiles`, keyed by tool name) plus per-tool
-- includes / excludes, or `all_tools` for "every tool". An agent session
-- stores the profile ids it runs under (`sessions.meta.mcp_profiles`, the
-- union of those profiles); an external connection names one in its
-- stdio args or HTTP URL. A profile only NARROWS the listed tools — RBAC,
-- enable list, policy and approvals still run exactly as before.
--
-- Additive: a new table only (rollback-safe — an older build never reads it).
-- Exactly ONE row is the default (the partial unique index below + the
-- repo's transactional switch); `full` is the locked built-in that can be
-- neither edited nor deleted and is the initial default, so a session or
-- connection with no profile behaves exactly as before.
CREATE TABLE IF NOT EXISTS mcp_profiles (
    id                 TEXT PRIMARY KEY,
    name               TEXT NOT NULL,
    description        TEXT NOT NULL DEFAULT '',
    all_tools          INTEGER NOT NULL DEFAULT 0,
    groups_json        TEXT NOT NULL DEFAULT '[]',   -- group ids
    include_tools_json TEXT NOT NULL DEFAULT '[]',   -- canonical tool names
    exclude_tools_json TEXT NOT NULL DEFAULT '[]',   -- canonical tool names
    builtin            INTEGER NOT NULL DEFAULT 0,   -- seeded preset
    locked             INTEGER NOT NULL DEFAULT 0,   -- read-only + undeletable (Full)
    is_default         INTEGER NOT NULL DEFAULT 0,
    created_by         TEXT,
    created_at         TEXT NOT NULL,
    updated_at         TEXT NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_mcp_profiles_one_default
    ON mcp_profiles(is_default) WHERE is_default = 1;
CREATE UNIQUE INDEX IF NOT EXISTS idx_mcp_profiles_name
    ON mcp_profiles(name COLLATE NOCASE);

-- The built-in presets (fixed ids; `otto_mcp::profiles::PRESETS` mirrors them).
INSERT OR IGNORE INTO mcp_profiles
    (id, name, description, all_tools, groups_json, builtin, locked, is_default, created_at, updated_at)
VALUES
    ('full', 'Full', 'Every Otto tool — the connection as it always was.',
        1, '[]', 1, 1, 1, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ('database', 'Database', 'Database connections: schema, objects and read-only queries.',
        0, '["database"]', 1, 0, 0, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ('issues', 'Issues', 'Jira issues and Confluence pages.',
        0, '["issues","confluence"]', 1, 0, 0, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ('git', 'Git & PRs', 'Repositories, pull requests and code review.',
        0, '["git","code_review"]', 1, 0, 0, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ('cloud', 'Cloud', 'AWS and Kubernetes consoles.',
        0, '["aws","kubernetes"]', 1, 0, 0, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), strftime('%Y-%m-%dT%H:%M:%SZ', 'now'));
