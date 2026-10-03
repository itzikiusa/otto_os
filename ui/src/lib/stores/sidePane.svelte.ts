// Side by side — the host (main window) half: which route the side pane
// shows, how the content column is split, and the bridge to the pane's
// document. Pure rules live in lib/sidePane.ts; the pane's half in
// lib/embedGuest.ts; the chrome in shell/SidePane.svelte + SplitDivider.svelte.
//
//   [sidebar] [ primary pane │ side pane ]      (swap flips the sides)
//
// The pane is an <iframe> of this app (`?embed=1`), so it has its own router
// and stores. One module lives in one pane: while the pane is showing, a
// navigation in the main window to the pane's module is delivered to the pane
// (router delegate), and the pane hands the main pane's module back here.
// Persisted per device: the pane's last route, the split and the placement,
// so a reload restores e.g. `connections/<id>` beside Agents.

import { untrack } from 'svelte';
import { router } from '../router.svelte';
import { viewport } from './viewport.svelte';
import { toasts } from '../toast.svelte';
import { isEmbedded, isPopout } from '../desktop';
import { paneSurfaceVisible, nativeSideFocused } from '../nativePanePolicy';
import { paneWindowMenuItems } from '../nativePaneMenu';
import { nativePane, nativePaneAvailable, observeNativePane, startNativePaneState, type PaneState, type PaneBounds } from '../nativePane';
import { winKey } from '../win';
import { lsGet, lsSet } from '../storage';
import { moduleLabel } from '../sidebar';
import { openExternal } from '../external';
import { ui, isTauri } from './ui.svelte';
import type { MenuItem } from '../contextmenu.svelte';
import {
  SIDE_NS,
  SPLIT_DEFAULT,
  clampShare,
  embedSrc,
  fitsTwoPanes,
  leadingShare,
  paneKey,
  parseSaved,
  readGuestMsg,
  restorableRoute,
  serializeSaved,
  sideMenuTarget,
  sideShareFor,
  targetOrigin,
  type HostMsg,
  type Placement,
  type RemoteCommand,
} from '../sidePane';

type Payload<T> = T extends unknown ? Omit<T, 'ns'> : never;

/** How long a fresh pane may take to boot before it shows "Couldn't load". */
const READY_TIMEOUT_MS = 20_000;

/** What the host window does for its pane (App.svelte supplies these). */
export interface HostHooks {
  /** Run a key-map action as if it were pressed in the main window. */
  runKey: (action: string, index?: number) => void;
  /** Open `route` in the main pane (the pane handed it back). */
  openInMain: (route: string) => void;
  /** The pane switched workspace. */
  selectWorkspace: (id: string) => void;
}

class SidePaneStore {
  /** The side pane's route (`connections/<id>`); null = no pane. */
  route: string | null = $state(null);
  /** The side pane's share of the content width (clamped 0.25–0.75). */
  share = $state(SPLIT_DEFAULT);
  placement: Placement = $state('trailing');
  /** The pane document's boot state (not persisted). */
  status: 'loading' | 'ready' | 'error' = $state('loading');
  /** Keyboard focus is inside the pane (its bar reads as active). */
  focused = $state(false);
  /** Width of the split container, px (App measures it). */
  width = $state(0);
  /** A divider drag is in progress (the iframe stops taking pointer events). */
  dragging = $state(false);
  /** Bumped to rebuild the iframe (Retry). */
  generation = $state(0);
  /** The pane's own ⌘K commands, mirrored into the host palette. */
  commands: RemoteCommand[] = $state([]);

  nativeState: PaneState | null = $state(null);
  nativeBusy = $state(false);
  private transportReady: Promise<void> = Promise.resolve();

  private frame: HTMLIFrameElement | null = null;
  private focusOnReady = false;
  private readyTimer: ReturnType<typeof setTimeout> | null = null;

  constructor() {
    if (isEmbedded || isPopout || typeof window === 'undefined') return;
    const saved = parseSaved(lsGet(winKey('otto_side_pane')));
    this.route = saved.route;
    this.share = saved.share;
    this.placement = saved.placement;
  }

