/** A broadcast has exactly one workspace, including the hidden scratch one.
 * Missing sessions and mixed scopes must never silently reduce the recipients. */
export function broadcastScope(ids: string[], sessions: { id: string; workspace_id: string }[]): string | null {
  if (!ids.length) return null;
  const scopes = ids.map((id) => sessions.find((s) => s.id === id)?.workspace_id);
  return scopes[0] && scopes.every((scope) => scope === scopes[0]) ? scopes[0] : null;
}

export interface FilterableItem { session_id?: string; id: string; repo?: string; cost_usd?: number }
export interface FilterableSession { id: string; provider: string; cwd: string }

export function matchesSavedView(item: FilterableItem, filter: Record<string, unknown>, sessions: FilterableSession[]): boolean {
  const session = sessions.find((s) => s.id === (item.session_id ?? item.id));
  if (typeof filter.provider === 'string' && filter.provider && session?.provider !== filter.provider) return false;
  if (typeof filter.repo === 'string' && filter.repo && (item.repo ?? session?.cwd) !== filter.repo) return false;
  return typeof filter.min_cost_usd !== 'number' || (item.cost_usd ?? 0) >= filter.min_cost_usd;
}
