-- Smart mode: the agent and model a production plans with (picked on
-- Create; NULL = the workspace default agent and the provider's default
-- model), kept so a steer runs with the same pick. Nullable columns only: the
-- previous build never reads them (its INSERT OR REPLACE writes NULL, which
-- reads as the default).
ALTER TABLE media_productions ADD COLUMN provider TEXT;
ALTER TABLE media_productions ADD COLUMN model TEXT;