  // Booleans as $derived (not plain getters over `width`): readers — the
  // 'split' / 'side-pane-commands' registry effects in App — then re-run only
  // when the answer flips, not on every px of a window resize.
  private supportedNow = $derived(!isEmbedded && !isPopout && (viewport.isDesktop || (nativePaneAvailable && this.route !== null)));
  private fitsNow = $derived(this.width === 0 || fitsTwoPanes(this.width));
  private showingNow = $derived(this.supportedNow && !this.detached && this.route !== null && this.fitsNow && this.key !== this.primaryKey);

  /** The window can host a side pane at all (main window, desktop width). */
  get supported(): boolean {
    return this.supportedNow;
  }

  /** The content column is wide enough for two panes. */
  get fits(): boolean {
    return this.fitsNow;
  }

  /** The side pane's module key (see `paneKey`), null without a pane. */
  get key(): string | null {
    return this.route === null ? null : paneKey(this.route);
  }

  /** The main pane's module key. */
  get primaryKey(): string {
    return paneKey(router.parts.join('/'));
  }

  /** The pane is on screen. A pane that can't show right now (a narrow
   *  window, the main pane on the same module) is hidden, never cleared. */
  get showing(): boolean {
    return this.showingNow;
  }

  /** Native documents stay mounted and own their module even while hidden. */
  get retained(): boolean { return nativePaneAvailable && !isEmbedded && !isPopout && this.route !== null; }
  get active(): boolean { return this.retained || this.showing; }
  get visible(): boolean { return paneSurfaceVisible(this.showing, this.detached, this.nativeState?.visible); }
  get detached(): boolean { return this.nativeState !== null && this.nativeState.mode !== 'attached'; }

  /** The leading pane's fraction (divider position, aria-valuenow). */
  get leading(): number {
    return leadingShare(this.share, this.placement);
  }

  private persist(): void {
    lsSet(
      winKey('otto_side_pane'),
      serializeSaved({ route: this.route, share: this.share, placement: this.placement }),
    );
  }

  /**
   * Show `route` in the side pane — a new pane, or the open one navigated in
   * place (no reload). The main pane's own module is refused with a note, as
   * is a window too narrow for two panes. Focus moves into the pane unless
   * `focus: false`.
   */
  open(route: string, opts: { focus?: boolean; label?: string } = {}): void {
    const r = restorableRoute(route);
    if (!r || !this.supported) return;
    const label = opts.label ?? moduleLabel(r.split('/')[0] || 'agents');
    if (paneKey(r) === this.primaryKey) {
      toasts.info(`${label} is already open`, 'Choose another section to show side by side.');
      return;
    }
    if (!this.fits && !this.detached) {
      toasts.info('Not enough room for two panes', 'Widen the window or collapse the sidebar (⌘1).');
      return;
    }
    const live = this.route !== null && (this.frame !== null || this.nativeState !== null);
    this.route = r;
    this.persist();
    if (live && this.status === 'ready') {
      this.post({ type: 'navigate', route: r });
      if (opts.focus !== false) this.focusPane();
    } else {
      // A fresh frame boots at this route; a booting one is re-pointed on ready.
      this.focusOnReady = opts.focus !== false;
    }
  }

  /** A navigation the main window handed over (router delegate): show it in
   *  the pane. A bare module route (a sidebar click on the pane's module)
   *  keeps the pane where it is and just focuses it. */
  deliver(route: string): void {
    const r = restorableRoute(route);
    if (!r) return;
    if (r === paneKey(r) && this.key === r) {
      this.focusPane();
      return;
    }
    this.open(r);
  }

  /** Close the pane; the main pane keeps everything. The next pane opens on
   *  the trailing side again (the split ratio is kept). */
  close(): void {
    if (this.route === null || this.nativeBusy) return;
    if (this.detached) { this.returnToSplit(); return; }
    if (this.retained && this.nativeState) {
      this.nativeBusy = true;
      void nativePane.close().then(() => this.clearPane()).catch(() => {
        toasts.error('Couldn’t close the side pane', 'Try again. Your pane is still open.');
      }).finally(() => { this.nativeBusy = false; });
      return;
    }
    this.clearPane();
  }

