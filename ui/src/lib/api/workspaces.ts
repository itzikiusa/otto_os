// Workspace read helpers (docs/contracts/api.md rows 11–14).

import { api, ApiError } from './client';
import type { Id, Workspace, WorkspaceWithRole } from './types';

/**
 * A FRESH read of one workspace, for read-merge-write of its `settings` (the
 * workspaces PATCH replaces the whole settings object, so merging into a
 * cached copy could revert keys changed since).
 *
 * The daemon has no `GET /workspaces/{id}` — that path only carries PATCH /
 * DELETE, so a GET answers 405 — so read the caller's own list
 * (`GET /workspaces`) and pick the row. A workspace the caller can't see is a
 * 404, the same answer a per-id read would give.
 */
export async function fetchWorkspace(id: Id): Promise<Workspace> {
  const rows = await api.get<WorkspaceWithRole[]>('/workspaces');
  const row = rows.find((w) => w.id === id);
  if (!row) throw new ApiError(404, { code: 'not_found', message: 'workspace not found' });
  return row;
}
