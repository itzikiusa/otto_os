-- The reasoning effort a runtime thread runs at, like its `model`: set by the
-- effort a person (or an opening agent) picked at create or with a prompt the
-- agent accepted, re-applied on every respawn (idle suspend, daemon restart,
-- a switch restart, fresh context). NULL = the provider's default. Nullable
-- with no default, so the previous build (which never names it) still runs.
ALTER TABLE agent_runtime_sessions ADD COLUMN effort TEXT;
