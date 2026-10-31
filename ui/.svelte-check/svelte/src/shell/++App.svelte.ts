///<reference types="svelte" />
;
import { transcript as transcriptStore } from '../lib/stores/transcript.svelte';
import { NO_WORKSPACE } from '../lib/labels';
import { toastError } from '../lib/toastError';
import { onMount, untrack } from 'svelte';
import { installShell } from './installShell.svelte';
// Main shell (post-auth): rail/navigator + tab bar + module router +
// right panel + status bar + palette + global keys.
//
// Special case: `router.module === 's'` is the guest share view — it renders
// SharePage full-screen and skips all the usual shell chrome entirely.
import Rail from './Rail.svelte';
import Navigator from './Navigator.svelte';
import TabBar from './TabBar.svelte';
import BottomNav from './BottomNav.svelte';
import Drawer from './Drawer.svelte';
import Skeleton from '../lib/components/Skeleton.svelte';
import NavButtons from './NavButtons.svelte';
import MobileActionBar from './MobileActionBar.svelte';
import Icon from '../lib/components/Icon.svelte';
import LoadState from '../lib/components/LoadState.svelte';
import LazyMount from '../lib/components/LazyMount.svelte';
import {
    SharePageChunk,
    HandoverChunk,
    AttachIssueChunk,
    AttachProductStoryChunk,
    BroadcastModalChunk,
    NewSessionChunk,
    NewWorkspaceChunk,
  } from './overlays.svelte';
import { loadErrorText } from '../lib/loadError';
import StatusBar from './StatusBar.svelte';
import CapsBanner from './CapsBanner.svelte';
import Palette from './Palette.svelte';
import FloatingBar from '../lib/components/FloatingBar.svelte';
import { barStore } from '../lib/stores/bar.svelte';
import ShortcutsOverlay from './ShortcutsOverlay.svelte';
import { confirmer } from '../lib/confirm.svelte';
import NotificationBell from './NotificationBell.svelte';
import { serviceHealth } from '../lib/stores/serviceHealth.svelte';
import ConfirmDialog from '../lib/components/ConfirmDialog.svelte';
import ContextMenu from '../lib/components/ContextMenu.svelte';
import FindInPage from '../lib/components/FindInPage.svelte';
import { findInPage } from '../lib/findinpage.svelte';
import { plugins } from '../lib/stores/plugins.svelte';
import { router } from '../lib/router.svelte';
import { navPending } from '../lib/navPending.svelte';
import { startSnip } from '../lib/snip';
import { ui, isTauri } from '../lib/stores/ui.svelte';
import { SIDE_PANEL } from '../lib/rightTabs';
import { rightPanel } from '../lib/stores/rightPanel.svelte';
import { startWindowDrag } from '../lib/windowDrag';
import { isPopout, isEmbedded, popoutTitle, currentRoute, closePopoutWindow } from '../lib/desktop';
import SidePane from './SidePane.svelte';
import {
    pageKeyOf,
    loadedPage,
    loadPage,
    pageError,
    prepareRoute,
    prefetchPagesWhenIdle,
    ensureRightPanel,
    installNavPrefetch,
    loadedRightPanel,
  } from './pages.svelte';
