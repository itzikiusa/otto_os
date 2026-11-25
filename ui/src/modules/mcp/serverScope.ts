// Where an MCP server is available — its home workspace only, chosen other
// workspaces, or all of them (also every workspace added later). One server
// row, shared: one place to edit it, one set of credentials, one health state
// (docs/features/mcp-control-plane.md, "Available in"). Pure, so the unit
// tests load it directly.

export type ScopeMode = 'this' | 'selected' | 'all';

export interface Scope {
  all: boolean;
  /** Other workspaces besides the home (never the home itself). */
  shared: string[];
}

interface Scoped {
  workspace_id: string;
  all_workspaces: boolean;
  shared_workspace_ids: string[];
}

export function scopeOf(s: Scoped): Scope {
  return { all: s.all_workspaces, shared: [...s.shared_workspace_ids] };
}

export function scopeMode(s: Scope): ScopeMode {
  return s.all ? 'all' : s.shared.length ? 'selected' : 'this';
}

/** The request fields for a scope (`selected` with nothing picked = home only). */
export function scopeBody(s: Scope): { all_workspaces: boolean; shared_workspace_ids: string[] } {
  return { all_workspaces: s.all, shared_workspace_ids: s.all ? [] : s.shared };
}

/** Whether `next` makes the server available somewhere `prev` didn't. */
export function widens(prev: Scope, next: Scope): boolean {
  if (next.all) return !prev.all;
  return !prev.all && next.shared.some((w) => !prev.shared.includes(w));
}

/** The confirm for a wider scope: exactly what is shared (the server's tools,
 *  called with the credentials Otto holds for it) and who can use it. */
export function scopeConfirm(name: string, next: Scope, names: (id: string) => string): { title: string; message: string; confirmLabel: string } {
  const where = next.all
    ? 'every workspace except Scratch — also ones added later —'
    : next.shared.map((w) => `“${names(w)}”`).join(', ');
  return {
    title: 'Share MCP server',
    confirmLabel: 'Share',
    message:
      `Members of ${where} can use “${name}”’s tools in their agent sessions. Otto calls them with the credentials and sign-in it holds for this server; ` +
      'those members can’t read or change them, or its settings, which stay in its home workspace. ' +
      'Its access rules still decide who may use it.',
  };
}

/** Why a direct (native) server can't be shared: its command and secrets are
 *  written into agent config files. */
export const NATIVE_SCOPE_HINT = 'A direct server stays in its home workspace. Run it through Otto to share it.';

/** The row badge: where a server comes from / is shared, or null for a
 *  home-only server of this workspace. */
export function scopeBadge(s: Scoped, wsId: string, names: (id: string) => string): string | null {
  if (s.workspace_id !== wsId) return `Shared from ${names(s.workspace_id)}`;
  if (s.all_workspaces) return 'All workspaces';
  if (s.shared_workspace_ids.length) return `Shared with ${s.shared_workspace_ids.length === 1 ? names(s.shared_workspace_ids[0]) : `${s.shared_workspace_ids.length} workspaces`}`;
  return null;
}
