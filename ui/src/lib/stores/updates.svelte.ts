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
import type { RestartImpact, RestartImpactItem } from '../api/types';

export type UpdatePhase = 'idle' | 'checking' | 'up_to_date' | 'available' | 'downloading' | 'staged' | 'restarting' | 'error';

/** How the last install went (the finisher's result), until `update_result_seen`. */
export interface UpdateResult {
  ok: boolean;
  /** The version itself failed, so the shell skipped it. */
  skip?: boolean;
  from: string;
  to: string;
  reason?: string;
  /** The finisher's log file. */
  log?: string;
}

/** Mirrors the shell's `otto://update` payload (updater/mod.rs `UpdateState`). */
export interface UpdateState {
  phase: UpdatePhase;
  channel: 'release' | 'source';
  current: string;
  version?: string | null;
  /** Release notes: UNTRUSTED markdown (rendered sanitised). */
  notes?: string | null;
  /** RFC 3339. */
  date?: string | null;
  /** Archive bytes. */
  size?: number | null;
  received?: number;
  error?: string | null;
  error_kind?: 'offline' | 'blocked' | 'disk' | 'verify' | 'failed' | null;
  /** Why this copy can't install updates (not in /Applications, not writable…). */
  blocked?: string | null;
  skipped?: string | null;
  auto_install?: boolean;
  /** Unix seconds of the last finished check. */
  last_check?: number | null;
  last_result?: UpdateResult | null;
  release_url?: string | null;
}


/** "Downloading… 42 %" (no percentage while the size is unknown). */
export function progressText(s: UpdateState): string {
  const pct = s.size ? Math.min(100, Math.floor(((s.received ?? 0) / s.size) * 100)) : null;
  return pct === null ? 'Downloading…' : `Downloading… ${pct} %`;
}

/** The quiet sidebar row, or null when there is nothing to say. Silent for
 *  up-to-date, checking, a failed automatic check, a skipped version and a
 *  source build. */
export function rowLabel(s: UpdateState | null, failed: UpdateResult | null): string | null {
  if (failed) return 'Update failed and was undone';
  if (!s || s.channel !== 'release' || !s.version) return null;
  if (s.phase === 'downloading') return progressText(s);
  if (s.phase === 'staged' || s.phase === 'restarting') return 'Restart to update';
  if ((s.phase === 'available' || s.phase === 'error') && s.version !== s.skipped) return `Update to ${s.version}`;
  return null;
}

export type DialogKind = 'checking' | 'uptodate' | 'available' | 'downloading' | 'staged' | 'restarting' | 'error' | 'source' | 'failed';

/** What the Update dialog shows, and its ONE primary button. `action` null =
 *  the button is a disabled progress label. */
export function dialogView(
  s: UpdateState | null,
  failed: UpdateResult | null,
  checkError: string | null,
): { kind: DialogKind; primary: { label: string; action: 'download' | 'restart' | 'close' | null } } {
  if (failed) return { kind: 'failed', primary: { label: 'Close', action: 'close' } };
  if (s?.channel === 'source') return { kind: 'source', primary: { label: 'Close', action: 'close' } };
  // A failed check shows its own inline Retry (LoadState); the button closes.
  if (checkError) return { kind: 'error', primary: { label: 'Close', action: 'close' } };
  if (!s) return { kind: 'checking', primary: { label: 'Checking…', action: null } };
  switch (s.phase) {
    case 'checking':
      return { kind: 'checking', primary: { label: 'Checking…', action: null } };
    case 'available':
      // This copy can't install (outside /Applications, not writable): say why, nothing to press.
      if (s.blocked) return { kind: 'available', primary: { label: 'Close', action: 'close' } };
      return { kind: 'available', primary: { label: 'Download and install', action: 'download' } };
    case 'downloading':
      return { kind: 'downloading', primary: { label: progressText(s), action: null } };
    case 'staged':
      return { kind: 'staged', primary: { label: 'Restart to finish', action: 'restart' } };
    case 'restarting':
      return { kind: 'restarting', primary: { label: 'Restarting…', action: null } };
    case 'error':
      // A failed download keeps its version: retrying is downloading again.
      return { kind: 'error', primary: s.version && !s.blocked ? { label: 'Try again', action: 'download' } : { label: 'Close', action: 'close' } };
    default:
      return { kind: 'uptodate', primary: { label: 'Close', action: 'close' } };
  }
}

