-- Per-channel preferred agent CLI. Empty string means "use the default agent"
-- (resolved at reply time: channel pick -> global default_provider -> "claude").
ALTER TABLE workspace_integrations ADD COLUMN preferred_cli TEXT NOT NULL DEFAULT '';
