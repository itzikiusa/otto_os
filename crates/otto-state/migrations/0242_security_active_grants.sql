-- Scheduled active security checks: a person's explicit grant, bound to the
-- exact confirmed target, its resolved loopback addresses, the exact schedule
-- row, the template policy and the person it runs as. A separate table that
-- older builds never read, so a rollback cannot carry consent anywhere: an
-- older build that re-confirms the target or re-saves the schedule writes a
-- new confirmed_at / armed_at, which no longer matches the grant.
CREATE TABLE ide_security_active_grants (
    project_id TEXT PRIMARY KEY REFERENCES ide_named_projects(id) ON DELETE CASCADE,
    granted_by TEXT NOT NULL,
    granted_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    target_run_config_id TEXT NOT NULL,
    target_url TEXT NOT NULL,
    target_confirmed_at TEXT NOT NULL,
    addresses TEXT NOT NULL,
    schedule_cron TEXT NOT NULL,
    schedule_timezone TEXT NOT NULL,
    schedule_armed_at TEXT NOT NULL,
    template_policy TEXT NOT NULL,
    invalidated_at TEXT,
    invalid_reason TEXT
);

-- 'manual' | 'scheduled'; skipped: JSON list of {engine, reason, failed}.
ALTER TABLE ide_security_scans ADD COLUMN trigger TEXT NOT NULL DEFAULT 'manual';
ALTER TABLE ide_security_scans ADD COLUMN skipped TEXT;
