// Chat plumbing between the thread's own turn index and the Conversation
// view's render items. The transcript (otto-transcript, via TurnItem) is the
// canonical text; the index (`assistant_turns`) only fills in until the CLI
// has written a transcript, and carries the provider per turn across hand-offs.
import type { AssistantTurn, Block, SystemNote, Turn } from '../../lib/api/types';

/** Structurally the Conversation view's `RenderItem` (agents/conversation/format.ts),
 *  restated here so this module stays node-testable (format.ts pulls in Svelte types). */
export interface RenderItem {
  id: string;
  role: 'user' | 'assistant';
  turns: Turn[];
  blocks: Block[];
  system: SystemNote[];
  duration_ms: number | null;
  ts: string | null;
  model: string | null;
  reasoning_steps: number;
}
import type { Provider } from './model';

/** A thread index turn as a Conversation-view render item (prose only). */
export function indexToRenderItem(t: AssistantTurn): RenderItem {
  return {
    id: t.id,
    role: t.role === 'user' ? 'user' : 'assistant',
    turns: [],
    blocks: t.text ? [{ kind: 'text', md: t.text }] : [],
    system: [],
    duration_ms: null,
    ts: t.created_at,
    model: t.model,
    reasoning_steps: 0,
  };
}

/** Best-effort provider from a model id ("claude-sonnet-4-5" → claude, "gpt-5-codex" → codex). */
export function inferProvider(model: string | null | undefined): Provider | null {
  if (!model) return null;
  const m = model.toLowerCase();
  if (/claude|opus|sonnet|haiku/.test(m)) return 'claude';
  if (/codex|gpt|^o\d/.test(m)) return 'codex';
  return null;
}

export interface ChatMessage {
  item: RenderItem;
  /** The index turn behind it (null for a live reply not indexed yet). */
  turn: AssistantTurn | null;
}

const MATCH_WINDOW_MS = 10 * 60_000;

/**
 * The thread's messages: the turn index is the backbone (it spans every
 * provider hand-off and keeps the user's own words, not the pasted packet);
 * each reply from the CURRENT session is swapped for its transcript render
 * item (tool steps, images, files) by nearest time, and a reply the CLI has
 * written but the daemon hasn't indexed yet is appended live.
 */
export function mergeWithTranscript(index: AssistantTurn[], live: RenderItem[], sessionId: string | null): ChatMessage[] {
  const base: ChatMessage[] = index
    .filter((t) => t.kind === 'message' && t.role !== 'system')
    .map((t) => ({ item: indexToRenderItem(t), turn: t }));
  if (!sessionId || !live.length) return base;
  const replies = live.filter((i) => i.role === 'assistant' && i.ts);
  const used = new Set<string>();
  for (const m of base) {
    const t = m.turn;
    if (!t || t.role !== 'assistant' || t.session_id !== sessionId) continue;
    const at = Date.parse(t.created_at);
    let best: RenderItem | null = null;
    let bestD = Infinity;
    for (const r of replies) {
      if (used.has(r.id)) continue;
      const d = Math.abs(Date.parse(r.ts as string) - at);
      if (Number.isFinite(d) && d <= MATCH_WINDOW_MS && d < bestD) {
        best = r;
        bestD = d;
      }
    }
    if (best) {
      used.add(best.id);
      m.item = best;
    }
  }
  const last = base.length ? Date.parse(base[base.length - 1].turn?.created_at ?? '') : Number.NEGATIVE_INFINITY;
  for (const r of replies) {
    if (!used.has(r.id) && Date.parse(r.ts as string) > last) base.push({ item: r, turn: null });
  }
  return base;
}

/**
 * Who wrote a response, for its badge: the index turn's provider (recorded per
 * turn, so it survives a mid-thread hand-off), else the model id, else the
 * thread's current provider.
 */
export function messageAuthor(m: ChatMessage, thread: { provider: string; model: string | null }): { provider: string; model: string | null } {
  const provider = m.turn?.provider ?? inferProvider(m.item.model) ?? thread.provider;
  const model = m.turn?.model ?? m.item.model ?? (provider === thread.provider ? thread.model : null);
  return { provider, model };
}
