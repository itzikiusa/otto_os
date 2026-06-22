-- API client: optionally route a saved request through an SSH tunnel so it
-- egresses from a whitelisted bastion IP. Holds the id of an `ssh`-kind row in
-- `connections`; NULL = send directly (the prior behaviour). No FK: connections
-- live in a separate table and may be global; a dangling id resolves to a
-- clear "connection not found" error at execute time rather than a delete-time
-- cascade that would silently drop the tunnel choice.
ALTER TABLE api_requests ADD COLUMN ssh_connection_id TEXT;
