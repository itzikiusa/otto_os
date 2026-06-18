-- Track how many times an analysis agent has been auto-resumed after a daemon
-- restart, so the orphan reaper can cap resumes and eventually give up + notify
-- instead of resuming forever.
ALTER TABLE product_analysis_agents ADD COLUMN resume_count INTEGER NOT NULL DEFAULT 0;
