// Agent UI control — the shell commands (`module: "shell"`: any document runs
// them): `state` (what the window shows), `open` (a module where the user can
// see it — the SIDE PANE beside the agent's session by default, never over a
// main pane showing something else) and `focus`.

import { auth } from '../stores/auth.svelte';
import { sidePane } from '../stores/sidePane.svelte';
import { router } from '../router.svelte';
import { isEmbedded, isTauri } from '../desktop';
import { hostPrimaryKey, postToHost } from '../embedGuest';
import { paneKey, restorableRoute, routeOf } from '../sidePane';
import { availableModules, moduleLabel } from '../sidebar';
import {
  UiCommandError,
  describeDocument,
  localModuleState,
  registerUiCommands,
  type UiCommandCtx,
} from '../uiCommands';

/** How long `open` waits for a fresh side pane to boot. */
const PANE_BOOT_MS = 15_000;

/** Same-origin hook: the main document reads its side pane's state through it
 *  (the pane is another document with its own stores). */
type StateHook = () => { document: ReturnType<typeof describeDocument>; view: unknown };
declare global {
  interface Window {
    __ottoUiState?: StateHook;
  }
}
if (typeof window !== 'undefined') {
  window.__ottoUiState = () => ({ document: describeDocument(), view: localModuleState() });
}

function sideFrame(): HTMLIFrameElement | null {
  return document.querySelector<HTMLIFrameElement>('[data-testid="side-pane-frame"]');
}

function sidePaneState(): unknown {
  try {
    return sideFrame()?.contentWindow?.__ottoUiState?.() ?? null;
  } catch {
    return null;
  }
}

/** The sidebar name of a route's module (`database` → Connections). */
function labelFor(route: string): string {
  return moduleLabel(paneKey(routeOf(route)));
}

/** Module ids this user may open (sidebar entries they can view + the
 *  Connections hub's own views). */
function openableModules(): Set<string> {
  const ids = availableModules((f) => auth.can(f, 'view'), []).map((m) => m.id);
  const set = new Set(ids);
  if (set.has('connections')) {
    set.add('database');
    set.add('brokers');
  }
  if (set.has('design')) set.add('canvas');
  set.add('settings');
  return set;
}

function sleep(ms: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    if (signal.aborted) return reject(signal.reason);
    const t = setTimeout(resolve, ms);
    signal.addEventListener('abort', () => {
      clearTimeout(t);
      reject(signal.reason);
    }, { once: true });
  });
}

/** Wait until the side pane shows `key` and its document has booted. */
async function paneReady(key: string, signal: AbortSignal): Promise<void> {
  const until = Date.now() + PANE_BOOT_MS;
  while (Date.now() < until) {
    if (sidePane.showing && sidePane.key === key && sidePane.status === 'ready') return;
    if (sidePane.status === 'error') throw new UiCommandError('failed', "The side pane couldn't load");
    await sleep(100, signal);
  }
  throw new UiCommandError('failed', 'The side pane took too long to load');
}

interface OpenArgs {
  module?: unknown;
  route?: unknown;
  placement?: unknown;
}

async function open(args: OpenArgs, ctx: UiCommandCtx): Promise<unknown> {
  const module = typeof args.module === 'string' ? args.module.trim() : '';
  if (!module) throw new UiCommandError('invalid_args', '`module` is required');
  if (!openableModules().has(module)) {
    throw new UiCommandError('not_found', `There's no ${module} module you can open in Otto`);
  }
  const route = restorableRoute(typeof args.route === 'string' && args.route.trim() ? args.route : module);
  if (!route) throw new UiCommandError('invalid_args', `Can't open route ${String(args.route)}`);
  const key = paneKey(route);
  if (key !== paneKey(module)) {
    throw new UiCommandError('invalid_args', `Route ${route} isn't part of ${module}`);
  }
  const placement = args.placement === 'main' ? 'main' : 'side';
  const label = labelFor(route);
  const done = (pane: 'main' | 'side', extra: Record<string, unknown> = {}) => ({
    ui_visible: true,
    pane,
    route,
    module: key,
    label,
    ...extra,
  });

  // ── In the side pane document ──
  if (isEmbedded) {
    if (placement === 'main' || hostPrimaryKey() === key) {
      // The main pane owns it (or should): hand it over.
      postToHost({ type: 'open-in-main', route });
      return done('main');
    }
    ctx.progress(`Opening ${label}`);
    router.go(route);
    return done('side');
  }

  // ── In a main document ──
  if (placement === 'main') {
    ctx.progress(`Opening ${label}`);
    router.go(route);
    return done('main');
  }
  if (sidePane.primaryKey === key) {
    // Already on screen in the main pane — drive it there, don't open it twice.
    return done('main', { already_open: true });
  }
  if (!sidePane.supported) {
    throw new UiCommandError(
      'failed',
      `This Otto window can't show a side pane (it needs the desktop layout). Ask the user to open ${label}, or use placement: main.`,
    );
  }
  if (!sidePane.fits) {
    throw new UiCommandError(
      'failed',
      `The Otto window is too narrow to show ${label} beside your session. Ask the user to widen it, or use placement: main.`,
    );
  }
  ctx.progress(`Opening ${label} beside your session`);
  const wasShowing = sidePane.showing && sidePane.key === key;
  // Focus stays where the user has it (typically your terminal).
  sidePane.open(route, { focus: false, label });
  await paneReady(key, ctx.signal);
  const pane = document.querySelector('[data-testid="side-pane"]');
  if (pane && !wasShowing) ctx.highlight(pane);
  return done('side');
}

async function focusWindow(): Promise<void> {
  if (isTauri) {
    try {
      const { getCurrentWindow } = await import('@tauri-apps/api/window');
      await getCurrentWindow().setFocus();
      return;
    } catch {
      /* fall through to the DOM */
    }
  }
  try {
    (isEmbedded ? window.parent : window).focus();
  } catch {
    window.focus();
  }
}

async function focus(args: { pane?: unknown }): Promise<unknown> {
  const want = args.pane === 'main' || args.pane === 'side' ? args.pane : null;
  await focusWindow();
  if (isEmbedded) {
    if (want === 'main') {
      try {
        window.parent.focus();
      } catch {
        /* host gone */
      }
      return { ui_visible: true, pane: 'main' };
    }
    window.focus();
    return { ui_visible: true, pane: 'side' };
  }
  const pane = want ?? (sidePane.showing ? 'side' : 'main');
  if (pane === 'side') {
    if (!sidePane.showing) throw new UiCommandError('not_found', 'No side pane is open');
    sidePane.focusPane();
  } else {
    sidePane.focusMain();
  }
  return { ui_visible: true, pane };
}

function state(): unknown {
  const doc = describeDocument();
  const out: Record<string, unknown> = {
    ui_visible: true,
    document: { ...doc, label: labelFor(doc.route) },
    view: localModuleState(),
  };
  if (isEmbedded) {
    const primary = hostPrimaryKey();
    out.main = primary ? { module: primary, label: moduleLabel(primary) } : null;
  } else {
    out.side = sidePane.route === null
      ? null
      : {
          route: sidePane.route,
          module: sidePane.key,
          label: labelFor(sidePane.route),
          showing: sidePane.showing,
          status: sidePane.status,
          focused: sidePane.focused,
          placement: sidePane.placement,
          ...(sidePane.showing ? { state: sidePaneState() } : {}),
        };
  }
  return out;
}

registerUiCommands('shell', {
  state: async () => state(),
  open: (args, ctx) => open(args ?? {}, ctx),
  focus: (args) => focus(args ?? {}),
});
