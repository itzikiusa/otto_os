// Words for `GET /machine/status`: the status-bar machine chip, its details
// dialog and the live line on Settings ▸ Daemon ▸ Machine resources. Pure
// (type-only imports), so `node --test` can load it.
import type { MachineStatus, MemoryPressure, QueuedAgent } from '../../lib/api/types/machine';

const GIB = 1024 ** 3;

/** Bytes as GB the way `df -h` counts them (1024³), one decimal. */
export function formatGiB(bytes: number | null | undefined): string {
  if (bytes == null || !Number.isFinite(bytes)) return 'unknown';
  const gib = bytes / GIB;
  return `${gib >= 100 ? Math.round(gib) : gib.toFixed(1)} GB`;
}

/** Waiting time, coarse: "40 s", "3 min", "1 h 5 min". */
export function formatWait(secs: number): string {
  const s = Math.max(0, Math.round(secs));
  if (s < 60) return `${s} s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m} min`;
  const h = Math.floor(m / 60);
  return m % 60 ? `${h} h ${m % 60} min` : `${h} h`;
}

const SOURCES: Record<string, string> = {
  review: 'Code review',
  review_summarizer: 'Review summary',
  workflow: 'Workflow step',
  scheduled_task: 'Scheduled task',
  goal_loop: 'Goal loop',
  personal_agent: 'Personal agent',
  insights: 'Insights report',
  'vault-docs': 'Vault docs',
  'vault-docs-review': 'Vault docs review',
  'product-analysis': 'Story analysis',
  skilleval: 'Skill eval',
  skillreview: 'Skill review',
  run_with_otto: 'Run with Otto',
  finding: 'Finding fix',
  // Headless orchestrator turns (no session row): otto-sessions admission.rs
  // ORCHESTRATOR_TURN_SOURCES.
  self_improvement: 'Self-improvement review',
  product: 'Story watcher',
  plugin: 'Plugin agent',
};

/** A queued agent's `meta.source` in words. */
export function sourceLabel(source: string): string {
  return SOURCES[source] ?? source.replace(/[_-]+/g, ' ');
}

/** macOS memory pressure in words ("warn" reads as "high"). */
export function memoryLabel(p: MemoryPressure): string {
  return p === 'warn' ? 'high' : p;
}

function plural(n: number, one: string, many = `${one}s`): string {
  return `${n} ${n === 1 ? one : many}`;
}

/** "2 of 6 background agents" / "2 background agents (no limit)". */
export function backgroundLine(s: MachineStatus): string {
  if (s.background_max_live == null) return `${plural(s.background_live, 'background agent')} (no limit)`;
  return `${s.background_live} of ${s.background_max_live} background agents`;
}

function queuedCount(s: MachineStatus): string {
  return s.queued.length ? `${s.queued.length} queued` : 'nothing queued';
}

function loadPart(s: MachineStatus): string {
  return s.load1 == null ? `load unknown on ${plural(s.cores, 'core')}` : `load ${s.load1.toFixed(1)} on ${plural(s.cores, 'core')}`;
}

/** The Daemon page's live line:
 *  "Now: load 3.1 on 8 cores · memory normal · 2 of 6 background agents · nothing queued · 41.2 GB free". */
export function statusLine(s: MachineStatus): string {
  const parts = [loadPart(s), `memory ${memoryLabel(s.memory_pressure)}`, backgroundLine(s), queuedCount(s)];
  if (s.disk_free_bytes != null) parts.push(`${formatGiB(s.disk_free_bytes)} free`);
  return `Now: ${parts.join(' · ')}`;
}

/** One queued agent: "Code review · waiting 3 min · load average 13.0 is above 12.0". */
export function queuedLine(q: QueuedAgent): string {
  const parts = [sourceLabel(q.source), `waiting ${formatWait(q.waiting_secs)}`];
  if (q.reason) parts.push(q.reason);
  return parts.join(' · ');
}

export type ChipTone = 'calm' | 'busy' | 'low-disk';

export interface ChipState {
  tone: ChipTone;
  /** Visible text ('' = the icon alone). */
  text: string;
  /** Accessible name + tooltip: the whole state in one sentence. */
  label: string;
}

/** The status-bar chip: an icon alone while calm; "Machine busy · N queued"
 *  (warning tone) while background agents would wait; "Low disk · 12.0 GB
 *  free" below the floor. `null` until the first read (nothing shown). */
export function chipState(s: MachineStatus | null): ChipState | null {
  if (!s) return null;
  const disk = s.low_disk ? ` Only ${formatGiB(s.disk_free_bytes)} free on disk.` : '';
  if (s.busy || s.queued.length > 0) {
    const n = s.queued.length;
    const why = s.busy_reason ? ` (${s.busy_reason})` : '';
    return {
      tone: 'busy',
      text: n ? `Machine busy · ${n} queued` : 'Machine busy',
      label: `Machine busy${why}: new background agents wait their turn. ${n ? `${plural(n, 'agent')} queued.` : 'Nothing queued yet.'}${disk} Show details`,
    };
  }
  if (s.low_disk) {
    return {
      tone: 'low-disk',
      text: `Low disk · ${formatGiB(s.disk_free_bytes)} free`,
      label: `Low disk space: ${formatGiB(s.disk_free_bytes)} free (warning below ${s.low_disk_gb ?? 30} GB). Show details`,
    };
  }
  return {
    tone: 'calm',
    text: '',
    label: `Machine: ${loadPart(s)}, memory ${memoryLabel(s.memory_pressure)}, ${backgroundLine(s)}. Show details`,
  };
}

/** The details dialog's rows, in reading order. */
export function detailRows(s: MachineStatus): { label: string; value: string }[] {
  const rows = [
    {
      label: 'Load',
      value: s.load1 == null ? 'unknown' : `${s.load1.toFixed(1)} on ${plural(s.cores, 'core')} (${s.load_per_core.toFixed(2)} per core; queueing above ${s.max_load_per_core})`,
    },
    { label: 'Memory pressure', value: memoryLabel(s.memory_pressure) },
    { label: 'Swap in use', value: s.swap_used_bytes == null ? 'unknown' : formatGiB(s.swap_used_bytes) },
    { label: 'Background agents', value: s.admission_enabled ? backgroundLine(s) : `${s.background_live} running (queueing is off)` },
    { label: 'All agents running', value: String(s.agents_live) },
  ];
  if (s.disk_free_bytes !== undefined) {
    const floor = s.low_disk_gb ? ` (warning below ${s.low_disk_gb} GB)` : '';
    rows.push({ label: 'Free disk', value: `${formatGiB(s.disk_free_bytes)}${floor}` });
  }
  return rows;
}
