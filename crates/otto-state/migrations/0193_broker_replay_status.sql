-- Broker replay lifecycle (data-integrity-4-05 / PC-5). A replay row is now
-- inserted BEFORE the first send as 'running', its evidence is updated as
-- messages are acknowledged, and it ends 'done' or 'failed'. A row still
-- 'running' at the next daemon start is marked 'interrupted'. Rows written by
-- the previous build (after the whole loop) read as 'done'; the column has a
-- DEFAULT, so a rolled-back binary that never names it keeps inserting.
ALTER TABLE broker_replays ADD COLUMN status TEXT NOT NULL DEFAULT 'done';
