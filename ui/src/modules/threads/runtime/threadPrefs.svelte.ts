// Per-device Threads prefs, set from the toolbar ⋯ › View (threadsPageMenu.ts)
// and kept in localStorage: what a send does while the agent works (Queue,
// the default, or Steer — composerSend.ts), and how many tool calls the
// timeline shows (workLog.ts visibleTools; default all).
import type { MidTurn } from './composerSend';
import type { ToolVisibility } from './workLog';

const KEY = 'otto_threads_prefs_v1';
const TOOLS: readonly ToolVisibility[] = ['all', 'hide-ok', 'none'];

class ThreadPrefs {
  midTurn = $state<MidTurn>('queue');
  tools = $state<ToolVisibility>('all');

  constructor() {
    try {
      const v = JSON.parse(localStorage.getItem(KEY) ?? '{}') as { midTurn?: unknown; tools?: unknown };
      if (v.midTurn === 'steer') this.midTurn = 'steer';
      if (TOOLS.includes(v.tools as ToolVisibility)) this.tools = v.tools as ToolVisibility;
    } catch {
      /* none saved, or private mode */
    }
  }

  setMidTurn(m: MidTurn): void {
    this.midTurn = m;
    this.save();
  }

  setTools(t: ToolVisibility): void {
    this.tools = t;
    this.save();
  }

  private save(): void {
    try {
      localStorage.setItem(KEY, JSON.stringify({ midTurn: this.midTurn, tools: this.tools }));
    } catch {
      /* private mode */
    }
  }
}

export const threadPrefs = new ThreadPrefs();