  private clearPane(): void {
    this.route = null;
    this.placement = 'trailing';
    this.commands = [];
    this.focused = false;
    this.status = 'loading';
    this.focusOnReady = false;
    this.clearTimer();
    this.persist();
  }

  /** Swap the panes' places. Purely visual: both documents keep their state
   *  (scroll, drafts, a running query) — nothing reloads. */
  swap(): void {
    this.placement = this.placement === 'trailing' ? 'leading' : 'trailing';
    this.persist();
  }

  /** Open the pane's page in the main pane and close the pane. */
  promote(): void {
    if (this.detached || this.nativeBusy) return;
    const r = this.route;
    if (r === null) return;
    if (this.retained && this.nativeState) {
      this.nativeBusy = true;
      void nativePane.close().then(() => { this.clearPane(); router.go(r); }).catch(() => {
        toasts.error('Couldn’t move the page', 'Try again. Your pane is still open.');
      }).finally(() => { this.nativeBusy = false; });
    } else { this.clearPane(); router.go(r); }
  }

  /** Move the divider: `leading` is the leading pane's fraction. `commit`
   *  persists (a drag commits once, on release). */
  setLeading(leading: number, commit = true): void {
    this.share = clampShare(sideShareFor(leading, this.placement), this.width);
    if (commit) this.persist();
  }

  resetSplit(): void {
    this.share = SPLIT_DEFAULT;
    this.persist();
  }

  /** Rebuild a pane that failed to load. */
  retry(): void {
    this.status = 'loading';
    this.generation += 1;
  }

  detachPane(pane: 'side' | 'primary'): void {
    if (!this.active || this.status !== 'ready') return;
    this.transition(() => nativePane.detach(pane));
  }

  returnToSplit(): void { this.transition(() => nativePane.return()); }

  private transition(action: () => Promise<unknown>): void {
    if (this.nativeBusy) return;
    this.nativeBusy = true;
    void action().catch(() => {
      toasts.error('Couldn’t change the pane window', 'Try again. Your work remains in its current pane.');
    }).finally(() => { this.nativeBusy = false; });
  }

  /** Subscribe before creating the child: ready may arrive during pane_open. */
  attachNative(bounds: PaneBounds): () => void {
    let cancelled = false;
    let opened = false;
    this.status = 'loading';
    this.clearTimer();
    this.readyTimer = setTimeout(() => { if (!cancelled && this.status === 'loading') this.status = 'error'; }, READY_TIMEOUT_MS);
    void this.transportReady.then(async () => {
      if (cancelled) return;
      opened = true;
      await nativePane.open(this.route ?? 'agents', bounds);
    }).catch(() => { if (!cancelled) { this.status = 'error'; this.clearTimer(); } });
    return () => {
      cancelled = true;
      this.clearTimer();
      if (opened) void nativePane.close().catch(() => {});
    };
  }

  /** The iframe src for a freshly built frame (read once per frame). */
  frameSrc(): string {
    const r = untrack(() => this.route) ?? 'agents';
    return embedSrc(window.location, r);
  }

  attach(frame: HTMLIFrameElement): void {
    this.frame = frame;
    this.status = 'loading';
    this.clearTimer();
    this.readyTimer = setTimeout(() => {
      if (this.frame === frame && this.status === 'loading') this.status = 'error';
    }, READY_TIMEOUT_MS);
  }

  detach(frame: HTMLIFrameElement): void {
    if (this.frame !== frame) return;
    this.frame = null;
    this.focused = false;
    this.clearTimer();
  }

  private clearTimer(): void {
    if (this.readyTimer !== null) clearTimeout(this.readyTimer);
    this.readyTimer = null;
  }

