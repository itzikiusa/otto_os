-- Swarm turn identity follows the latest prompt editor one level up
-- (access-control-6-02, after 0195's task-level editor). Project text (name,
-- description, goal, project goals) and swarm text (description, standing
-- goals with their verify_cmd) reach every turn of the project / swarm, so an
-- Editor's rewrite there must run as that Editor, not as each task's author.
--   * swarm_projects.prompt_updated_by / _at — the last person who changed the
--     project's name / description / goal or one of its project goals.
--   * swarms.prompt_updated_by / _at — the last person who changed the swarm's
--     description or replaced / edited its standing goals.
--   * swarm_tasks.prompt_updated_at — when 0195's prompt_updated_by was set, so
--     `acting_user` can pick the most RECENT editor across task, project and
--     swarm.
-- All nullable with no default: a row the previous build writes (or an
-- untouched one) reads NULL = "nobody rewrote it", so a rolled-back binary that
-- never names the columns keeps working.
ALTER TABLE swarm_projects ADD COLUMN prompt_updated_by TEXT;
ALTER TABLE swarm_projects ADD COLUMN prompt_updated_at TEXT;
ALTER TABLE swarms ADD COLUMN prompt_updated_by TEXT;
ALTER TABLE swarms ADD COLUMN prompt_updated_at TEXT;
ALTER TABLE swarm_tasks ADD COLUMN prompt_updated_at TEXT;
