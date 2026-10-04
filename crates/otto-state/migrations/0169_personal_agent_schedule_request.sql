ALTER TABLE personal_agent_schedules ADD COLUMN request_key TEXT;
CREATE UNIQUE INDEX personal_agent_schedule_request
ON personal_agent_schedules(agent_id, request_key);
