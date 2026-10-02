<script lang="ts">
  import PathField from '../../lib/components/PathField.svelte';
  // One pane: a compact, width-adaptive session header (status + title first;
  // everything secondary in the details chip or the ⋯ menu) + terminal or chat.
  import Terminal from '../../lib/components/Terminal.svelte';
  import { PRIMARY_SCROLLBACK } from '../../lib/components/termFlow';
  import StatusDot from '../../lib/components/StatusDot.svelte';
  import { events } from '../../lib/events.svelte';
  import { sessionState } from '../../lib/status';
  import Icon, { type IconName } from '../../lib/components/Icon.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import SessionNetworkStatus from '../connections/SessionNetworkStatus.svelte';
  import ProviderIcon, { hasProviderIcon } from '../../lib/components/ProviderIcon.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import AttachIssue from './AttachIssue.svelte';
  import AttachProductStory from './AttachProductStory.svelte';
  import Handover from './Handover.svelte';
  import HandoverDeliveryPanel from './HandoverDeliveryPanel.svelte';
  import ShareModal from './ShareModal.svelte';
  import { ws, isForeground } from '../../lib/stores/workspace.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { activity } from '../../lib/stores/activity.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { copyText } from '../../lib/clipboard';
  import StartRoomModal from '../rooms/StartRoomModal.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { popoutItems } from '../../lib/popoutMenu';
  import { now } from '../../lib/stores/now.svelte';
  import { idleSuspend } from '../../lib/stores/idleSuspend.svelte';
  import { suspendHint } from '../../lib/idleSuspend';
  import { ui } from '../../lib/stores/ui.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { router } from '../../lib/router.svelte';
  import { untrack } from 'svelte';
  import { transcript } from '../../lib/stores/transcript.svelte';
  import {
    cwdLabel,
    otherSessionView,
    paneDetails,
    paneDetailsTitle,
    paneTier,
    tierAtMost,
    type PaneTier,
    type SessionViewMode,
  } from '../../lib/paneHeader';
  import { presetItems } from './SplitNode.svelte';
  import ConversationView from './conversation/ConversationView.svelte';
  import type { AttachedIssue, SessionStatus } from '../../lib/api/types';
  import { uiControl } from '../../lib/stores/uiControl.svelte';
  import { commandLabel, providerName } from '../../lib/uiCommands/frames';
  import { moduleLabel } from '../../lib/sidebar';
  import { findInPage } from '../../lib/findinpage.svelte';

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
    /** Local terminal scrollback depth; unset = PRIMARY_SCROLLBACK (10k — a
     *  SessionView is a pane the user works in, while the bare Terminal
     *  defaults to the 2k embed depth). The tiled grid passes the smaller
     *  depth — 15 live tiles × 10k lines was 150–360 MB of xterm buffers (SA-05). */
    scrollback?: number;
    /** Opening the pane resumes a suspended session (Terminal `resumeOnOpen`).
     *  The tiled grid passes false: a tile shows a session, it does not ask
     *  for its CLI back — typing into it or Resume does. Default true. */
    resumeOnOpen?: boolean;
  }
  let { sessionId, focused, showClose, onfocus, onclosepane, showZoom = false, showGrip = false, dragKey, ondragpane, closeTitle = 'Close pane (keeps running)', scrollback = PRIMARY_SCROLLBACK, resumeOnOpen = true }: Props = $props();

  const maximized = $derived(ws.maximizedId === sessionId);

  const session = $derived(ws.sessions.find((s) => s.id === sessionId) ?? null);
  const status = $derived(ws.statusMap[sessionId] ?? session?.status ?? 'idle');

  // "X min idle / suspends in N" countdown hint for idle agent sessions.
  // `session.last_active_at` is updated whenever the status changes — so when
  // the session transitions to `idle` it stamps the moment. We use this as a
  // proxy for last-output time and count down to the daemon's suspend window:
  // `manual_idle_suspend_secs` (24 h) for sessions the user started,
  // `idle_suspend_grace_secs` (5 min) for engine-owned ones, read from the
  // daemon's settings (lib/idleSuspend.ts; defaults when not readable).
  const idleHint = $derived.by(() => {
    if (status !== 'idle') return null;
    if (!session?.kind || session.kind !== 'agent') return null;
    if (session.meta?.keep_alive === true) return null; // pinned — won't be suspended
    // User-started vs engine-owned MUST mirror `is_user_started` in
    // crates/otto-sessions/src/manager.rs: a background `meta.source`
    // (`isForeground`, the store's BACKGROUND_SOURCES mirror of
    // `BACKGROUND_SESSION_SOURCES`) OR a non-`manual` `meta.work.origin` makes
    // it engine-owned. Testing `!meta.source` instead would diverge for any
    // source outside that list (e.g. `personal_agent`) and count down on the
    // wrong grace.
    const origin = (session.meta?.work as { origin?: string } | undefined)?.origin;
    const userStarted = (origin === undefined || origin === 'manual') && isForeground(session);
    idleSuspend.ensure();
    const _tick = now(); // reactive dependency: re-computes every second
    return suspendHint(Date.now() - Date.parse(session.last_active_at), userStarted, idleSuspend.policy);
  });
  const readOnly = $derived(ws.myRole === 'viewer');

  // Live per-session activity roll-up (current in-progress task + done/total),
  // surfaced in the pane header so tiled/split panes show what each agent is on.
  const summary = $derived(activity.summary(sessionId));
  // Sticky "needs you" flag — the session is blocked on operator input. Distinct
  // from plain idle; cleared by the store when the user opens/inputs.
  const needsYou = $derived(ws.needsYou[sessionId] === true);
  /** The one shared session state (lib/status.ts) — a suspended session reads
   *  "Suspended", not a red "exited"; stale while the events socket is down. */
  const paneState = $derived(sessionState(session, status, needsYou, { stale: events.state !== 'connected' }));
  /** True when this agent session can be resumed after exiting. */
  const resumable = $derived(
    session?.kind === 'agent' && session?.provider_session_id != null,
  );
  /** Full display name for a themed session (e.g. "Cristiano Ronaldo"), shown
   *  beside the short handle ("Ronaldo") when the two differ. */
  const nameFull = $derived(
    ((session?.meta?.name_full as string | undefined) ?? '').trim(),
  );

  // Pane-header width tier (lib/paneHeader.ts). The header's CONTENT adapts to
  // the pane with CSS container queries on `.pane` (see the style block); this
  // script-side twin of the same breakpoints only decides which of the controls
  // the CSS hid come back as rows in the ⋯ menu, so nothing is unreachable.
  let headW = $state(0);
  const tier = $derived<PaneTier>(paneTier(headW));
  /** Zoom, ✕ and the details chip are hidden inline (CSS `minimal`). */
  const foldedMinimal = $derived(tierAtMost(tier, 'minimal'));
  /** Find folds into ⋯ from compact down (⌘F reaches the terminal anyway). */
  const foldedFind = $derived(tierAtMost(tier, 'compact'));
  /** The view switch is hidden inline too (CSS `micro`). */
  const foldedMicro = $derived(tier === 'micro');
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
    // Breakpoint changes should not steal focus from a draft in another pane.
    if (kbFocused && !readOnly && !untrack(() => viewport.isPhone)) termRef?.focus();
  });
  let draftTitle = $state('');
  let attachIssueOpen = $state(false);
  let attachProductOpen = $state(false);
  let handoverOpen = $state(false);
  let shareOpen = $state(false);
  let roomOpen = $state(false);
  /** ⋯ → Network profile…: show the network strip for a session without a
   *  profile (it is hidden then — see the markup). */
  let netOpen = $state(false);
  const networkProfileId = $derived(
    typeof session?.meta?.network_profile_id === 'string' ? session.meta.network_profile_id : '',
  );

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

  // --- Agent UI control (stores/uiControl.svelte.ts) --------------------------
  // The grant is server-owned (`meta.ui_control`); the header toggle flips it,
  // and an ungranted `otto.ui_*` call raises the inline prompt under the header.
  const uiGranted = $derived(isAgent && uiControl.granted(sessionId));
  /** The agent asked again after a Deny: no banner, just a mark on the toggle. */
  const uiAskedAgain = $derived(isAgent && uiControl.askedAfterDeny(sessionId));
  const uiBusy = $derived(uiControl.busy[sessionId] === true);
  /** Only the owner, signed in as themselves, may ALLOW it (it drives their
   *  window); anyone who sees it on may still turn it off (the daemon also
   *  lets a workspace admin revoke). */
  const uiCanGrant = $derived(
    isAgent && !readOnly && !auth.isImpersonating && !!session && session.created_by === auth.me?.id,
  );
  const uiToggleShown = $derived(uiCanGrant || (uiGranted && !readOnly));
  const uiPrompt = $derived(uiCanGrant ? uiControl.promptFor(sessionId) : null);
  /** The UI-control switch lives in ⋯; the header only carries it while it is
   *  ON or the agent asked again — a live permission stays visible (hidden by
   *  the `minimal` tier, where the ⋯ row still shows it checked). */
  const uiToggleInline = $derived(uiToggleShown && (uiGranted || uiAskedAgain));
  const agentWho = $derived(providerName(session?.provider ?? ''));
  const uiToggleTitle = $derived(
    uiGranted
      ? `UI control is on — ${agentWho} can open and drive Otto beside this session. Click to turn it off.`
      : uiAskedAgain
        ? `${agentWho} asked to drive Otto (you denied it earlier). Click to allow it for this session.`
        : `Allow UI control — let ${agentWho} open and drive Otto beside this session, where you can see it`,
  );
  function toggleUiControl(): void {
    void uiControl.setGrant(sessionId, !uiGranted);
  }
  /** What the agent asked for, in words: "“Run query” in Connections". */
  const uiAskWhat = $derived(
    uiPrompt
      ? `“${commandLabel(uiPrompt.command)}”${uiPrompt.module && uiPrompt.module !== 'shell' ? ` in ${moduleLabel(uiPrompt.module)}` : ''}`
      : '',
  );

  // --- Terminal · Chat (docs/design/conversation-view.md §5.1) ---------------
  // The chat is rebuilt from the provider transcript; probing it once per agent
  // session (cheap 200, `unavailable_reason` when nothing resolves) makes the
  // Chat view instant when picked. The default view is Terminal for every
  // session — the chat is opt-in per session, and the user's choice is
  // persisted (`otto_session_view:<id>`, winKey; a retired `split` reads as
  // chat — lib/paneHeader.ts `parseSessionView`).
  const defaultView: SessionViewMode = 'terminal';
  const savedView = $derived(isAgent ? transcript.view(sessionId) : null);
  const view = $derived<SessionViewMode>(isAgent ? (savedView ?? defaultView) : 'terminal');
  // Find: the terminal's own bar (an xterm has no DOM text for the page find
  // to walk — WebGL none at all, DOM only the visible rows); the chat IS DOM,
  // so the page-wide find covers it (ConversationView registers a provider
  // for every loaded turn). ⌘F lands here too without a click: this pane's
  // Terminal takes it while active (findRank → keys.ts routeFind); the
  // focused split pane outranks another pane showing the same session.
  const findRank = $derived(focused ? 2 : ws.activeSessionId === sessionId ? 1 : 0);
  const findLabel = $derived(view === 'chat' ? 'Find in chat' : 'Find in terminal');
  function openSessionFind(): void {
    if (view === 'chat') findInPage.show();
    else termRef?.openFind();
  }

  function setView(mode: SessionViewMode): void {
    transcript.setView(sessionId, mode);
  }
  /** ⌘⇧C and the narrow-pane flip button: Terminal ⇄ Chat. */
  function toggleView(): void {
    setView(otherSessionView(view));
  }
  const VIEW_META: [SessionViewMode, string, IconName][] = [
    ['terminal', 'Terminal', 'terminal'],
    ['chat', 'Chat', 'comment'],
  ];
  const viewLabel = (m: SessionViewMode): string => (m === 'chat' ? 'Chat' : 'Terminal');
  /** ←/→ (Home/End) move between the Terminal · Chat tabs, like any tablist;
   *  focus follows the selection (roving tabindex). */
  function onViewTabKey(e: KeyboardEvent): void {
    let next: SessionViewMode | null = null;
    if (e.key === 'ArrowRight' || e.key === 'ArrowLeft') next = otherSessionView(view);
    else if (e.key === 'Home') next = 'terminal';
    else if (e.key === 'End') next = 'chat';
    if (!next) return;
    e.preventDefault();
    setView(next);
    const list = e.currentTarget as HTMLElement;
    const target = next;
    queueMicrotask(() => list.querySelector<HTMLElement>(`[data-view="${target}"]`)?.focus());
  }
  $effect(() => {
    if (!isAgent) return;
    const onKey = (e: KeyboardEvent): void => {
      if (!(e.metaKey || e.ctrlKey) || !e.shiftKey || e.altKey) return;
      if (e.key !== 'c' && e.key !== 'C') return;
      // Only the active pane reacts (tiled/split views mount several).
      if (ws.activeSessionId !== sessionId) return;
      e.preventDefault();
      toggleView();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });
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
        await ws.restartSession(sessionId, { quiet: true });
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

  /** The size the terminal actually draws at: below the chosen size while the
   *  pane is too narrow for 80 columns (Terminal's column floor). The header
   *  shows it, so "13px" never labels 10px text. */
  let drawnFont = $state<number | null>(null);
  const fontShrunk = $derived(drawnFont !== null && drawnFont < ui.termFontSize);

  function onTermStatus(s: SessionStatus): void {
    ws.statusMap[sessionId] = s;
  }

  function startRename(): void {
    if (readOnly) return;
    draftTitle = session?.title ?? '';
    renaming = true;
  }

  async function commitRename(): Promise<void> {
    // Enter/Escape unmount the input, and WebKit fires `blur` on the removed
    // focused node — without this guard Escape COMMITTED the draft (and Enter
    // sent the rename twice).
    if (!renaming) return;
    renaming = false;
    const next = draftTitle.trim();
    if (!next || next === session?.title) return;
    try {
      await ws.renameSession(sessionId, next);
    } catch (e) {
      toasts.error('Rename failed', e instanceof Error ? e.message : String(e));
    }
  }

  // Asks first when the agent is working (it loses its in-flight turn); the
  // store bumps the restart nonce the Terminal below reconnects on.
  function restart(): Promise<void> {
    return ws.requestRestart(sessionId);
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

  // Asks first unless the user chose "Always delete" (ws.requestDeleteSession).
  function del(): Promise<void> {
    return ws.requestDeleteSession(sessionId);
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

  /** What the header no longer shows inline — the details chip's tooltip and
   *  menu, and the ⋯ info rows once the chip itself is folded away. */
  const detailRows = $derived(
    paneDetails({
      provider: session?.provider,
      nameFull: nameFull && nameFull !== session?.title ? nameFull : null,
      account: typeof session?.meta?.account_label === 'string' ? session.meta.account_label : null,
      state: paneState.key === 'needs-you' || paneState.key === 'suspended' || paneState.key === 'stale' ? paneState.label : null,
      idle: idleHint?.label,
      tasks: summary && summary.total > 0 ? { done: summary.done, total: summary.total } : null,
      now: summary?.in_progress,
      handoverFrom: handoverFromId ? (handoverFrom?.title ?? 'another session') : null,
      handoverPending,
      cwd: session?.cwd,
    }),
  );
  const detailTitle = $derived(paneDetailsTitle(detailRows));

  /** The details as menu rows: disabled label rows, plus the two things you
   *  can DO with them (copy the folder, jump to the handover source). */
  function detailItems(): MenuItem[] {
    const rows: MenuItem[] = detailRows.map(([k, v]) => ({ label: `${k}: ${v}`, disabled: true, title: v }) as MenuItem);
    if (session?.cwd) {
      const cwd = session.cwd;
      rows.push({
        label: 'Copy folder path',
        icon: 'copy',
        action: () => {
          void copyText(cwd).then((ok) =>
            ok ? toasts.info('Folder path copied') : toasts.error('Copy failed', 'The clipboard is not available here.'),
          );
        },
      });
    }
    if (handoverFromId) {
      rows.push({
        label: `Open handover source: ${handoverFrom?.title ?? 'source'}`,
        icon: 'link',
        action: () => ws.navigateToSession(handoverFromId),
      });
    }
    return rows;
  }
  function openDetails(e: MouseEvent | KeyboardEvent): void {
    ctxMenu.show(e, detailItems());
  }

  /** Terminal font + copy-on-select — they used to be header buttons; the
   *  shortcuts (⌘− ⌘+ ⌘0 in the terminal) are unchanged. */
  function terminalItems(): MenuItem[] {
    if (viewport.isPhone || view !== 'terminal') return [];
    const size = fontShrunk ? `${ui.termFontSize}px, drawn at ${drawnFont}px` : `${ui.termFontSize}px`;
    return [
      { label: 'Terminal font larger', icon: 'plus', hint: '⌘+', disabled: ui.termFontSize >= 28, action: () => ui.termZoomIn() },
      { label: 'Terminal font smaller', icon: 'minus', hint: '⌘−', disabled: ui.termFontSize <= 8, action: () => ui.termZoomOut() },
      {
        label: `Reset terminal font (${size})`,
        icon: 'text',
        hint: '⌘0',
        title: fontShrunk ? 'Drawn smaller so this narrow pane keeps 80 columns' : undefined,
        action: () => ui.termZoomReset(),
      },
      { label: 'Copy on select', checked: ui.termCopyOnSelect, action: () => ui.setTermCopyOnSelect(!ui.termCopyOnSelect) },
    ];
  }

  /** Single source of truth for the session actions menu — served both by the
   *  header ⋯ button and the title's right-click, through the global clamped
   *  ctxMenu (viewport clamp + max-height come for free).
   *
   *  It is also the overflow menu: the pane controls (font, copy-on-select,
   *  restart) always live here, and every control a narrow tier hides from the
   *  header (the view switch, zoom, ✕, the details chip) comes back as a row,
   *  so nothing is ever unreachable — a 100 px pane is a dot, a title and ⋯. */
  function sessionMenuItems(): MenuItem[] {
    // Pane controls first: the rows that used to be header buttons.
    const controls: MenuItem[] = [
      ...(foldedMicro && isAgent
        ? VIEW_META.map(([m, label, icon]) => ({
            label: `${label} view`,
            icon,
            checked: view === m,
            hint: view === m ? undefined : '⌘⇧C',
            action: () => setView(m),
          }) as MenuItem)
        : []),
      ...(foldedFind ? [{ label: findLabel, icon: 'search', hint: '⌘F', action: openSessionFind } as MenuItem] : []),
      ...(foldedMinimal && showZoom
        ? [
            {
              label: maximized ? 'Restore tiled view' : 'Zoom in on this session',
              icon: maximized ? 'minimize' : 'maximize',
              action: () => ws.toggleMaximize(sessionId),
            } as MenuItem,
          ]
        : []),
      ...terminalItems(),
      ...(!readOnly && isAgent
        ? [
            {
              label: 'Restart session',
              icon: 'refresh',
              title: status === 'working' ? 'Asks first — the agent is working' : undefined,
              action: () => void restart(),
            } as MenuItem,
          ]
        : []),
    ];
    // Layout presets — flat rows (the ctxMenu has no submenus), only with a
    // split to re-arrange. The same list the DB pane's ✕ shows, shared so the
    // two can't drift apart.
    const presets: MenuItem[] = showClose ? presetItems() : [];
    const details: MenuItem[] = foldedMinimal ? detailItems() : [];
    return [
      ...controls,
      ...(controls.length > 0 ? [{ separator: true } as MenuItem] : []),
      ...popoutItems(`agents/${sessionId}`, session?.title),
      // Editing rows are hidden from viewers.
      ...(readOnly
        ? []
        : [
            { label: 'Rename', icon: 'edit', action: startRename } as MenuItem,
            ...(isAgent ? [{ label: 'Additional directories…', icon: 'folder', action: openDirs } as MenuItem] : []),
            ...(isAgent ? [{ label: 'Hand over to…', icon: 'send', action: openHandover } as MenuItem] : []),
            // Parity with the tab's right-click menu — a tiled/split pane has no
            // tab to right-click, so Share was unreachable from here.
            ...((session?.kind === 'agent') ? [{ label: 'Start room…', icon: 'people', action: () => (roomOpen = true) } as MenuItem] : []),
            { label: 'Share…', icon: 'share', action: () => (shareOpen = true) } as MenuItem,
            { separator: true } as MenuItem,
            {
              label: attachedIssue ? 'Change Jira issue…' : 'Attach Jira issue…',
              icon: 'ticket',
              action: openAttachIssue,
            } as MenuItem,
            ...(attachedIssue ? [{ label: 'Detach issue', icon: 'link', action: detachIssue } as MenuItem] : []),
            { label: 'Attach product story…', icon: 'file', action: openAttachProductStory } as MenuItem,
            { label: 'Canvas…', icon: 'shapes', action: openCanvas } as MenuItem,
            ...(auth.can('connections', 'view') && !networkProfileId
              ? [{ label: netOpen ? 'Hide network profile' : 'Network profile…', icon: 'globe', action: () => (netOpen = !netOpen) } as MenuItem]
              : []),
            ...(isAgent
              ? [
                  { separator: true } as MenuItem,
                  ...(uiToggleShown
                    ? [
                        {
                          label: 'Allow UI control',
                          icon: 'cursor',
                          checked: uiGranted,
                          disabled: uiBusy,
                          title: uiToggleTitle,
                          action: toggleUiControl,
                        } as MenuItem,
                      ]
                    : []),
                  {
                    label: keepAlive ? 'Unpin (allow auto-suspend)' : 'Pin (keep alive)',
                    icon: 'pin',
                    action: () => void toggleKeepAlive(),
                  } as MenuItem,
                ]
              : []),
          ]),
      ...(details.length > 0 ? [{ separator: true } as MenuItem, ...details] : []),
      ...(showClose && foldedMinimal
        ? [
            { separator: true } as MenuItem,
            // Same words as the header ✕ it replaces: in a split that closes the
            // SESSION (archive/delete per Settings), not merely the pane.
            { label: closeTitle.split(' (')[0], icon: 'x', action: onclosepane } as MenuItem,
          ]
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
  class:current={kbFocused}
  onmousedown={() => {
    // Interacting with the pane attends to it — drop the "needs you" flag.
    ws.clearNeedsYou(sessionId);
    onfocus();
  }}
>
  <!-- ONE row, adapted to the PANE width by the `@container pane` rules below
       (full ≥720 · compact 420–719 · minimal 200–419 · micro <200). Priority:
       status + title always; then the view switch and ⋯; everything secondary
       sits in the details chip or the ⋯ menu. `data-tier` mirrors the script's
       twin of the breakpoints (which rows ⋯ adds back) for tests. -->
  <header class="pane-head" class:grip-on={gripOn} data-tier={tier} bind:clientWidth={headW}>
    {#if gripOn}
      <!-- C3a: drag this pane onto another to swap (centre) or move (edge). It
           sits in the header's leading padding and only shows on hover/focus;
           the title is a drag handle too. -->
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
    <StatusDot state={paneState} />
    {#if renaming}
      <!-- svelte-ignore a11y_autofocus -->
      <input
        class="rename-input"
        aria-label="Session name"
        bind:value={draftTitle}
        autofocus
        onblur={commitRename}
        onkeydown={(e) => {
          if (e.key === 'Enter') commitRename();
          else if (e.key === 'Escape') renaming = false;
        }}
        onmousedown={(e) => e.stopPropagation()}
      />
    {:else}
      <span
        class="pane-title"
        class:draggable={gripOn}
        role="button"
        tabindex="0"
        draggable={gripOn ? 'true' : undefined}
        title="{session?.title ?? sessionId}{nameFull && nameFull !== session?.title ? ` (${nameFull})` : ''} — double-click to rename, right-click for options"
        ondragstart={gripOn ? onGripDragStart : undefined}
        ondragend={gripOn ? onGripDragEnd : undefined}
        ondblclick={startRename}
        oncontextmenu={(e) => openPaneMenu(e)}
        onkeydown={(e) => {
          if (e.key === 'F2' || e.key === 'Enter') {
            e.preventDefault();
            startRename();
          } else if (e.key === 'ContextMenu' || (e.key === 'F10' && e.shiftKey)) {
            e.preventDefault();
            openPaneMenu(e);
          }
        }}
      >{session?.title ?? sessionId}</span>
    {/if}
    {#if paneState.key === 'needs-you'}
      <span class="needs-you-badge" title="This session is waiting on you (input or a permission)">
        <Icon name="bell" size={11} /><span class="head-lbl">Needs you</span>
      </span>
    {:else if paneState.key === 'suspended' || paneState.key === 'stale'}
      <span class="state-note" title={paneState.hint}>{paneState.label}</span>
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
      <span class="now-task" title="Current task: {summary.in_progress}">Now: {summary.in_progress}</span>
    {/if}
    {#if handoverFromId}
      <button
        class="handover-crumb"
        title="Handed over from “{handoverFrom?.title ?? 'another session'}” — open it"
        onmousedown={(e) => e.stopPropagation()}
        onclick={() => ws.navigateToSession(handoverFromId)}
        aria-label="Open handover source: {handoverFrom?.title ?? 'source'}"
      ><Icon name="undo" size={11} /><span class="crumb-text head-lbl">{handoverFrom?.title ?? 'source'}</span></button>
    {/if}
    {#if handoverPending}
      <span class="handover-pending" title="Preparing the handover brief…"><Icon name="clock" size={11} /><span class="head-lbl">Preparing handover…</span></span>
    {/if}
    <!-- The details chip: provider (+ idle countdown + folder when wide) —
         the tooltip and its menu carry everything else (themed name, account,
         state, tasks, full cwd). Icon-less providers keep their text label. -->
    <button
      class="meta-chip"
      class:has-icon={hasProviderIcon(session?.provider)}
      data-testid="pane-details"
      title={detailTitle}
      aria-label="Session details: {detailRows.map(([k, v]) => `${k} ${v}`).join(', ')}"
      aria-haspopup="menu"
      onmousedown={(e) => e.stopPropagation()}
      onclick={openDetails}
    >
      {#if hasProviderIcon(session?.provider)}
        <ProviderIcon provider={session?.provider ?? ''} size={13} />
      {/if}
      <span class="meta-text">
        <span class="provider-name">{session?.provider ?? '?'}</span>
        {#if idleHint}<span class="meta-extra meta-idle">{idleHint.label}</span>{/if}
        {#if session?.cwd}<span class="meta-extra meta-cwd mono">{cwdLabel(session.cwd)}</span>{/if}
      </span>
    </button>
    <span class="grow"></span>
    {#if isAgent}
      <div class="segmented view-seg" role="tablist" tabindex="-1" aria-label="Session view" onmousedown={(e) => e.stopPropagation()} onkeydown={onViewTabKey}>
        {#each VIEW_META as [m, label, icon] (m)}
          <button
            role="tab"
            class:active={view === m}
            aria-selected={view === m}
            aria-label={label}
            tabindex={view === m ? 0 : -1}
            data-view={m}
            onclick={() => setView(m)}
            title={m === 'chat' ? 'Chat — the conversation rebuilt from the transcript (⌘⇧C)' : 'Terminal (⌘⇧C)'}
          ><Icon name={icon} size={12} /><span class="head-lbl">{label}</span></button>
        {/each}
      </div>
      <!-- `minimal` tier stand-in: one button that flips to the other view. -->
      <button
        class="icon-btn view-flip"
        data-view-toggle
        aria-label="Switch to {viewLabel(otherSessionView(view))} view"
        title="Switch to {viewLabel(otherSessionView(view))} view (⌘⇧C)"
        onmousedown={(e) => e.stopPropagation()}
        onclick={toggleView}
      ><Icon name={otherSessionView(view) === 'chat' ? 'comment' : 'terminal'} size={13} /></button>
    {/if}
    {#if uiToggleInline}
      <button
        class="icon-btn ui-ctl"
        class:on={uiGranted}
        class:asked={uiAskedAgain}
        onmousedown={(e) => e.stopPropagation()}
        onclick={toggleUiControl}
        disabled={uiBusy}
        aria-pressed={uiGranted}
        title={uiToggleTitle}
        aria-label="Allow UI control"
        data-testid="ui-control-toggle"
      ><Icon name="cursor" size={13} /></button>
    {/if}
    <button
      class="icon-btn pane-find"
      data-testid="pane-find"
      onmousedown={(e) => e.stopPropagation()}
      onclick={openSessionFind}
      title="{findLabel} (⌘F)"
      aria-label={findLabel}
    ><Icon name="search" size={13} /></button>
    {#if showZoom}
      <button
        class="icon-btn pane-zoom"
        onmousedown={(e) => e.stopPropagation()}
        onclick={() => ws.toggleMaximize(sessionId)}
        title={maximized ? 'Restore tiled view' : 'Zoom in on this session'}
        aria-label={maximized ? 'Restore tiled view' : 'Zoom in on this session'}
      >
        <Icon name={maximized ? 'minimize' : 'maximize'} size={13} />
      </button>
    {/if}
    <!-- The overflow menu — always present: it holds the pane controls (font,
         copy-on-select, restart) and whatever a narrow tier hid. `title="More…"`
         is a pinned selector. -->
    <button
      class="icon-btn pane-more"
      onmousedown={(e) => e.stopPropagation()}
      onclick={openPaneMenu}
      onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && openPaneMenu(e)}
      title="More…"
      aria-label="More… — {session?.title ?? sessionId}"
      aria-haspopup="menu"
    ><Icon name="more" size={13} /></button>
    {#if showClose}
      <button class="icon-btn pane-close" onclick={onclosepane} title={closeTitle} aria-label={closeTitle}><Icon name="x" size={12} /></button>
    {/if}
  </header>
  <!-- The network strip only takes a row when the session HAS a profile (or
       the person asked for it from ⋯ → Network profile…) — "Network: none ·
       Direct" in every session header was chrome with nothing to say. -->
  {#if session && auth.can('connections', 'view') && (networkProfileId || netOpen)}
    {#key sessionId}<SessionNetworkStatus defaultOpen={netOpen && !networkProfileId} {sessionId} workspaceId={session.workspace_id} selectedProfileId={typeof session.meta?.network_profile_id === 'string' ? session.meta.network_profile_id : ''} editable={!readOnly} manageEditable={!readOnly && auth.can('connections', 'edit')} onchange={async (id) => { await ws.updateSessionMeta(sessionId, { network_profile_id: id || null }); }} />{/key}
  {/if}
  {#if uiPrompt}
    <!-- An agent called an `otto.ui_*` tool without the grant. Asked once per
         session: Deny is remembered on this device (the toggle then only
         carries a mark), Allow lasts for the session. -->
    <div class="ui-ask" role="group" aria-label="UI control request" data-testid="ui-control-request">
      <span class="ui-ask-mark" aria-hidden="true"><Icon name="cursor" size={13} /></span>
      <span class="ui-ask-text" aria-live="polite">
        <strong>{agentWho}{session?.title ? ` · ${session.title}` : ''} wants to drive Otto</strong> — {uiAskWhat}. Everything it does shows beside this session, and writes still ask you.
      </span>
      <span class="ui-ask-actions">
        <button class="btn small" onmousedown={(e) => e.stopPropagation()} onclick={() => void uiControl.deny(sessionId)} disabled={uiBusy} data-testid="ui-control-deny">Deny</button>
        <button class="btn small primary" onmousedown={(e) => e.stopPropagation()} onclick={() => void uiControl.allow(sessionId)} disabled={uiBusy} data-testid="ui-control-allow">Allow for this session</button>
      </span>
    </div>
  {/if}
  {#if session?.meta?.handover}<HandoverDeliveryPanel {session} readonly={readOnly} />{/if}
  <div class="pane-body" data-view={view}>
    {#if view === 'chat'}
      <div class="pane-chat">
        <ConversationView {sessionId} workspaceId={session?.workspace_id ?? ws.currentId ?? ''} readonly={readOnly} />
      </div>
    {:else}
      <div class="pane-term">
        <Terminal bind:this={termRef} {sessionId} {readOnly} {resumable} restartable={isAgent} onrestart={restart} restartNonce={ws.restartNonces[sessionId] ?? 0} onstatus={onTermStatus} onfontfit={(px) => (drawnFont = px)} showToolbar={false} autoFocus={kbFocused} {findRank} preferDom={isAgent} claimOnAttach={!readOnly} keepAlive={true} {scrollback} {resumeOnOpen} />
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

{#if roomOpen}<StartRoomModal {sessionId} onclose={() => (roomOpen = false)} />{/if}
{#if shareOpen}
  <ShareModal {sessionId} onclose={() => (shareOpen = false)} />
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
                aria-label="Remove {dir}"
                onclick={() => removeDir(dir)}
              ><Icon name="x" size={11} /></button>
            </li>
          {/each}
        </ul>
      {/if}
      <div class="dir-add">
        <PathField bind:value={dirDraft}><input
          id="sv-extra-dir"
          class="input mono"
          bind:value={dirDraft}
          spellcheck="false"
          placeholder="/absolute/path/to/repo"
          onkeydown={onDirKeydown}
        /></PathField>
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
  /* Agent UI control: the header toggle (accent while on — it is a selected
     state) and the one-time request strip under the header. */
  .ui-ctl.on {
    color: var(--accent-text);
    background: var(--accent-soft);
  }
  .ui-ctl.asked {
    position: relative;
  }
  .ui-ctl.asked::after {
    content: '';
    position: absolute;
    inset-block-start: 3px;
    inset-inline-end: 3px;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--status-warn);
  }
  .ui-ask {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px 10px;
    padding: 6px 10px;
    border-bottom: 1px solid var(--separator);
    background: var(--warning-soft);
    color: var(--text);
    font-size: var(--fs-s);
  }
  .ui-ask-mark {
    display: inline-flex;
    color: var(--warning);
    flex-shrink: 0;
  }
  .ui-ask-text {
    flex: 1 1 240px;
    min-width: 0;
    line-height: 1.4;
  }
  .ui-ask-actions {
    display: inline-flex;
    gap: 6px;
    flex-shrink: 0;
    margin-inline-start: auto;
  }
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
    /* The header adapts to the PANE's inline size (the `@container pane` rules
       at the end). The query container is the pane, not the header: a
       container can't query itself. Inline-size containment means the pane has
       no intrinsic width — hosts that shrink-wrap it must make it fill
       (SwarmPage `.session-panel > .pane`). */
    container: pane / inline-size;
  }
  .pane.focused {
    border-color: color-mix(in srgb, var(--accent) 55%, transparent);
  }
  .pane-head {
    position: relative;
    display: flex;
    align-items: center;
    gap: 6px;
    /* Constant at every tier — it is chrome; the tiers change WHAT is shown,
       never the height. */
    height: 30px;
    padding-block: 0;
    padding-inline: 10px 4px;
    background: var(--surface);
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
    /* Belt and braces: the title is the only flexible item and it ellipsizes,
       so the row always fits (e2e asserts scrollWidth − clientWidth ≤ 2). */
    overflow: clip;
  }
  /* Room in the leading padding for the hover-only drag grip. */
  .pane-head.grip-on {
    padding-inline-start: 16px;
  }
  /* The title gets the space: it is the one item that grows, and the chips
     beside it shrink (and fold away by tier) before it does. */
  .pane-title {
    flex: 0 1 auto;
    min-width: 0;
    font-size: var(--fs-s);
    font-weight: 600;
    color: var(--text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    border-radius: var(--radius-s);
    padding-inline: 2px;
    transition: color 140ms ease-out;
  }
  .pane-title[role='button'] {
    cursor: default;
  }
  .pane-title.draggable {
    cursor: grab;
  }
  .pane-title:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  /* The active pane reads at full contrast; the others step back (dim title,
     quieter controls until hovered) — distinguishable without extra chrome. */
  .pane:not(.current) .pane-title {
    color: var(--text-dim);
    font-weight: 500;
  }
  .pane:not(.current) .pane-head > :is(.view-seg, .view-flip, .ui-ctl, .pane-find, .pane-zoom, .pane-more, .pane-close, .meta-chip) {
    opacity: 0.7;
    transition: opacity 140ms ease-out;
  }
  .pane:not(.current) .pane-head:is(:hover, :focus-within) > :is(.view-seg, .view-flip, .ui-ctl, .pane-find, .pane-zoom, .pane-more, .pane-close, .meta-chip) {
    opacity: 1;
  }
  /* C3a drag handle: in the leading padding, visible on hover/focus only. The
     `grab` cursor is what reads as "pick this up". */
  .pane-head > .pane-grip {
    position: absolute;
    inset-inline-start: 2px;
    inset-block-start: 5px;
    width: 12px;
    height: 20px;
    opacity: 0;
    cursor: grab;
    transition: opacity 120ms ease-out;
  }
  .pane-head:hover > .pane-grip,
  .pane-head > .pane-grip:focus-visible {
    opacity: 1;
  }
  .pane-head > .pane-grip:active {
    cursor: grabbing;
  }
  .rename-input {
    flex: 0 1 220px;
    min-width: 60px;
    font-size: var(--fs-s);
    font-weight: 600;
    background: var(--surface-2);
    border: 1px solid var(--accent);
    border-radius: var(--radius-s);
    color: var(--text);
    padding: 1px 6px;
    outline: none;
  }
  .grow {
    flex: 1 1 0;
    min-width: 0;
  }
  /* "Needs you" — session blocked on operator input. Amber, attention-grabbing
     but tasteful; distinct from the (calmer) status dot for idle/working. */
  .needs-you-badge {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    flex-shrink: 0;
    height: 18px;
    padding: 0 7px;
    border-radius: 99px;
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--warning);
    background: var(--warning-soft);
    white-space: nowrap;
  }
  /* Quiet state word next to the dot for the states a dot alone can't say
     (suspended, reconnecting). */
  .state-note {
    flex-shrink: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    white-space: nowrap;
  }
  /* Per-session task roll-up "done/total" — matches the sidebar chip. */
  .task-chip {
    flex-shrink: 0;
    padding: 0 5px;
    height: 16px;
    line-height: 16px;
    border-radius: 999px;
    font-size: var(--fs-xs);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    color: var(--text-dim);
    background: var(--surface-2);
  }
  .task-chip.active {
    color: var(--accent-text);
    background: color-mix(in srgb, var(--accent) 16%, transparent);
  }
  .task-chip.done {
    color: var(--success);
    background: var(--success-soft);
  }
  /* "Now: «task»" — what the agent is doing this moment. Shrinks first (it
     never takes space from the title) and only shows on a wide pane. */
  .now-task {
    flex: 0 1000 auto;
    min-width: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .handover-crumb {
    flex: 0 100 auto;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    min-width: 22px;
    max-width: 130px;
    height: 18px;
    overflow: hidden;
    white-space: nowrap;
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text-dim);
    font-size: var(--fs-xs);
    padding: 0 6px;
    border-radius: 99px;
    cursor: pointer;
  }
  .crumb-text {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .handover-crumb:hover {
    color: var(--text);
    border-color: color-mix(in srgb, var(--accent) 55%, transparent);
  }
  .handover-pending {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    flex-shrink: 0;
    height: 18px;
    font-size: var(--fs-xs);
    color: var(--accent-text);
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    padding: 0 7px;
    border-radius: 99px;
    white-space: nowrap;
  }
  /* The details chip: provider icon (+ name · idle countdown · folder on a
     wide pane). A quiet pill-shaped button — its tooltip and menu carry the
     rest. It shrinks (text first) long before the title does. */
  .meta-chip {
    flex: 0 100 auto;
    display: inline-flex;
    align-items: center;
    gap: 5px;
    min-width: 22px;
    max-width: 320px;
    height: 20px;
    padding: 0 7px;
    border: 1px solid transparent;
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-xs);
    cursor: pointer;
    overflow: hidden;
    transition: border-color 130ms ease-out, color 130ms ease-out;
  }
  .meta-chip:hover,
  .meta-chip:focus-visible {
    color: var(--text);
    border-color: var(--border-strong);
  }
  .meta-chip:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .meta-chip > :global(svg),
  .meta-chip > :global(img) {
    flex-shrink: 0;
  }
  .meta-text {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta-extra::before {
    content: '·';
    margin-inline: 5px;
    opacity: 0.6;
  }
  .meta-idle {
    opacity: 0.85;
  }
  /* Terminal · Chat: a two-state segmented toggle — icon + label when wide,
     icon-only when compact, one flip button (`.view-flip`) when minimal. */
  .view-seg {
    flex-shrink: 0;
    padding: 1px;
  }
  .view-seg > button {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 20px;
    padding: 0 8px;
    font-size: var(--fs-xs);
  }
  .view-seg > button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -1px;
  }
  .view-flip {
    display: none;
  }
  .pane-head > :is(.view-flip, .ui-ctl, .pane-find, .pane-zoom, .pane-more, .pane-close) {
    flex-shrink: 0;
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
    font-size: var(--fs-xs);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text);
  }
  .dir-remove {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    background: none;
    border: none;
    cursor: pointer;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    padding: 2px 4px;
    border-radius: 3px;
    line-height: 1;
  }
  .dir-remove:hover {
    color: var(--danger);
    background: color-mix(in srgb, var(--danger) 12%, transparent);
  }
  .dir-add {
    display: flex;
    gap: 6px;
  }
  .dir-add .input {
    flex: 1;
    min-width: 0;
  }

  /* ── Adaptive pane header ────────────────────────────────────────────────
     Container queries on the PANE (`container: pane`). Tiers, widest first —
     keep in step with PANE_TIER_MIN in lib/paneHeader.ts, whose script twin
     adds the hidden controls back into ⋯:
       full    ≥720  everything: labelled toggle, provider name · idle · folder
       compact 420–719  icon-only toggle, details chip = provider icon, no "Now:", find → ⋯
       minimal 200–419  dot · title · view flip · ⋯   (zoom, ✕, chips → ⋯)
       micro   <200     dot · title · ⋯              (view switch → ⋯ too)
     The status dot, the title and ⋯ never go. */
  @container pane (width < 720px) {
    .pane-head .head-lbl,
    .pane-head .now-task,
    .pane-head .meta-text {
      display: none;
    }
    /* Icon-less providers keep their text label (the chip would be empty). */
    .pane-head .meta-chip:not(.has-icon) .meta-text {
      display: inline;
    }
    .pane-head .meta-chip:not(.has-icon) .meta-extra {
      display: none;
    }
    .pane-head .meta-chip.has-icon {
      padding: 0 4px;
    }
    .pane-head .needs-you-badge,
    .pane-head .handover-pending {
      padding: 0 4px;
    }
    .pane-head .handover-crumb {
      padding: 0 4px;
    }
    .pane-head .view-seg > button {
      padding: 0 6px;
    }
    .pane-head .pane-find {
      display: none;
    }
  }
  @container pane (width < 420px) {
    .pane-head .view-seg,
    .pane-head .meta-chip,
    .pane-head .task-chip,
    .pane-head .needs-you-badge,
    .pane-head .state-note,
    .pane-head .handover-crumb,
    .pane-head .handover-pending,
    .pane-head .ui-ctl,
    .pane-head .pane-zoom,
    .pane-head .pane-close {
      display: none;
    }
    .pane-head .view-flip {
      display: inline-flex;
    }
    .pane-head {
      gap: 4px;
    }
  }
  @container pane (width < 200px) {
    .pane-head .view-flip {
      display: none;
    }
    .pane-head {
      padding-inline: 6px 2px;
    }
    .pane-head.grip-on {
      padding-inline-start: 14px;
    }
  }
</style>
