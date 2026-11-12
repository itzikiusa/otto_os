// Settings ▸ Daemon ▸ Machine resources: the daemon settings behind the
// admission gate, the build budget, background priority, the background
// auto-archive and the low-disk warning — read exactly as the daemon parses
// them (crates/otto-sessions admission.rs `AdmissionPolicy::from_settings`,
// build_budget.rs `budget_mode`, machine_load.rs `low_disk_gb`,
// manager/background_archive.rs). The daemon falls back to a default on a
// bad value; this form refuses one instead. Pure, so `node --test` loads it.

export const MACHINE_KEYS = {
  admission: 'agent_admission_enabled',
  maxLive: 'agent_background_max_live',
  maxLoadPerCore: 'agent_admission_max_load_per_core',
  maxWaitSecs: 'agent_admission_max_wait_secs',
  lowPriority: 'agent_background_low_priority',
  build: 'agent_build_parallelism',
  bgArchiveDays: 'session_auto_archive_background_days',
  lowDiskGb: 'machine_low_disk_gb',
} as const;

export const DEFAULT_MAX_LOAD_PER_CORE = 1.5;
export const DEFAULT_MAX_WAIT_SECS = 300;
export const DEFAULT_BG_ARCHIVE_DAYS = 7;
export const DEFAULT_LOW_DISK_GB = 30;
const MAX_ARCHIVE_DAYS = 36_500;

export type BuildMode = 'auto' | 'fixed' | 'off';

/** A blank number field is `null` (Svelte binds an empty number input as
 *  `undefined`; both mean "the daemon's automatic / default value"). */
type Num = number | null | undefined;

export interface MachineForm {
  admission: boolean;
  /** Blank = automatic (three quarters of the cores, at least 2); 0 = no ceiling. */
  maxLive: Num;
  /** Blank = 1.5. */
  maxLoadPerCore: Num;
  /** Blank = 300. */
  maxWaitSecs: Num;
  lowPriority: boolean;
  buildMode: BuildMode;
  /** Used when `buildMode` is `fixed`. */
  buildWorkers: Num;
  /** 0 = never. Never blank: the daemon reads a stored blank as "off". */
  bgArchiveDays: Num;
  /** Blank = 30; 0 = no warning. */
  lowDiskGb: Num;
}

/** The automatic background-agent ceiling (`default_max_live`). */
export function defaultMaxLive(cores: number): number {
  return Math.max(2, Math.floor((Math.max(1, cores) * 3) / 4));
}

const isUint = (v: unknown): v is number => typeof v === 'number' && Number.isInteger(v) && v >= 0;
const blank = (v: Num): v is null | undefined => v == null || (typeof v === 'number' && Number.isNaN(v));

function buildFrom(v: unknown): { mode: BuildMode; workers: number | null } {
  if (v === false || (typeof v === 'string' && v.trim().toLowerCase() === 'off')) return { mode: 'off', workers: null };
  const n = typeof v === 'string' ? Number(v.trim()) : v;
  if (isUint(n) && n > 0) return { mode: 'fixed', workers: n };
  return { mode: 'auto', workers: null };
}

/** The stored values as the form shows them. */
export function machineFormFrom(settings: Record<string, unknown>): MachineForm {
  const get = (k: string): unknown => settings[k];
  const load = get(MACHINE_KEYS.maxLoadPerCore);
  const disk = get(MACHINE_KEYS.lowDiskGb);
  const archive = get(MACHINE_KEYS.bgArchiveDays);
  const build = buildFrom(get(MACHINE_KEYS.build));
  return {
    admission: get(MACHINE_KEYS.admission) !== false,
    maxLive: isUint(get(MACHINE_KEYS.maxLive)) ? (get(MACHINE_KEYS.maxLive) as number) : null,
    maxLoadPerCore: typeof load === 'number' && Number.isFinite(load) && load > 0 ? load : null,
    maxWaitSecs: isUint(get(MACHINE_KEYS.maxWaitSecs)) ? (get(MACHINE_KEYS.maxWaitSecs) as number) : null,
    lowPriority: get(MACHINE_KEYS.lowPriority) !== false,
    buildMode: build.mode,
    buildWorkers: build.workers,
    // Absent = 7; a stored value that is not a whole number reads as off (0).
    bgArchiveDays: archive === undefined ? DEFAULT_BG_ARCHIVE_DAYS : isUint(archive) ? archive : 0,
    lowDiskGb: typeof disk === 'number' && Number.isFinite(disk) && disk >= 0 ? disk : null,
  };
}

