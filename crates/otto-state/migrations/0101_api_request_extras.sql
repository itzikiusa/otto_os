-- API client: persist the request fields that previously lived ONLY in the
-- localStorage tab draft and were silently dropped on Save — scripts,
-- per-request settings (timeout / redirects / TLS verify), docs, and GraphQL
-- variables. All nullable TEXT; NULL = "not set" (legacy rows unchanged).
ALTER TABLE api_requests ADD COLUMN pre_request_script  TEXT;
ALTER TABLE api_requests ADD COLUMN post_response_script TEXT;
-- JSON: { "timeout_ms"?, "follow_redirects"?, "verify_ssl"? }
ALTER TABLE api_requests ADD COLUMN settings_json TEXT;
ALTER TABLE api_requests ADD COLUMN docs TEXT;
ALTER TABLE api_requests ADD COLUMN graphql_variables TEXT;
