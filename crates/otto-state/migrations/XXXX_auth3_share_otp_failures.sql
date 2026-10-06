-- S8-301: per-share count of wrong OTP guesses. The per-IP throttle is not a
-- hard bound on a 6-digit code (client IPs rotate behind tunnels / IPv6), so
-- `verify_share_otp` counts misses against the share itself and burns the
-- code (`otp_hash = NULL`) after a few; `extend` (itself capped) resets it.
ALTER TABLE auth_sessions ADD COLUMN otp_failures INTEGER NOT NULL DEFAULT 0;
