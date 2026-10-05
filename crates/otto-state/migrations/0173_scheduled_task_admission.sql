-- Pending dispatch eligibility is separate from the occurrence generation used
-- to settle already-running tasks. Pause/resume and each admission invalidate
-- captured scheduler scans without rearming a completed once occurrence.
ALTER TABLE scheduled_tasks ADD COLUMN admission_generation INTEGER NOT NULL DEFAULT 0;
