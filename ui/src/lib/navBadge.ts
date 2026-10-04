// The one answer to "what badge does this navigation entry carry?" — shared by
// the Rail, the Navigator and the phone BottomNav so the three never disagree on
// the count, the tone or the words a screen reader hears. A session waiting on
// you outranks "working" for the same entry.
import { ws } from './stores/workspace.svelte';
import { assistant } from './stores/assistant.svelte';

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
  } else if (id === 'assistant') {
    if (assistant.needsYouCount > 0) return badge(assistant.needsYouCount, 'needs', 'waiting on you');
  } else if (id === 'workflows') {
    const n = ws.activeWorkflowRuns.length;
    if (n > 0) return badge(n, 'working', 'running');
  }
  return null;
}