export type MachineErrors = Partial<Record<keyof MachineForm, string>>;

/** Why each field can't be saved (empty object = all fine). */
export function machineFormErrors(f: MachineForm): MachineErrors {
  const e: MachineErrors = {};
  const whole = (v: Num, max: number): boolean => isUint(v) && v <= max;
  if (!blank(f.maxLive) && !whole(f.maxLive, 256)) {
    e.maxLive = 'Enter a whole number from 0 (no limit) to 256, or leave it blank for automatic.';
  }
  if (!blank(f.maxLoadPerCore) && !(typeof f.maxLoadPerCore === 'number' && f.maxLoadPerCore > 0 && f.maxLoadPerCore <= 50)) {
    e.maxLoadPerCore = 'Enter a number above 0 and up to 50, or leave it blank for 1.5.';
  }
  if (!blank(f.maxWaitSecs) && !whole(f.maxWaitSecs, 86_400)) {
    e.maxWaitSecs = 'Enter whole seconds from 0 to 86,400, or leave it blank for 300.';
  }
  if (f.buildMode === 'fixed' && !(whole(f.buildWorkers, 256) && (f.buildWorkers as number) >= 1)) {
    e.buildWorkers = 'Enter a whole number of workers from 1 to 256.';
  }
  if (!whole(f.bgArchiveDays, MAX_ARCHIVE_DAYS)) {
    e.bgArchiveDays = `Enter a whole number of days from 0 (never) to ${MAX_ARCHIVE_DAYS.toLocaleString('en-US')}.`;
  }
  if (!blank(f.lowDiskGb) && !(typeof f.lowDiskGb === 'number' && f.lowDiskGb >= 0 && f.lowDiskGb <= 100_000)) {
    e.lowDiskGb = 'Enter a number of GB from 0 (no warning) to 100,000, or leave it blank for 30.';
  }
  return e;
}

/** The first error, for the page's single Save ('' = savable). */
export function machineFormError(f: MachineForm): string {
  return Object.values(machineFormErrors(f))[0] ?? '';
}

const norm = (v: Num): number | null => (blank(v) ? null : v);

function buildValue(f: MachineForm): number | string {
  if (f.buildMode === 'off') return 'off';
  if (f.buildMode === 'fixed') return norm(f.buildWorkers) ?? 0;
  return 0; // automatic
}

/** The setting values the form stands for (blank → `null`: the daemon then
 *  uses its automatic value / default). */
export function machineValues(f: MachineForm): Record<string, unknown> {
  return {
    [MACHINE_KEYS.admission]: f.admission,
    [MACHINE_KEYS.maxLive]: norm(f.maxLive),
    [MACHINE_KEYS.maxLoadPerCore]: norm(f.maxLoadPerCore),
    [MACHINE_KEYS.maxWaitSecs]: norm(f.maxWaitSecs),
    [MACHINE_KEYS.lowPriority]: f.lowPriority,
    [MACHINE_KEYS.build]: buildValue(f),
    [MACHINE_KEYS.bgArchiveDays]: norm(f.bgArchiveDays) ?? 0,
    [MACHINE_KEYS.lowDiskGb]: norm(f.lowDiskGb),
  };
}

/** Only the keys that differ from `saved` (PUT /settings upserts exactly the
 *  keys in its body). */
export function machineBody(f: MachineForm, saved: MachineForm): Record<string, unknown> {
  const now = machineValues(f);
  const before = machineValues(saved);
  const body: Record<string, unknown> = {};
  for (const k of Object.keys(now)) if (now[k] !== before[k]) body[k] = now[k];
  return body;
}

export function sameMachineForm(a: MachineForm, b: MachineForm): boolean {
  return Object.keys(machineBody(a, b)).length === 0;
}
