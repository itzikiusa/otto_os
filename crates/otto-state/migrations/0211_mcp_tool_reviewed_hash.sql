-- MCP tool review baseline (additive, nullable: the previous build ignores it).
-- The content hash (description + input schema + annotations) a person last
-- reviewed for a discovered tool:
--   NULL  a row from before this column; baselined on its next discovery.
--   ''    a tool new to an already-reviewed server: it starts disabled and
--         is reviewed when a person enables or acknowledges it.
--   hash  reviewed; a rediscovered description/schema that hashes differently
--         forces approval on every call until a person acknowledges it.
ALTER TABLE mcp_tools ADD COLUMN reviewed_hash TEXT;
