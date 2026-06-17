-- Store the full run configuration (StartSkillEvalReq) on each run so a single
-- validation can be re-run (it needs the criteria), the run can be re-launched,
-- and the UI can show what was configured.
ALTER TABLE skill_evals ADD COLUMN config_json TEXT NOT NULL DEFAULT '{}';
