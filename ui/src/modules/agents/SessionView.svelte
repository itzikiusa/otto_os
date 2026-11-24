<script lang="ts">
  import { rowMenu } from '../../lib/rowMenu';
  import { focusOnMount } from '../../lib/focusOnMount';
  import { toastError } from '../../lib/toastError';
  // One pane: a compact, width-adaptive session header (status + title first;
  // everything secondary in the details chip or the ⋯ menu) + the terminal.
  import Terminal from '../../lib/components/Terminal.svelte';
  import { PRIMARY_SCROLLBACK } from '../../lib/components/termFlow';
  import { events } from '../../lib/events.svelte';
  import { sessionState } from '../../lib/status';
  import { elapsed, sessionWord, type StatusWord } from '../../lib/statusWords';
  import { loadPtyConversationView } from '../../lib/runtimeThreadPort';
  import { lsGet, lsRemove, lsSet } from '../../lib/storage';
  import Icon from '../../lib/components/Icon.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import SessionNetworkStatus from '../connections/SessionNetworkStatus.svelte';
  import { mcpProfiles } from '../../lib/stores/mcp-profiles.svelte';
  import { sessionProfileIds } from '../../lib/mcpProfiles';
  import HandoverDeliveryPanel from './HandoverDeliveryPanel.svelte';
  import { ws, isForeground } from '../../lib/stores/workspace.svelte';
  import { activity } from '../../lib/stores/activity.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { copyText } from '../../lib/clipboard';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { sessionHandoffItems } from './sessionHandoff';
  import { registry } from '../../lib/commands.svelte';
  import { popoutItems } from '../../lib/popoutMenu';
  import { now, nowMinute, rel } from '../../lib/stores/now.svelte';
  import { sessionUsage } from '../../lib/stores/sessionUsage.svelte';
  import { sessionUsageLabel } from '../../lib/sessionUsage';
  import { suspendedNote } from '../../lib/status';
  import { idleSuspend } from '../../lib/stores/idleSuspend.svelte';
  import { suspendHint } from '../../lib/idleSuspend';
  import { ui } from '../../lib/stores/ui.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { router } from '../../lib/router.svelte';
  import { untrack } from 'svelte';
  import {
    cwdLabel,
    paneDetails,
    paneTier,
    tierAtMost,
    type PaneTier,
  } from '../../lib/paneHeader';
  import { presetItems } from './SplitNode.svelte';
  import LazyMount from '../../lib/components/LazyMount.svelte';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  const AttachIssueView = lazyComponent(() => import('./AttachIssue.svelte'));
  const AttachStoryView = lazyComponent(() => import('./AttachProductStory.svelte'));
  const HandoverView = lazyComponent(() => import('./Handover.svelte'));
  const StartRoomView = lazyComponent(() => import('../rooms/StartRoomModal.svelte'));
  const McpProfilesView = lazyComponent(() => import('./SessionMcpProfiles.svelte'));
  const ExtraDirsView = lazyComponent(() => import('./ExtraDirsModal.svelte'));
  // The transcript view of a PTY agent (agents/conversation, bound by the shell).
  const ChatView = lazyComponent(() => loadPtyConversationView());
  import type { AttachedIssue, SessionStatus } from '../../lib/api/types';
  import { uiControl } from '../../lib/stores/uiControl.svelte';
  import { commandLabel, providerName } from '../../lib/uiCommands/frames';
  import { moduleLabel } from '../../lib/sidebar';
  import { announceShown } from '../../lib/announce.svelte';
  import ApprovalActions from '../../lib/components/ApprovalActions.svelte';

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
    /** Local terminal scrollback depth; unset = PRIMARY_SCROLLBACK (4000 — a
     *  SessionView is a pane the user works in, while the bare Terminal
     *  defaults to the 2k embed depth). The tiled grid passes the smaller
     *  depth — 15 live tiles × 10k lines was 150–360 MB of xterm buffers (SA-05). */
    scrollback?: number;
    /** Opening the pane resumes a suspended session.
     *  The tiled grid passes false: a tile shows a session, it does not ask
     *  for its CLI back — typing into it or Resume does. Default true. */
    resumeOnOpen?: boolean;
  }
  let { sessionId, focused, showClose, onfocus, onclosepane, showZoom = false, showGrip = false, dragKey, ondragpane, closeTitle = 'Close pane (keeps running)', scrollback = PRIMARY_SCROLLBACK, resumeOnOpen = true }: Props = $props();

  const maximized = $derived(ws.maximizedId === sessionId);

  const session = $derived(ws.getSession(sessionId) ?? null);
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
    const idleMs = Date.now() - Date.parse(session.last_active_at);
    // Reactive dependency: the 1-second clock only while the label shows
    // seconds; past a minute the label is minute-grain, so re-run once a
    // minute (12 tiled panes stop recomputing every second — A3).
    if (idleMs > 60_000) nowMinute();
    else now();
    return suspendHint(idleMs, userStarted, idleSuspend.policy);
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
  /** The header's status WORD (lib/statusWords.ts): Needs you · Working ·
   *  Idle · Stopped …; the elapsed time only while Working, minute grain
   *  (an idle pane never ticks). Suspended / Reconnecting… keep their label. */
  let workingSince = $state<number | null>(null);
  $effect(() => {
    const w = status === 'working';
    untrack(() => {
      if (!w) workingSince = null;
      else if (workingSince === null) workingSince = Date.now();
    });
  });
  const word = $derived.by((): StatusWord => {
    if (paneState.key === 'suspended' || paneState.key === 'stale') return { word: paneState.label, tone: '' };
    const w = sessionWord(paneState);
    const ms = paneState.key === 'working' && workingSince ? nowMinute() - workingSince : 0;
    return ms >= 60_000 ? { word: `Working ${elapsed(ms)}`, tone: '' } : w;
  });
  /** Where it works, dim after the title: its branch, else its folder. */
  const where = $derived.by(() => {
    const m = session?.meta as Record<string, unknown> | undefined;
    const b = [m?.git_branch, m?.branch, m?.worktree_branch].find((v): v is string => typeof v === 'string' && v.trim() !== '');
    return b ?? (session?.cwd ? cwdLabel(session.cwd) : null);
  });

  // Terminal ⇄ Chat (an agent CLI with a transcript — a shell running one says
  // so in `meta.nested_provider`; a plain shell has none): the transcript view
  // of the same session, remembered per session.
  const chatOk = $derived(session?.kind === 'agent' && (session.provider !== 'shell' || typeof session.meta?.nested_provider === 'string'));
  const viewKey = $derived(`otto_pane_view:${sessionId}`);
  let viewTick = $state(0);
  const view = $derived.by(() => {
    void viewTick;
    return chatOk && lsGet(viewKey) === 'chat' ? 'chat' : 'terminal';
  });
  function setView(v: 'terminal' | 'chat'): void {
    if (v === 'chat') lsSet(viewKey, 'chat');
    else lsRemove(viewKey);
    viewTick++;
  }

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
  // Lazy: ShareModal bundles the QR encoder (qrcode), kept out of the Agents chunk.
  let ShareModal = $state<typeof import('./ShareModal.svelte').default | null>(null);
  async function openShare(): Promise<void> {
    try {
      ShareModal ??= (await import('./ShareModal.svelte')).default;
      shareOpen = true;
    } catch (e) {
      toastError('Couldn’t open sharing', e);
    }
  }
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
    handoverFromId ? (ws.getSession(handoverFromId) ?? null) : null,
  );
  // The source may not be a sidebar session (the list carries only those):
  // fetch it by id once so the breadcrumb resolves. Once per id: `getSession`
  // reads a Map rebuilt on every `sessions` write (status flushes included),
  // and a source that isn't listable here (deleted, other workspace, device
  // isolation) never lands — without the guard every status burst re-GETs it.
  let ensuredFrom: string | null = null;
  $effect(() => {
    const id = handoverFromId;
    if (!id || id === ensuredFrom) return;
    if (!untrack(() => ws.getSession(id))) {
      ensuredFrom = id;
      void ws.ensureSession(id);
    }
  });
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
  /** Which UI-control answer is in flight (ApprovalActions' busy label). */
  let uiAct = $state<'approve' | 'deny' | null>(null);
  async function runUiAct(kind: 'approve' | 'deny', run: () => Promise<unknown>): Promise<void> {
    uiAct = kind;
    try {
      await run();
    } finally {
      uiAct = null;
    }
  }
  function toggleUiControl(): void {
    void uiControl.setGrant(sessionId, !uiGranted);
  }
  /** What the agent asked for, in words: "“Run query” in Connections". */
  const uiAskWhat = $derived(
    uiPrompt
      ? `“${commandLabel(uiPrompt.command)}”${uiPrompt.module && uiPrompt.module !== 'shell' ? ` in ${moduleLabel(uiPrompt.module)}` : ''}`
      : '',
  );

  // A session pane is always its terminal: the old Terminal · Chat switch
  // (⌘⇧C) was removed in favour of Threads, which owns the structured chat.
  // Nothing reads the old `otto_session_view:<id>` choices, so a saved 'chat'
  // is harmless; main.ts sweeps them at boot (`transcript.sweepRetiredViewKeys`).
  // Find: the terminal's own bar (an xterm has no DOM text for the page find
  // to walk). ⌘F lands here too without a click: this pane's Terminal takes it
  // while active (findRank → keys.ts routeFind); the focused split pane
  // outranks another pane showing the same session.
  const findRank = $derived(focused ? 2 : ws.activeSessionId === sessionId ? 1 : 0);
  const findLabel = 'Find in terminal';
  function openSessionFind(): void {
    termRef?.openFind();
  }
  let dirsOpen = $state(false);
  let mcpOpen = $state(false); // ⋯ → MCP tools (the session's MCP profiles)
  const mcpToolsShown = $derived(isAgent && auth.can('mcp', 'view'));
  $effect(() => { if (mcpToolsShown) void mcpProfiles.load(); });

  function openDirs(): void {
    dirsOpen = true;
  }

  /** The size the terminal actually draws at: below the chosen size while the
   *  pane is too narrow for 80 columns (Terminal's column floor). The header
   *  shows it, so "13px" never labels 10px text. */

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
      toastError('Couldn’t rename the session', e);
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
      toastError('Couldn’t change keep-alive', e);
    }
  }

  async function archive(): Promise<void> {
    try {
      await ws.requestArchive(sessionId);
    } catch (e) {
      toastError('Couldn’t archive the session', e);
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
      toastError('Couldn’t detach the session', e);
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
   *  kinds (connections) have no right panel, so open Design Hall's Whiteboard
   *  studio (where Canvas boards live now) instead. */
  function openCanvas(): void {
    if (isAgent) {
      ui.openRight('canvas');
    } else {
      router.go('design/studio/whiteboard');
    }
  }

  // Per-session tokens + cost: read only when someone looks (the details
  // ⋯ › Details, openPaneMenu), at most once a minute (store-throttled). A turn that
  // ENDS is when the totals move: it marks them stale, so the next look reads
  // fresh numbers. Reading on every pane focus, every turn end and a 5-minute
  // working tick cost 402 reads in 30 h that nobody saw (perf A7a).
  const canUsage = $derived(auth.can('usage', 'view'));
  const usageLabel = $derived(canUsage ? sessionUsageLabel(sessionUsage.bySession[sessionId]) : null);
  let usagePrevStatus: SessionStatus | null = null;
  $effect(() => {
    const st = status;
    const prev = usagePrevStatus;
    usagePrevStatus = st;
    if (prev === 'working' && st !== 'working') untrack(() => sessionUsage.invalidate(sessionId));
  });

  /** Why + when a dormant session was suspended (A4) — "Suspended 3h ago ·
   *  idle too long"; only while it IS suspended (a resume clears the stamp,
   *  but a stale meta copy must never relabel a live pane). Minute clock. */
  const suspendNote = $derived(
    paneState.key === 'suspended' ? suspendedNote(session?.meta, nowMinute(), (ms) => rel(ms)) : null,
  );

  /** What the header no longer shows inline — the details chip's tooltip and
   *  menu, and the ⋯ info rows once the chip itself is folded away. */
  const detailRows = $derived(
    paneDetails({
      provider: session?.provider,
      nameFull: nameFull && nameFull !== session?.title ? nameFull : null,
      account: typeof session?.meta?.account_label === 'string' ? session.meta.account_label : null,
      state: suspendNote ?? (paneState.key === 'needs-you' || paneState.key === 'suspended' || paneState.key === 'stale' ? paneState.label : null),
      usage: usageLabel,
      idle: idleHint?.label,
      tasks: summary && summary.total > 0 ? { done: summary.done, total: summary.total } : null,
      now: summary?.in_progress,
      handoverFrom: handoverFromId ? (handoverFrom?.title ?? 'another session') : null,
      handoverPending,
      cwd: session?.cwd,
    }),
  );

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
            ok ? toasts.info('Folder path copied') : toasts.error('Couldn’t copy the folder path', 'The clipboard isn’t available here. Select the path and copy it manually.'),
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

  // ⌘K "Redraw terminal" for the pane the keyboard is in (same action as the
  // ⋯ menu row above).
  $effect(() => {
    if (!kbFocused || viewport.isPhone) return;
    return registry.register('session-terminal', [
      {
        id: 'terminal.redraw',
        title: 'Redraw terminal',
        group: 'Session',
        keywords: 'garbled repaint refresh screen broken tui',
        run: () => termRef?.redraw(),
      },
    ]);
  });

  /** The pane ⋯ (and the title's right-click), grouped: Session · Share ·
   *  Attach · the details · layout · Danger, then "Terminal settings…" —
   *  the terminal font and copy-on-select live in Settings (⌘+ ⌘− ⌘0 still
   *  work in the terminal). It is also the overflow menu: whatever a narrow
   *  tier hides from the header (zoom, ✕) comes back as a row, so nothing is
   *  unreachable — a 100 px pane is a word, a title and ⋯. */
  function sessionMenuItems(): MenuItem[] {
    const sep = { separator: true } as MenuItem;
    const session_: MenuItem[] = [
      { label: findLabel, icon: 'search', hint: '⌘F', action: openSessionFind },
      ...(foldedMinimal && chatOk ? [{ label: view === 'chat' ? 'Show the terminal' : 'Show the chat', icon: view === 'chat' ? 'terminal' : 'threads', action: () => setView(view === 'chat' ? 'terminal' : 'chat') } as MenuItem] : []),
      ...(foldedMinimal && showZoom
        ? [{ label: maximized ? 'Restore tiled view' : 'Zoom in on this session', icon: maximized ? 'minimize' : 'maximize', action: () => ws.toggleMaximize(sessionId) } as MenuItem]
        : []),
      ...(readOnly ? [] : [{ label: 'Rename', icon: 'edit', action: startRename } as MenuItem]),
      ...(viewport.isPhone ? [] : [{ label: 'Redraw terminal', icon: 'refresh', title: 'Rebuild the screen from the session — fixes a garbled or half-drawn pane', action: () => termRef?.redraw() } as MenuItem]),
      ...(!readOnly && isAgent ? [{ label: 'Restart session', icon: 'refresh', title: status === 'working' ? 'Asks first — the agent is working' : undefined, action: () => void restart() } as MenuItem] : []),
      ...(isAgent ? [{ label: 'Copy transcript (Markdown)', icon: 'copy', action: () => void import('./conversation/exportMarkdown').then((m) => m.copySessionTranscriptMarkdown(sessionId, { title: session?.title, provider: session?.provider, cwd: session?.cwd })) } as MenuItem] : []),
      ...(readOnly
        ? []
        : [
            ...(isAgent ? [{ label: 'Additional directories…', icon: 'folder', action: openDirs } as MenuItem] : []),
            ...(mcpToolsShown ? [{ label: 'MCP tools…', icon: 'plug', hint: mcpProfiles.labelFor(sessionProfileIds(session?.meta)), title: 'Which Otto MCP tools this agent sees — applies when it restarts', action: () => (mcpOpen = true) } as MenuItem] : []),
            ...(auth.can('connections', 'view') && !networkProfileId ? [{ label: netOpen ? 'Hide network profile' : 'Network profile…', icon: 'globe', action: () => (netOpen = !netOpen) } as MenuItem] : []),
            ...(isAgent && uiToggleShown ? [{ label: 'Allow UI control', icon: 'cursor', checked: uiGranted, disabled: uiBusy, title: uiToggleTitle, action: toggleUiControl } as MenuItem] : []),
            ...(isAgent ? [{ label: keepAlive ? 'Unpin (allow auto-suspend)' : 'Pin (keep alive)', icon: 'pin', action: () => void toggleKeepAlive() } as MenuItem] : []),
          ]),
    ];
    const share: MenuItem[] = [
      ...(readOnly ? [] : [{ label: 'Share…', icon: 'share', action: () => void openShare() } as MenuItem]),
      ...(!readOnly && isAgent ? [{ label: 'Hand over to…', icon: 'send', action: openHandover } as MenuItem] : []),
      ...(!readOnly && session?.kind === 'agent' ? [{ label: 'Start room…', icon: 'people', action: () => (roomOpen = true) } as MenuItem] : []),
      ...(readOnly ? [] : sessionHandoffItems(session)),
      ...popoutItems(`agents/${sessionId}`, session?.title),
    ];
    const attach: MenuItem[] = readOnly
      ? []
      : [
          { label: attachedIssue ? 'Change issue…' : 'Attach issue…', icon: 'ticket', action: openAttachIssue },
          ...(attachedIssue ? [{ label: 'Detach issue', icon: 'link', action: detachIssue } as MenuItem] : []),
          { label: 'Attach product story…', icon: 'file', action: openAttachProductStory },
          { label: 'Canvas…', icon: 'shapes', action: openCanvas },
        ];
    const danger: MenuItem[] = [
      // Same words as the header ✕ it replaces: in a split that closes the
      // SESSION (archive/delete per Settings), not merely the pane.
      ...(showClose && foldedMinimal ? [{ label: closeTitle.split(' (')[0], icon: 'x', action: onclosepane } as MenuItem] : []),
      ...(readOnly ? [] : [{ label: 'Archive', icon: 'archive', action: () => void archive() } as MenuItem, { label: 'Delete…', icon: 'trash', danger: true, action: () => void del() } as MenuItem]),
    ];
    // The layout presets only with a split to re-arrange (shared with the DB pane's ✕).
    const presets: MenuItem[] = showClose ? presetItems() : [];
    const groups = [session_, share, attach, [{ label: 'Details', icon: 'info', children: detailItems() } as MenuItem], presets, danger].filter((g) => g.length > 0);
    return [
      ...groups.flatMap((g, i) => (i === 0 ? g : [sep, ...g])),
      ...(viewport.isPhone ? [] : [sep, { label: 'Terminal settings…', icon: 'gear', title: 'Font, size and copy on select (⌘+ ⌘− ⌘0 in the terminal)', action: () => router.go('settings/appearance') } as MenuItem]),
    ];
  }
  function openPaneMenu(e: MouseEvent | KeyboardEvent): void {
    // Usage shows under Details: read it now (once a minute at most, A7a).
    if (isAgent && canUsage) void sessionUsage.loadSession(sessionId);
    ctxMenu.show(e, sessionMenuItems());
  }
  function attendPane(node: HTMLElement): { destroy(): void } {
    const onDown = () => { ws.clearNeedsYou(sessionId); onfocus(); };
    node.addEventListener('mousedown', onDown);
    return { destroy: () => node.removeEventListener('mousedown', onDown) };
  }
</script>

<section
  class="pane"
  class:focused
  class:current={kbFocused}
  use:attendPane
>
  <!-- ONE row (A3): status word · title · folder or branch (dim) ·
       [Terminal | Chat] · ⋯, adapted to the PANE width by the `@container
       pane` rules below (full ≥720 · compact 420–719 · minimal 200–419 ·
       micro <200). The word, the title and ⋯ never go; everything else is in
       ⋯. `data-tier` mirrors the script's twin of the breakpoints for tests. -->
  <header style:height={!viewport.isDesktop ? 'var(--mobile-topbar-h)' : undefined} class="pane-head" class:grip-on={gripOn} data-tier={tier} bind:clientWidth={headW}>
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
      >
        <Icon name="grip" size={12} />
      </button>
    {/if}
    <span class="pane-word {word.tone}" data-testid="pane-status" data-state={paneState.key} title={suspendNote ?? paneState.hint ?? paneState.label}>{word.word}</span>
    {#if renaming}
      <input dir="auto"
        class="rename-input"
        aria-label="Session name"
        bind:value={draftTitle}
        use:focusOnMount
        onblur={commitRename}
        onkeydown={(e) => {
          if (e.key === 'Enter') commitRename();
          else if (e.key === 'Escape') renaming = false;
        }}
        onmousedown={(e) => e.stopPropagation()}
      />
    {:else}
      <button use:rowMenu
        type="button"
        class="pane-title"
        class:draggable={gripOn}
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
      >{session?.title ?? sessionId}</button>
    {/if}
    {#if summary && summary.total > 0}
      <span
        class="task-chip"
        class:done={summary.done === summary.total}
        class:active={summary.in_progress != null}
        title={summary.in_progress ? `Now: ${summary.in_progress}` : `${summary.done}/${summary.total} tasks done`}
      >{summary.done}/{summary.total}</span>
    {/if}
    {#if handoverFromId}
      <button
        class="handover-crumb"
        title="Handed over from “{handoverFrom?.title ?? 'another session'}” — open it"
        onmousedown={(e) => e.stopPropagation()}
        onclick={() => ws.navigateToSession(handoverFromId)}
        aria-label="Open handover source: {handoverFrom?.title ?? 'source'}"
      ><Icon name="undo" size={12} /><span class="crumb-text head-lbl">{handoverFrom?.title ?? 'source'}</span></button>
    {/if}
    {#if handoverPending}
      <span class="handover-pending" title="Preparing the handover brief…"><Icon name="clock" size={12} /><span class="head-lbl">Preparing handover…</span></span>
    {/if}
    {#if where}<span class="pane-where mono" dir="ltr" title={session?.cwd ?? where}>{where}</span>{/if}
    <span class="grow"></span>
    {#if chatOk}
      <!-- Terminal ⇄ Chat (A2): the transcript view of this session. -->
      <div class="pane-view" role="group" aria-label="View" data-testid="pane-view">
        <button type="button" class:on={view === 'terminal'} aria-pressed={view === 'terminal'} onmousedown={(e) => e.stopPropagation()} onclick={() => setView('terminal')}>Terminal</button>
        <button type="button" class:on={view === 'chat'} aria-pressed={view === 'chat'} onmousedown={(e) => e.stopPropagation()} onclick={() => setView('chat')}>Chat</button>
      </div>
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
      title="More…"
      aria-label="More… — {session?.title ?? sessionId}"
      aria-haspopup="menu"
    ><Icon name="more" size={13} /></button>
    {#if showClose}<button class="icon-btn pane-close" onclick={onclosepane} title={closeTitle} aria-label={closeTitle}><Icon name="x" size={12} /></button>{/if}
  </header>
  <!-- Network strip only with a profile (or asked via ⋯ → Network profile…). -->
  {#if session && auth.can('connections', 'view') && (networkProfileId || netOpen)}
    {#key sessionId}<SessionNetworkStatus defaultOpen={netOpen && !networkProfileId} {sessionId} workspaceId={session.workspace_id} selectedProfileId={typeof session.meta?.network_profile_id === 'string' ? session.meta.network_profile_id : ''} editable={!readOnly} manageEditable={!readOnly && auth.can('connections', 'edit')} onchange={async (id) => { await ws.updateSessionMeta(sessionId, { network_profile_id: id || null }); }} />{/key}
  {/if}
  {#if uiPrompt}
    <!-- An agent called an `otto.ui_*` tool without the grant. Asked once per
         session: Deny is remembered on this device (the toggle then only
         carries a mark), Allow lasts for the session. -->
    <div class="ui-ask" role="group" aria-label="UI control request" data-testid="ui-control-request">
      <span class="ui-ask-mark" aria-hidden="true"><Icon name="cursor" size={13} /></span>
      <span class="ui-ask-text" use:announceShown={{ text: `${agentWho}${session?.title ? ` · ${session.title}` : ''} wants to drive Otto — ${uiAskWhat}.` }}>
        <strong>{agentWho}{session?.title ? ` · ${session.title}` : ''} wants to drive Otto</strong> — {uiAskWhat}. Everything it does shows beside this session, and writes still ask you.
      </span>
      <span class="ui-ask-actions">
        <!-- The shared approval shape (patterns §5). Deny is remembered on this
             device with nowhere to record a reason, so it fires at once. -->
        <ApprovalActions
          approveLabel="Allow for this session"
          approveBusyLabel="Allowing…"
          askReason={false}
          busy={uiAct}
          disabled={uiBusy && uiAct === null}
          onapprove={() => runUiAct('approve', () => uiControl.allow(sessionId))}
          ondeny={() => runUiAct('deny', () => uiControl.deny(sessionId))}
          testid="ui-control-actions"
        />
      </span>
    </div>
  {/if}
  {#if session?.meta?.handover}<HandoverDeliveryPanel {session} readonly={readOnly} />{/if}
  <div class="pane-body" data-view={view}>
    {#if view === 'chat' && session}
      <LazyMount lazy={ChatView} what="the conversation" variant="panel" props={{ sessionId, workspaceId: session.workspace_id, surface: 'agents', readonly: readOnly, ontoggleterminal: () => setView('terminal') }} />
    {:else}
    <div class="pane-term">
      <Terminal bind:this={termRef} {sessionId} {readOnly} {resumable} restartable={isAgent} onrestart={restart} restartNonce={ws.restartNonces[sessionId] ?? 0} onstatus={onTermStatus} showToolbar={false} autoFocus={kbFocused} {findRank} preferDom={isAgent} claimOnAttach={!readOnly} keepAlive={true} {scrollback} {resumeOnOpen} />
    </div>
    {/if}
  </div>
</section>

<!-- The rarely used dialogs are their own chunks, loaded when opened (perf A7c). -->
{#if attachIssueOpen}<LazyMount lazy={AttachIssueView} what="the issue picker" quiet onerror={() => (attachIssueOpen = false)} props={{ sessionId, onclose: () => (attachIssueOpen = false) }} />{/if}
{#if attachProductOpen}<LazyMount lazy={AttachStoryView} what="the story picker" quiet onerror={() => (attachProductOpen = false)} props={{ sessionId, onclose: () => (attachProductOpen = false) }} />{/if}
{#if handoverOpen}<LazyMount lazy={HandoverView} what="the handover" quiet onerror={() => (handoverOpen = false)} props={{ sessionId, onclose: () => (handoverOpen = false) }} />{/if}
{#if roomOpen}<LazyMount lazy={StartRoomView} what="the room" quiet onerror={() => (roomOpen = false)} props={{ sessionId, onclose: () => (roomOpen = false) }} />{/if}
{#if shareOpen && ShareModal}
  <ShareModal {sessionId} onclose={() => (shareOpen = false)} />
{/if}

{#if mcpOpen && session}<LazyMount lazy={McpProfilesView} what="MCP tools" quiet onerror={() => (mcpOpen = false)} props={{ sessionId, meta: session.meta, onclose: () => (mcpOpen = false) }} />{/if}
{#if dirsOpen}<LazyMount lazy={ExtraDirsView} what="additional directories" quiet onerror={() => (dirsOpen = false)} props={{ sessionId, session, onclose: () => (dirsOpen = false) }} />{/if}

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
    transition: border-color var(--dur-fast) ease-out;
    /* The header adapts to the PANE's inline size (the `@container pane` rules
       at the end). The query container is the pane, not the header: a
       container can't query itself. Inline-size containment means the pane has
       no intrinsic width — hosts that shrink-wrap it must make it fill
       (SwarmPage `.session-panel > .pane`). */
    container: pane / inline-size;
  }
  .pane.focused {
    border-color: var(--accent-line-strong);
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
    flex: 0 1 auto; min-width: 0; border: 0; background: none; font: inherit;
    font-size: var(--fs-s); font-weight: 600; color: var(--text); text-align: start;
    white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
    border-radius: var(--radius-s); padding-block: 0; padding-inline: 2px;
    cursor: default; transition: color var(--dur-fast) ease-out;
  }
  .pane-title.draggable {
    cursor: grab;
  }
  .pane-title:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 1px;
  }
  /* The active pane reads at full contrast; the others step back (dim title,
     quieter controls until hovered) — distinguishable without extra chrome. */
  .pane:not(.current) .pane-title {
    color: var(--text-dim);
    font-weight: 500;
  }
  .pane:not(.current) .pane-head > :is(.ui-ctl, .pane-view, .pane-zoom, .pane-more, .pane-close) {
    opacity: 0.7;
    transition: opacity var(--dur-fast) ease-out;
  }
  .pane:not(.current) .pane-head:is(:hover, :focus-within) > :is(.ui-ctl, .pane-view, .pane-zoom, .pane-more, .pane-close) {
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
    transition: opacity var(--dur-fast) ease-out;
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
  .rename-input:focus {
    border-color: var(--accent-text);
    box-shadow: 0 0 0 3px var(--accent-soft-strong);
  }
  .grow {
    flex: 1 1 0;
    min-width: 0;
  }
  /* The status WORD (lib/statusWords.ts): calm, toned only when it needs you,
     failed or finished unread — never a bare dot. */
  .pane-word {
    flex-shrink: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .pane-word.warning,
  .pane-word.danger,
  .pane-word.success {
    font-weight: 600;
  }
  .pane-word.warning {
    color: var(--warning);
  }
  .pane-word.danger {
    color: var(--danger);
  }
  .pane-word.success {
    color: var(--success);
  }
  /* Folder or branch, dim after the title; it gives way first. */
  .pane-where {
    flex: 0 1000 auto;
    min-width: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  /* Terminal | Chat. */
  .pane-view {
    flex-shrink: 0;
    display: inline-flex;
    padding: 1px;
    border-radius: var(--radius-s);
    background: var(--surface-2);
  }
  .pane-view button {
    block-size: 20px;
    padding-inline: var(--sp-3);
    border: 0;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .pane-view button.on {
    background: var(--bg);
    color: var(--text);
    box-shadow: var(--shadow-xs);
  }
  /* Per-session task roll-up "done/total" — matches the sidebar chip. */
  .task-chip {
    flex-shrink: 0;
    padding: 0 4px;
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
    background: var(--accent-soft);
  }
  .task-chip.done {
    color: var(--success);
    background: var(--success-soft);
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
    border-radius: 999px;
    cursor: pointer;
  }
  .crumb-text {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .handover-crumb:hover {
    color: var(--text);
    border-color: var(--accent-line-strong);
  }
  .handover-pending {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    flex-shrink: 0;
    height: 18px;
    font-size: var(--fs-xs);
    color: var(--accent-text);
    background: var(--accent-soft);
    padding: 0 6px;
    border-radius: 999px;
    white-space: nowrap;
  }
  .pane-head > :is(.ui-ctl, .pane-zoom, .pane-more, .pane-close) {
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
  /* Additional directories editor (mirrors New Session). */

  /* ── Adaptive pane header ────────────────────────────────────────────────
     Container queries on the PANE (`container: pane`). Tiers, widest first —
     keep in step with PANE_TIER_MIN in lib/paneHeader.ts, whose script twin
     adds the hidden controls back into ⋯:
       full    ≥720  word · title · folder · Terminal|Chat · ⋯
       compact 420–719  the same, chip labels shortened
       minimal 200–419  word · title · ⋯   (folder, view, zoom, ✕ → ⋯)
       micro   <200     word · title · ⋯   (tighter padding)
     The word, the title and ⋯ never go. */
  @container pane (width < 720px) {
    .pane-head .head-lbl {
      display: none;
    }
    .pane-head .handover-pending,
    .pane-head .handover-crumb {
      padding: 0 4px;
    }
  }
  @container pane (width < 420px) {
    .pane-head .pane-where,
    .pane-head .pane-view,
    .pane-head .task-chip,
    .pane-head .handover-crumb,
    .pane-head .handover-pending,
    .pane-head .ui-ctl,
    .pane-head .pane-zoom,
    .pane-head .pane-close {
      display: none;
    }
    .pane-head {
      gap: 4px;
    }
  }
  @container pane (width < 200px) {
    .pane-head {
      padding-inline: 6px 2px;
    }
    .pane-head.grip-on {
      padding-inline-start: 14px;
    }
  }
</style>
