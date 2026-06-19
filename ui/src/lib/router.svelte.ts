// Tiny hash router. Routes look like "#/agents", "#/git/<repoId>/pr/42",
// "#/settings/appearance". No SvelteKit — App.svelte switches on `router.module`.
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

/** Retrieve the in-memory share token captured for a given session.
 *  Returns null if the URL didn't carry one or the token has been consumed. */
export function getShareToken(sessionId: string): string | null {
  return _shareTokens.get(sessionId) ?? null;
}

class Router {
  /** path segments after '#/', e.g. ['git', '01H...', 'pr', '7'] */
  parts: string[] = $state([]);

  /** navigation history of hashes; `index` points at the current entry. */
  private stack: string[] = $state([]);
  private index = $state(-1);
  /** set while doing an internal back/forward so onHashChange doesn't push. */
  private navigating = false;

  canBack = $derived(this.index > 0);
  canForward = $derived(this.index < this.stack.length - 1);

  constructor() {
    if (typeof window !== 'undefined') {
      this.parse();
      this.stack = [this.currentHash()];
      this.index = 0;
      window.addEventListener('hashchange', () => this.onHashChange());
    }
  }

  private parse(): void {
    const raw = window.location.hash.replace(/^#\/?/, '');
    this.parts = raw === '' ? [] : raw.split('/').map(decodeURIComponent);

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
        this.parts = cleanRaw.split('/').map(decodeURIComponent);
      }
    }
  }

  private currentHash(): string {
    return window.location.hash || '#/';
  }

  private onHashChange(): void {
    this.parse();
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
    window.location.hash = hash;
  }

  replace(path: string): void {
    const hash = this.toHash(path);
    history.replaceState(null, '', hash);
    this.parse();
    if (this.index >= 0) this.stack[this.index] = this.currentHash();
  }

  back(): void {
    if (this.index <= 0) return;
    this.index -= 1;
    this.navigating = true;
    window.location.hash = this.stack[this.index];
  }

  forward(): void {
    if (this.index >= this.stack.length - 1) return;
    this.index += 1;
    this.navigating = true;
    window.location.hash = this.stack[this.index];
  }
}

export const router = new Router();
