-- Conversation view (design docs/design/conversation-view.md §4.2): the resolved
-- on-disk transcript of an agent session. Claude's path is derivable from
-- (cwd, provider_session_id); Codex rollouts are only found by scanning
-- ~/.codex/sessions, so the path is persisted the moment a capture resolves it
-- and later lookups are O(1). NULL = never resolved (or not applicable).
ALTER TABLE sessions ADD COLUMN transcript_path TEXT;
