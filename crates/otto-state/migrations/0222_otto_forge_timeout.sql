-- The built-in Otto Forge provider was seeded with the generic 900 s timeout,
-- but a local `forge generate` (Pixal3D, finish and game-ready, behind the
-- shared heavy lock) takes up to ~26 min. Raise a never-edited seed to the
-- 3,600 s cap; a provider the user configured keeps its value.
UPDATE media_3d_providers
   SET timeout_secs = 3600
 WHERE id = 'otto-forge' AND timeout_secs = 900 AND updated_at = created_at;
