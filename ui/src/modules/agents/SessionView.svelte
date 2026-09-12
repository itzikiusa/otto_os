<script lang="ts">
  // One pane: session header (status, provider, restart/kill) + terminal.
  import Terminal from '../../lib/components/Terminal.svelte';
  import StatusDot from '../../lib/components/StatusDot.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import ProviderIcon, { hasProviderIcon } from '../../lib/components/ProviderIcon.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import AttachIssue from './AttachIssue.svelte';
  import AttachProductStory from './AttachProductStory.svelte';
  import Handover from './Handover.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { activity } from '../../lib/stores/activity.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { now } from '../../lib/stores/now.svelte';
  import { ui } from '../../lib/stores/ui.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { router } from '../../lib/router.svelte';
  import { untrack } from 'svelte';
  import { transcript, type SessionViewMode } from '../../lib/stores/transcript.svelte';
  import { presetItems } from './SplitNode.svelte';
  import ConversationView from './conversation/ConversationView.svelte';
  import type { AttachedIssue, SessionStatus } from '../../lib/api/types';

  // Default idle-suspend grace period (5 minutes) used for the "suspends in N"
  // countdown hint. Reflects the backend's SUSPEND_GRACE constant; the backend
  // may override it via the `idle_suspend_grace_secs` setting, but a frontend
  // approximation is fine here — the hint is informational, not authoritative.
  const SUSPEND_GRACE_MS = 5 * 60 * 1000;

  interface Props {
    sessionId: string;
    focused: boolean;
    showClose: boolean;
    onfocus: () => void;
    onclosepane: () => void;
    /** Tooltip for the header ×; the split view ends the session, embedded viewers only hide it. */
    closeTitle?: string;
    /** Show a maximize/restore (zoom) control (tiled view). */
    showZoom?: boolean;
    /** Show the header drag grip (C3a). Splits passes `panes > 1`, TiledView
     *  `tiles > 1`; forced off on phones — HTML5 DnD never fires on iOS Safari. */
    showGrip?: boolean;
    /** What the grip drags: the LEAF KEY in the split tree, the session id in
     *  the tiled grid — the host decides what a drop means. */
    dragKey?: string;
    /** Drag lifecycle, so the host can arm its drop targets while one is in flight. */
    ondragpane?: (phase: 'start' | 'end') => void;
  }
  let { sessionId, focused, showClose, onfocus, onclosepane, showZoom = false, showGrip = false, dragKey, ondragpane, closeTitle = 'Close pane (keeps running)' }: Props = $props();

  const maximized = $derived(ws.maximizedId === sessionId);

  const session = $derived(ws.sessions.find((s) => s.id === sessionId) ?? null);
  const status = $derived(ws.statusMap[sessionId] ?? session?.status ?? 'idle');

  // "X min idle / suspends in N" countdown hint for idle agent sessions.
  // `session.last_active_at` is updated whenever the status changes — so when
  // the session transitions to `idle` it stamps the moment. We use this as a
  // proxy for last-output time and count down to the backend's suspend window.
  const idleHint = $derived.by(() => {
    if (status !== 'idle') return null;
    if (!session?.kind || session.kind !== 'agent') return null;
    if (session.meta?.keep_alive === true) return null; // pinned — won't be suspended
    // Sessions the user started from the Agents page (manual origin, no engine
    // `source`) are exempt from the daemon's idle sweep — no countdown to show.
    const origin = (session.meta?.work as { origin?: string } | undefined)?.origin;
    if ((origin === undefined || origin === 'manual') && !session.meta?.source) return null;
    const _tick = now(); // reactive dependency: re-computes every second
    const idleMs = Date.now() - Date.parse(session.last_active_at);
    if (idleMs < 0) return null;
    const resumesInMs = SUSPEND_GRACE_MS - idleMs;
    const idleMin = Math.floor(idleMs / 60_000);
    const idleSec = Math.floor((idleMs % 60_000) / 1000);
    const idleLabel = idleMin > 0 ? `${idleMin}m idle` : `${idleSec}s idle`;
    if (resumesInMs <= 0) return `${idleLabel} · suspending…`;
    const inMin = Math.ceil(resumesInMs / 60_000);
    return `${idleLabel} · suspends in ${inMin}m`;
  });
  const readOnly = $derived(ws.myRole === 'viewer');

  // Live per-session activity roll-up (current in-progress task + done/total),
  // surfaced in the pane header so tiled/split panes show what each agent is on.
  const summary = $derived(activity.summary(sessionId));
  // Sticky "needs you" flag — the session is blocked on operator input. Distinct
  // from plain idle; cleared by the store when the user opens/inputs.
  const needsYou = $derived(ws.needsYou[sessionId] === true);
  /** True when this agent session can be resumed after exiting. */
  const resumable = $derived(
    session?.kind === 'agent' && session?.provider_session_id != null,
  );
  /** Full display name for a themed session (e.g. "Cristiano Ronaldo"), shown
   *  beside the short handle ("Ronaldo") when the two differ. */
  const nameFull = $derived(
    ((session?.meta?.name_full as string | undefined) ?? '').trim(),
  );

  // Live pane-header width + element, so the header can shed controls as the
  // PANE shrinks (a tiled grid packs 15 of these side by side). ONE source of
  // truth: `tier` drives both the markup below and the `.t1`–`.t7` style rules
  // at the bottom of this stylesheet — the CSS used to key off `@container`,
  // which silently never matched the rules targeting `.pane-head` itself (a
  // container cannot query itself) and could not see the measured fold below.
  let headW = $state(0);
  let headEl: HTMLElement | null = $state(null);
  /** Fold tier by WIDTH, 0 (roomy) → 7 (status dot + ⋯ only). A starting point
   *  only: the inline set that fits depends on the session (task chip, handover
   *  crumb, themed name…), so `tier` below adds whatever the MEASURED header
   *  still needs. Everything a tier drops is reachable as a MenuItem, never
   *  clipped (C1). */
  const widthTier = $derived(
    headW <= 0
      ? 0
      : headW <= 140
        ? 7
        : headW <= 200
          ? 6
          : headW <= 270
            ? 5
            : headW <= 400
              ? 4
              : headW <= 520
                ? 3
                : headW <= 560
                  ? 2
                  : headW <= 620
                    ? 1
                    : 0,
  );
  /** Extra tiers the measured header asked for — a RATCHET: it only ever grows
   *  for a given width+content, so the fold converges in at most 7 passes and
   *  can never oscillate. Reset whenever either changes (see the $effect). */
  let foldBump = $state(0);
  /** Effective tier: what the markup and the `.t*` classes below both use. */
  const tier = $derived(Math.min(7, widthTier + foldBump));
  // Tier 4 is where `.term-ctl` goes; its actions move into the ⋯ menu.
  const termCtlFolded = $derived(tier >= 4 && !viewport.isPhone && ui.termToolbar);
  /** The drag grip is a mouse affordance — phones keep keyboard/palette moves. */
  const gripOn = $derived(showGrip && dragKey != null && !viewport.isPhone);

  function onGripDragStart(e: DragEvent): void {
    if (!dragKey) return;
    e.dataTransfer?.setData('text/plain', dragKey);
    // Own MIME so a foreign drag (a file, a tab) never lands on a pane veil.
    e.dataTransfer?.setData('application/x-otto-pane', dragKey);
    if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
    ondragpane?.('start');
  }
  function onGripDragEnd(): void {
    ondragpane?.('end');
  }

  let renaming = $state(false);
  // Bumped after a successful restart so the embedded <Terminal> drops its
  // exited overlay and reconnects to the freshly respawned/resumed PTY.
  let restartNonce = $state(0);

  // Keyboard follows the active pane: when this pane becomes the target one
  // (new session, tab/tile switch, sidebar navigation) move focus into its
  // terminal so typing works without a click. `focused` alone won't do — it is
  // the visual ring, and Splits suppresses it for a lone pane — so also key off
  // being the active session. Mount-time focus is covered by <Terminal
  // autoFocus> (the xterm textarea doesn't exist yet on the first run of this
  // effect). Phones keep tap-to-focus (soft-keyboard gesture rule).
  const kbFocused = $derived(focused || ws.activeSessionId === sessionId);
  let termRef = $state<Terminal | null>(null);
  $effect(() => {
    if (kbFocused && !readOnly && !viewport.isPhone) termRef?.focus();
  });
  let draftTitle = $state('');
  let attachIssueOpen = $state(false);
  let attachProductOpen = $state(false);
  let handoverOpen = $state(false);

  const attachedIssue = $derived(
    (session?.meta?.issue as AttachedIssue | undefined) ?? null,
  );

  // Handover breadcrumb (source session) + live "preparing brief" badge.
  const handoverFromId = $derived(
    typeof session?.meta?.handover_from === 'string'
      ? (session.meta.handover_from as string)
      : null,
  );
  const handoverFrom = $derived(
    handoverFromId ? (ws.sessions.find((s) => s.id === handoverFromId) ?? null) : null,
  );
  const handoverPending = $derived(session?.meta?.handover_pending === true);

  // --- Additional directories editor (meta.extra_dirs → `--add-dir` args) -----
  // Only agent sessions launch a CLI that honors `--add-dir`.
  const isAgent = $derived(session?.kind === 'agent');

  // --- Terminal · Chat · Split (docs/design/conversation-view.md §5.1) --------
  // The chat is rebuilt from the provider transcript; probing it once per agent
  // session (cheap 200, `unavailable_reason` when nothing resolves) makes the
  // Chat tab instant when picked. The default view is Terminal for every
  // session — the chat is opt-in per session, and the user's choice is
  // persisted (`otto_session_view:<id>`, winKey).
  const conv = $derived(isAgent ? transcript.conversation({ sessionId }) : null);
  $effect(() => {
    if (conv) transcript.ensure(conv.src);
  });
  const defaultView: SessionViewMode = 'terminal';
  const savedView = $derived(isAgent ? transcript.view(sessionId) : null);
  const view = $derived<SessionViewMode>(isAgent ? (savedView ?? defaultView) : 'terminal');
  // Below 1200px the window can't hold chat + terminal (+ the right panel):
  // Split degrades to Chat, with Terminal one tab away in the segmented control.
  let wide = $state(typeof window === 'undefined' ? true : window.matchMedia('(min-width: 1200px)').matches);
  $effect(() => {
    const mq = window.matchMedia('(min-width: 1200px)');
    const sync = () => (wide = mq.matches);
    sync();
    mq.addEventListener('change', sync);
    return () => mq.removeEventListener('change', sync);
  });
  const effView = $derived<SessionViewMode>(view === 'split' && !wide ? 'chat' : view);

  /** Everything that changes the header's intrinsic width. A change resets the
   *  ratchet so a pane that got roomier (or a chip that went away) folds back. */
  const fitSig = $derived(
    [
      headW,
      session?.title ?? '',
      nameFull,
      session?.cwd ?? '',
      session?.provider ?? '',
      effView,
      wide,
      ui.termToolbar,
      viewport.isPhone,
      summary?.total ?? 0,
      summary?.in_progress ?? '',
      needsYou,
      idleHint ?? '',
      handoverFromId ?? '',
      handoverPending,
      showZoom,
      showClose,
      gripOn,
      readOnly,
      renaming,
    ].join('|'),
  );
  let lastFitSig = '';
  // C1: no control may EVER be clipped. `.pane-head` is `overflow: clip`, so an
  // overflowing header silently pushes ✕/⋯ past its edge (they stay clickable
  // in the pane NEXT to it — the bug this guard exists for). Measure after every
  // render and fold one more tier until the inline set genuinely fits.
  $effect(() => {
    const el = headEl;
    const sig = fitSig;
    const t = tier; // track: re-measure once the fold we just asked for is applied
    if (!el || headW <= 0) return;
    if (sig !== lastFitSig) {
      lastFitSig = sig;
      // Re-runs with the reset tier; the measure below happens on that pass.
      if (untrack(() => foldBump) !== 0) {
        foldBump = 0;
        return;
      }
    }
    if (t < 7 && el.scrollWidth - el.clientWidth > 1) foldBump += 1;
  });
  function setView(mode: SessionViewMode): void {
    transcript.setView(sessionId, mode);
  }
  /** ⌘⇧C cycles Terminal → Chat → Split (Split skipped when the window is narrow). */
  function cycleView(): void {
    const order: SessionViewMode[] = wide ? ['terminal', 'chat', 'split'] : ['terminal', 'chat'];
    const i = order.indexOf(effView);
    setView(order[(i + 1) % order.length]);
  }
  const VIEW_META: [SessionViewMode, string, string][] = [
    ['terminal', 'Terminal', 'terminal'],
    ['chat', 'Chat', 'comment'],
    ['split', 'Split', 'split'],
  ];
  /** The three view choices as menu rows. `prefixed` is the tier-6 form that
   *  lives INSIDE the ⋯ menu ("View: Chat"); the bare form is the tier-5 icon
   *  button's own menu. Split is offered on the same rule as the inline tab. */
  function viewRows(prefixed: boolean): MenuItem[] {
    return VIEW_META.filter(([m]) => m !== 'split' || wide).map(([m, label, icon]) => ({
      label: prefixed ? `View: ${label}${effView === m ? ' ✓' : ''}` : `${effView === m ? '✓ ' : ''}${label}`,
      icon,
      action: () => setView(m),
    }));
  }
  function openViewMenu(e: MouseEvent | KeyboardEvent): void {
    ctxMenu.show(e, viewRows(false));
  }
  $effect(() => {
    if (!isAgent) return;
    const onKey = (e: KeyboardEvent): void => {
      if (!(e.metaKey || e.ctrlKey) || !e.shiftKey || e.altKey) return;
      if (e.key !== 'c' && e.key !== 'C') return;
      // Only the active pane reacts (tiled/split views mount several).
      if (ws.activeSessionId !== sessionId) return;
      e.preventDefault();
      cycleView();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });
  // Split: chat left / terminal right, the divider drag copied from the right
  // panel's resizer (shell/RightPanel.svelte) but as a fraction of this pane.
  let bodyEl = $state<HTMLDivElement | null>(null);
  let chatFrac = $state(untrack(() => transcript.splitFrac(sessionId)));
  let splitResizing = $state(false);
  function startSplit(e: MouseEvent): void {
    e.preventDefault();
    const el = bodyEl;
    if (!el) return;
    splitResizing = true;
    const rect = el.getBoundingClientRect();
    const rtl = getComputedStyle(el).direction === 'rtl';
    const onMove = (ev: MouseEvent) => {
      const x = rtl ? rect.right - ev.clientX : ev.clientX - rect.left;
      chatFrac = Math.min(0.8, Math.max(0.3, x / rect.width));
    };
    const onUp = () => {
      splitResizing = false;
      transcript.setSplitFrac(sessionId, chatFrac);
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', onUp);
      document.body.style.cursor = '';
      document.body.style.userSelect = '';
    };
    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
    document.body.style.cursor = 'col-resize';
    document.body.style.userSelect = 'none';
  }
  let dirsOpen = $state(false);
  let dirsBusy = $state(false);
  let extraDirs = $state<string[]>([]);
  let dirDraft = $state('');

  function openDirs(): void {
    const seed = session?.meta?.extra_dirs;
    extraDirs = Array.isArray(seed) ? seed.filter((d): d is string => typeof d === 'string') : [];
    dirDraft = '';
    dirsOpen = true;
  }

  function addDir(): void {
    const path = dirDraft.trim();
    if (path === '' || extraDirs.includes(path)) {
      dirDraft = '';
      return;
    }
    extraDirs = [...extraDirs, path];
    dirDraft = '';
  }

  function removeDir(dir: string): void {
    extraDirs = extraDirs.filter((d) => d !== dir);
  }

  function onDirKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter') {
      e.preventDefault();
      addDir();
    }
  }

  /** Collect the list, folding any typed-but-unadded draft, trimming + de-duping. */
  function collectDirs(): string[] {
    const dirs = [...extraDirs];
    const pending = dirDraft.trim();
    if (pending !== '' && !dirs.includes(pending)) dirs.push(pending);
    return dirs;
  }

  async function saveDirs(alsoRestart: boolean): Promise<void> {
    if (dirsBusy) return;
    dirsBusy = true;
    try {
      const dirs = collectDirs();
      await ws.updateSessionMeta(sessionId, { extra_dirs: dirs });
      if (alsoRestart) {
        await ws.restartSession(sessionId);
        toasts.success('Directories saved', 'Session restarted with the new directories.');
      } else {
        toasts.success('Directories saved', 'Applies the next time this session restarts.');
      }
      dirsOpen = false;
    } catch (e) {
      toasts.error('Save failed', e instanceof Error ? e.message : String(e));
    } finally {
      dirsBusy = false;
    }
  }

  function onTermStatus(s: SessionStatus): void {
    ws.statusMap[sessionId] = s;
  }

  function startRename(): void {
    if (readOnly) return;
    draftTitle = session?.title ?? '';
    renaming = true;
  }

  async function commitRename(): Promise<void> {
    renaming = false;
    const next = draftTitle.trim();
    if (!next || next === session?.title) return;
    try {
      await ws.renameSession(sessionId, next);
    } catch (e) {
      toasts.error('Rename failed', e instanceof Error ? e.message : String(e));
    }
  }

  async function restart(): Promise<void> {
    try {
      await ws.restartSession(sessionId);
      // Nudge the embedded Terminal to drop its exited overlay and reconnect to
      // the now-live PTY (the session id is unchanged, so this is the only signal).
      restartNonce++;
    } catch (e) {
      toasts.error('Restart failed', e instanceof Error ? e.message : String(e));
    }
  }

  const keepAlive = $derived(session?.meta?.keep_alive === true);

  async function toggleKeepAlive(): Promise<void> {
    try {
      await ws.updateSessionMeta(sessionId, { keep_alive: !keepAlive });
      toasts.info(keepAlive ? 'Keep-alive disabled' : 'Keep-alive enabled', keepAlive ? 'Session may auto-suspend.' : 'Session will not be auto-suspended.');
    } catch (e) {
      toasts.error('Keep-alive failed', e instanceof Error ? e.message : String(e));
    }
  }

  async function archive(): Promise<void> {
    try {
      await ws.archiveSession(sessionId);
    } catch (e) {
      toasts.error('Archive failed', e instanceof Error ? e.message : String(e));
    }
  }

  // "Always delete" (Settings → Appearance) is the answer to this confirm
  // already, so it skips the dialog — same as the tab ×.
  async function del(): Promise<void> {
    const ok =
      ui.closeTabPref === 'delete' ||
      (await confirmer.ask(
        'Delete this session and its entire history? This cannot be undone.',
        { title: 'Delete session', confirmLabel: 'Delete' },
      ));
    if (!ok) return;
    try {
      await ws.killSession(sessionId);
    } catch (e) {
      toasts.error('Delete failed', e instanceof Error ? e.message : String(e));
    }
  }

  async function detachIssue(): Promise<void> {
    try {
      await ws.detachIssue(sessionId);
      toasts.info('Issue detached');
    } catch (e) {
      toasts.error('Detach failed', e instanceof Error ? e.message : String(e));
    }
  }

  function openAttachIssue(): void {
    attachIssueOpen = true;
  }

  function openAttachProductStory(): void {
    attachProductOpen = true;
  }

  function openHandover(): void {
    handoverOpen = true;
  }

  /** Agent sessions open the Canvas panel beside the terminal; other session
   *  kinds (connections) have no right panel, so navigate to the Canvas
   *  module directly instead. */
  function openCanvas(): void {
    if (isAgent) {
      ui.openRight('canvas');
    } else {
      router.go('canvas');
    }
  }

  /** Single source of truth for the session actions menu — served both by the
   *  header ⋯ button and the title's right-click, through the global clamped
   *  ctxMenu (viewport clamp + max-height come for free).
   *
   *  It is also the C1 overflow menu: every control a fold TIER removes from the
   *  header comes back here as a row, so nothing is ever unreachable — a pane
   *  140 px wide is a status dot and this menu. */
  function sessionMenuItems(): MenuItem[] {
    // Rows the header shed at the current tier (info rows are disabled labels).
    const folded: MenuItem[] = [
      ...(tier >= 6 && isAgent ? viewRows(true) : []),
      ...(tier >= 5 && !readOnly && isAgent
        ? [{ label: 'Restart session', icon: 'refresh', action: () => void restart() } as MenuItem]
        : []),
      ...(tier >= 5 && showZoom
        ? [
            {
              label: maximized ? 'Restore tiled view' : 'Zoom in on this session',
              icon: maximized ? 'minimize' : 'maximize',
              action: () => ws.toggleMaximize(sessionId),
            } as MenuItem,
          ]
        : []),
      ...(tier >= 5 && summary && summary.total > 0
        ? [{ label: `Tasks ${summary.done}/${summary.total}`, disabled: true } as MenuItem]
        : []),
      ...(tier >= 4 && summary?.in_progress
        ? [{ label: `Now: ${summary.in_progress}`, disabled: true } as MenuItem]
        : []),
      ...(tier >= 4 && idleHint ? [{ label: idleHint, disabled: true } as MenuItem] : []),
      ...(tier >= 4 && handoverFromId
        ? [
            {
              label: `Open handover source ↰ ${handoverFrom?.title ?? 'source'}`,
              icon: 'link',
              action: () => ws.navigateToSession(handoverFromId),
            } as MenuItem,
          ]
        : []),
      ...(tier >= 4 && handoverPending
        ? [{ label: 'Preparing handover…', disabled: true } as MenuItem]
        : []),
      ...(tier >= 4 && session?.cwd ? [{ label: `cwd: ${session.cwd}`, disabled: true } as MenuItem] : []),
      // In a narrow pane the inline terminal font/copy toolbar is hidden (the
      // `.t4 .term-ctl` rule); surface its actions here so nothing is lost when
      // tiling many sessions.
      ...(termCtlFolded
        ? [
            { label: `Terminal font smaller (${ui.termFontSize}px)`, action: () => ui.termZoomOut() } as MenuItem,
            { label: 'Terminal font larger', icon: 'plus', action: () => ui.termZoomIn() } as MenuItem,
            {
              label: ui.termCopyOnSelect ? 'Copy-on-select: on' : 'Copy-on-select: off',
              icon: 'copy',
              action: () => ui.setTermCopyOnSelect(!ui.termCopyOnSelect),
            } as MenuItem,
          ]
        : []),
    ];
    // Layout presets — flat rows (the ctxMenu has no submenus), only with a
    // split to re-arrange. The same list the DB pane's ✕ shows, shared so the
    // two can't drift apart.
    const presets: MenuItem[] = showClose ? presetItems() : [];
    return [
      // Tier 7 dropped the title from the header — the menu carries it.
      ...(tier >= 7
        ? [{ label: session?.title ?? sessionId, disabled: true } as MenuItem, { separator: true } as MenuItem]
        : []),
      // Editing rows are hidden from viewers (the ⋯ button itself only appears
      // for a viewer once a tier has folded something into it).
      ...(readOnly
        ? []
        : [
            { label: 'Rename…', icon: 'edit', action: startRename } as MenuItem,
            ...(isAgent ? [{ label: 'Additional directories…', icon: 'folder', action: openDirs } as MenuItem] : []),
            ...(isAgent ? [{ label: 'Hand over to…', icon: 'send', action: openHandover } as MenuItem] : []),
            { separator: true } as MenuItem,
            {
              label: attachedIssue ? 'Change Jira issue…' : 'Attach Jira issue…',
              icon: 'ticket',
              action: openAttachIssue,
            } as MenuItem,
            ...(attachedIssue ? [{ label: 'Detach issue', icon: 'link', action: detachIssue } as MenuItem] : []),
            { label: 'Attach product story…', icon: 'file', action: openAttachProductStory } as MenuItem,
            { label: 'Canvas…', icon: 'shapes', action: openCanvas } as MenuItem,
            ...(isAgent
              ? [
                  { separator: true } as MenuItem,
                  {
                    label: keepAlive ? 'Unpin (allow auto-suspend)' : 'Pin (keep alive)',
                    icon: 'pin',
                    action: () => void toggleKeepAlive(),
                  } as MenuItem,
                ]
              : []),
            // In-progress agent only: respawn a stuck PTY (provider resume when
            // possible). Idle/exited/reconnectable sessions have their own paths.
            ...(isAgent && (status === 'running' || status === 'working')
              ? [{ label: 'Restart agent', icon: 'refresh', action: () => void restart() } as MenuItem]
              : []),
          ]),
      ...(folded.length > 0 ? [{ separator: true } as MenuItem, ...folded] : []),
      ...(showClose && tier >= 7
        ? [{ separator: true } as MenuItem, { label: 'Close pane', icon: 'x', action: onclosepane } as MenuItem]
        : []),
      ...(presets.length > 0 ? [{ separator: true } as MenuItem, ...presets] : []),
      ...(readOnly
        ? []
        : [
            { separator: true } as MenuItem,
            { label: 'Archive', icon: 'archive', action: () => void archive() } as MenuItem,
            { label: 'Delete', icon: 'trash', danger: true, action: () => void del() } as MenuItem,
          ]),
    ];
  }
  function openPaneMenu(e: MouseEvent | KeyboardEvent): void {
    ctxMenu.show(e, sessionMenuItems());
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
<section
  class="pane"
  class:focused
  onmousedown={() => {
    // Interacting with the pane attends to it — drop the "needs you" flag.
    ws.clearNeedsYou(sessionId);
    onfocus();
  }}
>
  <!-- `t1`–`t7` are CUMULATIVE fold classes (tier ≥ n), the style counterpart of
       the `tier` the script folds the ⋯ menu by — one source of truth. -->
  <header
    class="pane-head"
    class:t1={tier >= 1}
    class:t2={tier >= 2}
    class:t3={tier >= 3}
    class:t4={tier >= 4}
    class:t5={tier >= 5}
    class:t6={tier >= 6}
    class:t7={tier >= 7}
    bind:this={headEl}
    bind:clientWidth={headW}
  >
    <StatusDot {status} {needsYou} />
    {#if renaming}
      <!-- svelte-ignore a11y_autofocus -->
      <input
        class="rename-input"
        bind:value={draftTitle}
        autofocus
        onblur={commitRename}
        onkeydown={(e) => {
          if (e.key === 'Enter') commitRename();
          else if (e.key === 'Escape') renaming = false;
        }}
        onmousedown={(e) => e.stopPropagation()}
      />
    {:else if tier < 7}
      <span
        class="pane-title"
        role="button"
        tabindex="0"
        title="Double-click to rename; right-click for options"
        ondblclick={startRename}
        oncontextmenu={(e) => openPaneMenu(e)}
      >{session?.title ?? sessionId}</span>
    {/if}
    {#if gripOn}
      <!-- C3a: drag this pane onto another to swap (centre) or move (edge). -->
      <button
        class="icon-btn pane-grip"
        draggable="true"
        title="Drag to move or swap this pane"
        aria-label="Move pane"
        data-testid="pane-grip"
        onmousedown={(e) => e.stopPropagation()}
        ondragstart={onGripDragStart}
        ondragend={onGripDragEnd}
        onclick={openPaneMenu}
        onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && openPaneMenu(e)}
      >
        <Icon name="grip" size={12} />
      </button>
    {/if}
    {#if nameFull && nameFull !== session?.title}
      <span class="pane-fullname" title="Themed name — address this session by “{session?.meta?.name_handle ?? session?.title}”">({nameFull})</span>
    {/if}
    <!-- `has-icon` lets the container query collapse the chip to icon-only in a
         narrow pane; icon-less providers keep their text label (nothing else to
         show). -->
    <span class="chip provider-chip" class:has-icon={hasProviderIcon(session?.provider)} title={session?.provider}>
      {#if hasProviderIcon(session?.provider)}
        <ProviderIcon provider={session?.provider ?? ''} size={13} />
      {/if}
      <span class="provider-name">{session?.provider ?? '?'}</span>
    </span>
    {#if needsYou}
      <span class="needs-you-badge" title="This session is waiting on you (input or a permission)">
        <Icon name="bell" size={10} /> Needs you
      </span>
    {/if}
    {#if summary && summary.total > 0}
      <span
        class="task-chip"
        class:done={summary.done === summary.total}
        class:active={summary.in_progress != null}
        title={summary.in_progress ? `Now: ${summary.in_progress}` : `${summary.done}/${summary.total} tasks done`}
      >{summary.done}/{summary.total}</span>
    {/if}
    {#if summary?.in_progress}
      <span class="now-task" title="Current task: {summary.in_progress}">
        now: {summary.in_progress}
      </span>
    {/if}
    {#if handoverFromId}
      <button
        class="handover-crumb"
        title="Open the session this was handed over from"
        onmousedown={(e) => e.stopPropagation()}
        onclick={() => ws.navigateToSession(handoverFromId)}
      >↰ {handoverFrom?.title ?? 'source'}</button>
    {/if}
    {#if handoverPending}
      <span class="handover-pending" title="Preparing the handover brief…">⏳ handover…</span>
    {/if}
    {#if idleHint}
      <span class="idle-hint" title="Session is idle. Auto-suspend frees its RAM while keeping it resumable.">{idleHint}</span>
    {/if}
    {#if session?.cwd}<span class="pane-cwd mono" title={session.cwd}>{session.cwd}</span>{/if}
    <span class="grow"></span>
    {#if isAgent && tier < 5}
      <div class="segmented view-seg" role="tablist" tabindex="-1" aria-label="Session view" onmousedown={(e) => e.stopPropagation()}>
        <button role="tab" class:active={effView === 'terminal'} aria-selected={effView === 'terminal'} onclick={() => setView('terminal')} title="Terminal (⌘⇧C cycles)">Terminal</button>
        <button role="tab" class:active={effView === 'chat'} aria-selected={effView === 'chat'} onclick={() => setView('chat')} title="Chat — the conversation rebuilt from the transcript">Chat</button>
        {#if wide}
          <button role="tab" class:active={effView === 'split'} aria-selected={effView === 'split'} onclick={() => setView('split')} title="Chat beside the terminal">Split</button>
        {/if}
      </div>
    {:else if isAgent && tier < 6}
      <!-- Tier 5: the three tabs collapse into one icon that opens the choice. -->
      <button
        class="icon-btn view-seg-mini"
        data-view-mini
        aria-label="Session view: {VIEW_META.find(([m]) => m === effView)?.[1] ?? 'Terminal'}"
        title="Session view (⌘⇧C cycles)"
        onmousedown={(e) => e.stopPropagation()}
        onclick={openViewMenu}
        onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && openViewMenu(e)}
      >
        <Icon name={VIEW_META.find(([m]) => m === effView)?.[2] ?? 'terminal'} size={13} />
      </button>
    {/if}
    {#if !viewport.isPhone && ui.termToolbar && effView !== 'chat'}
      <!-- Terminal font zoom + copy-on-select, surfaced in the header bar so the
           controls never float over (and hide) terminal content. The embedded
           <Terminal> gets showToolbar={false} to drop its overlay counterpart. -->
      <div class="term-ctl" role="toolbar" tabindex="-1" aria-label="Terminal controls" onmousedown={(e) => e.stopPropagation()}>
        <button class="icon-btn" onclick={() => ui.termZoomOut()} title="Terminal font smaller (Ctrl+−)" aria-label="Zoom out">−</button>
        <span class="term-ctl-size" title="Terminal font size">{ui.termFontSize}px</span>
        <button class="icon-btn" onclick={() => ui.termZoomIn()} title="Terminal font larger (Ctrl+=)" aria-label="Zoom in">+</button>
        <button
          class="icon-btn term-ctl-copy"
          class:on={ui.termCopyOnSelect}
          onclick={() => ui.setTermCopyOnSelect(!ui.termCopyOnSelect)}
          title={ui.termCopyOnSelect ? 'Copy-on-select: on — click to disable' : 'Copy-on-select: off — click to enable'}
          aria-pressed={ui.termCopyOnSelect}
          aria-label="Copy on select"
        >copy</button>
      </div>
    {/if}
    {#if showZoom && tier < 5}
      <button
        class="icon-btn"
        onmousedown={(e) => e.stopPropagation()}
        onclick={() => ws.toggleMaximize(sessionId)}
        title={maximized ? 'Restore tiled view' : 'Zoom in on this session'}
      >
        <Icon name={maximized ? 'minimize' : 'maximize'} size={13} />
      </button>
    {/if}
    {#if !readOnly && isAgent && tier < 5}
      <button class="icon-btn" onclick={restart} title="Restart session"><Icon name="refresh" size={13} /></button>
    {/if}
    {#if !readOnly || tier >= 4}
      <!-- The overflow menu. `title="More…"` is a pinned selector; the title
           moves into `aria-label` once tier 7 drops it from the header. -->
      <button
        class="icon-btn"
        onmousedown={(e) => e.stopPropagation()}
        onclick={openPaneMenu}
        onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && openPaneMenu(e)}
        title="More…"
        aria-label={tier >= 7 ? `More… — ${session?.title ?? sessionId}` : 'More…'}
      >⋯</button>
    {/if}
    {#if showClose && tier < 7}
      <button class="icon-btn" onclick={onclosepane} title={closeTitle} aria-label={closeTitle}><Icon name="x" size={12} /></button>
    {/if}
  </header>
  <div class="pane-body" class:split={effView === 'split'} class:resizing={splitResizing} bind:this={bodyEl} data-view={effView}>
    {#if effView !== 'terminal'}
      <div class="pane-chat" style={effView === 'split' ? `flex: 0 0 ${(chatFrac * 100).toFixed(2)}%` : ''}>
        <ConversationView {sessionId} workspaceId={session?.workspace_id ?? ws.currentId ?? ''} readonly={readOnly} />
      </div>
    {/if}
    {#if effView === 'split'}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="pane-splitter" class:active={splitResizing} onmousedown={startSplit} title="Drag to resize"></div>
    {/if}
    {#if effView !== 'chat'}
      <div class="pane-term">
        <Terminal bind:this={termRef} {sessionId} {readOnly} {resumable} restartable={isAgent} onrestart={restart} {restartNonce} onstatus={onTermStatus} showToolbar={false} autoFocus={kbFocused} preferDom={isAgent} claimOnAttach={!readOnly} />
      </div>
    {/if}
  </div>
</section>

{#if attachIssueOpen}
  <AttachIssue {sessionId} onclose={() => (attachIssueOpen = false)} />
{/if}

{#if attachProductOpen}
  <AttachProductStory {sessionId} onclose={() => (attachProductOpen = false)} />
{/if}

{#if handoverOpen}
  <Handover {sessionId} onclose={() => (handoverOpen = false)} />
{/if}

{#if dirsOpen}
  <Modal title="Additional directories" onclose={() => (dirsOpen = false)}>
    <div class="field">
      <label for="sv-extra-dir">Directories the agent may access <span class="dim">(beyond its working dir)</span></label>
      {#if extraDirs.length > 0}
        <ul class="dir-list">
          {#each extraDirs as dir (dir)}
            <li class="dir-row">
              <span class="dir-path mono" title={dir}>{dir}</span>
              <button
                type="button"
                class="dir-remove"
                title="Remove directory"
                onclick={() => removeDir(dir)}
              >✕</button>
            </li>
          {/each}
        </ul>
      {/if}
      <div class="dir-add">
        <input
          id="sv-extra-dir"
          class="input mono"
          bind:value={dirDraft}
          spellcheck="false"
          placeholder="/absolute/path/to/repo"
          onkeydown={onDirKeydown}
        />
        <button type="button" class="btn" disabled={dirDraft.trim() === ''} onclick={addDir}>Add</button>
      </div>
      <span class="hint">Passed as <code>--add-dir</code>. Takes effect on the next session restart.</span>
    </div>

    {#snippet footer()}
      <button class="btn" onclick={() => (dirsOpen = false)}>Cancel</button>
      <button class="btn" disabled={dirsBusy} onclick={() => saveDirs(false)}>
        {dirsBusy ? 'Saving…' : 'Save'}
      </button>
      <button class="btn primary" disabled={dirsBusy} onclick={() => saveDirs(true)}>
        {dirsBusy ? 'Saving…' : 'Save & restart'}
      </button>
    {/snippet}
  </Modal>
{/if}

<style>
  .pane {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    height: 100%;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
    background: var(--term-bg);
    transition: border-color 140ms ease-out;
  }
  .pane.focused {
    border-color: color-mix(in srgb, var(--accent) 55%, transparent);
  }
  .pane-head {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 30px;
    padding: 0 8px 0 10px;
    background: var(--surface);
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
    /* Belt and braces only: the script MEASURES this header and folds another
       tier until the inline set genuinely fits (sessions-mobile and the desktop
       pane specs assert scrollWidth − clientWidth ≤ 2, which `clip` does NOT
       hide). This just stops a one-frame flash before the tier applies. */
    overflow: clip;
  }
  /* Provider label beside its brand icon — hidden (icon-only) in a narrow pane. */
  .provider-name {
    display: inline-block;
  }
  .pane-fullname {
    font-size: 11px;
    color: var(--text-muted, var(--muted));
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 160px;
    flex-shrink: 1;
  }
  .pane-title {
    font-size: 12px;
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 180px;
  }
  .provider-chip {
    height: 16px;
    font-size: 9.5px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  /* "Needs you" — session blocked on operator input. Amber, attention-grabbing
     but tasteful; distinct from the (calmer) status dot for idle/working. */
  .needs-you-badge {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    flex-shrink: 0;
    height: 16px;
    padding: 0 6px;
    border-radius: 99px;
    font-size: 9.5px;
    font-weight: 700;
    letter-spacing: 0.02em;
    text-transform: uppercase;
    color: #febc2e;
    background: color-mix(in srgb, #febc2e 16%, transparent);
    white-space: nowrap;
  }
  /* Per-session task roll-up "done/total" — matches the sidebar chip. */
  .task-chip {
    flex-shrink: 0;
    padding: 0 5px;
    height: 15px;
    line-height: 15px;
    border-radius: 999px;
    font-size: 9px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    color: var(--text-dim);
    background: color-mix(in srgb, var(--text-dim) 16%, transparent);
  }
  .task-chip.active {
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 16%, transparent);
  }
  .task-chip.done {
    color: var(--status-working, #3fb950);
    background: color-mix(in srgb, var(--status-working, #3fb950) 16%, transparent);
  }
  /* "now: «task»" — what the agent is doing this moment. Truncates so it never
     pushes the header controls off-screen in a narrow tile. */
  .now-task {
    min-width: 0;
    flex: 0 1 auto;
    font-size: 10.5px;
    color: var(--text-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .handover-crumb {
    flex-shrink: 0;
    max-width: 130px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text-dim);
    font-size: 10.5px;
    padding: 1px 7px;
    border-radius: 99px;
    cursor: pointer;
  }
  .handover-crumb:hover {
    color: var(--text);
    border-color: color-mix(in srgb, var(--accent) 55%, transparent);
  }
  .handover-pending {
    flex-shrink: 0;
    font-size: 10.5px;
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    padding: 1px 7px;
    border-radius: 99px;
    white-space: nowrap;
  }
  /* "X min idle / suspends in N" — faint countdown for idle agent panes. Sits
     between the title area and the cwd; truncates rather than wrapping. */
  .idle-hint {
    flex-shrink: 1;
    min-width: 0;
    font-size: 10px;
    color: var(--text-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    opacity: 0.75;
  }
  .pane-cwd {
    font-size: 10.5px;
    color: var(--text-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 220px;
  }
  .pane-body {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: row;
    min-width: 0;
  }
  .pane-term {
    flex: 1;
    min-height: 0;
    min-width: 0;
  }
  .pane-chat {
    flex: 1;
    min-height: 0;
    min-width: 0;
    background: var(--bg);
  }
  .pane-splitter {
    flex: 0 0 5px;
    cursor: col-resize;
    background: var(--border);
    transition: background 120ms ease-out;
  }
  .pane-splitter:hover,
  .pane-splitter.active {
    background: color-mix(in srgb, var(--accent) 60%, var(--border));
  }
  .pane-body.resizing {
    user-select: none;
  }
  .view-seg {
    padding: 1px;
    flex-shrink: 0;
  }
  .view-seg > button {
    height: 18px;
    font-size: 10.5px;
    padding: 0 8px;
  }
  /* Tier-5 stand-in for the segmented control: one icon, menu on click. */
  .view-seg-mini {
    flex-shrink: 0;
  }
  /* C3a drag handle. `grab`/`grabbing` is the only affordance that reads as
     "pick this up" — the pane itself stays clickable for focus. */
  .pane-grip {
    flex-shrink: 0;
    cursor: grab;
    color: var(--text-dim);
  }
  .pane-grip:active {
    cursor: grabbing;
  }
  .pane-grip:hover {
    color: var(--text);
  }
  .pane-title[role='button'] {
    cursor: text;
  }
  .rename-input {
    font-size: 12px;
    font-weight: 600;
    background: var(--surface-2);
    border: 1px solid var(--accent);
    border-radius: var(--radius-s);
    color: var(--text);
    padding: 1px 6px;
    max-width: 200px;
    outline: none;
  }
  /* Terminal zoom/copy controls, inline in the header bar. */
  .term-ctl {
    display: inline-flex;
    align-items: center;
    gap: 2px;
  }
  .term-ctl-size {
    font-size: 10px;
    font-family: var(--font-mono);
    color: var(--text-dim);
    min-width: 30px;
    text-align: center;
  }
  .term-ctl-copy {
    font-size: 10px;
  }
  .term-ctl-copy.on {
    color: var(--accent);
  }
  /* Additional directories editor (mirrors New Session). */
  .dir-list {
    list-style: none;
    margin: 0 0 6px;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .dir-row {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    padding: 5px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
  }
  .dir-path {
    flex: 1;
    min-width: 0;
    font-size: 11px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text);
  }
  .dir-remove {
    flex-shrink: 0;
    background: none;
    border: none;
    cursor: pointer;
    color: var(--text-dim);
    font-size: 10px;
    padding: 2px 4px;
    border-radius: 3px;
    line-height: 1;
  }
  .dir-remove:hover {
    color: var(--danger, #e5534b);
    background: color-mix(in srgb, var(--danger, #e5534b) 12%, transparent);
  }
  .dir-add {
    display: flex;
    gap: 6px;
  }
  .dir-add .input {
    flex: 1;
    min-width: 0;
  }

  /* ── Responsive pane header ──────────────────────────────────────────────
     Driven by the `t1`–`t7` classes the script puts on `.pane-head` (tier ≥ n),
     NOT by `@container`: a container cannot query ITSELF, so the rules below
     that size the header itself silently never applied, and a container query
     can't see the measured fold the script adds when the inline set still
     doesn't fit. Degradation order, widest→narrowest: cwd → provider text →
     themed full-name → font/copy toolbar → segmented control (+ zoom/restart)
     → grip → title + ✕. The status dot and ⋯ never drop; everything else comes
     back as a ⋯ row (see the script's `paneMenuItems`). */

  /* 1. The cwd path is the first to go — longest, least critical inline. */
  .pane-head.t1 .pane-cwd {
    display: none;
  }

  /* 2. Provider chip collapses to its brand icon (text-labelled providers with
        no icon keep their text — the chip would otherwise render empty). */
  .pane-head.t2 .provider-chip.has-icon .provider-name {
    display: none;
  }
  .pane-head.t2 .provider-chip.has-icon {
    padding: 0 4px;
  }

  /* 3. Drop the themed full-name in parens; the short handle title carries it. */
  .pane-head.t3 .pane-fullname {
    display: none;
  }
  /* Tidy the header up: shorter, slightly smaller, tighter gaps. */
  .pane-head.t3 {
    height: 26px;
    gap: 6px;
  }
  .pane-head.t3 .pane-title {
    font-size: 11px;
    max-width: 130px;
  }

  /* 4. Fold the inline terminal font/copy toolbar away, plus the task/handover/
        idle chips — all of them come back as ⋯ rows (script, `tier >= 4`). */
  .pane-head.t4 .term-ctl,
  .pane-head.t4 .now-task,
  .pane-head.t4 .idle-hint,
  .pane-head.t4 .handover-crumb,
  .pane-head.t4 .handover-pending {
    display: none;
  }
  .pane-head.t4 .view-seg > button {
    padding: 0 5px;
    font-size: 10px;
  }
  .pane-head.t4 .pane-title {
    max-width: 96px;
  }

  /* 5. The segmented control becomes one view icon (script swaps the markup);
        zoom + restart and the task/needs-you chips move into ⋯. */
  .pane-head.t5 .task-chip,
  .pane-head.t5 .needs-you-badge {
    display: none;
  }
  .pane-head.t5 .pane-title {
    max-width: 80px;
  }

  /* 6. The grip goes — dragging is a mouse gesture and ⌘⌥arrows / the palette
        still move the pane; the view icon folds into ⋯ (script). */
  .pane-head.t6 .pane-grip {
    display: none;
  }
  .pane-head.t6 .pane-title {
    max-width: 60px;
  }

  /* 7. Required by the 15-pane cap (15 columns at 1280px ≈ 85px each): the
        title and ✕ move into ⋯, which is then the whole header beside the dot.
        The rename input stays — renaming must not need a wider pane. */
  .pane-head.t7 .pane-title {
    display: none;
  }
  .pane-head.t7 {
    padding: 0 4px;
    gap: 4px;
  }
</style>
