-- Product analysis agents are now run as real, openable SessionManager sessions
-- (one per lens×provider, plus a summarizer), mirroring PR review. Record each
-- agent's session id so the UI can Open the live terminal like a review agent.
ALTER TABLE product_analysis_agents ADD COLUMN session_id TEXT;
