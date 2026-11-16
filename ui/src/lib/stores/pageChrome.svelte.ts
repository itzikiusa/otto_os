// A page's own, NON-persisted chrome trim (Threads, like T3: no side-panel
// rail, no status bar). The shell reads it; the page sets it while mounted.
// The rail still shows once the person opens the panel; the status bar still
// shows while the daemon is disconnected.
//
// `foldNav`: on a tablet the shell's Navigator is a fixed 220 px column; a
// page with its own list column (Threads) asks for the icon rail instead, as
// desktop already folds it there (App.svelte). Expanding the rail still shows
// the full Navigator (`ui.railExpanded`).
class PageChrome {
  hideSideRail = $state(false);
  hideStatusBar = $state(false);
  foldNav = $state(false);

  set(opts: { hideSideRail?: boolean; hideStatusBar?: boolean; foldNav?: boolean }): () => void {
    this.hideSideRail = !!opts.hideSideRail;
    this.hideStatusBar = !!opts.hideStatusBar;
    this.foldNav = !!opts.foldNav;
    return () => {
      this.hideSideRail = false;
      this.hideStatusBar = false;
      this.foldNav = false;
    };
  }
}

export const pageChrome = new PageChrome();
