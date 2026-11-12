// Session-to-session relay (Related threads › Message…, relatedMessage.ts). A runtime
// thread takes the message through its own prompt route
// (POST /runtime-sessions/{id}/prompt, queued while a turn runs) and shows it
// as "From <thread>"; a terminal thread through /sessions/{id}/message. A
// suspended terminal thread can't take a message: the person is asked to
// revive it first instead of seeing the route's raw 409.
import type { Session } from '../../lib/api/types';
import { confirmer } from '../../lib/confirm.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { asThreadSession, runtimeThreads } from './runtime/runtimeThreads.svelte';
import { legacyReviveItem, reviveLegacyThread } from './threadMenu';
import { threadOrigin } from './threadOrigins';
import { isThreadSession } from './threadsHelpers';
import { threads } from './threadsStore.svelte';

/** A thread by id, terminal or runtime (as a sidebar row). */
export function anyThread(id: string | null): Session | null {
  if (!id) return null;
  const runtime = runtimeThreads.get(id);
  return ws.getSession(id) ?? (runtime ? asThreadSession(runtime) : null);
}

/** Live threads a relay from `fromId` can reach: terminal threads and the
 *  person's own runtime threads (not agent runs or other modules' helpers). */
export function relayTargets(fromId: string | null): Session[] {
  return [
    ...ws.sessions.filter((s) => !s.archived && isThreadSession(s)),
    ...runtimeThreads.rows().filter((s) => threadOrigin(s) === 'main'),
  ].filter((s) => s.id !== fromId);
}

export async function relayToThread(opts: { fromSessionId: string | null; target: string; text: string; workspaceId: string | null }): Promise<boolean> {
  const fromTitle = anyThread(opts.fromSessionId)?.title;
  const runtime = opts.target === 'broadcast' ? null : runtimeThreads.get(opts.target);
  if (runtime) {
    return threads.sendPeerMessage({
      ...opts,
      fromTitle,
      targetTitle: runtime.title,
      // The daemon's own "message from" form: the timeline shows it as "From <thread>".
      via: (msg, from) => runtimeThreads.prompt(runtime.id, { text: from ? `[Message from ${from}]: ${msg}` : msg, delivery: 'queue' }),
    });
  }
  const target = opts.target === 'broadcast' ? null : ws.getSession(opts.target);
  if (target && legacyReviveItem(target).length > 0) {
    const ok = await confirmer.ask(`“${target.title}” is suspended, so it can’t take a message. Revive it and send?`, {
      title: 'Revive thread?',
      confirmLabel: 'Revive and send',
    });
    if (!ok || !(await reviveLegacyThread(target, false))) return false;
  }
  return threads.sendPeerMessage({ ...opts, fromTitle });
}
