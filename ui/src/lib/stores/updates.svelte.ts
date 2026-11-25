// App updates (the desktop shell's updater, apps/desktop/src-tauri/src/updater.rs).
// The shell checks, downloads, verifies and stages a release build and owns the
// truth; this store mirrors it (`otto://update`) for the sidebar row, the
// Update dialog and Settings ▸ Updates. Only the root account in the desktop
// app ever sees any of it (the phone and the remote web never do), and a
// source build only says where its updates come from (git).
//
// Restarting ends some work (connection terminals, background agents, debug
// runs…): the dialog lists what keeps running and what ends from the daemon's
// own count (`GET /system/restart-impact`) and asks once more only when
// something would end. Nothing here restarts on its own.

import { isTauri } from '../desktop';
import { api } from '../api/client';
import { confirmer } from '../confirm.svelte';
import { toasts } from '../toast.svelte';
import { loadErrorText } from '../loadError';

export type UpdatePhase = 'idle' | 'checking' | 'available' | 'downloading' | 'staged' | 'error' | 'source';

/** How the last update went, read once after the relaunch. */
export interface UpdateResult {
  ok: boolean;
  from: string;
  to: string;
  reason?: string | null;
  log?: string | null;
}

/** Mirrors the shell's `otto://update` payload. */
export interface UpdateState {
  phase: UpdatePhase;
  current: string;
  version?: string | null;
  /** Release notes: UNTRUSTED markdown (rendered sanitised). */
  notes?: string | null;
  date?: string | null;
  size?: number | null;
  pct?: number | null;
  error?: string | null;
  channel: 'release' | 'source';
  last_check?: string | null;
  last_result?: UpdateResult | null;
}

export interface UpdatePrefs {
  auto_install: boolean;
  skipped: string | null;
}

/** One kind of work a restart keeps or ends ("2 connection terminals"). */
export interface ImpactLine {
  label: string;
  count: number;
  detail?: string[];
}
export interface RestartImpact {
  keeps: ImpactLine[];
  ends: ImpactLine[];
}

const pctText = (pct: number | null | undefined): string => (pct == null ? 'Downloading…' : `Downloading… ${Math.floor(pct)} %`);

/** The quiet sidebar row, or null when there is nothing to say. Silent for
 *  up-to-date, checking, a failed automatic check, a skipped version and a
 *  source build. */
export function rowLabel(s: UpdateState | null, prefs: UpdatePrefs, failed: UpdateResult | null): string | null {
  if (failed) return 'Update failed and was undone';
  if (!s || s.channel !== 'release' || !s.version) return null;
  if (s.phase === 'downloading') return pctText(s.pct);
  if (s.phase === 'staged') return 'Restart to update';
  if ((s.phase === 'available' || s.phase === 'error') && s.version !== prefs.skipped) return `Update to ${s.version}`;
  return null;
}

export type DialogKind = 'checking' | 'uptodate' | 'available' | 'downloading' | 'staged' | 'error' | 'source' | 'failed';

/** What the Update dialog shows, and its ONE primary button. `action` null =
 *  the button is a disabled progress label. */
export function dialogView(
  s: UpdateState | null,
  failed: UpdateResult | null,
  checkError: string | null,
): { kind: DialogKind; primary: { label: string; action: 'download' | 'restart' | 'close' | null } } {
  if (failed) return { kind: 'failed', primary: { label: 'Close', action: 'close' } };
  if (s?.channel === 'source' || s?.phase === 'source') return { kind: 'source', primary: { label: 'Close', action: 'close' } };
  // A failed check shows its own inline Retry (LoadState); the button closes.
  if (checkError) return { kind: 'error', primary: { label: 'Close', action: 'close' } };
  if (!s) return { kind: 'checking', primary: { label: 'Checking…', action: null } };
  switch (s.phase) {
    case 'checking':
      return { kind: 'checking', primary: { label: 'Checking…', action: null } };
    case 'available':
      return { kind: 'available', primary: { label: 'Download and install', action: 'download' } };
    case 'downloading':
      return { kind: 'downloading', primary: { label: pctText(s.pct), action: null } };
    case 'staged':
      return { kind: 'staged', primary: { label: 'Restart to finish', action: 'restart' } };
    case 'error':
      // A failed download keeps its version: retrying is downloading again.
      return { kind: 'error', primary: s.version ? { label: 'Try again', action: 'download' } : { label: 'Close', action: 'close' } };
    default:
      return { kind: 'uptodate', primary: { label: 'Close', action: 'close' } };
  }
}

const line = (l: ImpactLine): string => `${l.count} ${l.label}${l.detail?.length ? ` (${l.detail.join(', ')})` : ''}`;

/** The restart step: what keeps running, what ends, and the button. An
 *  unknown impact (the count failed) is treated as "something may end". */
