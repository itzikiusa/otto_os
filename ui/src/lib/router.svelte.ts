// Tiny hash router. Routes look like "#/agents", "#/git/<repoId>/pr/42",
// "#/settings/appearance", "#/history/<sessionId>". No SvelteKit — App.svelte
// switches on `router.module`.
//
// Note: `#/agents/<x>` reads `<x>` as a SESSION id, so sibling agent surfaces
// get their own top-level module — History is `#/history` (optional second
// segment = the session id to preselect), Mission Control `#/mission-control`.
//
// Keeps a browser-style navigation stack so back/forward (buttons + ⌘⇧←/→)
// can return to previously-viewed pages.

// ---------------------------------------------------------------------------
// Share-token in-memory store (Task 3.1)
// ---------------------------------------------------------------------------
// When a visitor arrives at `#/s/<sessionId>/<token>` the raw token is
// captured here (keyed by sessionId), then IMMEDIATELY stripped from the
// visible URL + history via `history.replaceState`. This ensures the token
// never persists in browser history, logs, or the address bar.
// The token is intentionally NOT stored in localStorage (would clobber a real
// owner login under the 'otto_token' key and survive the session).
const _shareTokens: Map<string, string> = new Map();

import { winKey } from './win';
import { lsGet, lsSet } from './storage';
import { isEmbedded } from './desktop';

// Per-window last-route persistence (multi-window restore). Desktop-app only:
// a fresh Tauri window loads with an empty hash, so restoring the saved route
// reopens the exact view the window showed before relaunch. Plain-browser
// behavior is untouched (deep links / reloads already carry a hash; a fresh
// web load should keep landing on the default view).
const LS_LAST_ROUTE = 'otto_last_route';
// The side-by-side pane (an iframe of the main window) never reads or writes
// the window's last route: its own route lives in the host's side-pane state.
const IS_TAURI = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window && !isEmbedded;

function restoreLastRoute(): void {
  if (!IS_TAURI) return;
  // A freshly-minted window may carry an injected initial route (e.g. the snip
  // editor window: windows.rs adds `window.__OTTO_ROUTE__='snip/<id>'` to the
  // init script). Consume-once — delete before applying — so a later hard
  // reload of the window falls back to the normal per-window restore below.
  const injected = (window as { __OTTO_ROUTE__?: unknown }).__OTTO_ROUTE__;
  delete (window as { __OTTO_ROUTE__?: unknown }).__OTTO_ROUTE__;
  if (typeof injected === 'string' && /^[A-Za-z0-9/_-]+$/.test(injected)) {
    history.replaceState(null, '', '#/' + injected);
    return;
  }
  const h = window.location.hash;
  if (h !== '' && h !== '#/' && h !== '#') return; // explicit route wins
  const saved = lsGet(winKey(LS_LAST_ROUTE));
  // Never restore into a share route (`#/s/…` is one-time-view by design).
  if (saved && saved.startsWith('#/') && !saved.startsWith('#/s/')) {
    history.replaceState(null, '', saved);
  }
}

function persistLastRoute(hash: string): void {
  if (!IS_TAURI) return;
  if (hash.startsWith('#/s/')) return; // share tokens/views are never sticky
  lsSet(winKey(LS_LAST_ROUTE), hash);
}

/** Retrieve the in-memory share token captured for a given session.
 *  Returns null if the URL didn't carry one or the token has been consumed. */
export function getShareToken(sessionId: string): string | null {
  return _shareTokens.get(sessionId) ?? null;
}

/** decodeURIComponent that never throws: a malformed `%` escape in a pasted
 *  or restored hash (`#/vault/100%`) threw from the Router constructor and
 *  blanked the app at boot. Undecodable segments are kept verbatim. */
function safeDecode(seg: string): string {
  try {
    return decodeURIComponent(seg);
  } catch {
    return seg;
  }
}

/**
 * A split-view partner that owns some routes: the side-by-side pane (see
 * stores/sidePane.svelte.ts) owns its module in the main window, and the main
 * pane owns its module inside the side pane. A navigation to a route the
 * delegate `claims` is handed to it (`deliver`) instead of replacing this
 * pane's page, so one module is never open in both panes. Routes are passed
 * without the leading `#/`.
 */
