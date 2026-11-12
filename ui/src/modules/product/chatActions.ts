// The story Chat's cards and tab ids — small pure helpers shared by
// RefineChat / ActionCard / ProductPage (and unit-tested on their own).
import { api } from '../../lib/api/client';
import type { ApplyResult, DiscoveryAction, RefinementMessage } from './types';

/** Card kinds the Chat renders; a proposed story body is a suggested version,
 *  never a card (mirrors `chat_actions::CARD_ACTIONS` in otto-product). */
export const CARD_ACTION_TYPES = ['add_questions', 'add_notes', 'create_canvas'] as const;

/** Tab ids that were merged into another tab: old deep links and ⌘K entries
 *  land on the merged one. */
const MERGED_TABS: Record<string, string> = { discovery: 'chat', refine: 'chat' };

/** The live tab for `tab` (itself unless it was merged away). */
export function canonicalProductTab(tab: string): string {
  return MERGED_TABS[tab] ?? tab;
}

/** Parse a Chat message's `meta_json` (tolerant: bad JSON → `{}`). */
export function chatMessageMeta(message: Pick<RefinementMessage, 'meta_json'>): Record<string, unknown> {
  if (!message.meta_json) return {};
  try {
    const meta = JSON.parse(message.meta_json);
    return meta && typeof meta === 'object' ? (meta as Record<string, unknown>) : {};
  } catch {
    return {};
  }
}

/** The appliable cards an agent message carries (`meta_json.actions`). */
export function chatMessageCards(message: Pick<RefinementMessage, 'meta_json'>): DiscoveryAction[] {
  const actions = chatMessageMeta(message).actions;
  if (!Array.isArray(actions)) return [];
  return actions.filter(
    (a): a is DiscoveryAction =>
      !!a && typeof a === 'object' && (CARD_ACTION_TYPES as readonly string[]).includes((a as { type?: string }).type ?? ''),
  );
}

/** The suggested version a turn wrote, when it wrote one. */
export function chatMessageSuggestion(message: Pick<RefinementMessage, 'meta_json'>): number | null {
  const meta = chatMessageMeta(message);
  return meta.story_updated && meta.version_no != null ? Number(meta.version_no) : null;
}

/** One-line summary of what an earlier Discovery Chat reply proposed (read-only). */
export function proposalSummary(actionsJson: string | null): string {
  if (!actionsJson) return '';
  let actions: unknown;
  try {
    actions = JSON.parse(actionsJson);
  } catch {
    return '';
  }
  if (!Array.isArray(actions)) return '';
  const parts: string[] = [];
  for (const a of actions as { type?: string; questions?: unknown[]; notes?: unknown[] }[]) {
    if (a?.type === 'apply_draft') parts.push('a story draft');
    else if (a?.type === 'add_questions') parts.push(`${a.questions?.length ?? 0} question(s)`);
    else if (a?.type === 'add_notes') parts.push(`${a.notes?.length ?? 0} note(s)`);
    else if (a?.type === 'create_canvas') parts.push('a diagram');
  }
  return parts.length ? `Proposed ${parts.join(', ')}` : '';
}

/** Apply one card from Chat conversation `tid` (`POST …/apply`). */
export function applyChatAction(tid: string, action: DiscoveryAction): Promise<ApplyResult> {
  return api.post<ApplyResult>(`/product/refinement-threads/${tid}/apply`, { action });
}
