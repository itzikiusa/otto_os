-- The workspace's main agent used to be seeded PAUSED, yet its chat worked
-- and set up routines on it ("every weekday at 9 …"), which then never
-- fired: the scheduler only runs routines of active agents. Resume a paused
-- main agent that has an active routine — setting one up was the user
-- asking it to run. Data only (no schema change); an older build reads the
-- same columns.
UPDATE personal_agents
SET enabled = 1
WHERE enabled = 0
  AND id IN (
      SELECT agent_id FROM personal_agent_autonomy
      WHERE json_extract(config_json, '$.primary') = 1
  )
  AND EXISTS (
      SELECT 1 FROM personal_agent_schedules s
      WHERE s.agent_id = personal_agents.id AND s.enabled = 1
  );

-- The seeded main agent also had no owner, so its runs took the headless
-- fallback instead of a session in its chat. A main agent with an active
-- routine is adopted by this Mac's root user (only when there is exactly
-- one) — the person who set the routine up from its chat.
UPDATE personal_agents
SET created_by = (SELECT id FROM users WHERE is_root = 1 AND disabled = 0)
WHERE (created_by IS NULL OR created_by = '')
  AND (SELECT COUNT(*) FROM users WHERE is_root = 1 AND disabled = 0) = 1
  AND id IN (
      SELECT agent_id FROM personal_agent_autonomy
      WHERE json_extract(config_json, '$.primary') = 1
  )
  AND EXISTS (
      SELECT 1 FROM personal_agent_schedules s
      WHERE s.agent_id = personal_agents.id AND s.enabled = 1
  );
