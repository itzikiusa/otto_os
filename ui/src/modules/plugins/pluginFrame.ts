// The plugin frame's trust rules, out of PluginFrame.svelte so they are tested
// by behaviour (ui/unit/frameMessages.test.ts), not by matching its source.
//
//   - A slug is a plain identifier (= plugins.rs `valid_slug`). Anything else
//     (`..`, `/`, `%2e`) is never framed: `/plugins/../ui/` resolves to Otto's
//     own SPA (served 200), which would then load inside the frame and receive
//     `otto:init` with the viewer's token — so an invalid slug gets NO init.
//   - Messages are honoured only from the plugin's own frame window.

/** = otto-server plugins.rs `valid_slug`. */
export function pluginSlugOk(slug: string): boolean {
  return /^[a-z][a-z0-9-]{0,63}$/.test(slug);
}

export interface PluginInitDeps {
  /** Daemon base URL (no trailing slash). */
  baseUrl: string;
  /** The viewer's bearer token. */
  token: string | null;
  providers: unknown;
}

/** Hand the frame its `otto:init` extras (API base + token) through `post` —
 *  only for a valid slug. Returns whether it posted. */
export function postPluginInit(
  slug: string,
  post: (extra: Record<string, unknown>) => void,
  deps: PluginInitDeps,
): boolean {
  if (!pluginSlugOk(slug)) return false;
  post({
    slug,
    apiBase: `${deps.baseUrl}/api/v1/plugins/${encodeURIComponent(slug)}`,
    token: deps.token,
    providers: deps.providers,
  });
  return true;
}

/** What a frame message asks the shell to do. */
export type PluginFrameAction =
  | { kind: 'focus-leave' }
  | { kind: 'keydown'; init: KeyboardEventInit }
  | null;

/** Decode a `message` event for the plugin frame whose window is
 *  `frameWindow`; anything from another source is ignored. */
export function pluginFrameAction(
  ev: { source: unknown; data: unknown },
  frameWindow: unknown,
): PluginFrameAction {
  if (frameWindow == null || ev.source == null || ev.source !== frameWindow) return null;
  const m = ev.data as ({ type?: unknown; key?: unknown } & KeyboardEventInit) | null;
  if (!m) return null;
  if (m.type === 'otto:focus-leave') return { kind: 'focus-leave' };
  if (m.type === 'otto:keydown' && typeof m.key === 'string') return { kind: 'keydown', init: m };
  return null;
}
