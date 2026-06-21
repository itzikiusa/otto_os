-- Audit ledger for first-party Otto MCP tool calls (Task B2b). Every invocation
-- of an `otto_*` tool exposed to an agent session via `.mcp.json` writes one row
-- here: which workspace/session asked, which tool, the (redacted) arguments, and
-- whether it succeeded plus how many rows it returned. Append-only; written
-- best-effort by the `ottod mcp-tools` subprocess (a failed audit must never fail
-- the tool call). `args_json` is already redacted via `otto_core::redact` before
-- it lands here, so no raw secret is persisted.
CREATE TABLE mcp_tool_calls (
    id           TEXT PRIMARY KEY,
    workspace_id TEXT,             -- nullable: the session's workspace, when known
    session_id   TEXT,             -- nullable: the agent session that called the tool
    tool         TEXT NOT NULL,    -- e.g. 'otto_db_schema', 'otto_git_pr_review'
    args_json    TEXT NOT NULL,    -- redacted JSON of the call arguments
    ok           INTEGER NOT NULL, -- 1 = success, 0 = error/denied/capped-out
    rows         INTEGER,          -- row/item count returned (nullable; tool-specific)
    created_at   TEXT NOT NULL
);
CREATE INDEX idx_mcp_tool_calls_session ON mcp_tool_calls(session_id, created_at);
CREATE INDEX idx_mcp_tool_calls_tool ON mcp_tool_calls(tool, created_at);
