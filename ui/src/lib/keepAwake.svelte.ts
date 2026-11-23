// Keep awake (the desktop shell's keep_awake.rs): keep this Mac and its
// display awake for a while, or until turned off — a lighter `caffeinate`.
// The shell holds the power assertions and the truth; this store mirrors it
// for the status bar item and the ⌘K commands, and broadcasts arrive as
// `otto://keep-awake`. Outside the desktop app it stays off and inert.

import { isTauri } from './desktop';
import { confirmer } from './confirm.svelte';
import { toastError } from './toastError';

export interface KeepAwakeState {
  active: boolean;
  /** Epoch ms when it turns itself off; null = until turned off. */
  until_ms: number | null;
}

/** Time left like the menu-bar title: "42m", "1h", "1h 5m" (minutes rounded
 *  up, never "0m" while on); null = until turned off. */
export function timeLeft(s: KeepAwakeState, now: number): string | null {
  if (s.until_ms === null) return null;
  const m = Math.max(1, Math.ceil((s.until_ms - now) / 60_000));
  const h = Math.floor(m / 60);
  return [h && `${h}h`, (m % 60 || !h) && `${m % 60}m`].filter(Boolean).join(' ');
}

/** "90", "90m", "2h", "1h 30m", "1:30" → minutes; null when unreadable. */
export function parseMinutes(input: string): number | null {
  const s = input.trim().toLowerCase();
  const clock = /^(\d+):([0-5]\d)$/.exec(s);
  const hm = /^(?:(\d+)\s*h(?:ours?|rs?)?)?\s*(?:(\d+)\s*(?:m(?:in(?:ute)?s?)?)?)?$/.exec(s);
  const m = clock ? +clock[1] * 60 + +clock[2] : hm && s ? +(hm[1] ?? 0) * 60 + +(hm[2] ?? 0) : 0;
  return m > 0 ? m : null;
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import('@tauri-apps/api/core');
  return invoke<T>(cmd, args);
}

class KeepAwakeStore {
  state = $state<KeepAwakeState>({ active: false, until_ms: null });
  #started = false;

  /** Read the shell's state once and follow its broadcasts (idempotent). */
  init(): void {
    if (!isTauri || this.#started) return;
    this.#started = true;
    void call<KeepAwakeState>('keep_awake_state').then((s) => (this.state = s)).catch(() => {});
    void import('@tauri-apps/api/event')
      .then(({ listen }) => listen<KeepAwakeState>('otto://keep-awake', (e) => (this.state = e.payload)))
      .catch(() => {});
  }

  /** `minutes: null` = until turned off. */
  async start(minutes: number | null): Promise<void> {
    try {
      this.state = await call<KeepAwakeState>('keep_awake_start', { minutes });
    } catch (e) {
      toastError('Couldn’t keep this Mac awake', e);
    }
  }

  async stop(): Promise<void> {
    try {
      this.state = await call<KeepAwakeState>('keep_awake_stop');
    } catch (e) {
      toastError('Couldn’t turn off keep awake', e);
    }
  }

  async promptCustom(): Promise<void> {
    const v = await confirmer.promptText('Keep this Mac and its display awake for how long?', {
      title: 'Keep awake',
      confirmLabel: 'Keep awake',
      placeholder: '45m, 3h, 1h 30m',
      validate: (s) => (parseMinutes(s) ? null : 'Type minutes or hours, like 45m, 3h or 1h 30m.'),
    });
    const minutes = v === null ? null : parseMinutes(v);
    if (minutes) await this.start(minutes);
  }
}

export const keepAwake = new KeepAwakeStore();
