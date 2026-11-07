// Agent UI control — Personal Agents page ports (the ./pagePort.ts pattern).
// Kept apart from ./personalAgents.ts, and dependency-free, so the pages bind
// them without pulling the handler file (or pagePort's helpers) into their
// chunk; the handlers wait on `peek()` with pagePort's `waitFor`.

import type { PersonalAgent, PersonalAgentRun } from '../api/types';
import type { PaCreateFill } from './personalAgentsPlan';

/** A page-bound API: the page `bind`s it from an `$effect` (returns the unbind). */
export interface PaPort<T> {
  bind(impl: T): () => void;
  peek(): T | null;
}

function port<T>(): PaPort<T> {
  let impl: T | null = null;
  return {
    bind(next: T) {
      impl = next;
      return () => {
        if (impl === next) impl = null;
      };
    },
    peek: () => impl,
  };
}

/** The agents list page: opens the New-agent sheet. */
export const paListPort = port<{ openCreate(): void }>();

/** The New-agent sheet (create mode only): fill visibly, then save or close. */
export const paSheetPort = port<{
  fill(f: PaCreateFill): void;
  /** Runs the sheet's own Save; `agent` null = not created (`error` says why). */
  submit(): Promise<{ agent: PersonalAgent | null; error: string }>;
  close(): void;
}>();

/** One agent's page. */
export const paAgentPort = port<{
  agentId: string;
  /** Re-read the Autonomy tab (after a save made outside it). */
  reloadAutonomy(): void;
  /** Open a run's report in the page's report sheet. */
  viewReport(run: PersonalAgentRun): void;
}>();

/** The agent settings page's sections (`#/personal-agents/<id>/<tab>`). */
export const PA_TABS = ['about', 'activity', 'memory'] as const;
export type PaTab = (typeof PA_TABS)[number];
/** A URL slug as a section. Older slugs (notifications, `otto.ui_pa_open`,
 *  bookmarks) map to the section that holds them now; anything else → `about`. */
export function paTab(slug: string | undefined): PaTab {
  if ((PA_TABS as readonly string[]).includes(slug ?? '')) return slug as PaTab;
  return slug === 'runs' || slug === 'history' ? 'activity' : slug === 'knowledge' || slug === 'context' ? 'memory' : 'about';
}
