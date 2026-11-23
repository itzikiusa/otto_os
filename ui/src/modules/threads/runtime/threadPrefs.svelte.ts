// Per-device Threads prefs, set from the toolbar ⋯ › View (threadsPageMenu.ts)
// and kept in localStorage: what a send does while the agent works (Queue,
// the default, or Steer — composerSend.ts).
import type { MidTurn } from './composerSend';

const KEY = 'otto_threads_prefs_v1';

class ThreadPrefs {
  midTurn = $state<MidTurn>('queue');

  constructor() {
    try {
      const v = JSON.parse(localStorage.getItem(KEY) ?? '{}') as { midTurn?: unknown };
      if (v.midTurn === 'steer') this.midTurn = 'steer';
    } catch {
      /* none saved, or private mode */
    }
  }

  setMidTurn(m: MidTurn): void {
    this.midTurn = m;
    this.save();
  }

  private save(): void {
    try {
      localStorage.setItem(KEY, JSON.stringify({ midTurn: this.midTurn }));
    } catch {
      /* private mode */
    }
  }
}

export const threadPrefs = new ThreadPrefs();
