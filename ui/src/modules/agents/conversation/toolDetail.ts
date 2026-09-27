// Lazy full tool output for a step the live push shortened (SA-04).
//
// An over-cap `transcript_appended` delta ships each tool result cut to a
// preview with `result.elided: true` instead of dropping every turn and making
// the view re-fetch the whole page. The rest is fetched only when the user
// expands that step: `GET /sessions/{id}/transcript/tool/{tool_id}` returns
// the stored block (64 KB fold cap, never elided). Kept out of ToolStep.svelte
// so the contract is unit-testable (unit/toolDetail.test.ts).
import { api } from '../../../lib/api/client';
import type { Block, ToolResult } from '../../../lib/api/types';

export type ToolCallBlock = Extract<Block, { kind: 'tool_call' }>;

/** True when `block`'s result is a live-push preview with more on the server. */
export function isElided(block: ToolCallBlock): boolean {
  return block.result?.elided === true;
}

/**
 * The result a step renders: the fetched full result when the block it came
 * from is still elided, otherwise the block's own (a later delta or a page
 * re-fetch that carries the whole result wins over a stale fetch).
 */
export function effectiveResult(block: ToolCallBlock, full: ToolResult | null): ToolResult | null {
  if (!isElided(block)) return block.result;
  return full ?? block.result;
}

/** Recently fetched details, so collapsing and re-expanding (or a re-render
 *  that remounts the step) never refetches. Small: one entry per expanded
 *  elided step. */
const CACHE_MAX = 40;
const cache = new Map<string, Promise<ToolResult | null>>();

function toolUrl(sessionId: string, toolId: string): string {
  return `/sessions/${encodeURIComponent(sessionId)}/transcript/tool/${encodeURIComponent(toolId)}`;
}

/** Fetch the full result of tool call `toolId` (cached; a failure is not). */
export function fetchToolResult(
  sessionId: string,
  toolId: string,
  get: (path: string) => Promise<ToolCallBlock> = (p) => api.get<ToolCallBlock>(p),
): Promise<ToolResult | null> {
  const key = `${sessionId}\u0000${toolId}`;
  const hit = cache.get(key);
  if (hit) {
    cache.delete(key);
    cache.set(key, hit);
    return hit;
  }
  const p = get(toolUrl(sessionId, toolId)).then((b) => b.result ?? null);
  cache.set(key, p);
  p.catch(() => cache.delete(key));
  while (cache.size > CACHE_MAX) {
    const oldest = cache.keys().next().value;
    if (oldest === undefined) break;
    cache.delete(oldest);
  }
  return p;
}

/** Test hook: forget every cached detail. */
export function clearToolDetailCache(): void {
  cache.clear();
}