export interface RouteDelegate {
  claims(route: string): boolean;
  deliver(route: string): void;
}

/**
 * A leave-guard (see {@link Router.guard}): asked before this pane navigates
 * away. `to` is the target route without the leading `#/`. Return false (or
 * resolve false) to keep the current route — e.g. after the person declines a
 * "Discard unsaved changes?" confirm.
 */
export type LeaveGuard = (to: string) => boolean | Promise<boolean>;

class Router {
  /** path segments after '#/', e.g. ['git', '01H...', 'pr', '7'] */
  parts: string[] = $state([]);

  /** navigation history of hashes; `index` points at the current entry. */
  private stack: string[] = $state([]);
  private index = $state(-1);
  /** set while doing an internal back/forward so onHashChange doesn't push. */
  private navigating = false;
  private delegate: RouteDelegate | null = null;
  /** Registered leave-guards ({@link guard}). Plain field — not reactive. */
  private guards = new Set<LeaveGuard>();
  /** A hash whose guards already passed, so the hashchange it fires must not
   *  ask again. */
  private approved: string | null = null;
  /** Bumped per guarded navigation: an older one resolving late is dropped. */
  private guardSeq = 0;

  canBack = $derived(this.index > 0);
  canForward = $derived(this.index < this.stack.length - 1);

  constructor() {
    if (typeof window !== 'undefined') {
      restoreLastRoute();
      this.parse();
      this.stack = [this.currentHash()];
      this.index = 0;
      persistLastRoute(this.currentHash());
      window.addEventListener('hashchange', () => this.onHashChange());
    }
  }

