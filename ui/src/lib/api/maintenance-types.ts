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
}

/** Body of `POST /admin/db/compact` (root). */
export interface DbCompactReq {
  confirm: true;
}

/** Response of `POST /admin/db/compact`. */
export interface DbCompactReport {
  before_bytes: number;
  after_bytes: number;
  freed_bytes: number;
  duration_ms: number;
  auto_vacuum: number;
}