  /** Move keyboard focus into the pane (once it has booted). */
  focusPane(): void {
    if (this.retained) {
      if (this.status !== 'ready') { this.focusOnReady = true; return; }
      if (!this.showing && !this.detached) {
        toasts.info('The side pane is hidden', 'Widen the window or use Pane window to detach it.');
        return;
      }
      if (this.nativeState?.visible !== true) { this.focusOnReady = true; return; }
      void nativePane.focus('side').catch(() => {});
      this.focused = true;
      return;
    }
    const f = this.frame;
    if (!f) return;
    if (this.status !== 'ready') {
      this.focusOnReady = true;
      return;
    }
    f.focus();
    try {
      f.contentWindow?.focus();
    } catch {
      /* cross-origin (never, but harmless) */
    }
    this.focused = true;
  }

  /** Tell the pane which module the main pane shows, and whether its top
   *  row sits under the traffic lights (it leads and the sidebar is the
   *  narrow Rail — Tauri only). */
  postHost(): void {
    this.post({
      type: 'host',
      primary: this.primaryKey,
      padTraffic: isTauri && !this.detached && this.placement === 'leading' && !ui.railExpanded,
    });
  }

  /** Hand keyboard focus back to the main pane. */
  focusMain(): void {
    if (this.retained) void nativePane.focus('primary').catch(() => {});
    if (this.frame && document.activeElement === this.frame) this.frame.blur();
    window.focus();
    this.focused = false;
  }

  post(msg: Payload<HostMsg>): void {
    if (this.retained) {
      void nativePane.toGuest({ ns: SIDE_NS, ...msg }).catch(() => {});
      return;
    }
    try {
      this.frame?.contentWindow?.postMessage({ ns: SIDE_NS, ...msg }, targetOrigin(window.location.origin));
    } catch {
      /* frame navigating / gone */
    }
  }

  /**
   * A native menu item (⌘W, ⌘A, End Session…) while the pane may have focus:
   * forward it to the pane or close the pane (lib/sidePane.ts
   * `sideMenuTarget`). True = handled here; false = the main window's.
   */
  handleMenu(id: string): boolean {
    if (!this.active) return false;
    const focused = this.retained
      ? nativeSideFocused(this.focused, document.hasFocus())
      : this.focused || (this.frame !== null && document.activeElement === this.frame);
    const target = sideMenuTarget(id, focused, this.key);
    if (target === 'main') return false;
    if (target === 'close-pane') this.close();
    else this.post({ type: 'menu', id });
    return true;
  }

