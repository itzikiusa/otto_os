-- Email-OTP gate for share links (mobile plan Tasks 7.2–7.3, design addendum
-- "Email-OTP gate for share links").
--
-- A share token alone must NOT be enough to enter a shared session. When the
-- owner mints a share with a `recipient_email`, Otto also generates a 6-digit
-- one-time code, emails it OUT-OF-BAND to that address, and stores only the
-- SHA-256 of it here. The guest must redeem the code via `POST /share/verify`
-- before the scoped token can attach (`/ws/term`) or read its session — so a
-- leaked/forwarded link, on its own, is useless.
--
-- All five columns are NULL on every existing row and on every plain (no-email)
-- share, so adding them is behavior-preserving: a share with `recipient_email IS
-- NULL` keeps the exact pre-OTP behavior (`otp_pending = false`). The columns
-- live on `auth_sessions` alongside the share-token columns from 0041
-- (discriminated by `kind='share'`), the same single-table approach the api /
-- impersonation / share kinds already use.
ALTER TABLE auth_sessions ADD COLUMN recipient_email TEXT;    -- if set, the OTP gate applies; the recipient address is LOCKED (Task 7.4)
ALTER TABLE auth_sessions ADD COLUMN otp_hash        TEXT;    -- SHA-256 hex of the current 6-digit code; cleared on successful verify (single-use)
ALTER TABLE auth_sessions ADD COLUMN otp_expires_at  INTEGER; -- unix seconds; the code is only redeemable while now < this (~10 min)
ALTER TABLE auth_sessions ADD COLUMN verified_at     INTEGER; -- unix seconds; set when the guest passes the OTP (NULL = not yet verified)
ALTER TABLE auth_sessions ADD COLUMN max_expires_at  INTEGER; -- unix seconds; the share's session window end (≤12h), set on creation
