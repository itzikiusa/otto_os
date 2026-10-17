-- The schedule generation fences a run's settlement (`settle_captured_schedule`
-- only advances the cursor of the generation the run captured). The 0182
-- trigger bumped it on ANY write naming its columns — and SQLite fires
-- `UPDATE OF` triggers whenever a listed column is in the SET clause, even
-- unchanged — so a directive-only edit (or a form re-sending the same
-- cadence) during a run left that run's occurrence unsettled: it ran, and
-- delivered, a second time on the next tick. Bump the schedule generation only
-- when timing really changes; content-only edits still fence stale captured
-- scans through the admission generation. Triggers only: an older build keeps
-- working on this schema.
DROP TRIGGER IF EXISTS personal_agent_schedule_generation;

CREATE TRIGGER personal_agent_schedule_generation
AFTER UPDATE OF schedule_json, timezone, armed_at, enabled ON personal_agent_schedules
WHEN OLD.schedule_json IS NOT NEW.schedule_json
  OR OLD.timezone IS NOT NEW.timezone
  OR OLD.armed_at IS NOT NEW.armed_at
  OR OLD.enabled IS NOT NEW.enabled
BEGIN
  UPDATE personal_agent_schedules SET schedule_generation = schedule_generation + 1,
    admission_generation = admission_generation + 1 WHERE id = NEW.id;
END;

CREATE TRIGGER personal_agent_schedule_content_generation
AFTER UPDATE OF directive, permission ON personal_agent_schedules
WHEN OLD.directive IS NOT NEW.directive OR OLD.permission IS NOT NEW.permission
BEGIN
  UPDATE personal_agent_schedules SET admission_generation = admission_generation + 1
    WHERE id = NEW.id;
END;
