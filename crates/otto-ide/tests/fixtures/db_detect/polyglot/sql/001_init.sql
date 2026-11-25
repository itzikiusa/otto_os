-- Seed data for the wallet service. Rows from the old system are imported below.
/* DELETE FROM never_run_table; */
CREATE TABLE IF NOT EXISTS currencies (code CHAR(3) PRIMARY KEY, note TEXT);
INSERT INTO currencies (code, note) VALUES ('EUR', 'imported from legacy'), ('USD', NULL);
UPDATE currencies SET code = 'GBP' WHERE code = 'UKP';
