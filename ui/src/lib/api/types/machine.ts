// `GET /machine/status` — mirrors `MachineStatus` / `QueuedAgent` in
// crates/otto-sessions/src/admission.rs (docs/contracts/api.md, "Machine
// status"). Workspace-independent; any signed-in user may read it.

/** macOS memory pressure (`unknown` off macOS or when unreadable). */
export type MemoryPressure = 'unknown' | 'normal' | 'warn' | 'critical';

/** A gated background agent filling the ceiling (`AgentHolder`). */
export interface AgentHolder {
  /** `null` for a place reserved before its session exists. */
  session_id: string | null;
  source: string;
  label: string;
  run_id: string | null;
  since: string;
}

/** Why a background agent is not starting yet (`AdmissionWait`). `live` =
 *  gated background agents running; `floor` = from how many pressure holds. */
export type AdmissionWait =
  | { kind: 'queue'; ahead: number }
  | { kind: 'spacing' }
  | { kind: 'live_ceiling'; live: number; max: number; holders: AgentHolder[] }
  | { kind: 'load'; load1: number; limit: number; live: number; floor: number }
  | { kind: 'memory_pressure'; level: MemoryPressure; live: number; floor: number };

/** One background agent waiting for the machine (oldest first). */
export interface QueuedAgent {
  /** Its `meta.source` (`review`, `workflow`, `scheduled_task`, …). */
  source: string;
  waiting_secs: number;
  /** Why it is waiting, in words ("Waiting for memory — 2 Otto agents are
   *  running"), if known yet. */
  reason: string | null;
  /** The same as data. Optional: a daemon from before this field omits it. */
  wait?: AdmissionWait | null;
}

export interface MachineStatus {
  /** 1-minute load average (`null` when unreadable). */
  load1: number | null;
  cores: number;
  load_per_core: number;
  memory_pressure: MemoryPressure;
  swap_used_bytes: number | null;
  /** Physical memory (`null` = unknown). Optional: older daemons omit it. */
  ram_bytes?: number | null;
  admission_enabled: boolean;
  max_load_per_core: number;
  max_wait_secs: number;
  /** Gated background agents running or starting (PTY and runtime threads). */
  background_live: number;
  /** The ceiling (`null` = none). */
  background_max_live: number | null;
  /** Load / memory pressure holds a new background agent only while at least
   *  this many run (0 = always). Optional: older daemons omit it. */
  pressure_floor?: number;
  /** Every agent process Otto started that is still running. */
  agents_live: number;
  /** The machine is under load or memory pressure (whatever Otto runs). */
  busy: boolean;
  busy_reason: string | null;
  /** What a background agent arriving now would wait for (`null` = it starts
   *  at once). Optional: a daemon from before this field omits it (then
   *  `busy` meant "would wait"). */
  admission?: AdmissionWait | null;
  queued: QueuedAgent[];
  /** Free bytes on the volume holding Otto's data dir (`null` = unknown).
   *  Optional: a daemon from before this field omits it. */
  disk_free_bytes?: number | null;
  /** The low-disk floor in GiB (`machine_low_disk_gb`; 0 = off). */
  low_disk_gb?: number;
  /** Free space is below the floor. Shown, never acted on. */
  low_disk?: boolean;
}

/** One kind of work on one side of a daemon restart
 *  (`GET /system/restart-impact`; `crates/otto-core/src/restart.rs`). */
export interface RestartImpactItem {
  /** `sessions` | `background_agents` | `connection_terminals` | `ide_runs` |
   *  `debug_sessions` | `container_runtime` | `broker_tunnels` |
   *  `session_rooms` | `live_browser` | `phones` | `db_tunnels` | `daemon_jobs`. */
  kind: string;
  /** Noun phrase already matching `count` ("1 debug session", "2 debug sessions"). */
  label: string;
  count: number;
  /** Up to 5 names (session / run / room titles); omitted when none. */
  detail?: string[];
}

/** What a restart keeps running and what it ends (empty kinds omitted). */
export interface RestartImpact {
  keeps: RestartImpactItem[];
  ends: RestartImpactItem[];
}
