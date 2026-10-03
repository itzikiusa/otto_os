// Database maintenance (14-daemon-perf P3) — mirrors `routes/settings.rs`
// `DbStatsResp` and `otto_state::maintenance::CompactReport`. See
// docs/contracts/api.md "Database maintenance".

/** `GET /admin/db/stats` (root). */
export interface DbStatsResp {
  size_bytes: number;
  /** Bytes on the SQLite freelist — what a compaction would give back. */
  free_bytes: number;
  /** 0 = none, 1 = full, 2 = incremental (already compacted). */
  auto_vacuum: number;
  /** A compaction is running right now. */
  compacting: boolean;
  /** An offline compaction runs at the next daemon start (requested or automatic). */
  compaction_scheduled: boolean;
  /** Rough duration of that offline compaction (it delays that start; no write stall). */
  estimated_offline_ms: number;
}

/** Body of `POST /admin/db/compact` (root). `at` defaults to `now`. */
export interface DbCompactReq {
  confirm: true;
  at?: 'now' | 'next_restart' | 'cancel';
}

/** Response of `POST /admin/db/compact` with `at: "next_restart" | "cancel"`. */
export interface DbCompactScheduled {
  compaction_scheduled: boolean;
  estimated_offline_ms: number;
}

/** Response of `POST /admin/db/compact`. */
export interface DbCompactReport {
  before_bytes: number;
  after_bytes: number;
  freed_bytes: number;
  duration_ms: number;
  auto_vacuum: number;
}

// Secret store status + "Secure secrets…" (p-daemon SEC-1) — mirrors
// `otto_keychain::SecretsStatus` / `control::MigrationReport`. Counts and
// states only; values and key names never cross the API.

/** `GET /admin/secrets/status` (root). */
export interface SecretsStatus {
  /** Active backend. `plaintext` = unencrypted `secrets.json`. */
  mode: 'plaintext' | 'encrypted' | 'keychain';
  /** `secrets.json` exists on disk. */
  plaintext_file: boolean;
  /** Entries in `secrets.json` (0 when absent). */
  plaintext_entries: number;
  /** Master-key state; `locked` while a Keychain prompt waits. */
  key_state: 'unlocked' | 'locked' | 'not_loaded' | 'error';
  /** "Secure secrets…" is available (plaintext store in use). */
  migration_available: boolean;
  /** A migration is running right now. */
  migrating: boolean;
  /** An encrypted backup from an unfinished migration is still on disk. */
  backup_present: boolean;
}

/** Body of `POST /admin/secrets/secure` (root). */
export interface SecretsSecureReq {
  confirm: true;
}

/** Response of `POST /admin/secrets/secure`. */
export interface SecretsMigrationReport {
  /** Entries moved out of `secrets.json`. */
  migrated: number;
  /** Entries in the encrypted store afterwards. */
  total: number;
  duration_ms: number;
}
