-- Per-user email sender (mobile plan Task 7.1) — the Gmail App Password sender
-- that later powers the email-OTP gate for share links (Tasks 7.2–7.4).
--
-- A user configures ONE Gmail sender (their address + a 16-char Gmail App
-- Password). Otto sends OTP mail via Gmail SMTP `smtp.gmail.com:587` (STARTTLS,
-- SMTP AUTH). The app password is stored in the **macOS Keychain**
-- (`otto-keychain`), NEVER in this table — the DB holds only an opaque
-- `secret_ref` (e.g. `email-sender-{user_id}`), the same secret-indirection
-- pattern connections use (`conn-{id}`). `verified_at` is set once a real SMTP
-- login with that app password succeeds; a NULL means "not yet verified".
CREATE TABLE email_senders (
  user_id       TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  gmail_address TEXT NOT NULL,
  secret_ref    TEXT NOT NULL,           -- Keychain reference, NOT the password
  verified_at   INTEGER
);
