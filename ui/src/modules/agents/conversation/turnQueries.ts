// Pure per-turn queries ConversationView runs on every render pass, memoized
// per Turn object (turns are replaced, never mutated, when they change).
import type { Turn } from '../../../lib/api/types';
import type { RenderItem } from './format';

const turnTextCache = new WeakMap<Turn, string>();

/** Lowercased searchable text of a turn (prose, tool titles/results, queued). */
export function turnText(turn: Turn): string {
  const hit = turnTextCache.get(turn);
  if (hit !== undefined) return hit;
  const parts: string[] = [];
  for (const b of turn.blocks) {
    if (b.kind === 'text') parts.push(b.md);
    else if (b.kind === 'tool_call') parts.push(b.title, b.name, b.result?.text ?? '');
    else if (b.kind === 'queued') parts.push(b.text);
  }
  const text = parts.join('\n').toLowerCase();
  turnTextCache.set(turn, text);
  return text;
}

const editsIn = new WeakMap<Turn, boolean>();

/** Whether the turn edited or wrote a file. */
export function turnEdits(turn: Turn): boolean {
  let v = editsIn.get(turn);
  if (v === undefined) {
    v = turn.blocks.some((b) => b.kind === 'tool_call' && (b.tool === 'edit' || b.tool === 'write'));
    editsIn.set(turn, v);
  }
  return v;
}

/** A user turn containing the optimistic prompt's opening text. */
export function turnMatchesOptimistic(turn: Turn, needle: string): boolean {
  if (!needle || turn.role !== 'user') return false;
  return turn.blocks.some((b) => b.kind === 'text' && b.md.includes(needle));
}

/** Per item: true for an assistant turn whose model differs from the previous
 *  assistant turn's (the first one included) — Threads shows the model only there. */
export function modelChangeFlags(items: readonly Pick<RenderItem, 'role' | 'model'>[]): boolean[] {
  let prev: string | null = null;
  return items.map((it) => {
    if (it.role !== 'assistant' || !it.model) return false;
    const changed = it.model !== prev;
    prev = it.model;
    return changed;
  });
}
