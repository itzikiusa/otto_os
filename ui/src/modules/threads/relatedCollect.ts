// Reads the stores for relatedRows (relatedThreads.ts): the header chip's
// count and the popover's list come from this one call, inside a $derived.
import { ws } from '../../lib/stores/workspace.svelte';
import { sameCouncil } from './runtime/council';
import { runtimeThreads } from './runtime/runtimeThreads.svelte';
import { relatedRows, type RelatedRow, type RelatedSource } from './relatedThreads';
import { anyThread } from './threadRelay';
import { sideChatFor } from './threadSideChat';
import { delegatedParentId } from './threadsHelpers';

function source(id: string | null): RelatedSource | null {
  const s = anyThread(id);
  return s ? { id: s.id, title: s.title, provider: s.provider, status: runtimeThreads.get(s.id)?.status ?? ws.statusMap[s.id] ?? s.status } : null;
}

export function collectRelated(threadId: string | null): RelatedRow[] {
  if (!threadId) return [];
  return relatedRows(threadId, {
    parent: source(delegatedParentId(ws.getSession(threadId))),
    linked: runtimeThreads
      .group(threadId)
      .filter((r) => !sameCouncil(runtimeThreads.get(threadId), r))
      .map((r) => ({ id: r.id, title: r.title, provider: r.provider, status: r.status })),
    workers: ws.sessions.filter((s) => !s.archived && delegatedParentId(s) === threadId).flatMap((s) => source(s.id) ?? []),
    side: source(sideChatFor(threadId)?.id ?? null),
  });
}