import AgentDrivingBar from '../lib/components/AgentDrivingBar.svelte';
import { uiControl } from '../lib/stores/uiControl.svelte';
// Agent UI control: every module's `ui_*` handlers, registered before the
// events socket introduces this document (`hello.capabilities`).
import '../lib/uiCommands/index';
import SplitDivider from './SplitDivider.svelte';
import { sidePane } from '../lib/stores/sidePane.svelte';
import { startGuest, postToHost } from '../lib/embedGuest';
import { embeddedKeyTarget, paneKey, routeOf, clampShare } from '../lib/sidePane';
import { nativePane } from '../lib/nativePane';
import { paneWindowAction, showPaneDisplays } from '../lib/nativePaneMenu';
import { ctxMenu } from '../lib/contextmenu.svelte';
import { viewport } from '../lib/stores/viewport.svelte';
import { ws } from '../lib/stores/workspace.svelte';
import { git } from '../lib/stores/git.svelte';
import { auth, impersonationStartedMs } from '../lib/stores/auth.svelte';
import { events } from '../lib/events.svelte';
import { dismissTopDialog, installKeyMap, modalKeyVerdict, routeFind, type KeyAction } from '../lib/keys';
import { attachMenuBridge, attachCloseHandler, handleMenu } from '../lib/menu';
import { gcWindowKeys } from '../lib/win';
import { openExternal, isExternalUrl } from '../lib/external';
import { registry, type Command } from '../lib/commands.svelte';
import { availableModules, groupLabel, moduleLabel, resolveOrder, visibleOrder } from '../lib/sidebar';
import { prefetchQueue, shouldPrefetch } from '../lib/prefetchPlan';
import { api, baseUrl, getToken } from '../lib/api/client';
import type { Session } from '../lib/api/types';
import { toasts } from '../lib/toast.svelte';
import { now } from '../lib/stores/now.svelte';
import { notifications } from '../lib/stores/notifications.svelte';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  // Effects that used to sit between the imports, in their original order.
  // Transcript store: follows page visibility; reset on unmount.
  $effect(() => {
    const onVis = (): void => transcriptStore.setVisible(!document.hidden);
    onVis();
    document.addEventListener('visibilitychange', onVis);
    return () => {
      document.removeEventListener('visibilitychange', onVis);
      transcriptStore.reset();
    };
  });
  // Old bookmarks lead to the workspace's existing context editor.
  $effect(() => { if (router.module === 'projects') router.go('settings/context-soul'); });
  // Transcript cache identity: daemon + signed-in user.
  $effect(() => {transcriptStore.setIdentity(JSON.stringify([baseUrl(),auth.me?.id ?? '']));});

  const moduleName = $derived(router.module === '' ? 'agents' : router.module);

  // Overlay close handlers, shared by each dialog's `onclose` and its
  // LazyMount `onerror` (a failed chunk resets the flag so the next open retries).
  function closeSessionAction(): void {
    sessionAction = null;
  }
  function closeNewSession(): void {
    ui.newSessionOpen = false;
    ui.newSessionScratch = false;
  }
  function closeNewWorkspace(): void {
    ui.newWorkspaceOpen = false;
  }

  // ---- module pages: one lazy chunk each (shell/pages.svelte.ts) ----
  // The router holds the current route until the next page's chunk is in
  // (`prepareRoute`), so `pageKey` only ever names a page that can render now:
  // the old page keeps painting until the new one mounts in the same frame.
  // A pop-out / side pane loads only what it shows; the main window warms the
  // rest at idle after first paint and on sidebar hover/focus.
  const pageKey = $derived(pageKeyOf(router.parts) ?? 'agents');
  const Page = $derived(loadedPage(pageKey));
  const pageFailed = $derived(Page ? undefined : pageError(pageKey));
  $effect(() => {
    router.setPrepare(prepareRoute);
    return () => router.setPrepare(null);
  });
  $effect(() => {
    // The first page normally arrives with the shell (the root App preloads
    // it); a deep link to a page whose load failed retries from here.
    if (!untrack(() => loadedPage(pageKey))) void loadPage(pageKey).catch(() => {});
  });
  $effect(() => installNavPrefetch());
  // Boot timing marks (perf F1: e2e/desktop-boot-perf.spec.ts budgets them).
  // Once per document: `otto:shell-mounted` on the shell's first effect,
  // `otto:page-painted` after the frame that first paints a page.
  $effect(() => {
    if (!performance.getEntriesByName('otto:shell-mounted').length) performance.mark('otto:shell-mounted');
  });
  let pagePaintMarked = false;
  $effect(() => {
    if (!Page || pagePaintMarked) return;
    pagePaintMarked = true;
    requestAnimationFrame(() => setTimeout(() => performance.mark('otto:page-painted'), 0));
  });
  // The Agents loader starts the right panel's chunk without waiting on it;
  // re-ask while it's on screen so a failed load retries.
  $effect(() => {
    if (showRightPanel && !loadedRightPanel()) void ensureRightPanel();
  });
  $effect(() => {
    if (isEmbedded || isPopout) return;
    // Planned once per shell: the visible, permitted modules (Favorites
    // first), capped — and nothing on a phone / Save-Data / remote daemon.
    const queue = untrack(() => {
      const conn = (navigator as Navigator & { connection?: { saveData?: boolean } }).connection;
      if (!shouldPrefetch({ isPhone: viewport.isPhone, saveData: conn?.saveData, base: baseUrl(), href: location.href })) return [];
      const visible = visibleOrder(resolveOrder(splitModules, ui.sidebarOrder), ui.sidebarHidden).map((m) => m.id);
      return prefetchQueue(visible, ui.sidebarFavorites, (id) => pageKeyOf(id.split('/')), { skip: [pageKey] });
    });
    return queue.length ? prefetchPagesWhenIdle(queue) : undefined;
  });
  const compactShell = $derived(!viewport.isDesktop && !isPopout && !isEmbedded);
  // The page draws its own PageHeader (the first registered header, in the
  // centre column — not one inside the right panel's browser tab).
  const pageTitled = $derived(compactShell && !!uiControl.barOwner?.closest('.mcenter'));

  // Native sidebar vibrancy (desktop shell): the window has an NSVisualEffect
  // view behind the page, so the document goes transparent and ONLY chrome
  // lets it through — the sidebar's `.sidebar-material` 78% tint and the
  // titlebar strips — while `.center` / right panel / status bar stay opaque.
  // Off for the full-screen routes (snip editor, share view) and under
  // reduced transparency (CSS below).
  const vibrant = $derived(
    isTauri &&
      !isEmbedded &&
      (viewport.isDesktop || isPopout) &&
      router.module !== 'snip' &&
      router.module !== 's',
  );
  $effect(() => {
    document.documentElement.classList.toggle('otto-vibrant', vibrant);
    return () => document.documentElement.classList.remove('otto-vibrant');
  });
  // Phone top-bar title: the registry label ("Mission Control", "Skills Lab"),
  // a plugin's own name, never the raw route id.
  const moduleTitle = $derived(
    moduleName === 'plugin'
      ? (plugins.list.find((p) => p.slug === router.parts[1])?.name ?? 'Plugin')
      : moduleLabel(moduleName),
  );
  // Browser/PWA tab, history entries and the window list name the page, not
  // just "Otto". Not in a pop-out (its title comes from the shell) or the
  // embedded side pane (its document title is never shown).
  $effect(() => {
    if (isEmbedded || isPopout) return;
    const page = moduleName === 'agents' ? (ws.activeSession?.title ?? 'Agents') : moduleTitle;
    document.title = page ? `${page} — Otto` : 'Otto';
  });
  // The phone Navigator drawer closes on every navigation (tapping a module or
  // a session row in it should land on that page, not leave it covered).
  $effect(() => {
    void router.parts.join('/');
    ui.navDrawerOpen = false;
  });

  // The Side panel (Git/Files/Notes/…, plus a page's own tabs) is on every
  // page, pop-out windows included (a popped-out page keeps its tools; e2e
  // desktop-rpanel-tabs): beside an agent session it follows the session,
  // elsewhere a workspace folder (stores/rightPanel.svelte.ts). The
  // side-by-side pane's document keeps the old rule — only beside an agent
  // session — so a split never shows two rails.
  const showRightPanel = $derived(!isEmbedded || rightPanel.session);

  // ---- "needs you" raise/lower flag (lib/attention.ts) ----
  // The operator is present while this window is visible and focused. Focus
  // moving INTO an iframe of this window (file preview, Outputs, Browser,
  // canvas) blurs the window but the operator is still here:
  // `document.hasFocus()` is true while a nested frame has focus, so a blur
  // is re-checked a tick later instead of trusted.
  $effect(() => {
    let blurTimer: ReturnType<typeof setTimeout> | undefined;
    const sync = (): void => {
      ws.operatorPresent = document.visibilityState === 'visible' && document.hasFocus();
    };
    const onVis = (): void => sync();
    const onFocus = (e: Event): void => {
      clearTimeout(blurTimer);
      if (e.type === 'focus') ws.operatorPresent = document.visibilityState === 'visible';
      else blurTimer = setTimeout(sync, 0);
    };
    // Another window lowered / raised a flag: adopt it.
    const onStorage = (e: StorageEvent): void => {
      if (e.storageArea === localStorage && e.key === 'otto_attention') ws.reloadAttention();
    };
    document.addEventListener('visibilitychange', onVis);
    window.addEventListener('focus', onFocus);
    window.addEventListener('blur', onFocus);
    window.addEventListener('storage', onStorage);
    return () => {
      clearTimeout(blurTimer);
      document.removeEventListener('visibilitychange', onVis);
      window.removeEventListener('focus', onFocus);
      window.removeEventListener('blur', onFocus);
      window.removeEventListener('storage', onStorage);
    };
  });
  // LOWER: entering / focusing a session (even doing nothing) puts its flag
  // down and answers its waiting/idle notices; it stays down until the next
  // turn end or prompt.
  $effect(() => {
    const id = ws.activeSessionId;
    if (!id || moduleName !== 'agents' || !ws.operatorPresent) return;
    untrack(() => {
      ws.clearNeedsYou(id, notifications.sessionNoticeAt(id));
      void notifications.markSessionSeen(id);
    });
  });
  // A flag restored from STORAGE is settled once its session's status is
  // known: working again or exited means it's stale (the live
  // `session_status` rule, applied once to loaded rows). Flags raised in this
  // run — e.g. a permission prompt while the status still reads working — are
  // never touched here.
  $effect(() => {
    void ws.needsYou;
    const known = ws.restoredFlagIds().map((id) => [id, ws.statusMap[id]] as const).filter(([, st]) => st !== undefined);
    if (known.length) untrack(() => known.forEach(([id, st]) => ws.settleRestoredFlag(id, st)));
  });

  // Load the runtime plugin list once authenticated (drives the sidebar). Reads
  // auth.phase only; the write to plugins.list isn't read here, so no loop.
  $effect(() => {
    if (auth.phase === 'ready') {
      plugins.load();
    }
  });

  // ---- impersonation countdown ---
  // Counts down the 30-minute impersonation token from when THIS tab started
  // it (kept in sessionStorage next to the admin token — see
  // `impersonationStartedMs`). Unknown start (an older build's session) →
  // the conservative "30 min from this load".
  const IMP_DURATION_MS = 30 * 60 * 1000;
  const impLoadedAt = Date.now();

  const impSecsLeft = $derived.by(() => {
    if (!auth.isImpersonating) return 0;
    void now(); // reactive tick
    const startMs = impersonationStartedMs() ?? impLoadedAt;
    const elapsed = Date.now() - startMs;
    return Math.max(0, Math.ceil((IMP_DURATION_MS - elapsed) / 1000));
  });

  /** What the polite sr-only region says: thresholds only (5 min, 1 min,
   *  expired), so the ticking countdown is never spoken every second. The
   *  string changes only at an edge, and an unchanged string isn't re-read. */
  const impAnnounce = $derived.by(() => {
    if (!auth.isImpersonating) return '';
    if (impSecsLeft <= 0) return 'Impersonation session may have expired';
    if (impSecsLeft <= 60) return '1 minute of impersonation left';
    if (impSecsLeft <= 300) return '5 minutes of impersonation left';
    return '';
  });

  function fmtImpCountdown(s: number): string {
    if (s <= 0) return '0:00';
    const m = Math.floor(s / 60);
    const sec = s % 60;
    return `${m}:${sec.toString().padStart(2, '0')}`;
  }

  // ---- route → store sync (one-way; avoids route↔store loop) ----
  // When the URL is `#/agents/<sessionId>` (set by navigateToSession or Back/Forward),
  // apply the session id into the workspace store's pane/tab state. This is
  // intentionally ONE-WAY: the store mutation (openSession) does NOT re-navigate,
  // so there is no loop. The router's own `navigating` flag guards against
  // recursive hash changes during back/forward.
  $effect(() => {
    const sessionId = router.parts[1];
    const module = router.module;
    if (module === 'agents' && sessionId) {
      // React ONLY to route changes. `activeSessionId` is read untracked: it
      // changes whenever you focus a different pane (focusedPane moves), and if
      // this effect tracked it, focusing pane B would re-fire here with the URL
      // still on pane A and "restore" A into the focused pane — clobbering the
      // click. Reading it untracked lets focusPane move the frame freely; a real
      // route change (tab/navigator/back-forward) still falls through to sync.
      if (untrack(() => ws.activeSessionId) !== sessionId) {
        ws.openSession(sessionId);
      }
    }
  });

  // ---- `?` shortcuts cheat-sheet + focused-session action modals ----
  let shortcutsOpen = $state(false);
  // Session-scoped modals the palette opens against the active session. They
  // mirror the per-pane ⋯ menu but target ws.activeSession globally.
  let sessionAction: { kind: 'handover' | 'attach-issue' | 'attach-product'; sessionId: string } | null =
    $state(null);

  function openSessionAction(kind: 'handover' | 'attach-issue' | 'attach-product'): void {
    const s = ws.activeSession;
    if (!s) return;
    sessionAction = { kind, sessionId: s.id };
  }

  async function renameActiveSession(): Promise<void> {
    const s = ws.activeSession;
    if (!s) return;
    const next = await confirmer.promptText('Rename session', {
      title: 'Rename session',
      confirmLabel: 'Rename',
      initial: s.title,
      placeholder: 'Session name',
    });
    if (!next || next === s.title) return;
    try {
      await ws.renameSession(s.id, next);
    } catch (e) {
      toastError('Couldn’t rename the session', e);
    }
  }

  // ---- mobile action bar helpers ----
  // These are the SAME actions the keyboard map calls (App.svelte:187-270).
  // Exposed as named functions so MobileActionBar can receive them as props
  // without duplicating any logic.
  function mobileOpenPalette(): void {
    if (ui.paletteOpen) ui.paletteOpen = false;
    else ui.openPalette('commands');
  }
  function mobileNewSession(): void {
    ui.newSessionOpen = true;
  }
  function mobileCloseTab(): void {
    ws.closeActiveTab();
  }
  function mobileFind(): void {
    // A focused component (terminal OR the DB query editor) owns find, then the
    // active session pane's terminal; else the page-wide find-in-page overlay.
    routeFind(() => findInPage.show());
  }
  function mobileBroadcast(): void {
    ui.openBroadcast();
  }

  // `?` (no modifier, not while typing) opens the shortcuts cheat-sheet. The
  // global keymap in lib/keys.ts only handles modifier chords, so `?` lives here.
  $effect(() => {
    function onHelpKey(e: KeyboardEvent): void {
      if (e.key !== '?' || e.metaKey || e.ctrlKey || e.altKey) return;
      // A module that owns `?` (e.g. the Kubernetes workspace) handles it first
      // in the capture phase and marks it consumed.
      if (e.defaultPrevented) return;
      const el = document.activeElement as HTMLElement | null;
      const typing =
        !!el &&
        (el.tagName === 'INPUT' ||
          el.tagName === 'TEXTAREA' ||
          el.isContentEditable ||
          !!el.closest('.cm-editor'));
      if (typing || ui.overlayOpen) return;
      e.preventDefault();
      // In the side-by-side pane the sheet belongs to the window.
      if (isEmbedded) postToHost({ type: 'key', action: 'shortcuts' });
      else shortcutsOpen = true;
    }
    window.addEventListener('keydown', onHelpKey);
    return () => window.removeEventListener('keydown', onHelpKey);
  });

  // Reload scoped data when impersonation changes the effective identity.
  // Keep this separate from the once-per-window event/native subscriptions.
  let workspaceLoadError = $state<string | null>(null);
  let workspaceLoading = $state(false);
  let workspaceLoadGeneration = 0;
  async function loadWorkspaces(): Promise<void> {
    const generation = ++workspaceLoadGeneration;
    const token = getToken();
    const current = () => generation === workspaceLoadGeneration && token === getToken();
    workspaceLoading = true;
    try {
      await ws.load();
      if (current()) workspaceLoadError = null;
    } catch (error) {
      if (current()) workspaceLoadError = loadErrorText(error);
    } finally {
      if (current()) workspaceLoading = false;
    }
  }
  $effect(() => {
    if (auth.me?.id) untrack(() => {
      workspaceLoadError = null;
      void loadWorkspaces();
    });
    return () => { ++workspaceLoadGeneration; };
  });

  // ---- boot ----
  $effect(() => {
    events.start();
    let unlistenMenu: (() => void) | null = null;
    let unlistenClose: (() => void) | null = null;
    // Once per WINDOW, never in the side-by-side pane (an iframe of this
    // window): window-key GC and the native menu bridge (the host forwards
    // the pane its menu items — a second listener would run each one twice).
    if (!isEmbedded) {
      void gcWindowKeys();
      void attachMenuBridge().then((fn) => (unlistenMenu = fn));
      void attachCloseHandler().then((fn) => (unlistenClose = fn));
    }

    // Suppress the native WKWebView context menu globally, except on editable
    // elements and elements that opt in with .allow-native-menu.
    function suppressNativeMenu(e: MouseEvent): void {
      const target = e.target as Element | null;
      if (!target) return;
      if (
        target.closest('input') ||
        target.closest('textarea') ||
        target.closest('[contenteditable]') ||
        target.closest('.allow-native-menu')
      ) {
        return;
      }
      e.preventDefault();
    }
    window.addEventListener('contextmenu', suppressNativeMenu);

    // External links: a `<a target="_blank">` to an http(s) URL won't reach the
    // system browser inside the Tauri webview, so intercept those clicks and
    // hand them to the shell `open` command. Internal hash routes are untouched.
    function onLinkClick(e: MouseEvent): void {
      if (e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey) return;
      const a = (e.target as Element | null)?.closest?.('a');
      if (!a) return;
      const href = a.getAttribute('href');
      if (a.target === '_blank' && isExternalUrl(href)) {
        e.preventDefault();
        void openExternal(href);
      }
    }
    window.addEventListener('click', onLinkClick, { capture: true });

    return () => {
      events.stop();
      unlistenMenu?.();
      unlistenClose?.();
      window.removeEventListener('contextmenu', suppressNativeMenu);
      window.removeEventListener('click', onLinkClick, { capture: true });
    };
  });

  // Automatic network work belongs to the visible Git page. Leaving it stops
  // queued/background work; returning resumes due repos with the same backoff.
  $effect(() => {
    if (moduleName !== 'git') return;
    let stopped = false;
    void untrack(() => git.initializeOpenTabs()).then(() => {
      if (!stopped) git.startAutoFetch();
    });
    const wake = (): void => git.requestAutoFetch();
    window.addEventListener('focus', wake);
    document.addEventListener('visibilitychange', wake);
    return () => {
      stopped = true;
      window.removeEventListener('focus', wake);
      document.removeEventListener('visibilitychange', wake);
      git.stopAutoFetch();
    };
  });

  // keep git store in sync with workspace
  $effect(() => {
    if (ws.currentId) void git.loadRepos(ws.currentId);
  });
  // …and retry a failed load when the window comes back (S13-305): the
  // effect above only re-runs on a workspace switch.
  $effect(() => {
    const retry = (): void => {
      if (git.reposError && ws.currentId) void git.loadRepos(ws.currentId);
    };
    window.addEventListener('focus', retry);
    return () => window.removeEventListener('focus', retry);
  });

  // ---- keyboard map ----
  // One dispatcher for the key map, the side-by-side pane (which hands the
  // window its window-level chords) and nothing else.
  function runKeyAction(action: KeyAction | 'shortcuts', index?: number): void {
    // A dialog is up: ⌘W closes it, window verbs never act behind it.
    const verdict =
      ui.paletteOpen && ui.modalCount === 0 && (action === 'palette' || action === 'askOtto')
        ? 'run'
        : modalKeyVerdict(action, ui.modalCount > 0 || ui.paletteOpen);
    if (verdict === 'dismiss') {
      if (ui.paletteOpen && ui.modalCount === 0) {
        ui.paletteOpen = false;
        return;
      }
      return dismissTopDialog();
    }
    if (verdict === 'drop') return;
    switch (action) {
      case 'shortcuts':
        shortcutsOpen = true;
        break;
      case 'toggleSidePane':
        if (sidePane.route !== null && sidePane.active) sidePane.close();
        else openSidePicker();
        break;
      case 'palette':
        // Desktop: ⌘K focuses the floating bar (the one command surface).
        // Phone/tablet, pop-outs and a hidden bar keep the palette sheet.
        if (ui.paletteOpen) ui.paletteOpen = false;
        else if (barStore.mounted) barStore.requestFocus();
        else ui.openPalette('commands');
        break;
      case 'askOtto':
        ui.openPalette('english');
        break;
      case 'settings':
        router.go('settings/appearance');
        break;
      case 'updateCLIs':
        void updateAllCLIs();
        break;
      case 'snip':
        void startSnip();
        break;
      case 'broadcast':
        ui.openBroadcast();
        break;
      case 'hardReload':
        // Full UI refresh, like a browser hard reload. Sessions, workspaces,
        // and everything else live in the daemon (the active workspace + open
        // panes are restored from localStorage), so nothing is lost — this just
        // re-fetches fresh state and clears any stale in-memory UI.
        window.location.reload();
        break;
      case 'toggleRail':
        ui.toggleRail();
        break;
      case 'toggleRight':
        ui.toggleRight();
        break;
      case 'newSession':
        ui.newSessionOpen = true;
        break;
      case 'closeTab':
        // Off the Agents page there is no tab to close: a pop-out closes itself.
        if (!ws.closeActiveTab()) void closePopoutWindow();
        break;
      case 'reopenTab':
        void ws.reopenClosedTab();
        break;
      case 'nextTab':
        ws.cycleTab(1);
        break;
      case 'prevTab':
        ws.cycleTab(-1);
        break;
      case 'nextSession':
        ws.cycleTab(1);
        break;
      case 'prevSession':
        ws.cycleTab(-1);
        break;
      case 'jumpSession':
        if (index) ws.focusSessionByIndex(index);
        break;
      case 'splitVertical':
        ws.split('col');
        break;
      case 'splitHorizontal':
        ws.split('row');
        break;
      case 'find':
        routeFind(() => findInPage.show());
        break;
      case 'appZoomIn':
        ui.zoomIn();
        break;
      case 'appZoomOut':
        ui.zoomOut();
        break;
      case 'appZoomReset':
        ui.zoomReset();
        break;
      case 'termZoomIn':
        ui.termZoomIn();
        break;
      case 'termZoomOut':
        ui.termZoomOut();
        break;
      case 'termZoomReset':
        ui.termZoomReset();
        break;
      case 'navBack':
        router.back();
        break;
      case 'navForward':
        router.forward();
        break;
    }
  }
  $effect(() => {
    return installKeyMap((action, _e, index) => {
      if (isEmbedded) {
        // A dialog open in the pane owns its chords (see runKeyAction).
        if (ui.modalCount > 0 || ui.paletteOpen) return runKeyAction(action, index);
        // The side pane: pane verbs run here, window verbs in the host.
        const target = embeddedKeyTarget(action, paneKey(router.parts.join('/')));
        if (target === 'host') postToHost({ type: 'key', action, ...(index ? { index } : {}) });
        else if (target === 'close-pane') postToHost({ type: 'close' });
        else runKeyAction(action, index);
        return;
      }
      runKeyAction(action, index);
    });
  });

  // ---- update CLIs helper (shared by palette + any future callers) ----
  async function updateAllCLIs(): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId) { toasts.error(NO_WORKSPACE); return; }
    // One chord (⌘U) used to launch a global install with no way back — ask.
    const ok = await confirmer.ask(
      'Otto opens an "Update CLIs" session that installs the latest Claude Code, Codex and other agent CLIs on this Mac. Running sessions keep their current version until they restart.',
      { title: 'Update all agent CLIs?', confirmLabel: 'Update', danger: false },
    );
    if (!ok) return;
    try {
      const session = await api.post<Session>(`/workspaces/${wsId}/providers/update`, {});
      ws.addSession(session);
      toasts.info('Updating CLIs…', 'Watch the Update CLIs session for progress');
    } catch (e) {
      toastError('Couldn’t update the CLIs', e);
    }
  }

  // ---- palette sources + one-shot installers (installShell.svelte.ts) ----
  onMount(() =>
    installShell({
      moduleName: () => moduleName,
      openShortcuts: () => (shortcutsOpen = true),
      updateAllCLIs,
    }),
  );

  // ---- palette commands: focused session ----
  // Lifecycle verbs for the currently-active session (mirrors the per-pane ⋯
  // menu, which is otherwise undiscoverable). Registered only when a session is
  // focused; the closures act on ws.activeSession at run time. Hand over is
  // agent-only, matching the ⋯ menu.
  // Keyed on id + kind (all the commands use): a status flip replaces the
  // session object and would otherwise re-register them.
  const focusedKey = $derived(ws.activeSession ? `${ws.activeSession.id}\u0001${ws.activeSession.kind}` : '');
  $effect(() => {
    void focusedKey;
    const active = untrack(() => ws.activeSession);
    if (!active) return registry.register('focused-session', []);
    const isAgent = active.kind === 'agent';
    const cmds = [
      { id: 'focus.restart', title: 'Restart focused session', group: 'Session', keywords: 'reload reboot relaunch active current', run: () => void ws.requestRestart(active.id) },
      { id: 'focus.archive', title: 'Archive focused session', group: 'Session', keywords: 'close hide stash active current', run: () => void ws.requestArchive(active.id).catch((e: unknown) => toastError('Couldn’t archive the session', e)) },
      { id: 'focus.rename', title: 'Rename focused session…', group: 'Session', keywords: 'title name active current', run: () => void renameActiveSession() },
      ...(isAgent
        ? [{ id: 'focus.handover', title: 'Hand over focused session…', group: 'Session', keywords: 'handoff transfer pass context active current', run: () => openSessionAction('handover') }]
        : []),
      { id: 'focus.attach-issue', title: 'Attach issue to focused session…', group: 'Session', keywords: 'jira linear issue ticket link story active current', run: () => openSessionAction('attach-issue') },
      { id: 'focus.attach-product', title: 'Attach product story to focused session…', group: 'Session', keywords: 'product story link context active current', run: () => openSessionAction('attach-product') },
    ];
    return registry.register('focused-session', cmds);
  });

  // Apply the persisted zoom via native WKWebView page-zoom in Tauri (crisp;
  // CSS `zoom` would blur the terminal). Re-runs whenever the zoom changes.
  $effect(() => {
    void ui.zoom; // track
    void ui.applyNativeZoom();
  });

  // ---- side by side ----------------------------------------------------
  // The content column splits into the main pane and a side pane (another
  // document: lib/sidePane.ts, stores/sidePane.svelte.ts). Desktop main
  // window only; the entry points are the sidebar rows' context menu and
  // ⌥-click, ⌘\ and the ⌘K commands below.
  const splitModules = $derived.by(() => {
    const pluginEntries = plugins.list
      .filter((p) => auth.canPlugin(p.slug, 'view'))
      .map((p) => ({ id: `plugin/${p.slug}`, icon: p.icon, label: p.name }));
    return availableModules((f) => auth.can(f, 'view'), pluginEntries);
  });

  /** A pane's display name for a route: the sidebar label (a plugin's own
   *  name), or the route's name when it has no entry (Database, Settings…). */
  function paneMeta(route: string): { label: string } {
    const key = paneKey(route);
    const first = routeOf(route).split('/')[0] || 'agents';
    const m = splitModules.find((x) => x.id === key);
    return { label: m && (first === key || first === 'plugin') ? m.label : moduleLabel(first) };
  }
  const mainMeta = $derived(paneMeta(router.parts.join('/')));
  const sideMeta = $derived(sidePane.route === null ? null : paneMeta(sidePane.route));

  /** ⌘\ / "Open side pane…": pick a module for the side pane (the main pane's
   *  own module is left out; the pane's current one is checked). */
  function openSidePicker(): void {
    if (!sidePane.supported) return;
    const primary = sidePane.primaryKey;
    const current = sidePane.active ? sidePane.key : null;
    const items = splitModules
      .filter((m) => m.id !== primary)
      .map((m) => ({
        label: m.label,
        icon: m.icon,
        checked: m.id === current,
        action: () => sidePane.open(m.id, { label: m.label }),
      }));
    ctxMenu.showAt(null, items, { filter: true, filterPlaceholder: 'Open side by side…', maxVisible: 12 });
  }

  /** The pane handed back a route of the main pane's module. A session the
   *  pane just created may not be in this window's list yet — load it first
   *  so the route→store sync can open its tab. */
  async function openInMain(route: string): Promise<void> {
    const m = /^agents\/([^/?]+)/.exec(route);
    if (m && !ws.getSession(m[1])) await ws.ensureSession(m[1]);
    router.go(route);
  }

  // Host: the pane's messages, focus tracking and the router delegate.
  $effect(() =>
    sidePane.listen({
      runKey: (action, index) => {
        const run = () => runKeyAction(action as KeyAction | 'shortcuts', index);
        if (sidePane.nativeState) void nativePane.focus('primary').then(run).catch(() => {});
        else run();
      },
      openInMain: (route) => void openInMain(route),
      selectWorkspace: (id) => {
        if (ws.currentId !== id && ws.workspaces.some((w) => w.id === id)) void ws.select(id);
      },
    }),
  );

  // The host's workspace, applied once this pane's list knows it.
  let guestWorkspace = $state<string | null>(null);
  $effect(() => {
    const id = guestWorkspace;
    if (!id || !ws.listSettled) return;
    guestWorkspace = null;
    if (ws.currentId !== id && ws.workspaces.some((w) => w.id === id)) void ws.select(id);
  });

  // Host → pane: which module the main pane shows, and its workspace (the
  // pane follows a workspace switch).
  $effect(() => {
    void sidePane.primaryKey;
    void sidePane.placement;
    void sidePane.nativeState?.mode;
    void ui.railExpanded;
    const wsId = ws.currentId;
    if (!sidePane.active || sidePane.status !== 'ready') return;
    sidePane.postHost();
    if (wsId) sidePane.post({ type: 'workspace', id: wsId });
  });

  // The split container's width: the pane only shows while two panes fit.
  let splitWidth = $state(0);
  $effect(() => {
    sidePane.width = splitWidth;
  });
  const sideShare = $derived(clampShare(sidePane.share, splitWidth));

  // ⌘K: the side-by-side verbs.
  $effect(() => {
    if (!sidePane.supported) return registry.register('split', []);
    const cmds: Command[] = [
      { id: 'split.pick', title: 'Open side pane…', group: 'Pane', shortcut: '⌘\\', keywords: 'split side by side two panes pane module picker compare', run: () => openSidePicker() },
      ...splitModules
        .filter((m) => m.id !== sidePane.primaryKey && m.id !== (sidePane.active ? sidePane.key : null))
        .map((m) => ({
          id: `split.open-${m.id}`,
          title: `Open ${m.label} side by side`,
          group: 'Pane',
          detail: groupLabel(m.group),
          keywords: `split side by side pane ${m.id.replace(/[-/]/g, ' ')} ${m.keywords ?? ''}`,
          run: () => sidePane.open(m.id, { label: m.label }),
        })),
    ];
    if (sidePane.active && sideMeta) {
      const side = sideMeta.label;
      if (isTauri) {
        if (sidePane.detached) cmds.push(
          { id: 'split.return', title: 'Return to split', group: 'Pane', keywords: 'pane window attach restore', run: () => sidePane.returnToSplit() },
          { id: 'split.top', title: 'Toggle pane keep on top', group: 'Pane', keywords: 'pane window float', run: () => paneWindowAction('toggle-top') },
          { id: 'split.fill', title: 'Fill display with pane', group: 'Pane', keywords: 'pane window maximize', run: () => paneWindowAction('maximize') },
          { id: 'split.fullscreen', title: 'Toggle pane fullscreen', group: 'Pane', keywords: 'pane window fullscreen', run: () => paneWindowAction('toggle-fullscreen') },
          { id: 'split.display', title: 'Move pane to display…', group: 'Pane', keywords: 'pane window monitor', run: () => showPaneDisplays() },
        );
        else cmds.push(
          { id: 'split.detach-primary', title: 'Detach main pane', group: 'Pane', keywords: 'pane window float display', run: () => sidePane.detachPane('primary') },
          { id: 'split.detach-side', title: 'Detach side pane', group: 'Pane', keywords: 'pane window float display', run: () => sidePane.detachPane('side') },
        );
      }
      cmds.push(
        { id: 'split.close', title: 'Close side pane', group: 'Pane', shortcut: '⌘\\', keywords: `split side by side ${side}`, run: () => sidePane.close() },
        { id: 'split.swap', title: 'Swap panes', group: 'Pane', keywords: `split side by side flip ${side}`, run: () => sidePane.swap() },
        { id: 'split.promote', title: `Open ${side} in main pane`, group: 'Pane', keywords: 'split side by side promote maximize', run: () => sidePane.promote() },
        { id: 'split.reset', title: 'Reset split to 50/50', group: 'Pane', keywords: 'split side by side divider equal half', run: () => sidePane.resetSplit() },
        sidePane.focused
          ? { id: 'split.focus-main', title: `Focus ${mainMeta.label} (main pane)`, group: 'Pane', keywords: 'split pane switch', run: () => sidePane.focusMain() }
          : { id: 'split.focus-side', title: `Focus ${side} (side pane)`, group: 'Pane', keywords: 'split pane switch', run: () => sidePane.focusPane() },
      );
    }
    return registry.register('split', cmds);
  });

  // ⌘K: the side pane's own commands, mirrored (they run in the pane). Ids the
  // window already has — the shell registers the same set in both documents —
  // stay the window's.
  $effect(() => {
    const remote = sidePane.active ? sidePane.commands : [];
    const label = sideMeta?.label ?? 'Side pane';
    const own = new Set(untrack(() => registry.all).filter((c) => !c.id.startsWith('side:')).map((c) => c.id));
    return registry.register(
      'side-pane-commands',
      remote
        .filter((c) => !own.has(c.id))
        .map((c) => ({
          ...c,
          id: `side:${c.id}`,
          detail: c.detail ? `${c.detail} · ${label}` : label,
          run: () => sidePane.post({ type: 'run-command', id: c.id }),
        })),
    );
  });

  // Both documents: an appearance change made in one pane (a Settings pane)
  // reaches the other; the pane NOT showing Agents adopts the tabs the other
  // one changed (they share this window's keys).
  $effect(() => {
    if (!isEmbedded && sidePane.route === null) return;
    const onStorage = (e: StorageEvent): void => {
      if (e.storageArea !== localStorage) return;
      if (ui.reloadAppearance(e.key)) return;
      if (ws.ownsLayoutKey(e.key) && paneKey(router.parts.join('/')) !== 'agents') ws.adoptPersistedLayout();
    };
    window.addEventListener('storage', onStorage);
    return () => window.removeEventListener('storage', onStorage);
  });

  // ---- the side pane's own half (embedded document only) ----
  $effect(() => {
    if (!isEmbedded) return;
    // A fresh pane never opens on a leftover right panel; ⌘J still toggles it.
    ui.rightOpen = false;
    const stop = startGuest({
      runMenu: (id) => handleMenu(id),
      selectWorkspace: (id) => {
        // The host can name its workspace before this pane's own list landed:
        // hold it until the list settles instead of selecting an unknown id.
        guestWorkspace = id;
      },
      runCommand: (id) => void registry.all.find((c) => c.id === id)?.run(),
    });
    return stop;
  });
  $effect(() => {
    if (!isEmbedded) return;
    void router.parts.join('/');
    postToHost({ type: 'route', route: currentRoute() });
  });
  $effect(() => {
    const id = ws.currentId;
    if (isEmbedded && id) postToHost({ type: 'workspace', id });
  });
  $effect(() => {
    if (!isEmbedded) return;
    const list = registry.all.map(({ id, title, group, detail, keywords, shortcut }) => ({ id, title, group, detail, keywords, shortcut }));
    const t = setTimeout(() => postToHost({ type: 'commands', list }), 150);
    return () => clearTimeout(t);
  });
