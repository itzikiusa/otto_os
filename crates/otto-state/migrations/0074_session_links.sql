-- Link a managed Otto session to a discovery chat and to a canvas scene, so the
-- agent runs as a real, visible, resumable session (the same infra Agents/Swarm
-- use) instead of a throwaway per-turn PTY. NULL until the first agent turn
-- creates the session.
ALTER TABLE product_discovery_chats ADD COLUMN session_id TEXT;
ALTER TABLE canvas_scenes ADD COLUMN session_id TEXT;
