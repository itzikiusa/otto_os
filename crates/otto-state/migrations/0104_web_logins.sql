-- Web Logins — a small per-user credential store for web sign-in pages the user
-- meets inside Otto's embedded Connectors browser (e.g. the Azure AD / SSO
-- password prompt). Each row is one saved login: a friendly `name`, a
-- case-insensitive `url_match` substring tested against the current page URL
-- (e.g. `login.microsoftonline.com`), the `username`, and an opaque
-- `secret_ref` pointing at the **macOS Keychain** item (`weblogin-{id}`) that
-- holds the password. The password is **never** stored here — the DB carries
-- only the ref, mirroring the secret-indirection pattern connections
-- (`conn-{id}`) and the email sender (`email-sender-{user_id}`) already use.
--
-- Owner-scoped: `created_by` FKs `users(id)` and cascades on user delete. A
-- one-click "Fill" in the Connectors browser matches the live URL against
-- `url_match` and injects the username/password into the visible fields — so the
-- user stops copy-pasting the SSO password from an external manager every time.
CREATE TABLE web_logins (
  id         TEXT PRIMARY KEY,
  name       TEXT NOT NULL,
  url_match  TEXT NOT NULL,
  username   TEXT NOT NULL,
  secret_ref TEXT NOT NULL,        -- Keychain reference (weblogin-{id}), NOT the password
  created_by TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  created_at TEXT NOT NULL
);

-- The two hot lookups are "this user's logins" (settings list) and the match
-- probe (also owner-scoped), both filtering on `created_by`.
CREATE INDEX idx_web_logins_created_by ON web_logins(created_by);