;
async () => {

if(router.module === 's'){
  
   { const $$_tnuoMyzaL0C = __sveltets_2_ensureComponent(LazyMount); new $$_tnuoMyzaL0C({ target: __sveltets_2_any(), props: {      "lazy":SharePageChunk,"what":`the shared session`,"props":{ sessionId: router.parts[1] ?? '' },}});}
} else if (router.module === 'snip'){
  
  if(Page){
    router.parts[1]; {
       { const $$_egaP0C = __sveltets_2_ensureComponent(Page); new $$_egaP0C({ target: __sveltets_2_any(), props: {}});}
    }
  }
  
   { const $$_golaiDmrifnoC0C = __sveltets_2_ensureComponent(ConfirmDialog); new $$_golaiDmrifnoC0C({ target: __sveltets_2_any(), props: {}});}
}else{
  const centerContent/*Ωignore_positionΩ*/ = ()/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
  
   { svelteHTML.createElement("div", {   "class":`sr-only`,"role":`status`,});impAnnounce; }
  if(auth.isImpersonating){
     { svelteHTML.createElement("div", {     "class":`provider-banner impersonation-banner`,"role":`region`,"aria-label":`Impersonation`,});
       { svelteHTML.createElement("span", {});
           { svelteHTML.createElement("strong", {});auth.me?.username ?? '…'; }
         { svelteHTML.createElement("span", { "class":`imp-real`,});    { svelteHTML.createElement("strong", {});auth.realUser?.username ?? '…'; }  }
       }
       { svelteHTML.createElement("span", { "class":`grow`,}); }
      if(impSecsLeft > 0){
         { svelteHTML.createElement("span", {  "class":`imp-countdown`,});impSecsLeft < 120;
          fmtImpCountdown(impSecsLeft); 
         }
      }else{
         { svelteHTML.createElement("span", { "class":`imp-countdown imp-urgent`,});    }
      }
       { svelteHTML.createElement("button", {     "class":`pb-dismiss`,"onclick":() => {
          void auth.stopImpersonating().catch((e: unknown) => {
            toastError('Couldn’t exit impersonation', e);
          });
        },});  }
     }
  }
  if(serviceHealth.visible){
     { svelteHTML.createElement("div", {   "class":`provider-banner`,"role":`alert`,});
       { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":14,}});}
       { svelteHTML.createElement("span", {});
                  { svelteHTML.createElement("strong", {});  }
           
       }
       { svelteHTML.createElement("span", { "class":`grow`,}); }
       { svelteHTML.createElement("button", {       "class":`pb-dismiss`,"onclick":() => serviceHealth.dismiss(),"aria-label":`Dismiss notice`,"title":`Dismiss notice`,});
         { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}
       }
     }
  }
  if(moduleName === 'agents'){
     { const $$_raBbaT0C = __sveltets_2_ensureComponent(TabBar); new $$_raBbaT0C({ target: __sveltets_2_any(), props: {}});}
  }
  if(workspaceLoadError){
     { svelteHTML.createElement("div", { ...__sveltets_2_empty({"data-testid":`workspace-load-error`}),});
       { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {          "what":`workspaces`,"variant":`compact`,"error":workspaceLoadError,"loading":workspaceLoading,"empty":true,"onretry":() => void loadWorkspaces(),}});}
     }
  }
  if(!uiControl.barOwner){
    
     { const $$_raBgnivirDtnegA0C = __sveltets_2_ensureComponent(AgentDrivingBar); new $$_raBgnivirDtnegA0C({ target: __sveltets_2_any(), props: {}});}
  }
  if(navPending.slow){
    
     { svelteHTML.createElement("div", { "class":`nav-pending-slot`,});
       { svelteHTML.createElement("div", {       "class":`nav-pending-bar`,"role":`progressbar`,"aria-label":`Loading page`,...__sveltets_2_empty({"data-testid":`nav-pending-bar`}),}); }
     }
  }
   { svelteHTML.createElement("div", {     "class":`content`,"id":`app-content`,"tabindex":-1,});
    
    if(Page){
       { const $$_egaP1C = __sveltets_2_ensureComponent(Page); new $$_egaP1C({ target: __sveltets_2_any(), props: {}});}
    } else if (pageFailed){
       { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL1 = new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {          "variant":`page`,"what":`this page`,"error":`Otto may have been updated in the background. Retry, or reload Otto to pick up the new version.`,"empty":true,"onretry":() => void loadPage(pageKey).catch(() => {}),errorActions:() => { async ()/*Ωignore_positionΩ*/ => { { svelteHTML.createElement("button", {   "class":`btn`,"onclick":() => location.reload(),});  }};return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {errorActions} = $$_etatSdaoL1.$$prop_def;/*Ωignore_endΩ*/
        
       LoadState}
    }
   }
};return __sveltets_2_any(0)};




 { svelteHTML.createElement("div", {      "class":`shell`,});vibrant;isEmbedded;compactShell;compactShell && viewport.isTablet;pageTitled;
  if(compactShell){
  
   { svelteHTML.createElement("header", {       "class":`mtopbar`,...__sveltets_2_empty({"data-tauri-drag-region":true}),"onmousedown":isTauri ? startWindowDrag : undefined,});isTauri;
    if(viewport.isPhone){
       { svelteHTML.createElement("button", {             "class":`mtop-btn`,"onclick":() => (ui.navDrawerOpen = !ui.navDrawerOpen),"title":`Open navigator`,"aria-label":`Open navigator`,"aria-haspopup":`dialog`,"aria-expanded":ui.navDrawerOpen,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`sidebar`,"size":20,}});}
       }
    }
    
     { const $$_snottuBvaN2C = __sveltets_2_ensureComponent(NavButtons); new $$_snottuBvaN2C({ target: __sveltets_2_any(), props: {}});}
     { svelteHTML.createElement("span", { "class":`mtop-title`,});moduleName === 'agents' ? (ws.activeSession?.title ?? 'Agents') : moduleTitle; }
     { svelteHTML.createElement("span", { "class":`grow`,}); }
    
    if(viewport.isPhone){
       { const $$_lleBnoitacifitoN2C = __sveltets_2_ensureComponent(NotificationBell); new $$_lleBnoitacifitoN2C({ target: __sveltets_2_any(), props: {  "placement":`below`,}});}
    }
    if(showRightPanel){
       { svelteHTML.createElement("button", {            "class":`mtop-btn`,"onclick":() => ui.toggleRight(),"title":SIDE_PANEL,"aria-label":SIDE_PANEL,"aria-expanded":ui.rightOpen,});ui.rightOpen;
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`panel`,"size":20,}});}
       }
    }
   }

  
  if(viewport.isPhone && moduleName === 'agents'){
     { const $$_raBnoitcAeliboM1C = __sveltets_2_ensureComponent(MobileActionBar); new $$_raBnoitcAeliboM1C({ target: __sveltets_2_any(), props: {             "onpalette":mobileOpenPalette,"onnewSession":mobileNewSession,"oncloseTab":mobileCloseTab,"onfind":mobileFind,"showBroadcast":ws.sessions.length > 0,"onbroadcast":mobileBroadcast,}});}
  }

  }
  if(isPopout && isTauri && !isEmbedded){
    
    
     { svelteHTML.createElement("div", {     "class":`popout-titlebar sidebar-material`,"role":`presentation`,...__sveltets_2_empty({"data-tauri-drag-region":true}),"onmousedown":startWindowDrag,});
       { svelteHTML.createElement("span", { "class":`popout-title`,});popoutTitle(); }
     }
  }
   { svelteHTML.createElement("div", {  "class":`shell-main`,});compactShell;
    if(!isPopout && !isEmbedded && !viewport.isPhone){
     { svelteHTML.createElement("div", {   "class":`sidebar`,});compactShell;isTauri && !compactShell;
      
      if(isTauri && !compactShell){
         { svelteHTML.createElement("div", {     "class":`titlebar-drag`,"role":`presentation`,...__sveltets_2_empty({"data-tauri-drag-region":true}),"onmousedown":startWindowDrag,}); }
      }
      if(compactShell || ui.railExpanded){
         { const $$_rotagivaN3C = __sveltets_2_ensureComponent(Navigator); new $$_rotagivaN3C({ target: __sveltets_2_any(), props: {}});}
      }else{
         { const $$_liaR3C = __sveltets_2_ensureComponent(Rail); new $$_liaR3C({ target: __sveltets_2_any(), props: {}});}
      }
     }
    }

    
     { const $$_div2 = svelteHTML.createElement("div", {       "class":`split`,});!!sideMeta && (sidePane.showing || sidePane.retained);sidePane.showing && sidePane.placement === 'leading';splitWidth= $$_div2.clientWidth;__sveltets_2_ensureType(String, Number, sideShare);
       { svelteHTML.createElement("div", { "class":`primary-pane`,});
         { svelteHTML.createElement("div", { "class":`primary-row`,});
           { svelteHTML.createElement("div", {  "class":`center`,});compactShell;
            ;__sveltets_2_ensureSnippet(centerContent());
            if(!isPopout && !isEmbedded && viewport.isDesktop){
              
               { const $$_raBgnitaolF6C = __sveltets_2_ensureComponent(FloatingBar); new $$_raBgnitaolF6C({ target: __sveltets_2_any(), props: {  "host":`app`,}});}
            }
           }

          
          if(showRightPanel){
             { const $$_rewarD5C = __sveltets_2_ensureComponent(Drawer); const $$_rewarD5 = new $$_rewarD5C({ target: __sveltets_2_any(), props: {          children:() => { return __sveltets_2_any(0); },open:ui.rightOpen,"inline":!compactShell,"side":`right`,"label":SIDE_PANEL,"width":`min(92vw, 360px)`,}});/*Ωignore_startΩ*/() => ui.rightOpen = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rewarD5.$$bindings = 'open';
              
              const RightPanel = loadedRightPanel();
              if(RightPanel){ { const $$_lenaPthgiR6C = __sveltets_2_ensureComponent(RightPanel); new $$_lenaPthgiR6C({ target: __sveltets_2_any(), props: {  "forceOpen":compactShell,}});}}else{ { svelteHTML.createElement("div", {  "class":`rp-loading`,});!compactShell;if(compactShell){ { const $$_notelekS7C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS7C({ target: __sveltets_2_any(), props: {      "rows":4,"height":28,"label":`the ${SIDE_PANEL}`,}});}} }}
             Drawer}
          }
         }
       }
      if(sidePane.showing && sideMeta){
         { const $$_rediviDtilpS3C = __sveltets_2_ensureComponent(SplitDivider); new $$_rediviDtilpS3C({ target: __sveltets_2_any(), props: {  "label":`${mainMeta.label} and ${sideMeta.label}`,}});}
      }
      if(sideMeta && (sidePane.showing || sidePane.retained)){
         { const $$_enaPediS3C = __sveltets_2_ensureComponent(SidePane); new $$_enaPediS3C({ target: __sveltets_2_any(), props: {  "label":sideMeta.label,}});}
      }
     }
   }

   { const $$_rennaBspaC1C = __sveltets_2_ensureComponent(CapsBanner); new $$_rennaBspaC1C({ target: __sveltets_2_any(), props: {}});}
  if(compactShell && viewport.isPhone){
     { const $$_vaNmottoB1C = __sveltets_2_ensureComponent(BottomNav); new $$_vaNmottoB1C({ target: __sveltets_2_any(), props: {}});}
  }
  if(!isPopout && !isEmbedded && (!viewport.isPhone || events.state !== 'connected')){
     { const $$_raBsutatS1C = __sveltets_2_ensureComponent(StatusBar); new $$_raBsutatS1C({ target: __sveltets_2_any(), props: {}});}
  }
 }