  private parse(): void {
    const raw = window.location.hash.replace(/^#\/?/, '');
    this.parts = raw === '' ? [] : raw.split('/').map(safeDecode);

    // Task 3.1: share route `#/s/<sessionId>/<token>` — capture the token
    // into _shareTokens then strip it from the visible URL + history so it
    // never lingers in the address bar, referrer headers, or browser history.
    if (this.parts[0] === 's' && this.parts.length >= 3) {
      const sessionId = this.parts[1];
      const token = this.parts[2];
      if (sessionId && token) {
        _shareTokens.set(sessionId, token);
        // Remove the token segment from the URL immediately (replaceState so
        // it doesn't create a new history entry — the token is one-time-view).
        const cleanHash = `#/s/${encodeURIComponent(sessionId)}`;
        history.replaceState(null, '', cleanHash);
        // Re-parse the now-clean URL so this.parts reflects the stripped form.
        const cleanRaw = cleanHash.replace(/^#\/?/, '');
        this.parts = cleanRaw.split('/').map(safeDecode);
      }
    }
  }

  private currentHash(): string {
    return window.location.hash || '#/';
  }

  /** Install (or clear) the split-view partner — see {@link RouteDelegate}. */
  setDelegate(d: RouteDelegate | null): void {
    this.delegate = d;
  }

  private claimed(hash: string): boolean {
    return !!this.delegate && this.delegate.claims(hash.replace(/^#\/?/, ''));
  }

  /** Hand `hash` to the delegate when it claims it; true = handled there. */
  private divert(hash: string): boolean {
    if (!this.claimed(hash)) return false;
    this.delegate!.deliver(hash.replace(/^#\/?/, ''));
    return true;
  }

  /**
   * Register a leave-guard; returns its unregister function (hand it straight
   * back as an `$effect` cleanup). `go()`, `back()`/`forward()`, links and
   * sidebar navigation all await every guard first; one false keeps the route
   * (a link/hash change is reverted to the previous hash). `replace()` — a
   * programmatic canonicalisation, not a person leaving — is never guarded.
   * For the usual unsaved-editor case use `guardUnsaved` (lib/leaveGuard.ts).
   */
  guard(fn: LeaveGuard): () => void {
    this.guards.add(fn);
    return () => {
      this.guards.delete(fn);
    };
  }

  /** Ask every guard about leaving for `hash`; true = go ahead. A newer guarded
   *  navigation started meanwhile makes this one resolve false. */
  private async mayLeave(hash: string): Promise<boolean> {
    const seq = ++this.guardSeq;
    const to = hash.replace(/^#\/?/, '');
    for (const g of [...this.guards]) {
      let ok = false;
      try {
        ok = await g(to);
      } catch {
        ok = false;
      }
      if (!ok || seq !== this.guardSeq) return false;
    }
    return seq === this.guardSeq;
  }

  /** Set the hash after its guards passed (so onHashChange doesn't re-ask). */
  private setHash(hash: string): void {
    this.approved = hash;
    window.location.hash = hash;
  }

  private onHashChange(): void {
    // A link (`<a href="#/…">`) or a direct hash write bypasses go(): divert a
    // claimed route after the fact and put this pane's hash back, leaving the
    // page, the stack and the persisted route untouched.
    if (!this.navigating) {
      const prev = this.stack[this.index];
      const h = this.currentHash();
      if (prev !== undefined && h !== prev && this.divert(h)) {
        history.replaceState(null, '', prev);
        return;
      }
      // Unapproved navigation (a link, a direct hash write) while a guard is
      // registered: put the old hash back at once so the page doesn't change,
      // then re-issue the navigation only if every guard agrees.
      if (prev !== undefined && h !== prev && this.guards.size > 0 && this.approved !== h) {
        history.replaceState(null, '', prev);
        void this.mayLeave(h).then((ok) => {
          if (ok) this.setHash(h);
        });
        return;
      }
    }
    this.approved = null;
    this.parse();
    persistLastRoute(this.currentHash());
    if (this.navigating) {
      this.navigating = false;
      return;
    }
    // A normal navigation: truncate any forward history and push.
    const h = this.currentHash();
    if (this.stack[this.index] === h) return;
    this.stack = [...this.stack.slice(0, this.index + 1), h];
    this.index = this.stack.length - 1;
  }

  /** first segment, '' when none */
  get module(): string {
    return this.parts[0] ?? '';
  }

  private toHash(path: string): string {
    return path.startsWith('#') ? path : `#/${path.replace(/^\//, '')}`;
  }

  go(path: string): void {
    const hash = this.toHash(path);
    if (hash === this.currentHash()) return;
    if (this.divert(hash)) return;
    if (this.guards.size === 0) {
      window.location.hash = hash;
      return;
    }
    void this.mayLeave(hash).then((ok) => {
      if (ok) this.setHash(hash);
    });
  }

  replace(path: string): void {
    const hash = this.toHash(path);
    if (hash !== this.currentHash() && this.divert(hash)) return;
    history.replaceState(null, '', hash);
    this.parse();
    persistLastRoute(this.currentHash());
    if (this.index >= 0) this.stack[this.index] = this.currentHash();
  }

  /** Back/forward step over entries the split-view partner now owns (a
   *  module that moved into the other pane) rather than opening it twice. */
  back(): void {
    let i = this.index - 1;
    while (i >= 0 && this.claimed(this.stack[i])) i -= 1;
    if (i < 0) return;
    this.step(i);
  }

  forward(): void {
    let i = this.index + 1;
    while (i < this.stack.length && this.claimed(this.stack[i])) i += 1;
    if (i >= this.stack.length) return;
    this.step(i);
  }

  /** Move the stack pointer to `i` once the leave-guards agree. */
  private step(i: number): void {
    const target = this.stack[i];
    if (this.guards.size === 0 || target === this.currentHash()) {
      this.index = i;
      this.moveTo(target);
      return;
    }
    const from = this.index;
    void this.mayLeave(target).then((ok) => {
      // The stack moved while the guard was asking (another navigation won).
      if (!ok || this.index !== from || this.stack[i] !== target) return;
      this.index = i;
      this.moveTo(target);
    });
  }

  /** Internal back/forward. Setting the hash to its CURRENT value fires no
   *  hashchange, which left `navigating` stuck true and made the next real
   *  navigation skip its history push — only flag it when a change will fire. */
  private moveTo(hash: string): void {
    if (hash === this.currentHash()) return;
    this.navigating = true;
    window.location.hash = hash;
  }
}

export const router = new Router();
