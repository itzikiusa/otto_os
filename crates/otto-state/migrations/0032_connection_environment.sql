-- Connection environment labels + production read-only guardrail.
--
-- `environment` tags a profile as dev / staging / prod; `read_only` locks a
-- profile against writes regardless of environment. Both are surfaced in the UI
-- (danger styling for prod) and enforced by the DB-explorer write-gate, which
-- rejects write/DDL statements on a guarded connection unless the request
-- carries an explicit confirm flag. Existing rows default to the safe,
-- non-guarded values (dev / not read-only) so behaviour is unchanged on upgrade.
ALTER TABLE connections
    ADD COLUMN environment TEXT NOT NULL DEFAULT 'dev'
        CHECK (environment IN ('dev','staging','prod'));

ALTER TABLE connections
    ADD COLUMN read_only INTEGER NOT NULL DEFAULT 0
        CHECK (read_only IN (0,1));
