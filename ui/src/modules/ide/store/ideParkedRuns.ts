// Runs are per project (review I3-03). Switching away from a project used to drop
// the run's handle while the process kept running — no output, no Stop, nothing
// that could list it again. Now the live run is PARKED under the project's target
// key (memory + sessionStorage, so a reload keeps it), watched in the background,
// and RE-ADOPTED when you come back: the Run panel shows the same run, its output
// keeps coming, and Stop works.
//
// A run that ends while you are elsewhere is never silent: one toast names the
// project, with "Show" to switch back (the finished run is adopted there).
// Debug sessions are not parked: `leaveDebugSessionOnSwitch` stops them on switch.

import { api, ApiError } from '../../../lib/api/client';
import { pollWhileVisible, type Poller } from '../../../lib/poll';
import { ssGet, ssSet } from '../../../lib/storage';
import { toasts } from '../../../lib/toast.svelte';
import type { IdeRunState, IdeTargetSpec } from './ideTypes';

export const PARKED_RUNS_KEY = 'otto.ide.parkedRuns';
const WATCH_MS = 3000;
const MAX_PARKED = 12;

export interface ParkedRunTarget {
  kind: IdeTargetSpec['kind'];
  root: string;
  connection_id?: string | null;
  connection_name?: string | null;
  workspace_id?: string | null;
  /** The named multi-repo project whose primary root this is (switch back through it). */
  project_id?: string | null;
}

export interface ParkedRun {
  runId: string;
  title: string;
  targetKey: string;
  label: string;
  target: ParkedRunTarget;
  /** Set once the run ended while parked: adopted as-is on return. */
  final?: IdeRunState | null;
  /** The end was already announced. */
  notified?: boolean;
}

export interface RunHost {
  target: IdeTargetSpec;
  envState: { activeRun: IdeRunState | null; runsHistory: IdeRunState[] };
}

let parked: ParkedRun[] = readParked();
let watchTimer: Poller | null = null;
let follower: { runId: string; key: string; poller: Poller } | null = null;
/** Switches back to a project; installed by the wiring (keeps this module free of the store). */
let reopen: ((t: ParkedRunTarget) => void) | null = null;
let currentKey: (() => string) | null = null;

export function setParkedRunCurrentKey(fn: () => string): void {
  currentKey = fn;
}

function readParked(): ParkedRun[] {
  try {
    const raw = JSON.parse(ssGet(PARKED_RUNS_KEY) ?? '[]') as unknown;
    return Array.isArray(raw) ? (raw as ParkedRun[]).filter((p) => p && typeof p.runId === 'string' && typeof p.targetKey === 'string') : [];
  } catch {
    return [];
  }
}

function persist(): void {
  try {
    // Output never goes to storage: only ids and labels (a finished run keeps its tail in memory).
    ssSet(PARKED_RUNS_KEY, JSON.stringify(parked.map(({ final: _f, ...rest }) => rest)));
  } catch { /* storage blocked */ }
}

export function setParkedRunReopener(fn: (t: ParkedRunTarget) => void): void {
  reopen = fn;
}

/** Restart the background watch (after a reload, parked runs of other projects). */
export function resumeParkedRunWatch(): void {
  ensureWatcher();
}

export function parkedRuns(): readonly ParkedRun[] {
  return parked;
}

function bounded(run: IdeRunState): IdeRunState {
  return run.output.length > 2000 ? { ...run, output: run.output.slice(-2000) } : run;
}

/** Leaving `key`: park its live run (if any). Returns the parked entry. */
export function parkRun(host: RunHost, key: string, label: string, target: ParkedRunTarget): ParkedRun | null {
  stopFollowing();
  const run = host.envState.activeRun;
  if (!key || !run || run.status !== 'running') return null;
  const entry: ParkedRun = { runId: run.id, title: run.title || 'Run', targetKey: key, label, target };
  parked = [entry, ...parked.filter((p) => p.targetKey !== key && p.runId !== run.id)].slice(0, MAX_PARKED);
  persist();
  ensureWatcher();
  return entry;
}

/** Entering `key`: adopt its parked run into the Run panel. */
export async function adoptParkedRun(host: RunHost, key: string): Promise<IdeRunState | null> {
  const entry = parked.find((p) => p.targetKey === key);
  if (!entry) return null;
  parked = parked.filter((p) => p !== entry);
  persist();
  const stillHere = (): boolean => targetKeyOf(host) === key;
  let run: IdeRunState | null = entry.final ?? null;
  if (!run) {
    try {
      run = (await api.bg.get<{ run: IdeRunState }>(`/ide/runs/${encodeURIComponent(entry.runId)}`)).run;
    } catch (e) {
      if (!(e instanceof ApiError && e.status === 404)) {
        // Unknown failure: keep it parked so the next visit can try again.
        parked = [entry, ...parked];
        persist();
        ensureWatcher();
      }
      return null;
    }
  }
  if (!run || !stillHere()) return null;
  applyRun(host, run);
  if (run.status === 'running') follow(host, key, run.id);
  return run;
}

