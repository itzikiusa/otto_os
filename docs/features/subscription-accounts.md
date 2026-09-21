# Subscription accounts

New Session has an account picker for each selected Claude/Codex provider. **Default CLI account** preserves existing login behavior. **Add account** creates a labeled profile; **Sign in** opens the provider's native login terminal inside the form. Complete its browser/device login, then use **Check sign-in**. No API key is required.

Named profiles are private to the Otto user who created them. Each has a private native configuration home under Otto's data directory. Claude uses `CLAUDE_CONFIG_DIR`; Codex uses a stable `CODEX_HOME`, ChatGPT login mode and the operating-system keyring. Native login, launch, token refresh and resume all use that same home. Otto does not read or return provider credentials. It does not copy the default CLI account's authentication/config/history into a named profile.

Sessions store their selected account ID and display its label. Restart/resume keeps that identity. Changing the account on an existing session is rejected; create another session instead. Different profiles can run concurrently. Inherited alternative API/OAuth token environment variables are removed from named-profile launches. Incompatible enterprise routing flags or custom API base URLs reject the named-profile launch with an actionable error; Otto does not silently bypass them. Existing sessions without an account ID retain the old credential paths.

Workspace and project context is still assembled for named profiles. Codex keeps the native account home stable instead of using the legacy per-directory shadow home; selected workspace skills are supplied through the existing context index with absolute paths. Native profile skills remain provider-owned. Profile transcripts/activity are resolved from that profile's home; default-history pruning does not delete named-profile sessions.

The profile metadata endpoints are owner-scoped:

- `GET /auth/provider-accounts`: list `{id, provider, label, created_at}` records.
- `POST /auth/provider-accounts`: create with `{provider: "claude"|"codex", label}`.
- `POST /auth/provider-accounts/{id}/login`: `{workspace_id}`; requires workspace Editor and returns a terminal Session.
- `GET /auth/provider-accounts/{id}/status`: returns `{signed_in: boolean}` from the native CLI's status command, with a 10-second timeout. No raw provider output is returned.

Creation and login are unavailable while impersonating another user. Names need 1–80 characters and are unique per owner/provider. Provider errors or a missing profile never trigger automatic fallback to another account. Profile deletion, account rotation, billing aggregation and API-key onboarding are outside this first version.

Automated verification uses isolated homes and synthetic profiles. Real two-account browser login and subscription token refresh require signing into those accounts; they have not been exercised during development.

References: [Claude authentication](https://code.claude.com/docs/en/authentication), [Codex authentication](https://learn.chatgpt.com/docs/auth), [Codex configuration](https://learn.chatgpt.com/docs/config-file/config-reference), [Codex keyring identity implementation](https://github.com/openai/codex/blob/main/codex-rs/login/src/auth/storage.rs).
