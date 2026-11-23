-- Smart mode: the reasoning effort a production plans with (picked on
-- Create beside its agent and model; NULL = the model's default). A nullable
-- column only: the previous build never reads it (its INSERT OR REPLACE
-- writes NULL, which reads as the default).
ALTER TABLE media_productions ADD COLUMN effort TEXT;
