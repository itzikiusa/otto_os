-- Work the main agent hands to another personal agent
-- (`otto.personal_agent_delegate`), tracked until its result is back in the
-- main agent's chat — so a delegation is never silently dropped: a daemon
-- restart, a failed or stopped specialist, or a closed chat each end with a
-- message in the lead's chat, and the Personal Agents page shows the live
-- status of every hand-off.
--
-- `id` = the delegated (linked) thread. `status`: running | done | failed |
-- stopped | interrupted. `delivered_at` = when the outcome reached the lead's
-- chat (NULL = still owed). New table only: a rolled-back build ignores it.
CREATE TABLE IF NOT EXISTS personal_agent_delegations (
    id              TEXT PRIMARY KEY,
    workspace_id    TEXT NOT NULL,
    lead_agent_id   TEXT NOT NULL,
    lead_thread_id  TEXT NOT NULL,
    member_agent_id TEXT NOT NULL,
    member_name     TEXT NOT NULL DEFAULT '',
    task            TEXT NOT NULL DEFAULT '',
    status          TEXT NOT NULL DEFAULT 'running',
    summary         TEXT NOT NULL DEFAULT '',
    created_at      TEXT NOT NULL,
    finished_at     TEXT,
    delivered_at    TEXT
);
CREATE INDEX IF NOT EXISTS idx_pa_delegations_lead
    ON personal_agent_delegations (lead_agent_id, created_at);
CREATE INDEX IF NOT EXISTS idx_pa_delegations_member
    ON personal_agent_delegations (member_agent_id, created_at);
CREATE INDEX IF NOT EXISTS idx_pa_delegations_open
    ON personal_agent_delegations (status) WHERE delivered_at IS NULL;

-- A routine's own destination (`personal_agent_schedules.delivery_json`): a
-- routine an agent sets up from chat ("every day at 9 send me a Slack message
-- with …") delivers there instead of the agent's default. NULL = the agent's
-- delivery (every routine before this, and a rolled-back build, which ignores
-- the column).
ALTER TABLE personal_agent_schedules ADD COLUMN delivery_json TEXT;
