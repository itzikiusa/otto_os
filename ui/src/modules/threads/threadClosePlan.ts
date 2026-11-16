// What closing a thread asks (runtime/runtimeMenu.ts): a thread with open
// child threads (`meta.child_of`, docs/features/threads-structured-runtime.md
// "Threads inside a thread") says they keep running and offers to close them
// too. Import-free, unit-tested in unit/threadsStage.test.ts.

interface ThreadRow {
  id: string;
  status: string;
  meta?: Record<string, unknown> | null;
}

const parentOf = (r: ThreadRow): string | null => {
  const c = r.meta?.child_of as { id?: unknown } | undefined;
  return c && typeof c.id === 'string' ? c.id : null;
};

/** The open (not closed) threads opened from `id`, their own children
 *  included, each once, nearest first. */
export function openDescendants<R extends ThreadRow>(rows: readonly R[], id: string): R[] {
  const out: R[] = [];
  const seen = new Set([id]);
  let frontier = [id];
  while (frontier.length) {
    const next: string[] = [];
    for (const r of rows) {
      const p = parentOf(r);
      if (p && frontier.includes(p) && !seen.has(r.id)) {
        seen.add(r.id);
        next.push(r.id);
        if (r.status !== 'closed') out.push(r);
      }
    }
    frontier = next;
  }
  return out;
}

export type CloseChoice = 'this' | 'all';

/** The dialog for closing a thread with `n` open children (n ≥ 1). */
export function closeWithChildrenDialog(title: string, n: number): {
  title: string;
  message: string;
  options: { label: string; value: CloseChoice; kind: 'normal' | 'danger' }[];
} {
  const kids = n === 1 ? '1 thread opened from it keeps running.' : `${n} threads opened from it keep running.`;
  return {
    title: 'Close thread?',
    message: `This stops the agent of “${title}” and moves it to History, where you can reopen it. ${kids}`,
    options: [
      { label: 'Close only this', value: 'this', kind: 'normal' },
      { label: `Close all ${n + 1}`, value: 'all', kind: 'danger' },
    ],
  };
}
