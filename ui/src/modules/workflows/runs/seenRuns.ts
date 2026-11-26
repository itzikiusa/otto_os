// "Unread" finished runs, per device (ux.md §2.1): a run that finished after
// the person last looked at the Runs inbox and that they never opened shows
// Done / Failed in bold. Nothing durable depends on it (`localStorage`, a
// per-viewer nicety: every access is wrapped).
const KEY = 'otto.wf.seenRuns';
const MAX_OPENED = 200;

interface Seen {
  /** When the person last left the inbox (epoch ms). */
  at: number;
  opened: string[];
}

function read(): Seen {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? 'null') as Seen | null;
    if (v && typeof v.at === 'number' && Array.isArray(v.opened)) return v;
  } catch { /* unreadable: nothing is unread */ }
  return { at: Date.now(), opened: [] };
}

function save(v: Seen): void {
  try { localStorage.setItem(KEY, JSON.stringify(v)); } catch { /* per-viewer nicety only */ }
}

/** A snapshot for one inbox visit: `unread(id, finishedAt)`. */
export function seenRuns(): (id: string, finishedAt: string | null) => boolean {
  const v = read();
  const opened = new Set(v.opened);
  return (id, finishedAt) => !!finishedAt && Date.parse(finishedAt) > v.at && !opened.has(id);
}

/** The person leaves the inbox: what finished until now is read. */
export function markInboxSeen(): void {
  save({ ...read(), at: Date.now() });
}

/** The person opened this run. */
export function markRunOpened(id: string): void {
  const v = read();
  if (v.opened.includes(id)) return;
  save({ ...v, opened: [id, ...v.opened].slice(0, MAX_OPENED) });
}
