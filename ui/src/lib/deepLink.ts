// Deep links to a session / thread (`#/agents/<id>`, `#/threads/<id>`) from a
// QR ("Continue on phone"), a notification or a pasted URL. The id alone does
// not say which workspace it lives in, and the workspace store only loads the
// CURRENT workspace's sessions — so a link into another workspace used to open
// an empty "no session" view. This resolves the owning workspace first (pure:
// fetchers are injected, so it is unit-testable without the app).

/** Anything with the owning workspace id (a Session or a RuntimeSession). */
export interface WorkspaceOwned {
  workspace_id: string;
}

export interface DeepLinkFetchers {
  /** The PTY session by id, or null when it doesn't exist. May throw an
   *  error carrying `status` (403/404). */
  session(id: string): Promise<WorkspaceOwned | null>;
  /** The structured runtime thread by id (Threads only), same contract. */
  runtime?(id: string): Promise<WorkspaceOwned | null>;
}

export type DeepLinkResolution =
  | { kind: 'ok'; workspaceId: string; switchTo: string | null }
  | { kind: 'missing' }
  | { kind: 'forbidden' }
  | { kind: 'error'; message: string };

function status(e: unknown): number | null {
  const s = (e as { status?: unknown } | null)?.status;
  return typeof s === 'number' ? s : null;
}

async function attempt(
  fetch: ((id: string) => Promise<WorkspaceOwned | null>) | undefined,
  id: string,
): Promise<WorkspaceOwned | 'missing' | 'forbidden' | { error: string }> {
  if (!fetch) return 'missing';
  try {
    return (await fetch(id)) ?? 'missing';
  } catch (e) {
    const s = status(e);
    if (s === 404) return 'missing';
    if (s === 403 || s === 401) return 'forbidden';
    return { error: e instanceof Error ? e.message : String(e) };
  }
}

/**
 * Where `id` lives and whether the window must switch workspace to show it.
 * `switchTo` is null when the item is already in `currentWorkspace` or in the
 * always-loaded scratch workspace. The session lookup runs first; for a
 * thread link the runtime-thread lookup is the fallback.
 */
export async function resolveDeepLink(
  id: string,
  currentWorkspace: string | null,
  scratchWorkspace: string,
  fetchers: DeepLinkFetchers,
): Promise<DeepLinkResolution> {
  let forbidden = false;
  let error: string | null = null;
  for (const f of [fetchers.session, fetchers.runtime]) {
    const r = await attempt(f, id);
    if (r === 'missing') continue;
    if (r === 'forbidden') {
      forbidden = true;
      continue;
    }
    if ('error' in r) {
      error ??= r.error;
      continue;
    }
    const wsId = r.workspace_id;
    const switchTo = wsId === currentWorkspace || wsId === scratchWorkspace ? null : wsId;
    return { kind: 'ok', workspaceId: wsId, switchTo };
  }
  if (forbidden) return { kind: 'forbidden' };
  if (error) return { kind: 'error', message: error };
  return { kind: 'missing' };
}

/** The person-facing explanation for a failed resolution. */
export function deepLinkErrorText(r: DeepLinkResolution, what: 'session' | 'thread'): string | null {
  switch (r.kind) {
    case 'ok':
      return null;
    case 'missing':
      return `This ${what} doesn’t exist any more — it may have been deleted.`;
    case 'forbidden':
      return `You don’t have access to this ${what}. Ask its owner to share it, or sign in as them.`;
    case 'error':
      return `Couldn’t open this ${what}: ${r.message}`;
  }
}
