-- Persist admission/settlement epochs while remaining compatible with older builds.
ALTER TABLE personal_agents ADD COLUMN admission_generation INTEGER NOT NULL DEFAULT 0;
ALTER TABLE personal_agent_schedules ADD COLUMN schedule_generation INTEGER NOT NULL DEFAULT 0;
ALTER TABLE personal_agent_schedules ADD COLUMN admission_generation INTEGER NOT NULL DEFAULT 0;

-- Include all user configuration paths (including old binaries and rearm helpers).
CREATE TRIGGER personal_agent_config_generation AFTER UPDATE OF name, avatar, soul_md, provider, model, cwd, browser, delivery_json, enabled ON personal_agents
BEGIN
  UPDATE personal_agents SET admission_generation = admission_generation + 1 WHERE id = NEW.id;
END;
CREATE TRIGGER personal_agent_schedule_generation AFTER UPDATE OF schedule_json, timezone, directive, enabled, armed_at, permission ON personal_agent_schedules
BEGIN
  UPDATE personal_agent_schedules SET schedule_generation = schedule_generation + 1,
    admission_generation = admission_generation + 1 WHERE id = NEW.id;
END;
CREATE TRIGGER personal_agent_autonomy_insert_generation AFTER INSERT ON personal_agent_autonomy
BEGIN
  UPDATE personal_agents SET admission_generation = admission_generation + 1 WHERE id = NEW.agent_id;
END;
CREATE TRIGGER personal_agent_autonomy_update_generation AFTER UPDATE OF config_json ON personal_agent_autonomy
BEGIN
  UPDATE personal_agents SET admission_generation = admission_generation + 1 WHERE id = NEW.agent_id;
END;
CREATE TRIGGER personal_agent_autonomy_delete_generation AFTER DELETE ON personal_agent_autonomy
BEGIN
  UPDATE personal_agents SET admission_generation = admission_generation + 1 WHERE id = OLD.agent_id;
END;
