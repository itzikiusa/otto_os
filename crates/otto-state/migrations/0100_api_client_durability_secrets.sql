-- API client durability & secrets (see
-- docs/superpowers/specs/2026-07-04-api-client-durability-secrets-design.md).
--
-- `extras_json` persists the previously draft-only request fields as one
-- versioned extension object the UI owns:
--   { v, transport, graphql_variables, docs_md, scripts:{pre,post},
--     settings:{timeout_ms,follow_redirects,tls_verify} }
-- NULL = none of these were ever set (older rows stay valid untouched).
ALTER TABLE api_requests ADD COLUMN extras_json TEXT;

-- Names of environment variables whose VALUES live in the macOS Keychain
-- (ref `otto.api.env.<env_id>`), not in `variables_json`. JSON array of
-- strings; NULL/absent = no secret variables. The row never stores a secret
-- value once a key is listed here.
ALTER TABLE api_environments ADD COLUMN secret_keys_json TEXT;

-- NOTE: `mcp_servers` needs no change — `secret_ref` / `secret_env_keys`
-- already exist since 0077 (the workspace CRUD surface now adopts them).
