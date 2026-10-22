-- Swarm turn identity follows the latest prompt editor (access-control-5-01).
-- A turn's session is created as the task's human originator (PC-2); an
-- Editor who rewrote another person's task (title, description, labels) or
-- its goals made that person's identity run the Editor's text. The runtime now
-- prefers the latest person who changed what the turn executes.
--   * swarm_tasks.prompt_updated_by — the last person who changed the task's
--     title / description / labels or added/edited one of its goals.
--   * swarm_goals.updated_by — the last person who edited the goal
--     (`created_by` already records its author).
-- Both are nullable with no default: a row the previous build writes (or an
-- untouched one) reads NULL = "nobody rewrote it", so a rolled-back binary that
-- never names the columns keeps working, and the new one falls back to the
-- originator walk exactly as before.
ALTER TABLE swarm_tasks ADD COLUMN prompt_updated_by TEXT;
ALTER TABLE swarm_goals ADD COLUMN updated_by TEXT;
