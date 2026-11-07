import type { Session } from './api/types';
import { router } from './router.svelte';
import { isForeground } from './stores/sessionBuckets';
import { ws } from './stores/workspace.svelte';
import { toastError } from './toastError';

/** Only existing coding-agent PTYs have both a terminal and transcript view.
 * Structured threads use a different runtime; moving those would require
 * spawning a second writer to the native conversation. */
export function canSwitchSessionSurface(s: Session | null | undefined): s is Session {
  return !!s && s.kind === 'agent' && !s.archived && isForeground(s)
    && s.meta?.runtime !== 'structured' && !s.meta?.side_of && !s.meta?.threads_terminal
    && (s.provider !== 'shell' || typeof s.meta?.nested_provider === 'string');
}

/** Change the view, never the process: the id, provider session, permissions,
 * approvals and delegated workers stay attached to the original session.
 * The server shallow-merges this one metadata key. */
export async function openSessionSurface(s: Session, surface: 'agents' | 'threads'): Promise<boolean> {
  if (!canSwitchSessionSurface(s) || ws.myRole === 'viewer') return false;
  try {
    await ws.updateSessionMeta(s.id, { surface });
    router.go(`${surface}/${s.id}`);
    return true;
  } catch (e) {
    toastError(`Couldn’t open ${surface === 'threads' ? 'Threads' : 'terminal'}`, e);
    return false;
  }
}
