-- A runtime thread whose title is still the placeholder Otto gave it (the
-- first words of its opening message): the agent's own name for the
-- conversation (Codex `thread/name/updated`, an ACP `session_info_update`,
-- Claude Code's `ai-title`) replaces it. A rename clears it.
ALTER TABLE agent_runtime_sessions ADD COLUMN title_auto INTEGER NOT NULL DEFAULT 0;
