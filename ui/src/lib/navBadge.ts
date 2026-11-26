// The one answer to "what badge does this navigation entry carry?" — shared by
// the Rail, the Navigator and the phone BottomNav so the three never disagree on
// the count, the tone or the words a screen reader hears. A session waiting on
// you outranks "working" for the same entry.
import type { RunState } from './api/types';
import { ws } from './stores/workspace.svelte';

/** Run states that wait on a person (a held run blocked or out of budget too). */
const RUN_NEEDS: ReadonlySet<RunState> = new Set(['needs_you', 'blocked', 'exhausted']);

export interface NavBadge {
  count: number;
  /** `needs` = waiting on you (warning), `working` = running (accent). */
  tone: 'needs' | 'working';
  /** Tooltip text, e.g. "2 waiting on you". */
  title: string;
  /** Appended to the entry's accessible name, e.g. ", 2 waiting on you". */
  spoken: string;
}

function badge(count: number, tone: NavBadge['tone'], phrase: string): NavBadge {
  const title = `${count} ${phrase}`;
  return { count, tone, title, spoken: `, ${title}` };
}

/** Reads reactive stores — call it from a template or `$derived`. */
export function navBadge(id: string): NavBadge | null {
  if (id === 'agents') {
    if (ws.needsYouCount > 0) return badge(ws.needsYouCount, 'needs', 'waiting on you');
    if (ws.workingCount > 0) return badge(ws.workingCount, 'working', 'working');
  } else if (id === 'threads') {
    if (ws.threadsNeedsYouCount > 0) return badge(ws.threadsNeedsYouCount, 'needs', 'waiting on you');
    if (ws.threadsWorkingCount > 0) return badge(ws.threadsWorkingCount, 'working', 'working');
  } else if (id === 'workflows') {
    // Active runs: the ones waiting on you outrank the running ones; a paused
    // run is neither (an older daemon sends no overview: running).
    const runs = ws.activeWorkflowRuns;
    const needs = runs.filter((r) => r.overview && RUN_NEEDS.has(r.overview.state)).length;
    if (needs > 0) return badge(needs, 'needs', 'waiting on you');
    const running = runs.filter((r) => r.overview?.state !== 'paused').length;
    if (running > 0) return badge(running, 'working', 'running');
  }
  return null;
}
