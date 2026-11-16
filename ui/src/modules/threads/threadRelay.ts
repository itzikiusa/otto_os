// Session-to-session relay (Related threads › Message…, relatedMessage.ts). A runtime
// thread takes the message through its own prompt route
// (POST /runtime-sessions/{id}/prompt, queued while a turn runs) and shows it
// as "From <thread>"; a terminal thread through /sessions/{id}/message. A
// suspended terminal thread can't take a message: the person is asked to
// revive it first instead of seeing the route's raw 409. Fan-outs ("All open
// threads in this project", the canvas's "Message every pane…") send one
// message per thread through the same person routes, each settled on its
// own, and say who could not receive it.
import { api } from '../../lib/api/client';
import type { Session } from '../../lib/api/types';
import { confirmer } from '../../lib/confirm.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { toasts } from '../../lib/toast.svelte';
import { paneMessage } from './runtime/paneMessage.svelte';
import { projectPeers, sendSummary } from './runtime/paneStrip';
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

/** Who "All open threads in this project" reaches from `fromId`. */
export function broadcastTargets(fromId: string | null): Session[] {
  const from = anyThread(fromId);
  return from ? projectPeers(from, relayTargets(fromId)) : [];
}

/** Deliver `text` to one thread as the person (queued behind a running
 *  turn); `from` names the thread it is relayed from. */
function deliver(target: Session, text: string, from: Session | null): Promise<unknown> {
  if (runtimeThreads.get(target.id)) {
    // The daemon's own "message from" form: the timeline shows it as "From <thread>".
    return runtimeThreads.prompt(target.id, { text: from ? `[Message from ${from.title.replace(/[[\]]/g, '')}]: ${text}` : text, delivery: 'queue' });
  }
  const terminalFrom = from && ws.getSession(from.id) ? from : null;
  return api.post(`/sessions/${encodeURIComponent(target.id)}/message`, {
    text: from ? `[From ${from.title}] ${text}` : text,
    ...(terminalFrom ? { from_session_id: terminalFrom.id } : {}),
  });
}

/** Send `text` to every target, each on its own; toasts what happened.
 *  Returns how many received it. */
export async function fanOut(targets: readonly Session[], text: string, from: Session | null, noun = 'thread'): Promise<number> {
  const r = await Promise.allSettled(targets.map((t) => deliver(t, text, from)));
  const s = sendSummary(targets.map((t, i) => ({ title: t.title, ok: r[i].status === 'fulfilled' })), noun);
  toasts[s.tone](s.title, s.body);
  return r.filter((x) => x.status === 'fulfilled').length;
}

async function broadcast(fromId: string | null, text: string): Promise<boolean> {
  const from = anyThread(fromId);
  const targets = broadcastTargets(fromId);
  const sent = await fanOut(targets, text.trim(), from);
  if (sent > 0) {
    threads.recordLocalPeerEvent({
      kind: 'broadcast',
      fromSessionId: fromId,
      fromTitle: from?.title ?? 'Operator',
      toSessionId: null,
      toTitle: 'All open threads in this project',
      summary: text.trim(),
    });
  }
  return sent > 0;
}

/** The canvas's "Message every pane…": one instruction to each shown thread. */
export async function messagePanes(paneIds: readonly string[]): Promise<void> {
  const shown = paneIds.map(anyThread).filter((s): s is Session => !!s);
  const open = shown.filter((s) => !s.archived);
  const closed = shown.filter((s) => s.archived).map((s) => `“${s.title}”`);
  if (!open.length) return;
  const text = await paneMessage.ask({
    title: 'Message every pane',
    intro: `${open.length} ${open.length === 1 ? 'thread' : 'threads'} will receive it, each as your next message (queued while one works).`,
    note: closed.length ? `${closed.join(', ')} ${closed.length === 1 ? 'is' : 'are'} closed and won’t receive it.` : undefined,
    confirmLabel: `Send to ${open.length}`,
    placeholder: 'One instruction for every pane…',
  });
  if (text) await fanOut(open, text, null);
}

export async function relayToThread(opts: { fromSessionId: string | null; target: string; text: string; workspaceId: string | null }): Promise<boolean> {
  if (opts.target === 'broadcast') return broadcast(opts.fromSessionId, opts.text);
  const fromTitle = anyThread(opts.fromSessionId)?.title;
  const runtime = runtimeThreads.get(opts.target);
  if (runtime) {
    return threads.sendPeerMessage({
      ...opts,
      fromTitle,
      targetTitle: runtime.title,
      // The daemon's own "message from" form: the timeline shows it as "From <thread>".
      via: (msg, from) => runtimeThreads.prompt(runtime.id, { text: from ? `[Message from ${from}]: ${msg}` : msg, delivery: 'queue' }),
    });
  }
  const target = ws.getSession(opts.target);
  if (target && legacyReviveItem(target).length > 0) {
    const ok = await confirmer.ask(`“${target.title}” is suspended, so it can’t take a message. Revive it and send?`, {
      title: 'Revive thread?',
      confirmLabel: 'Revive and send',
    });
    if (!ok || !(await reviveLegacyThread(target, false))) return false;
  }
  return threads.sendPeerMessage({ ...opts, fromTitle });
}
