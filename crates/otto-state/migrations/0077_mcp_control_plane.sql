-- MCP Control Plane. Turns the thin per-workspace MCP-server *config* registry
-- (0036) into a governed control plane: Otto becomes an outbound MCP client that
-- discovers tools, health-checks servers, labels prompt-injection risk, enforces
-- per-tool permissions / per-workspace allowlists / policy-as-code, gates
-- dangerous calls behind human approval, supports dry-run, and audits every call
-- (the source of per-tool latency/error/bytes stats).
--
-- Backward compatibility: the columns below are ADDED to `mcp_servers`; the
-- legacy `.mcp.json` merge path (McpServersRepo + DbMcpServerProvider) reads
-- named columns via SELECT *, so it is unaffected. Existing rows default to
-- transport='stdio', managed=1 — i.e. they become governed stdio servers.

-- ---- mcp_servers: control-plane columns ------------------------------------
ALTER TABLE mcp_servers ADD COLUMN transport          TEXT NOT NULL DEFAULT 'stdio'; -- 'stdio' | 'http'
ALTER TABLE mcp_servers ADD COLUMN url                TEXT;            -- http transport endpoint
ALTER TABLE mcp_servers ADD COLUMN description        TEXT;
ALTER TABLE mcp_servers ADD COLUMN headers_json       TEXT NOT NULL DEFAULT '{}'; -- http header NAMES only (secret values live in keychain)
ALTER TABLE mcp_servers ADD COLUMN secret_ref         TEXT;            -- keychain ref 'mcp-{id}' holding {env,headers} secret values
ALTER TABLE mcp_servers ADD COLUMN secret_env_keys    TEXT NOT NULL DEFAULT '[]'; -- names of env vars stored in keychain
ALTER TABLE mcp_servers ADD COLUMN secret_header_keys TEXT NOT NULL DEFAULT '[]'; -- names of headers stored in keychain
ALTER TABLE mcp_servers ADD COLUMN injection_risk     TEXT NOT NULL DEFAULT 'medium'; -- server default label: low|medium|high
ALTER TABLE mcp_servers ADD COLUMN managed            INTEGER NOT NULL DEFAULT 1;  -- governed by the control plane
ALTER TABLE mcp_servers ADD COLUMN default_tool_access TEXT NOT NULL DEFAULT 'allow'; -- per-workspace posture: 'allow'|'deny'
ALTER TABLE mcp_servers ADD COLUMN health_status      TEXT NOT NULL DEFAULT 'unknown'; -- unknown|healthy|unhealthy|disabled
ALTER TABLE mcp_servers ADD COLUMN health_checked_at  TEXT;
ALTER TABLE mcp_servers ADD COLUMN health_latency_ms  INTEGER;
ALTER TABLE mcp_servers ADD COLUMN health_error       TEXT;
ALTER TABLE mcp_servers ADD COLUMN tools_count        INTEGER NOT NULL DEFAULT 0;
ALTER TABLE mcp_servers ADD COLUMN tools_discovered_at TEXT;

-- ---- mcp_tools: discovered catalog + per-tool governance -------------------
-- One row per tool a server advertised at the last discovery. risk_overridden=1
-- pins a human's risk/injection labels so rediscovery never lowers them.
CREATE TABLE mcp_tools (
    id                TEXT PRIMARY KEY,
    server_id         TEXT NOT NULL REFERENCES mcp_servers(id) ON DELETE CASCADE,
    name              TEXT NOT NULL,
    title             TEXT,
    description       TEXT,
    input_schema_json TEXT NOT NULL DEFAULT '{}',
    annotations_json  TEXT NOT NULL DEFAULT '{}',
    risk_label        TEXT NOT NULL DEFAULT 'unknown', -- read|write|dangerous|unknown
    injection_risk    TEXT NOT NULL DEFAULT 'medium',  -- low|medium|high
    mutating          INTEGER NOT NULL DEFAULT 0,
    supports_dry_run  INTEGER NOT NULL DEFAULT 0,
    enabled           INTEGER NOT NULL DEFAULT 1,       -- per-tool permission (global)
    require_approval  INTEGER NOT NULL DEFAULT 0,       -- per-tool approval override
    risk_overridden   INTEGER NOT NULL DEFAULT 0,
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL,
    UNIQUE (server_id, name)
);
CREATE INDEX idx_mcp_tools_server ON mcp_tools(server_id);

