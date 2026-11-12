// Which URL each native live-tab webview shows, kept per window in module
// state rather than in a `BrowserView` instance. The Browser page and the
// right panel's v2 Browser tab both mount `BrowserView`, and the panel mounts
// it afresh on every panel-tab switch: a per-instance map started empty, so
// the re-mounted view navigated the existing webview again — reloading the
// page and losing its scroll, form and app state. With the map here, a
// re-mounted view only SHOWS the webview; a navigation happens only when the
// tab's URL really changed. (The desktop shell's `browser_open` is also a
// no-op for navigation when the webview already shows that URL, which covers
// a webview another host closed and this map still lists.)

const shown = new Map<string, string>();
let mru: string[] = [];

/** One page for comparison: `https://a.example` and `https://a.example/`. */
export function sameLiveUrl(a: string | undefined, b: string): boolean {
  if (a === undefined) return false;
  const norm = (u: string): string => {
    try {
      return new URL(u.trim()).href;
    } catch {
      return u.trim();
    }
  };
  return norm(a) === norm(b);
}

/** History / loading commands for the active live tab. */
export type LiveNav = 'back' | 'forward' | 'reload' | 'stop';

/** The page script a native webview runs for `nav`; reload goes through the
 *  shell's own `browser_reload`. */
export const NATIVE_NAV_JS: Record<Exclude<LiveNav, 'reload'>, string> = {
  back: 'history.back()',
  forward: 'history.forward()',
  stop: 'window.stop()',
};

export const liveWebviews = {
  /** `navigate` when tab `id`'s webview doesn't show `url` yet (never
   *  opened, closed, or the tab moved on); `show` when it already does. */
  action(id: string, url: string): 'navigate' | 'show' {
    return sameLiveUrl(shown.get(id), url) ? 'show' : 'navigate';
  },
  /** Tab `id`'s webview now shows `url` (opened, or an in-page navigation). */
  set(id: string, url: string): void {
    shown.set(id, url);
  },
  /** Tab `id`'s webview was closed: its next activation opens it afresh. */
  forget(id: string): void {
    shown.delete(id);
    mru = mru.filter((x) => x !== id);
  },
  /** Mark `id` the most recently shown; returns the opened webviews beyond
   *  the `max` most recent, already forgotten — the caller closes them. */
  touch(id: string, max: number): string[] {
    mru = [id, ...mru.filter((x) => x !== id)];
    const evicted: string[] = [];
    while (mru.length > max) {
      const old = mru.pop();
      if (old && old !== id && shown.has(old)) {
        shown.delete(old);
        evicted.push(old);
      }
    }
    return evicted;
  },
};
