// Per-device Threads prefs, kept in localStorage and set from Settings ›
// Threads (ThreadsSettings.svelte) and the toolbar ⋯ › View shortcuts
// (threadsPageMenu.ts): what a send does while the agent works (Queue, the
// default, or Steer — composerSend.ts), how many tool calls the timeline
// shows (workLog.ts visibleTools; default all), and where a subagent opens
// (the subagent views read it; default the side panel).
import type { MidTurn } from './composerSend';
import type { ToolVisibility } from './workLog';

const KEY = 'otto_threads_prefs_v1';
const TOOLS: readonly ToolVisibility[] = ['all', 'hide-ok', 'none'];

export type SubagentOpenIn = 'panel' | 'main';

class ThreadPrefs {
  midTurn = $state<MidTurn>('queue');
  tools = $state<ToolVisibility>('all');
  subagentIn = $state<SubagentOpenIn>('panel');

  constructor() {
    try {
      const v = JSON.parse(localStorage.getItem(KEY) ?? '{}') as { midTurn?: unknown; tools?: unknown; subagentIn?: unknown };
      if (v.midTurn === 'steer') this.midTurn = 'steer';
      if (TOOLS.includes(v.tools as ToolVisibility)) this.tools = v.tools as ToolVisibility;
      if (v.subagentIn === 'main') this.subagentIn = 'main';
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

  setSubagentIn(s: SubagentOpenIn): void {
    this.subagentIn = s;
    this.save();
  }

  private save(): void {
    try {
      localStorage.setItem(KEY, JSON.stringify({ midTurn: this.midTurn, tools: this.tools, subagentIn: this.subagentIn }));
    } catch {
      /* private mode */
    }
  }
}

export const threadPrefs = new ThreadPrefs();
