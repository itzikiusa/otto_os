// A page's own, NON-persisted chrome trim (Threads, like T3: no status bar).
// The shell reads it; the page sets it while mounted. The status bar still
// shows while the daemon is disconnected. (The Side panel is on every page —
// design.md §1 — so no page hides its rail any more; a tablet shows the Rail
// by default, so no page asks the shell to fold the Navigator either.)
class PageChrome {
  hideStatusBar = $state(false);

  set(opts: { hideStatusBar?: boolean }): () => void {
    this.hideStatusBar = !!opts.hideStatusBar;
    return () => {
      this.hideStatusBar = false;
    };
  }
}

export const pageChrome = new PageChrome();
