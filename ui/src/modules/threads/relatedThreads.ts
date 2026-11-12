// The "Related threads" popover's rows, in one place: the threads an agent
// linked (`otto.thread_open`), the parent / workers of a delegated terminal
// thread, and the thread's side chat. Rooms are listed beside them by the
// popover itself (they come from GET /rooms). Pure, so the ordering and
// de-duplication are unit-tested (unit/relatedThreads.test.ts).
// Ported in spirit from T3 Code's ThreadRelationshipsControl.tsx: one control
// for every thread this one is connected to, each with its live status.

export type RelatedKind = 'linked' | 'parent' | 'worker' | 'side';

export interface RelatedSource {
  id: string;
  title: string;
  provider: string;
  /** Runtime status (`working`, `idle`, `awaiting_permission`, `closed`…) or a terminal session's. */
  status: string;
}

export interface RelatedRow extends RelatedSource {
  kind: RelatedKind;
}

export const RELATED_KIND_LABEL: Record<RelatedKind, string> = {
  linked: 'Linked',
  parent: 'Parent',
  worker: 'Worker',
  side: 'Side chat',
};

/** Every thread related to `selfId`, each once: a parent first, then linked
 *  threads and workers (oldest first, as given), then the side chat. A
 *  closed linked thread is left out (it sits in History). */
export function relatedRows(
  selfId: string,
  parts: { linked?: RelatedSource[]; parent?: RelatedSource | null; workers?: RelatedSource[]; side?: RelatedSource | null },
): RelatedRow[] {
  const out: RelatedRow[] = [];
  const seen = new Set([selfId]);
  const add = (kind: RelatedKind, s: RelatedSource | null | undefined): void => {
    if (!s || seen.has(s.id)) return;
    seen.add(s.id);
    out.push({ ...s, kind });
  };
  add('parent', parts.parent);
  for (const s of parts.linked ?? []) if (s.status !== 'closed') add('linked', s);
  for (const s of parts.workers ?? []) add('worker', s);
  add('side', parts.side);
  return out;
}

/** The status in words, for the row and its dot (`data-status`). */
export function relatedStatus(status: string): { tone: 'live' | 'ask' | 'idle' | 'off'; text: string } {
  switch (status) {
    case 'working':
    case 'starting':
    case 'running':
      return { tone: 'live', text: 'Working' };
    case 'awaiting_permission':
    case 'needs_input':
      return { tone: 'ask', text: 'Needs you' };
    case 'idle':
    case 'ready':
      return { tone: 'idle', text: 'Idle' };
    case 'closed':
      return { tone: 'off', text: 'Closed' };
    default:
      return { tone: 'off', text: status ? status.charAt(0).toUpperCase() + status.slice(1).replaceAll('_', ' ') : 'Unknown' };
  }
}
