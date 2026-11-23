// The thread minimap (T3 Code's TimelineMinimap / timelineMinimapItems, MIT,
// Copyright (c) 2026 T3 Tools Inc.): one tick per message the person sent,
// marked when that turn failed or stopped for an approval. Pure
// (unit/runtimeMinimap.test.ts).
import type { ProviderTurn, TurnItem } from '../../../lib/api/types';
import { messageAttribution } from './timeline.ts';

/** Fewer user turns than this: no rail (the thread is short enough). */
export const MINIMAP_MIN = 6;

export interface MinimapTick {
  /** The user message's item id (`data-rt-msg` in the timeline). */
  id: string;
  /** Its first line, for the hover label and the jump list. */
  text: string;
  mark: 'failed' | 'approval' | null;
}

/** One tick per top-level message the person typed (framed messages from
 *  other threads left out), oldest first. */
export function minimapTicks(items: readonly TurnItem[], turns: readonly ProviderTurn[]): MinimapTick[] {
  const failed = new Set(turns.filter((t) => t.status === 'failed').map((t) => t.id));
  const asked = new Set<string>();
  for (const i of items) if (i.type === 'approval_request' && i.turn_id) asked.add(i.turn_id);
  const out: MinimapTick[] = [];
  for (const i of items) {
    if (i.type !== 'user_message' || i.parent_item_id || messageAttribution(i.text)) continue;
    const line = i.text.split('\n').find((l) => l.trim()) ?? '';
    const text = line.trim().slice(0, 80) || (i.attachments?.length ? 'Attachment' : 'Message');
    const t = i.turn_id;
    out.push({ id: i.id, text, mark: t && failed.has(t) ? 'failed' : t && asked.has(t) ? 'approval' : null });
  }
  return out;
}

/** The tick the reader is at: the last one whose message starts above the
 *  reading line (a third down the viewport). `tops` are offsets from the
 *  scroller's top edge, in tick order (NaN for a message not rendered);
 *  scrolled to the end, the last tick. */
export function currentTick(tops: readonly number[], viewport: number, atEnd = false): number {
  if (atEnd && tops.length) return tops.length - 1; // at the bottom the last turn is the one you read
  let cur = 0;
  for (let i = 0; i < tops.length; i++) if (tops[i] <= viewport / 3) cur = i;
  return cur;
}