-- ---- mcp_allowlist: per-workspace allow/deny over a server's tools ---------
-- tool_name NULL = the whole server. A 'deny' beats an 'allow'; absence falls
-- back to the server's default_tool_access. allowlist.workspace_id is always the
-- server's owning workspace (enforced in the handler).
CREATE TABLE mcp_allowlist (
    id           TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    server_id    TEXT NOT NULL REFERENCES mcp_servers(id) ON DELETE CASCADE,
    tool_name    TEXT,                       -- NULL = whole server
    mode         TEXT NOT NULL DEFAULT 'allow', -- 'allow' | 'deny'
    created_by   TEXT NOT NULL REFERENCES users(id),
    created_at   TEXT NOT NULL,
    UNIQUE (workspace_id, server_id, tool_name)
);
CREATE INDEX idx_mcp_allowlist_ws ON mcp_allowlist(workspace_id);
CREATE INDEX idx_mcp_allowlist_server ON mcp_allowlist(server_id);

-- ---- mcp_policies: policy-as-code rules ------------------------------------
-- Evaluated most-restrictive-wins (deny > require_approval > require_dry_run >
-- allow); priority orders display only. workspace_id NULL = a global rule.
CREATE TABLE mcp_policies (
    id           TEXT PRIMARY KEY,
    workspace_id TEXT REFERENCES workspaces(id) ON DELETE CASCADE, -- NULL = global
    name         TEXT NOT NULL,
    enabled      INTEGER NOT NULL DEFAULT 1,
    priority     INTEGER NOT NULL DEFAULT 100,
    match_json   TEXT NOT NULL DEFAULT '{}',
    effect       TEXT NOT NULL,              -- allow|deny|require_approval|require_dry_run
    reason       TEXT,
    created_by   TEXT NOT NULL REFERENCES users(id),
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
);
CREATE INDEX idx_mcp_policies_ws ON mcp_policies(workspace_id, priority);

-- ---- mcp_call_log: audit of every governed call + the stats source ---------
-- Append-only. args_redacted_json + error are redacted before they land here.
-- Written on EVERY terminal path (allowed/denied/approved/dry_run/pending/error)
-- as a guaranteed step (fail-closed: no audit row => the call does not execute).
CREATE TABLE mcp_call_log (
    id                 TEXT PRIMARY KEY,
    workspace_id       TEXT,
    server_id          TEXT,
    server_name        TEXT,
    tool               TEXT NOT NULL,
    direction          TEXT NOT NULL DEFAULT 'outbound', -- outbound|inbound
    caller_user_id     TEXT,
    caller_kind        TEXT,                  -- ui|agent|agent_readonly|mcp_server|gateway
    args_redacted_json TEXT NOT NULL DEFAULT '{}',
    decision           TEXT NOT NULL,         -- allowed|denied|approved|dry_run|pending_approval|error
    decision_reason    TEXT,
    risk_label         TEXT,
    injection_risk     TEXT,
    dry_run            INTEGER NOT NULL DEFAULT 0,
    ok                 INTEGER NOT NULL DEFAULT 0,
    error              TEXT,
    latency_ms         INTEGER,
    bytes              INTEGER,
    rows               INTEGER,
    approval_id        TEXT,
    created_at         TEXT NOT NULL
);
CREATE INDEX idx_mcp_call_log_tool ON mcp_call_log(tool, created_at);
CREATE INDEX idx_mcp_call_log_ws ON mcp_call_log(workspace_id, created_at);
CREATE INDEX idx_mcp_call_log_server ON mcp_call_log(server_id, created_at);

-- ---- mcp_approvals: the approval queue -------------------------------------
-- Shared by dangerous-action gating (kind='tool_call') and otto.ask_human_approval
-- (kind='human_ask'). args_hash binds an approval to the EXACT full arguments
-- (sha256 of canonical JSON); consumed_at makes an approval single-use.
CREATE TABLE mcp_approvals (
    id                 TEXT PRIMARY KEY,
    workspace_id       TEXT,
    kind               TEXT NOT NULL,          -- tool_call | human_ask
    server_id          TEXT,
    server_name        TEXT,
    tool               TEXT,
    title              TEXT NOT NULL,
    detail             TEXT,
    args_redacted_json TEXT NOT NULL DEFAULT '{}',
    args_hash          TEXT,                   -- sha256(canonical full args); binds execution
    risk_label         TEXT,
    status             TEXT NOT NULL DEFAULT 'pending', -- pending|approved|denied|expired|cancelled|consumed
    requested_by       TEXT,
    requested_by_kind  TEXT,
    decided_by         TEXT,
    decision_note      TEXT,
    created_at         TEXT NOT NULL,
    decided_at         TEXT,
    consumed_at        TEXT,
    expires_at         TEXT
);
CREATE INDEX idx_mcp_approvals_status ON mcp_approvals(status, created_at);
CREATE INDEX idx_mcp_approvals_ws ON mcp_approvals(workspace_id, status, created_at);