const line = (l: RestartImpactItem): string => `${l.count} ${l.label}${l.detail?.length ? ` (${l.detail.join(', ')})` : ''}`;

/** The restart step: what keeps running, what ends, and the button. An
 *  unknown impact (the count failed) is treated as "something may end". */
export function restartPlan(impact: RestartImpact | null): { keeps: string[]; ends: string[]; endCount: number; label: string; confirm: boolean } {
  const live = (ls: RestartImpactItem[] | undefined) => (ls ?? []).filter((l) => l.count > 0);
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
    void import('@tauri-apps/api/event')
      .then(({ listen }) => listen<UpdateState>('otto://update', (e) => this.apply(e.payload)))
      .catch(() => {});
  }

  apply(s: UpdateState): void {
    const r = s.last_result;
    if (r && this.#toasts) {
      // A success is said once (then forgotten); a rollback stays until dismissed.
      if (r.ok) {
        toasts.success(`Updated to ${r.to}`);
        void call('update_result_seen').catch(() => {});
      } else this.failed = r;
    }
    this.state = s;
    if (s.phase !== 'staged') this.impact = null;
  }

  get row(): string | null {
    return rowLabel(this.state, this.failed);
  }

  /** `check`: check now and say the outcome (Settings' button). Otto ▸ Check
   *  for Updates… opens it without: the shell already started that check. */
  openDialog(check = false): void {
    this.open = true;
    this.actionError = null;
    if (check) void this.check();
    if (this.state?.phase === 'staged') void this.loadImpact();
  }

  close(): void {
    this.open = false;
  }

  /** A check the person asked for; its failure (offline…) arrives as an
   *  error state and is shown, unlike an automatic one's. */
  async check(): Promise<void> {
    if (!isTauri) return;
    this.checkError = null;
    try {
      const s = await call<UpdateState>('update_check');
      this.apply(s);
      if (s.phase === 'error' && !s.version) this.checkError = s.error || 'The check failed.';
    } catch (e) {
      this.checkError = loadErrorText(e);
    }
  }

  async download(): Promise<void> {
    await this.#act(() => call<UpdateState>('update_download'));
  }

  /** Run a shell command that answers with the new state; a throw shows inline. */
  async #act(run: () => Promise<UpdateState>): Promise<void> {
    this.actionError = null;
    try {
      this.apply(await run());
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

  /** "Install updates automatically": download + stage in the background. */
  async setAuto(on: boolean): Promise<void> {
    await this.#act(() => call<UpdateState>('update_set_auto', { on }));
    if (this.actionError) toasts.error('Couldn’t save the update setting', this.actionError);
  }

  /** Skip `version` (hidden from the sidebar), or unskip with null. */
  async skip(version: string | null): Promise<void> {
    if (version) this.open = false;
    await this.#act(() => call<UpdateState>('update_skip', { version }));
  }

  /** Reveal the finisher's log in Finder. */
  async showLog(): Promise<void> {
    const log = this.failed?.log;
    if (!log) return;
    try {
      await call('plugin:opener|reveal_item_in_dir', { paths: [log] });
    } catch (e) {
      toasts.error('Couldn’t show the update log', loadErrorText(e));
    }
  }

  dismissFailed(): void {
    this.failed = null;
    this.open = false;
    void call('update_result_seen').catch(() => {});
  }
}

export const updates = new UpdatesStore();
