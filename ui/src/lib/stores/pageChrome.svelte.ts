// A page's own, NON-persisted chrome trim (Threads, like T3: no side-panel
// rail, no status bar). The shell reads it; the page sets it while mounted.
// The rail still shows once the person opens the panel; the status bar still
// shows while the daemon is disconnected.
class PageChrome {
  hideSideRail = $state(false);
  hideStatusBar = $state(false);

  set(opts: { hideSideRail?: boolean; hideStatusBar?: boolean }): () => void {
    this.hideSideRail = !!opts.hideSideRail;
    this.hideStatusBar = !!opts.hideStatusBar;
    return () => {
      this.hideSideRail = false;
      this.hideStatusBar = false;
    };
  }
}

export const pageChrome = new PageChrome();
