-- Chained 3D jobs: the transform stages (gameready | rig | animate) a job runs
-- after its first stage, as a JSON array (`StartMedia3dJobReq.then_stages`).
-- Additive with a DEFAULT: previous builds never read the column.
ALTER TABLE media_3d_jobs ADD COLUMN then_stages_json TEXT NOT NULL DEFAULT '[]';