if(compactShell && viewport.isPhone){
   { const $$_rewarD0C = __sveltets_2_ensureComponent(Drawer); const $$_rewarD0 = new $$_rewarD0C({ target: __sveltets_2_any(), props: {        children:() => { return __sveltets_2_any(0); },open:ui.navDrawerOpen,"side":`left`,"label":`Navigator`,"width":`min(86vw, 280px)`,}});/*Ωignore_startΩ*/() => ui.navDrawerOpen = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rewarD0.$$bindings = 'open';
     { const $$_rotagivaN1C = __sveltets_2_ensureComponent(Navigator); new $$_rotagivaN1C({ target: __sveltets_2_any(), props: {}});}
   Drawer}
}


 { const $$_ettelaP0C = __sveltets_2_ensureComponent(Palette); new $$_ettelaP0C({ target: __sveltets_2_any(), props: {}});}

 { const $$_yalrevOstuctrohS0C = __sveltets_2_ensureComponent(ShortcutsOverlay); new $$_yalrevOstuctrohS0C({ target: __sveltets_2_any(), props: {    "open":shortcutsOpen,"onclose":() => (shortcutsOpen = false),}});}



if(sessionAction?.kind === 'handover'){
  const id = sessionAction.sessionId;
   { const $$_tnuoMyzaL0C = __sveltets_2_ensureComponent(LazyMount); new $$_tnuoMyzaL0C({ target: __sveltets_2_any(), props: {        "lazy":HandoverChunk,"what":`the handover dialog`,"quiet":true,"onerror":closeSessionAction,"props":{ sessionId: id, onclose: closeSessionAction },}});}
} else if (sessionAction?.kind === 'attach-issue'){
  const id = sessionAction.sessionId;
   { const $$_tnuoMyzaL0C = __sveltets_2_ensureComponent(LazyMount); new $$_tnuoMyzaL0C({ target: __sveltets_2_any(), props: {        "lazy":AttachIssueChunk,"what":`the attach-issue dialog`,"quiet":true,"onerror":closeSessionAction,"props":{ sessionId: id, onclose: closeSessionAction },}});}
} else if (sessionAction?.kind === 'attach-product'){
  const id = sessionAction.sessionId;
   { const $$_tnuoMyzaL0C = __sveltets_2_ensureComponent(LazyMount); new $$_tnuoMyzaL0C({ target: __sveltets_2_any(), props: {        "lazy":AttachProductStoryChunk,"what":`the attach-story dialog`,"quiet":true,"onerror":closeSessionAction,"props":{ sessionId: id, onclose: closeSessionAction },}});}
}

