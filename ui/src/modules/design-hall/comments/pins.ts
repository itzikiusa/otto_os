// Review comments on a design — the pure half: pin maths (a point on the
// rendered stage as FRACTIONS of its content box, so a pin stays on the same
// spot when the pane resizes or the device frame changes), threading (one
// level of replies), numbering and which pins the shown version draws.
// No DOM, no API: unit-tested in ui/unit/designComments.test.ts.

import type { DesignComment, DesignCommentPin } from '../../../lib/api/types';

export interface Box {
  left: number;
  top: number;
  width: number;
  height: number;
}

const clamp01 = (v: number): number => (Number.isFinite(v) ? Math.min(1, Math.max(0, v)) : 0);
/** 4 decimals: sub-pixel on any real stage, and a stable value to compare. */
const round4 = (v: number): number => Math.round(v * 10_000) / 10_000;

/** A pointer position → a pin on `box` (clamped onto it); null for an empty box. */
export function pinAt(clientX: number, clientY: number, box: Box): DesignCommentPin | null {
  if (!(box.width > 0) || !(box.height > 0)) return null;
  return { x: round4(clamp01((clientX - box.left) / box.width)), y: round4(clamp01((clientY - box.top) / box.height)) };
}

/** Where a pin draws, in px inside `box`'s own coordinates. */
export function pinPoint(pin: DesignCommentPin, box: Pick<Box, 'width' | 'height'>): { x: number; y: number } {
  return { x: clamp01(pin.x) * box.width, y: clamp01(pin.y) * box.height };
}

/** Nudge a pin with the keyboard (arrow keys move a draft pin by `step`). */
export function nudge(pin: DesignCommentPin, dx: number, dy: number, step = 0.01): DesignCommentPin {
  return { ...pin, x: round4(clamp01(pin.x + dx * step)), y: round4(clamp01(pin.y + dy * step)) };
}

/** "42% across, 18% down" — for screen readers and the agent brief. */
export function pinWords(pin: DesignCommentPin): string {
  return `${Math.round(clamp01(pin.x) * 100)}% across, ${Math.round(clamp01(pin.y) * 100)}% down`;
}

export interface Thread {
  root: DesignComment;
  replies: DesignComment[];
  /** 1-based, by creation order across every thread — stable as threads resolve. */
  n: number;
}

/** Group comments into threads (oldest first, replies under their root).
 *  A reply whose root is missing (a page cut) is dropped, never shown as a root. */
export function threadsOf(comments: readonly DesignComment[]): Thread[] {
  const roots = comments.filter((c) => !c.parent_id).slice().sort(byCreated);
  const replies = new Map<string, DesignComment[]>();
  for (const c of comments) {
    if (!c.parent_id) continue;
    const list = replies.get(c.parent_id) ?? [];
    list.push(c);
    replies.set(c.parent_id, list);
  }
  return roots.map((root, i) => ({ root, replies: (replies.get(root.id) ?? []).sort(byCreated), n: i + 1 }));
}

function byCreated(a: DesignComment, b: DesignComment): number {
  return a.created_at < b.created_at ? -1 : a.created_at > b.created_at ? 1 : a.id < b.id ? -1 : a.id > b.id ? 1 : 0;
}

/** The list order: open threads first, then resolved; each oldest first. */
export function splitThreads(threads: readonly Thread[]): { open: Thread[]; resolved: Thread[] } {
  return { open: threads.filter((t) => !t.root.resolved), resolved: threads.filter((t) => t.root.resolved) };
}

/** Pins the stage draws: every OPEN pinned thread. One made on another
 *  version is drawn MUTED (with its "v3" chip) rather than hidden — autosave
 *  moves the head every few seconds of editing, so hiding would make a
 *  reviewer's pins vanish the moment someone types. Resolved pins leave the stage. */
export function pinsFor(threads: readonly Thread[], versionId: string | null): { thread: Thread; muted: boolean }[] {
  return threads
    .filter((t) => t.root.pin && !t.root.resolved)
    .map((thread) => ({ thread, muted: (thread.root.version_id ?? null) !== versionId }));
}

/** "made on v3" chip text, null when the thread is on the shown version. */
export function versionChip(c: Pick<DesignComment, 'version_id' | 'version_seq'>, shownVersionId: string | null): string | null {
  if (c.version_id === shownVersionId || c.version_seq == null) return null;
  return `made on v${c.version_seq}`;
}

/** A thread's comments in the order they were written (root first). */
export function threadComments(t: Thread): DesignComment[] {
  return [t.root, ...t.replies];
}

/** Open-thread count (the tab badge). */
export function openCount(threads: readonly Thread[]): number {
  return threads.reduce((n, t) => n + (t.root.resolved ? 0 : 1), 0);
}

/** Typing targets that keep the C shortcut (and every other letter) theirs. */
export function isTypingTarget(el: unknown): boolean {
  const e = el as { tagName?: string; isContentEditable?: boolean; closest?: (s: string) => unknown } | null;
  if (!e) return false;
  if (e.isContentEditable) return true;
  const tag = (e.tagName ?? '').toUpperCase();
  if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || tag === 'IFRAME') return true;
  return !!e.closest?.('.cm-editor, [contenteditable="true"], [role="textbox"], .excalidraw');
}
