// `GET /machine/status` — mirrors `MachineStatus` / `QueuedAgent` in
// crates/otto-sessions/src/admission.rs (docs/contracts/api.md, "Machine
// status"). Workspace-independent; any signed-in user may read it.

/** macOS memory pressure (`unknown` off macOS or when unreadable). */
export type MemoryPressure = 'unknown' | 'normal' | 'warn' | 'critical';

/** One background agent waiting for the machine (oldest first). */
export interface QueuedAgent {
  /** Its `meta.source` (`review`, `workflow`, `scheduled_task`, …). */
  source: string;
  waiting_secs: number;
  /** Why it is waiting ("load average 13.0 is above 12.0"), if known yet. */
  reason: string | null;
}

export interface MachineStatus {
  /** 1-minute load average (`null` when unreadable). */
  load1: number | null;
  cores: number;
  load_per_core: number;
  memory_pressure: MemoryPressure;
  swap_used_bytes: number | null;
  admission_enabled: boolean;
  max_load_per_core: number;
  max_wait_secs: number;
  /** Gated background agents running or starting. */
  background_live: number;
  /** The ceiling (`null` = none). */
  background_max_live: number | null;
  /** Every agent process Otto started that is still running. */
  agents_live: number;
  /** A background agent arriving now would queue. */
  busy: boolean;
  busy_reason: string | null;
  queued: QueuedAgent[];
  /** Free bytes on the volume holding Otto's data dir (`null` = unknown).
   *  Optional: a daemon from before this field omits it. */
  disk_free_bytes?: number | null;
  /** The low-disk floor in GiB (`machine_low_disk_gb`; 0 = off). */
  low_disk_gb?: number;
  /** Free space is below the floor. Shown, never acted on. */
  low_disk?: boolean;
}
