-- Review S5-02: a blank channel allow-list used to admit EVERY sender. It now
-- admits nobody unless the integration carries the explicit `open_to_all`
-- opt-in. Integrations that predate the flag with a blank list keep their
-- behaviour (they were configured under "blank = everyone"): the migration
-- sets the flag on them, and the daemon warns about every open bot on start.
ALTER TABLE workspace_integrations ADD COLUMN open_to_all INTEGER NOT NULL DEFAULT 0;
UPDATE workspace_integrations
   SET open_to_all = 1
 WHERE trim(replace(allowed_users, ',', '')) = '';
