// Threads in the ⌘K bar (components/FloatingBar.svelte). lib/ never imports
// modules/, so Threads installs its switcher source here while its page is
// mounted (modules/threads/threadSwitcherSource.svelte.ts): every live thread,
// what needs you first, a "New thread: <query>" row and ⌘↵ for beside.
// Anywhere else the palette falls back to `needsYouRows`: only the threads
// waiting on the person, from the stores the shell already keeps.
import type { Command } from './commands.svelte';
import { router } from './router.svelte';
import { filterThreadRows, frozenOrder } from './threadSwitch';
import { inThreadsScope } from './stores/sessionBuckets';
import { runtimeWaiting } from './stores/runtimeAttention.svelte';
import { ws } from './stores/workspace.svelte';

export type PaletteThreadTone = 'warning' | 'danger' | 'success' | null;

export interface PaletteThreadRow {
  id: string;
  title: string;
  /** The project (folder) name; '' when unknown. */
  project: string;
  /** One status word (Approval, Question, Needs you, Failed, Done, Working) or a relative time. */
  word: string;
  tone: PaletteThreadTone;
}

export interface PaletteThreadSource {
  /** The group's heading. */
  label: string;
  /** In switcher order (most urgent first). */
  rows(): PaletteThreadRow[];
  /** Enter: in the focused pane; ⌘↵ (`beside`): beside it when a split is open. */
  open(id: string, beside: boolean): void;
  /** "New thread: <query>" (Threads only). */
  newThread?(draft: string): void;
}

class PaletteThreads {
  source = $state<PaletteThreadSource | null>(null);

  /** Install a source; returns the uninstall. */
  install(s: PaletteThreadSource): () => void {
    this.source = s;
    return () => {
      if (this.source === s) this.source = null;
    };
  }
}

export const paletteThreads = new PaletteThreads();

const projectOf = (cwd: string | null | undefined): string => (cwd ?? '').replace(/\/+$/, '').split('/').pop() ?? '';

/** Outside Threads: the threads waiting on the person, newest first. */
export const needsYouSource: PaletteThreadSource = {
  label: 'Threads needing you',
  rows() {
    const out: PaletteThreadRow[] = runtimeWaiting().map((w) => ({ id: w.id, title: w.title || 'Thread', project: '', word: 'Needs you', tone: 'warning' }));
    const seen = new Set(out.map((r) => r.id));
    for (const s of ws.sessions) {
      if (s.archived || seen.has(s.id) || ws.needsYou[s.id] !== true || !inThreadsScope(s, ws.sessions)) continue;
      out.push({ id: s.id, title: s.title, project: projectOf(s.cwd), word: 'Needs you', tone: 'warning' });
    }
    return out;
  },
  open(id) {
    router.go(`threads/${encodeURIComponent(id)}`);
  },
};

/** The bar's thread rows as commands: `query` is null while the bar is
 *  closed. The first group in order (frozen at `freeze`), at most 8 (5 when
 *  typing), each `thread:<id>` with its status word as the group and the
 *  project as the detail; `tail` is "New thread: <query>" (Threads only). */
export function threadCommands(query: () => string | null, close: () => void) {
  // Threads' page stays mounted while hidden: its source counts only while it shows.
  const inThreads = $derived(router.module === 'threads' ? paletteThreads.source : null);
  const source = $derived(inThreads ?? needsYouSource);
  let frozen: string[] = [];
  const rows = $derived.by(() => {
    const q = query();
    return q === null ? [] : filterThreadRows(frozenOrder(source.rows(), frozen), q).slice(0, q.trim() ? 5 : 8);
  });
  const open = (id: string, beside: boolean): void => {
    const src = source;
    close();
    src.open(id, beside);
  };
  const commands: Command[] = $derived(
    rows.map((r) => ({ id: `thread:${r.id}`, title: r.title, detail: r.project || undefined, group: r.word, run: () => open(r.id, false) })),
  );
  const tail: Command[] = $derived.by(() => {
    const q = query()?.trim() ?? '';
    const src = inThreads;
    if (!q || !src?.newThread) return [];
    return [{ id: 'thread:new', title: `New thread: ${q}`, group: 'Threads', run: () => (close(), src.newThread?.(q)) }];
  });
  return {
    get commands() { return commands; },
    get tail() { return tail; },
    get label() { return source.label; },
    /** The status word's tone for a `thread:<id>` row. */
    tone(id: string): PaletteThreadTone {
      return rows.find((r) => `thread:${r.id}` === id)?.tone ?? null;
    },
    freeze(): void {
      frozen = source.rows().map((r) => r.id);
    },
    /** ⌘↵ on a thread row: beside the focused one (when a split is open); false for any other row. */
    openBeside(cmdId: string): boolean {
      if (!cmdId.startsWith('thread:') || cmdId === 'thread:new') return false;
      open(cmdId.slice('thread:'.length), true);
      return true;
    },
  };
}