function targetKeyOf(host: RunHost): string {
  const t = host.target;
  return `${t.kind}:${t.connection_id ?? ''}:${t.root}`;
}

function applyRun(host: RunHost, incoming: IdeRunState): void {
  const run = bounded(incoming);
  host.envState.activeRun = run;
  if (run.status !== 'running') {
    const archived = run.output.length > 200 ? { ...run, output: run.output.slice(-200) } : run;
    host.envState.runsHistory = [archived, ...host.envState.runsHistory.filter((x) => x.id !== archived.id)].slice(0, 8);
  }
}

/** Keep an adopted run's output and status live while this project stays open. */
function follow(host: RunHost, key: string, runId: string): void {
  stopFollowing();
  const poller = pollWhileVisible(async (signal) => {
    if (targetKeyOf(host) !== key || host.envState.activeRun?.id !== runId) { stopFollowing(); return; }
    try {
      const r = await api.bg.get<{ run: IdeRunState }>(`/ide/runs/${encodeURIComponent(runId)}`, signal);
      if (targetKeyOf(host) !== key || host.envState.activeRun?.id !== runId) return;
      applyRun(host, r.run);
      if (r.run.status !== 'running') stopFollowing();
    } catch (e) {
      if (e instanceof ApiError && e.status === 404) {
        const cur = host.envState.activeRun;
        if (cur && cur.id === runId && cur.status === 'running') applyRun(host, { ...cur, status: 'failed' });
        stopFollowing();
      }
    }
  }, { ms: 1000, floorMs: 750, immediate: false });
  follower = { runId, key, poller };
}

export function stopFollowing(): void {
  follower?.poller.stop();
  follower = null;
}

function ensureWatcher(): void {
  if (watchTimer || !parked.some((p) => !p.notified)) return;
  // Paused while the window is hidden; one check runs on return.
  watchTimer = pollWhileVisible(() => watchOnce(), { ms: WATCH_MS, immediate: false });
}

/** One background check of every parked, not-yet-ended run. Exported for tests. */
export async function watchOnce(): Promise<void> {
  const pending = parked.filter((p) => !p.notified);
  if (pending.length === 0) {
    watchTimer?.stop();
    watchTimer = null;
    return;
  }
  for (const p of pending) {
    let ended: IdeRunState | null = null;
    try {
      const run = (await api.bg.get<{ run: IdeRunState }>(`/ide/runs/${encodeURIComponent(p.runId)}`)).run;
      if (run.status !== 'running') ended = bounded(run);
    } catch (e) {
      if (e instanceof ApiError && e.status === 404) ended = { id: p.runId, title: p.title, runner: '', status: 'failed', output: [], tests: [] };
      else continue; // daemon hiccup: try again next tick
    }
    if (!ended || !parked.includes(p)) continue;
    p.final = ended;
    if (currentKey?.() === p.targetKey) {
      // Back on that project already (adoption raced the watch): nothing to announce.
      parked = parked.filter((x) => x !== p);
      persist();
      continue;
    }
    p.notified = true;
    persist();
    announceEnded(p, ended);
  }
}

export function runEndedCopy(p: Pick<ParkedRun, 'title' | 'label'>, run: Pick<IdeRunState, 'status' | 'exit_code'>): { level: 'info' | 'error'; title: string } {
  const failed = run.status === 'failed';
  const verb = run.status === 'stopped' ? 'stopped' : failed ? 'failed' : 'finished';
  const code = failed && typeof run.exit_code === 'number' ? ` (exit ${run.exit_code})` : '';
  return { level: failed ? 'error' : 'info', title: `${p.title} ${verb}${code} in ${p.label}` };
}

function announceEnded(p: ParkedRun, run: IdeRunState): void {
  const copy = runEndedCopy(p, run);
  toasts.push(copy.level, copy.title, undefined, 10_000, {
    action: { label: 'Show', run: () => reopen?.(p.target) },
  });
}

/** Test seam. */
export function resetParkedRunsForTests(): void {
  parked = [];
  stopFollowing();
  watchTimer?.stop();
  watchTimer = null;
}