if(ui.broadcastOpen){
   { const $$_tnuoMyzaL0C = __sveltets_2_ensureComponent(LazyMount); new $$_tnuoMyzaL0C({ target: __sveltets_2_any(), props: {      "lazy":BroadcastModalChunk,"what":`the broadcast dialog`,"quiet":true,"onerror":() => (ui.broadcastOpen = false),}});}
}

if(ui.newSessionOpen){
   { const $$_tnuoMyzaL0C = __sveltets_2_ensureComponent(LazyMount); new $$_tnuoMyzaL0C({ target: __sveltets_2_any(), props: {        "lazy":NewSessionChunk,"what":`the new-session sheet`,"quiet":true,"onerror":closeNewSession,"props":{ initialScratch: ui.newSessionScratch, onclose: closeNewSession },}});}
}

if(ui.newWorkspaceOpen){
   { const $$_tnuoMyzaL0C = __sveltets_2_ensureComponent(LazyMount); new $$_tnuoMyzaL0C({ target: __sveltets_2_any(), props: {        "lazy":NewWorkspaceChunk,"what":`the new-workspace dialog`,"quiet":true,"onerror":closeNewWorkspace,"props":{ onclose: closeNewWorkspace },}});}
}

 { const $$_golaiDmrifnoC0C = __sveltets_2_ensureComponent(ConfirmDialog); new $$_golaiDmrifnoC0C({ target: __sveltets_2_any(), props: {}});}
 { const $$_uneMtxetnoC0C = __sveltets_2_ensureComponent(ContextMenu); new $$_uneMtxetnoC0C({ target: __sveltets_2_any(), props: {}});}
 { const $$_egaPnIdniF0C = __sveltets_2_ensureComponent(FindInPage); new $$_egaPnIdniF0C({ target: __sveltets_2_any(), props: {}});}
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const App__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type App__SvelteComponent_ = ReturnType<typeof App__SvelteComponent_>;
/*Ωignore_endΩ*/export default App__SvelteComponent_;