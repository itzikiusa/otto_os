// What the person asked Otto, per turn / variants run. The daemon's turn
// record carries the agent's side (status, summary, citations, findings) but
// not the request text, so the Otto panel remembers the asks it sent — and the
// lobby hand-off records the brief the same way — to render the thread as a
// conversation. Per device and best effort (localStorage, try/catch): after a
// reload on another Mac the thread falls back to the committed version's
// `provenance.prompt_summary`.

import type { Intent } from './model';

export interface Ask {
  /** turn_id (a single turn) or run_id (a variants run). */
  key: string;
  artifactId: string;
  /** What the agent was sent. */
  prompt: string;
  /** How the ask reads in the thread (a quick action's label); falls back to `prompt`. */
  display?: string;
  intent: Intent;
  /** "Section: Hero" when the ask focused a selection. */
  selectionLabel: string | null;
  at: string;
}

const STORE_KEY = 'otto.design.assist.asks.v1';
const CAP = 300;

function read(): Ask[] {
  try {
    const raw = localStorage.getItem(STORE_KEY);
    const v = raw ? (JSON.parse(raw) as unknown) : [];
    return Array.isArray(v) ? (v as Ask[]).filter((a) => a && typeof a.key === 'string') : [];
  } catch {
    return [];
  }
}

class AskLog {
  private list: Ask[] = $state(read());

  get(key: string): Ask | null {
    return this.list.find((a) => a.key === key) ?? null;
  }

  add(a: Ask): void {
    this.list = [a, ...this.list.filter((x) => x.key !== a.key)].slice(0, CAP);
    try {
      localStorage.setItem(STORE_KEY, JSON.stringify(this.list));
    } catch {
      /* private window / blocked storage: remembered for this page only */
    }
  }
}

export const asks = new AskLog();
