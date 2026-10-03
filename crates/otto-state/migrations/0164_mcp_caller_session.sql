-- Which agent session made a governed `otto.*` call, and which one asked for
-- an approval. The audit row already carried the caller's user id; with the
-- session the audit list and stats can be split per agent, and an approval
-- card can name (and link) the session that is waiting on it instead of a raw
-- user id. Both nullable: pre-existing rows, external MCP clients and human
-- callers have no session.
ALTER TABLE mcp_call_log ADD COLUMN caller_session_id TEXT;
ALTER TABLE mcp_approvals ADD COLUMN requested_by_session_id TEXT;
