-- Phone access (docs/features/phone-access.md): QR pairing of the owner's
-- phone over Tailscale, and the per-phone device tokens it yields.
--
-- Device tokens live in their OWN table, not `auth_sessions`: a rollback boots
-- the previous binary on this schema, and that binary would treat an unknown
-- `auth_sessions.kind` as a plain full-power login token (no peer check, no
-- admin/credential narrowing). Here an older daemon simply never sees them —
-- a paired phone is signed out by a rollback, never widened.

-- One paired phone. Only the SHA-256 of the raw token is stored.
CREATE TABLE IF NOT EXISTS phone_devices (
    id                  TEXT PRIMARY KEY,
    user_id             TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash          TEXT NOT NULL UNIQUE,
    token_prefix        TEXT NOT NULL DEFAULT '',
    name                TEXT NOT NULL,
    user_agent          TEXT,
    created_at          TEXT NOT NULL,
    last_seen_at        TEXT NOT NULL,
    last_ip             TEXT,
    -- Sliding idle expiry (30 days from last use), never past `expires_at`.
    idle_expires_at     TEXT NOT NULL,
    -- Absolute lifetime (180 days from pairing); never slid.
    expires_at          TEXT NOT NULL,
    pairing_id          TEXT
);
CREATE INDEX IF NOT EXISTS idx_phone_devices_user ON phone_devices(user_id);

-- One QR pairing attempt. The QR code (>=32 random bytes) and the phone's
-- poll secret are stored hashed; the 6-digit confirmation code is shown on
-- both screens so it is kept as is (it authorizes nothing on its own: the
-- owner's Approve on the Mac does).
CREATE TABLE IF NOT EXISTS phone_pairings (
    id                  TEXT PRIMARY KEY,
    user_id             TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    code_hash           TEXT NOT NULL UNIQUE,
    status              TEXT NOT NULL DEFAULT 'waiting'
                        CHECK (status IN ('waiting','claimed','approved','denied','consumed','expired')),
    next_route          TEXT,
    created_at          TEXT NOT NULL,
    expires_at          TEXT NOT NULL,
    claimed_at          TEXT,
    decided_at          TEXT,
    device_label        TEXT,
    user_agent          TEXT,
    peer_ip             TEXT,
    confirm_code        TEXT,
    poll_secret_hash    TEXT,
    device_id           TEXT
);
CREATE INDEX IF NOT EXISTS idx_phone_pairings_user ON phone_pairings(user_id, status);
