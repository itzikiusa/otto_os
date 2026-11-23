// Pure list logic of the clipboard-history panel (ClipboardPanel.svelte):
// day groups, row titles, labels and keyboard selection. Filtering happens
// in the shell (full text), so this only shapes what came back.

import type { ClipEntry, ClipKind } from '../../lib/clipboardHistory';

export type { ClipEntry, ClipKind };

export interface ClipGroup {
  label: string;
  items: ClipEntry[];
}

/** Rows rendered at most; the filter narrows the rest. */
export const RENDER_CAP = 200;

export const KIND_LABEL: Record<ClipKind, string> = { text: 'Text', link: 'Link', file: 'File', image: 'Image' };

function dayStart(ms: number): number {
  const d = new Date(ms);
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
}

export function dayLabel(ms: number, now: number): string {
  const days = Math.round((dayStart(now) - dayStart(ms)) / 86_400_000);
  if (days <= 0) return 'Today';
  if (days === 1) return 'Yesterday';
  return new Date(ms).toLocaleDateString([], { weekday: 'long', month: 'short', day: 'numeric' });
}

/** Pinned first, then by the day each entry was last copied (newest first,
 *  as the shell sends them), capped at RENDER_CAP rows. */
export function groupByDay(entries: readonly ClipEntry[], now: number): ClipGroup[] {
  const groups: ClipGroup[] = [];
  const pinned = entries.filter((e) => e.pinned);
  const rest = entries.filter((e) => !e.pinned);
  for (const e of [...pinned, ...rest].slice(0, RENDER_CAP)) {
    const label = e.pinned ? 'Pinned' : dayLabel(e.last_ms, now);
    const last = groups[groups.length - 1];
    if (last?.label === label) last.items.push(e);
    else groups.push({ label, items: [e] });
  }
  return groups;
}

/** The row's one line: "Image (381×60)", a file name (+ how many more), or
 *  the first non-empty line of the text. */
export function title(e: ClipEntry): string {
  if (e.kind === 'image') return `Image (${e.width}×${e.height})`;
  const lines = e.text.split('\n').map((l) => l.trim()).filter(Boolean);
  if (e.kind === 'file') {
    const name = (lines[0] ?? '').split('/').filter(Boolean).pop() ?? '';
    return lines.length > 1 ? `${name} and ${lines.length - 1} more` : name;
  }
  return lines[0] ?? '';
}

export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const kb = bytes / 1024;
  return kb < 1024 ? `${kb.toFixed(kb < 10 ? 1 : 0)} KB` : `${(kb / 1024).toFixed(1)} MB`;
}

/** Arrow keys: clamp inside the list (no wrap, like Raycast). */
export function moveSelection(index: number, delta: number, length: number): number {
  return length === 0 ? 0 : Math.min(length - 1, Math.max(0, index + delta));
}

/** The default action's label in the bottom bar. */
export function pasteLabel(target: string | null): string {
  return target ? `Paste to ${target}` : 'Paste';
}
