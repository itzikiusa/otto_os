// Getting to the thread that needs you in one move: every Threads list row
// derives its state here (lib/status.ts `sessionState` over the row, which
// carries a runtime thread's pending request and last turn outcome), the
// sidebar publishes the order it shows threads in, and "Next that needs
// you" / next / previous thread walk that order. `openNextNeedsYou` is the
// one call other pages use (Home, a notification): it works before Threads
// has loaded, from the persisted waiting list. Logic is pure in
// threadAttention.ts (unit/threadsAttention.test.ts).
import type { Session } from '../../lib/api/types';
import { mcpAsks } from './runtime/mcpAsksStore.svelte';
import { events } from '../../lib/events.svelte';
import { keyContext } from '../../lib/keys';
import { router } from '../../lib/router.svelte';
import { sessionState, type SessionStateInfo } from '../../lib/status';
import { isRuntimeWaiting, runtimeWaiting } from '../../lib/stores/runtimeAttention.svelte';
import { ui } from '../../lib/stores/ui.svelte';
import { viewport } from '../../lib/stores/viewport.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { initialThread, nextNeedsYou, stepThread, threadChord, urgency } from './threadAttention';
import { asThreadSession, runtimeThreads } from './runtime/runtimeThreads.svelte';
import { withBackgroundState } from './runtime/taskStop';
import { threads } from './threadsStore.svelte';

/** A thread waits on the person: its runtime row has a pending request, or
 *  the sticky flag is up (a terminal thread's prompt, a live event). */
export function threadNeedsYou(id: string): boolean {
  return ws.needsYou[id] === true || runtimeThreads.get(id)?.status === 'awaiting_permission' || isRuntimeWaiting(id);
}

/** The one state a Threads list row shows for `s` (dot, label, tooltip). */
export function threadRowState(s: Session): SessionStateInfo {
  const needs = ws.needsYou[s.id] === true || runtimeThreads.get(s.id)?.status === 'awaiting_permission' || mcpAsks.waiting(s.id);
  return withBackgroundState(
    sessionState(s, ws.statusMap[s.id] ?? s.status, needs, { stale: events.state !== 'connected' }),
    runtimeThreads.get(s.id)?.background_tasks?.length ?? 0,
  );
}

/** `threadRowState` for a thread known only by id (a family's child); null
 *  while its row has not loaded. */
export function threadStateById(id: string): SessionStateInfo | null {
  const rt = runtimeThreads.get(id);
  const s = ws.getSession(id) ?? (rt ? asThreadSession(rt) : null);
  return s ? threadRowState(s) : null;
}

/** Sort key for the inbox order (threadFamilies.ts `inboxOrder`). */
export function threadUrgency(s: Session): number {
  return urgency(threadRowState(s));
}

class ThreadNav {
  /** The live threads in the order the sidebar shows them (set by ThreadSidebar). */
  order = $state<string[]>([]);
}

export const threadNav = new ThreadNav();

function open(id: string): void {
  if (router.module !== 'threads') router.go(`threads/${encodeURIComponent(id)}`);
  else threads.selectThread(id);
}

/** The order to walk: the sidebar's when it is mounted, else the waiting
 *  threads, most recent first. */
function walkOrder(): string[] {
  return threadNav.order.length > 0 ? threadNav.order : runtimeWaiting().map((w) => w.id);
}

/** The thread "Next that needs you" opens (never the one in front of you). */
export function nextNeedsYouId(): string | null {
  const current = router.module === 'threads' && !threads.newDraftMode ? threads.focusedSessionId : null;
  return (
    nextNeedsYou(walkOrder(), current, threadNeedsYou) ??
    runtimeWaiting().find((w) => w.id !== current)?.id ??
    null
  );
}

/** Open the next thread that needs you; false when none other does. */
export function openNextNeedsYou(): boolean {
  const id = nextNeedsYouId();
  if (!id) return false;
  open(id);
  return true;
}

/** Open the next (1) / previous (-1) thread in the sidebar's order. */
export function openAdjacentThread(dir: 1 | -1): boolean {
  const id = stepThread(threadNav.order, threads.focusedSessionId, dir);
  if (!id || id === threads.focusedSessionId) return false;
  open(id);
  return true;
}

/** The thread Threads opens on with nothing in the route: on a phone, the
 *  most recent one that needs you; otherwise (and when none does) the most
 *  recently active one. `liveNewestFirst`: the live threads, newest first. */
export function initialThreadId(liveNewestFirst: readonly string[]): string | null {
  if (!viewport.isPhone) return liveNewestFirst[0] ?? null;
  const flagged = liveNewestFirst.filter((id) => ws.needsYou[id] === true || runtimeThreads.get(id)?.status === 'awaiting_permission');
  return initialThread(liveNewestFirst, [...runtimeWaiting().map((w) => w.id), ...flagged]);
}

/** The Threads chords, shown in titles and ⌘K. ⌥⌘ chords: the app's global
 *  map has none (keys.ts drops ⌥-augmented ⌘ chords), the browser and the
 *  desktop shell bind no ⌥⌘↑/↓/N, and a focused terminal or editor keeps them. */
export const THREAD_KEYS = { next: '⌥⌘↓', previous: '⌥⌘↑', needsYou: '⌥⌘N' } as const;

function onThreadKey(e: KeyboardEvent): void {
  if (router.module !== 'threads' || ui.overlayOpen || keyContext.terminalFocused) return;
  const chord = threadChord(e);
  if (!chord) return;
  const el = document.activeElement as HTMLElement | null;
  if (el?.closest?.('.xterm, .cm-editor')) return;
  e.preventDefault();
  if (chord === 'needsYou') openNextNeedsYou();
  else openAdjacentThread(chord === 'next' ? 1 : -1);
}

// Installed once, with the Threads code: works from anywhere on the Threads
// page, with the sidebar hidden or folded into the phone's drawer too.
if (typeof window !== 'undefined') window.addEventListener('keydown', onThreadKey);