  /**
   * Install the host side: the pane's messages, focus tracking, and the
   * router delegate that hands the pane's module to the pane. Returns the
   * teardown. Call once from the main shell.
   */
  listen(hooks: HostHooks): () => void {
    if (isEmbedded || isPopout) return () => {};

    router.setDelegate({
      claims: (route) => this.active && paneKey(route) === this.key,
      deliver: (route) => this.deliver(route),
    });

    const receive = (data: unknown): void => {
      if (this.route === null) return;
      const msg = readGuestMsg(data);
      if (!msg) return;
      switch (msg.type) {
        case 'ready': {
          this.status = 'ready';
          this.clearTimer();
          this.postHost();
          const want = this.route;
          if (want !== null && msg.route !== want) this.post({ type: 'navigate', route: want });
          if (this.focusOnReady) {
            this.focusOnReady = false;
            this.focusPane();
          }
          break;
        }
        case 'route':
          // The pane navigated itself: remember it (a reload restores it).
          if (this.route !== null && restorableRoute(msg.route) && paneKey(msg.route) !== this.primaryKey) {
            this.route = msg.route;
            this.persist();
          }
          break;
        case 'open-in-main':
          hooks.openInMain(msg.route);
          break;
        case 'key':
          hooks.runKey(msg.action, msg.index);
          break;
        case 'close':
          this.close();
          break;
        case 'swap':
          this.swap();
          break;
        case 'promote':
          this.promote();
          break;
        case 'focus':
          this.focused = true;
          break;
        case 'workspace':
          hooks.selectWorkspace(msg.id);
          break;
        case 'commands':
          this.commands = msg.list;
          break;
        case 'open-external':
          void openExternal(msg.url);
          break;
      }
    };

    const onMessage = (e: MessageEvent): void => {
      if (!nativePaneAvailable && this.frame && e.source === this.frame.contentWindow) receive(e.data);
    };
    let stopped = false;
    let unlisten: (() => void) | undefined;
    const stopState = nativePaneAvailable ? observeNativePane((state) => {
      this.nativeState = state;
      if (state?.visible && this.focusOnReady && this.status === 'ready') {
        this.focusOnReady = false;
        this.focusPane();
      }
    }) : () => {};
    if (nativePaneAvailable) {
      this.transportReady = nativePane.onHost(receive).then(async (stop) => {
        if (stopped) { stop(); return; }
        unlisten = stop;
        await startNativePaneState();
      });
      void this.transportReady.catch(() => { this.status = 'error'; });
    }

    // Focus: the window blurs when focus moves into the iframe; any focus in
    // this document means the main pane is active again.
    const onBlur = (): void => {
      requestAnimationFrame(() => {
        if (this.frame && document.activeElement === this.frame) this.focused = true;
      });
    };
    const onFocusIn = (): void => {
      this.focused = this.frame !== null && document.activeElement === this.frame;
    };
    const onNativeFocus = (): void => {
      if (this.retained && document.hasFocus()) this.focused = false;
    };
    // A press in the main document makes the main pane active — except on
    // the side pane's own bar (its buttons act on the side pane).
    const onPointerDown = (e: PointerEvent): void => {
      if ((e.target as Element | null)?.closest?.('.side-pane')) return;
      this.focused = false;
    };

    window.addEventListener('message', onMessage);
    window.addEventListener('blur', onBlur);
    window.addEventListener('focus', onNativeFocus);
    document.addEventListener('focusin', onFocusIn);
    document.addEventListener('pointerdown', onPointerDown, true);
    return () => {
      stopped = true;
      unlisten?.();
      stopState();
      router.setDelegate(null);
      window.removeEventListener('message', onMessage);
      window.removeEventListener('blur', onBlur);
      window.removeEventListener('focus', onNativeFocus);
      document.removeEventListener('focusin', onFocusIn);
      document.removeEventListener('pointerdown', onPointerDown, true);
    };
  }
}

export const sidePane = new SidePaneStore();

/** Tooltip hint on sidebar rows. */
export const SPLIT_HINT = '⌥-click to open side by side';

/**
 * Context-menu rows for a sidebar module (`id` is its sidebar id = route):
 * "Open side by side" (or "Show in side pane" while a pane is open); for the
 * module the pane already shows, "Open in main pane" + "Close side pane".
 * Nothing for the main pane's own module, and nothing off desktop.
 */
export function splitMenuItems(id: string, label: string): MenuItem[] {
  if (!sidePane.supported) return [];
  if (sidePane.route !== null && sidePane.key === id) {
    if (sidePane.detached) return paneWindowMenuItems();
    return [
      ...(nativePaneAvailable ? paneWindowMenuItems() : []),
      { label: 'Open in main pane', icon: 'maximize', action: () => sidePane.promote() },
      { label: 'Close side pane', icon: 'x', hint: '⌘\\', action: () => sidePane.close() },
    ];
  }
  if (sidePane.primaryKey === id) return sidePane.retained ? paneWindowMenuItems() : [];
  return [
    {
      label: sidePane.route !== null ? 'Show in side pane' : 'Open side by side',
      icon: 'columns',
      hint: '⌥-click',
      action: () => sidePane.open(id, { label }),
    },
  ];
}

/** A sidebar row click: ⌥-click opens the module side by side (or focuses
 *  the pane that already shows it); a plain click navigates as always. */
export function navClick(e: MouseEvent, id: string, label: string): void {
  if (e.altKey && sidePane.supported) {
    e.preventDefault();
    if (sidePane.route !== null && sidePane.key === id && sidePane.active) sidePane.focusPane();
    else sidePane.open(id, { label });
    return;
  }
  router.openModule(id);
}