export function restartPlan(impact: RestartImpact | null): { keeps: string[]; ends: string[]; endCount: number; label: string; confirm: boolean } {
  const live = (ls: ImpactLine[] | undefined) => (ls ?? []).filter((l) => l.count > 0);
  const ends = live(impact?.ends);
  const endCount = ends.reduce((n, l) => n + l.count, 0);
  return {
    keeps: live(impact?.keeps).map(line),
    ends: ends.map(line),
    endCount,
    label: endCount ? `Restart and end ${endCount}` : 'Restart now',
    confirm: endCount > 0 || impact === null,
  };
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import('@tauri-apps/api/core');
  return invoke<T>(cmd, args);
}

class UpdatesStore {
  state = $state<UpdateState | null>(null);
  prefs = $state<UpdatePrefs>({ auto_install: false, skipped: null });
  /** The last update was rolled back (shown until dismissed). */
  failed = $state<UpdateResult | null>(null);
  open = $state(false);
  /** A check the person asked for failed (automatic ones stay silent). */
  checkError = $state<string | null>(null);
  /** A download / restart / prefs action failed, shown inline. */
  actionError = $state<string | null>(null);
  impact = $state<RestartImpact | null>(null);
  impactError = $state<string | null>(null);
  restarting = $state(false);
  #started = false;
  #toasts = false;

  /** Read the shell's state once and follow its broadcasts (idempotent).
   *  `toasts`: this window announces how the last update went (main only). */
  init(toastsHere: boolean): void {
    if (!isTauri || this.#started) return;
    this.#started = true;
    this.#toasts = toastsHere;
    void call<UpdateState>('update_state').then((s) => this.apply(s)).catch(() => {});
    void call<UpdatePrefs>('update_prefs_get').then((p) => (this.prefs = p)).catch(() => {});
    void import('@tauri-apps/api/event')
      .then(({ listen }) => listen<UpdateState>('otto://update', (e) => this.apply(e.payload)))
      .catch(() => {});
  }

  apply(s: UpdateState): void {
    const r = s.last_result;
    if (r && this.#toasts) {
      if (r.ok) toasts.success(`Updated to ${r.to}`);
      else this.failed = r;
    }
    this.state = s;
    if (s.phase !== 'staged') this.impact = null;
  }

  get row(): string | null {
    return rowLabel(this.state, this.prefs, this.failed);
  }

  /** `manual`: Otto ▸ Check for Updates… — check now and say the outcome. */
  openDialog(manual = false): void {
    this.open = true;
    this.actionError = null;
    if (manual) void this.check();
    if (this.state?.phase === 'staged') void this.loadImpact();
  }

  close(): void {
    this.open = false;
  }

  async check(): Promise<void> {
    if (!isTauri) return;
    this.checkError = null;
    try {
      this.apply(await call<UpdateState>('update_check'));
    } catch (e) {
      this.checkError = loadErrorText(e);
    }
  }

  async download(): Promise<void> {
    this.actionError = null;
    try {
      await call('update_download');
    } catch (e) {
      this.actionError = loadErrorText(e);
    }
  }

  async loadImpact(): Promise<void> {
    this.impactError = null;
    try {
      this.impact = await api.get<RestartImpact>('/system/restart-impact');
    } catch (e) {
      this.impact = null;
      this.impactError = loadErrorText(e);
    }
  }

  async restart(): Promise<void> {
    const plan = restartPlan(this.impact);
    if (plan.confirm) {
      const ok = await confirmer.ask(
        plan.endCount
          ? `Restarting Otto ends: ${plan.ends.join('; ')}. Everything else keeps running.`
          : 'Otto couldn’t count what a restart ends. Some work may stop.',
        { title: `Restart to update to ${this.state?.version ?? 'the new version'}?`, confirmLabel: plan.label, danger: plan.endCount > 0 },
      );
      if (!ok) return;
    }
    this.actionError = null;
    this.restarting = true;
    try {
      await call('update_restart');
    } catch (e) {
      this.restarting = false;
      this.actionError = loadErrorText(e);
    }
  }

  async setPrefs(p: Partial<UpdatePrefs>): Promise<void> {
    const before = this.prefs;
    this.prefs = { ...before, ...p };
    try {
      await call('update_prefs_set', { prefs: p });
    } catch (e) {
      this.prefs = before;
      this.actionError = loadErrorText(e);
      toasts.error('Couldn’t save the update setting', loadErrorText(e));
    }
  }

  skip(): void {
    if (this.state?.version) void this.setPrefs({ skipped: this.state.version });
    this.open = false;
  }

  async showLog(): Promise<void> {
    try {
      await call('update_open_log');
    } catch (e) {
      toasts.error('Couldn’t open the update log', loadErrorText(e));
    }
  }

  dismissFailed(): void {
    this.failed = null;
    this.open = false;
  }
}

export const updates = new UpdatesStore();
