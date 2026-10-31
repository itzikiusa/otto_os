///<reference types="svelte" />
;
import { rowMenu } from '../../lib/rowMenu';
import PathField from '../../lib/components/PathField.svelte';
import { focusOnMount } from '../../lib/focusOnMount';
import { toastError } from '../../lib/toastError';
import { loadErrorText } from '../../lib/loadError';
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
import SessionMcpProfiles from './SessionMcpProfiles.svelte';
import { mcpProfiles } from '../../lib/stores/mcp-profiles.svelte';
import { sessionProfileIds } from '../../lib/mcpProfiles';
import ProviderIcon, { hasProviderIcon } from '../../lib/components/ProviderIcon.svelte';
import Modal from '../../lib/components/Modal.svelte';
import AttachIssue from './AttachIssue.svelte';
import AttachProductStory from './AttachProductStory.svelte';
import Handover from './Handover.svelte';
import HandoverDeliveryPanel from './HandoverDeliveryPanel.svelte';
import { ws, isForeground } from '../../lib/stores/workspace.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { activity } from '../../lib/stores/activity.svelte';
import { toasts } from '../../lib/toast.svelte';
import { copyText } from '../../lib/clipboard';
import StartRoomModal from '../rooms/StartRoomModal.svelte';
import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
import { registry } from '../../lib/commands.svelte';
import { popoutItems } from '../../lib/popoutMenu';
import { now, nowMinute, rel } from '../../lib/stores/now.svelte';
import { sessionUsage } from '../../lib/stores/sessionUsage.svelte';
import { pollWhileVisible } from '../../lib/poll';
import { sessionUsageLabel } from '../../lib/sessionUsage';
import { suspendedNote } from '../../lib/status';
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
import ApprovalActions from '../../lib/components/ApprovalActions.svelte';
import { onTabKey } from '../../lib/tabKeys';
;

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
    /** Opening the pane resumes a suspended session in Terminal or Chat.
     *  The tiled grid passes false: a tile shows a session, it does not ask
     *  for its CLI back — typing into it or Resume does. Default true. */
    resumeOnOpen?: boolean;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

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
  // Loaded on first use: ShareModal bundles the QR encoder (qrcode), which
  // the Agents chunk — the default landing page — otherwise carries.
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
  /** The shared tablist keys, then focus re-lands on the selected tab: the
   *  terminal view grabs focus when it mounts, which would strand the user. */
  function onViewTabKey(e: KeyboardEvent): void {
    onTabKey(e);
    if (!e.defaultPrevented) return;
    const list = e.currentTarget as HTMLElement;
    queueMicrotask(() => list.querySelector<HTMLElement>(`[data-view="${view}"]`)?.focus());
  }
  $effect(() => {
    if (!isAgent) return;
    const onKey = (e: KeyboardEvent): void => {
      // A split starts with the SAME session in both panes, so two listeners
      // pass the active-session guard; the first one handles it (and
      // preventDefaults) — a second flip would undo it.
      if (e.defaultPrevented) return;
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
  let mcpOpen = $state(false); // ⋯ → MCP tools (the session's MCP profiles)
  const mcpToolsShown = $derived(isAgent && auth.can('mcp', 'view'));
  $effect(() => { if (mcpToolsShown) void mcpProfiles.load(); });
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
    // Same working-guard as every other restart path (ws.requestRestart).
    if (alsoRestart && !(await ws.confirmRestart(sessionId))) return;
    dirsBusy = true;
    try {
      try {
        await ws.updateSessionMeta(sessionId, { extra_dirs: collectDirs() });
      } catch (e) {
        // Nothing was saved: keep the dialog open so the edit isn't lost.
        toastError('Couldn’t save the session', e);
        return;
      }
      // The directories ARE saved from here on: close the dialog (a second
      // Save would only repeat it), and report a failed restart as exactly that.
      dirsOpen = false;
      if (!alsoRestart) {
        toasts.success('Directories saved', 'Applies the next time this session restarts.');
        return;
      }
      try {
        await ws.restartSession(sessionId, { quiet: true });
        toasts.success('Directories saved', 'Session restarted with the new directories.');
      } catch (e) {
        toasts.push('error', 'Directories saved, but the session didn’t restart', loadErrorText(e), 12000, {
          action: { label: 'Restart', run: () => ws.requestRestart(sessionId) },
        });
      }
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
   *  kinds (connections) have no right panel, so navigate to the Canvas
   *  module directly instead. */
  function openCanvas(): void {
    if (isAgent) {
      ui.openRight('canvas');
    } else {
      router.go('canvas');
    }
  }

  // Per-session tokens + cost (A5), event-driven (perf R5): loaded when the
  // pane gains focus (the store throttles to once a minute), again when a
  // turn ENDS (working → idle/exited — that is when the totals move), and
  // whenever the details open. A slow safety tick runs only while the agent
  // is working; an idle focused pane used to re-fetch every 60 s regardless.
  const canUsage = $derived(auth.can('usage', 'view'));
  const usageLabel = $derived(canUsage ? sessionUsageLabel(sessionUsage.bySession[sessionId]) : null);
  const usageLive = $derived(isAgent && focused && canUsage);
  $effect(() => {
    if (!usageLive) return;
    const id = sessionId;
    untrack(() => void sessionUsage.loadSession(id));
  });
  let usagePrevStatus: SessionStatus | null = null;
  $effect(() => {
    const st = status;
    const prev = usagePrevStatus;
    usagePrevStatus = st;
    if (usageLive && prev === 'working' && (st === 'idle' || st === 'exited')) {
      const id = sessionId;
      untrack(() => void sessionUsage.loadSession(id, true));
    }
  });
  $effect(() => {
    if (!usageLive || status !== 'working') return;
    const id = sessionId;
    const p = pollWhileVisible(() => sessionUsage.loadSession(id), { ms: 300_000, immediate: false });
    return () => p.stop();
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
  function openDetails(e: MouseEvent | KeyboardEvent): void {
    if (isAgent && canUsage) void sessionUsage.loadSession(sessionId);
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
      {
        label: 'Redraw terminal',
        icon: 'refresh',
        title: 'Rebuild the screen from the session — fixes a garbled or half-drawn pane',
        action: () => termRef?.redraw(),
      },
    ];
  }

  // ⌘K "Redraw terminal" for the pane the keyboard is in (same action as the
  // ⋯ menu row above).
  $effect(() => {
    if (!kbFocused || view !== 'terminal' || viewport.isPhone) return;
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
            ...(mcpToolsShown ? [{ label: 'MCP tools…', icon: 'plug', hint: mcpProfiles.labelFor(sessionProfileIds(session?.meta)), title: 'Which Otto MCP tools this agent sees — applies when it restarts', action: () => (mcpOpen = true) } as MenuItem] : []),
            ...(isAgent ? [{ label: 'Hand over to…', icon: 'send', action: openHandover } as MenuItem] : []),
            // Parity with the tab's right-click menu — a tiled/split pane has no
            // tab to right-click, so Share was unreachable from here.
            ...((session?.kind === 'agent') ? [{ label: 'Start room…', icon: 'people', action: () => (roomOpen = true) } as MenuItem] : []),
            { label: 'Share…', icon: 'share', action: () => void openShare() } as MenuItem,
            { separator: true } as MenuItem,
            {
              label: attachedIssue ? 'Change issue…' : 'Attach issue…',
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
                  ...(uiToggleShown ? [{ label: 'Allow UI control', icon: 'cursor', checked: uiGranted, disabled: uiBusy, title: uiToggleTitle, action: toggleUiControl } as MenuItem] : []),
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
            { label: 'Delete…', icon: 'trash', danger: true, action: () => void del() } as MenuItem,
          ]),
    ];
  }
  function openPaneMenu(e: MouseEvent | KeyboardEvent): void {
    ctxMenu.show(e, sessionMenuItems());
  }
;
async () => {


  { svelteHTML.createElement("section", {      "class":`pane`,"onmousedown":() => {
    // Interacting with the pane attends to it — drop the "needs you" flag.
    ws.clearNeedsYou(sessionId);
    onfocus();
  },});focused;kbFocused;
  
   { const $$_header1 = svelteHTML.createElement("header", {     "class":`pane-head`,...__sveltets_2_empty({"data-tier":tier}),});gripOn;headW= $$_header1.clientWidth;
    if(gripOn){
      
       { svelteHTML.createElement("button", {                     "class":`icon-btn pane-grip`,"draggable":`true`,"title":`Drag to move or swap this pane`,"aria-label":`Move pane`,...__sveltets_2_empty({"data-testid":`pane-grip`}),"onmousedown":(e) => e.stopPropagation(),"ondragstart":onGripDragStart,"ondragend":onGripDragEnd,"onclick":openPaneMenu,"onkeydown":(e) => (e.key === 'Enter' || e.key === ' ') && openPaneMenu(e),});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`grip`,"size":12,}});}
       }
    }
     { const $$_toDsutatS2C = __sveltets_2_ensureComponent(StatusDot); new $$_toDsutatS2C({ target: __sveltets_2_any(), props: {  "state":paneState,}});}
    if(renaming){
       {const $$action_0 = __sveltets_2_ensureAction(focusOnMount(svelteHTML.mapElementTag('input')));{ svelteHTML.createElement("input", __sveltets_2_union($$action_0), {               "dir":`auto`,"class":`rename-input`,"aria-label":`Session name`,"bind:value":draftTitle,"onblur":commitRename,"onkeydown":(e) => {
          if (e.key === 'Enter') commitRename();
          else if (e.key === 'Escape') renaming = false;
        },"onmousedown":(e) => e.stopPropagation(),});/*Ωignore_startΩ*/() => draftTitle = __sveltets_2_any(null);/*Ωignore_endΩ*/}}
    }else{
       {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('span')));{ svelteHTML.createElement("span", __sveltets_2_union($$action_0), {                       "class":`pane-title`,"role":`button`,"tabindex":0,"draggable":gripOn ? 'true' : undefined,"title":`${session?.title ?? sessionId}${nameFull && nameFull !== session?.title ? ` (${nameFull})` : ''} — double-click to rename, right-click for options`,"ondragstart":gripOn ? onGripDragStart : undefined,"ondragend":gripOn ? onGripDragEnd : undefined,"ondblclick":startRename,"oncontextmenu":(e) => openPaneMenu(e),"onkeydown":(e) => {
          if (e.key === 'F2' || e.key === 'Enter') {
            e.preventDefault();
            startRename();
          } else if (e.key === 'ContextMenu' || (e.key === 'F10' && e.shiftKey)) {
            e.preventDefault();
            openPaneMenu(e);
          }
        },});gripOn;session?.title ?? sessionId; }}
    }
    if(paneState.key === 'needs-you'){
       { svelteHTML.createElement("span", {   "class":`needs-you-badge`,"title":`This session is waiting on you (input or a permission)`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`bell`,"size":12,}});} { svelteHTML.createElement("span", { "class":`head-lbl`,});  }
       }
    } else if (paneState.key === 'suspended' || paneState.key === 'stale'){
       { svelteHTML.createElement("span", {   "class":`state-note`,"title":suspendNote ? `${suspendNote} — resumes when you open it` : paneState.hint,});paneState.label; }
    }
    if(summary && summary.total > 0){
       { svelteHTML.createElement("span", {       "class":`task-chip`,"title":summary.in_progress ? `Now: ${summary.in_progress}` : `${summary.done}/${summary.total} tasks done`,});summary.done === summary.total;summary.in_progress != null;summary.done; summary.total; }
    }
    if(summary?.in_progress){
       { svelteHTML.createElement("span", {   "class":`now-task`,"title":`Current task: ${summary.in_progress}`,}); summary.in_progress; }
    }
    if(handoverFromId){
       { svelteHTML.createElement("button", {           "class":`handover-crumb`,"title":`Handed over from “${handoverFrom?.title ?? 'another session'}” — open it`,"onmousedown":(e) => e.stopPropagation(),"onclick":() => ws.navigateToSession(handoverFromId),"aria-label":`Open handover source: ${handoverFrom?.title ?? 'source'}`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`undo`,"size":12,}});} { svelteHTML.createElement("span", { "class":`crumb-text head-lbl`,});handoverFrom?.title ?? 'source'; } }
    }
    if(handoverPending){
       { svelteHTML.createElement("span", {   "class":`handover-pending`,"title":`Preparing the handover brief…`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`clock`,"size":12,}});} { svelteHTML.createElement("span", { "class":`head-lbl`,});  } }
    }
    
     { svelteHTML.createElement("button", {                "class":`meta-chip`,...__sveltets_2_empty({"data-testid":`pane-details`}),"title":detailTitle,"aria-label":`Session details: ${detailRows.map(([k, v]) => `${k} ${v}`).join(', ')}`,"aria-haspopup":`menu`,"onmousedown":(e) => e.stopPropagation(),"onclick":openDetails,});hasProviderIcon(session?.provider);
      if(hasProviderIcon(session?.provider)){
         { const $$_nocIredivorP3C = __sveltets_2_ensureComponent(ProviderIcon); new $$_nocIredivorP3C({ target: __sveltets_2_any(), props: {    "provider":session?.provider ?? '',"size":13,}});}
      }
       { svelteHTML.createElement("span", { "class":`meta-text`,});
         { svelteHTML.createElement("span", { "class":`provider-name`,});session?.provider ?? '?'; }
        if(idleHint){ { svelteHTML.createElement("span", { "class":`meta-extra meta-idle`,});idleHint.label; }}
        if(session?.cwd){ { svelteHTML.createElement("span", { "class":`meta-extra meta-cwd mono`,});cwdLabel(session.cwd); }}
       }
     }
     { svelteHTML.createElement("span", { "class":`grow`,}); }
    if(isAgent){
       { svelteHTML.createElement("div", {           "class":`segmented view-seg`,"role":`tablist`,"tabindex":-1,"aria-label":`Session view`,"onmousedown":(e) => e.stopPropagation(),"onkeydown":onViewTabKey,});
           for(let [m, label, icon] of __sveltets_2_ensureArray(VIEW_META)){m;
           { svelteHTML.createElement("button", {                  "role":`tab`,"aria-selected":view === m,"aria-label":label,"tabindex":view === m ? 0 : -1,...__sveltets_2_empty({"data-view":m}),"onclick":() => setView(m),"title":label,"aria-keyshortcuts":`Meta+Shift+C`,});view === m; { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":icon,"size":12,}});} { svelteHTML.createElement("span", { "class":`head-lbl`,});label; } }
        }
       }
      
       { svelteHTML.createElement("button", {              "class":`icon-btn view-flip`,...__sveltets_2_empty({"data-view-toggle":true}),"aria-label":`Switch to ${viewLabel(otherSessionView(view))} view`,"title":`Switch to ${viewLabel(otherSessionView(view))} view`,"aria-keyshortcuts":`Meta+Shift+C`,"onmousedown":(e) => e.stopPropagation(),"onclick":toggleView,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":otherSessionView(view) === 'chat' ? 'comment' : 'terminal',"size":13,}});} }
    }
    if(uiToggleInline){
       { svelteHTML.createElement("button", {                   "class":`icon-btn ui-ctl`,"onmousedown":(e) => e.stopPropagation(),"onclick":toggleUiControl,"disabled":uiBusy,"aria-pressed":uiGranted,"title":uiToggleTitle,"aria-label":`Allow UI control`,...__sveltets_2_empty({"data-testid":`ui-control-toggle`}),});uiGranted;uiAskedAgain; { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`cursor`,"size":13,}});} }
    }
     { svelteHTML.createElement("button", {               "class":`icon-btn pane-find`,...__sveltets_2_empty({"data-testid":`pane-find`}),"onmousedown":(e) => e.stopPropagation(),"onclick":openSessionFind,"title":findLabel,"aria-label":findLabel,"aria-keyshortcuts":`Meta+F`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`search`,"size":13,}});} }
    if(showZoom){
       { svelteHTML.createElement("button", {           "class":`icon-btn pane-zoom`,"onmousedown":(e) => e.stopPropagation(),"onclick":() => ws.toggleMaximize(sessionId),"title":maximized ? 'Restore tiled view' : 'Zoom in on this session',"aria-label":maximized ? 'Restore tiled view' : 'Zoom in on this session',});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":maximized ? 'minimize' : 'maximize',"size":13,}});}
       }
    }
    
     { svelteHTML.createElement("button", {               "class":`icon-btn pane-more`,"onmousedown":(e) => e.stopPropagation(),"onclick":openPaneMenu,"onkeydown":(e) => (e.key === 'Enter' || e.key === ' ') && openPaneMenu(e),"title":`More…`,"aria-label":`More… — ${session?.title ?? sessionId}`,"aria-haspopup":`menu`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":13,}});} }
    if(showClose){
       { svelteHTML.createElement("button", {       "class":`icon-btn pane-close`,"onclick":onclosepane,"title":closeTitle,"aria-label":closeTitle,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
    }
   }
  
  if(session && auth.can('connections', 'view') && (networkProfileId || netOpen)){
    sessionId; { { const $$_sutatSkrowteNnoisseS1C = __sveltets_2_ensureComponent(SessionNetworkStatus); new $$_sutatSkrowteNnoisseS1C({ target: __sveltets_2_any(), props: {             "defaultOpen":netOpen && !networkProfileId,sessionId,"workspaceId":session.workspace_id,"selectedProfileId":typeof session.meta?.network_profile_id === 'string' ? session.meta.network_profile_id : '',"editable":!readOnly,"manageEditable":!readOnly && auth.can('connections', 'edit'),"onchange":async (id) => { await ws.updateSessionMeta(sessionId, { network_profile_id: id || null }); },}});}}
  }
  if(uiPrompt){
    
     { svelteHTML.createElement("div", {       "class":`ui-ask`,"role":`group`,"aria-label":`UI control request`,...__sveltets_2_empty({"data-testid":`ui-control-request`}),});
       { svelteHTML.createElement("span", {   "class":`ui-ask-mark`,"aria-hidden":`true`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`cursor`,"size":13,}});} }
       { svelteHTML.createElement("span", {   "class":`ui-ask-text`,"aria-live":`polite`,});
         { svelteHTML.createElement("strong", {});agentWho;session?.title ? ` · ${session.title}` : '';     }  uiAskWhat;            
       }
       { svelteHTML.createElement("span", { "class":`ui-ask-actions`,});
        
         { const $$_snoitcAlavorppA3C = __sveltets_2_ensureComponent(ApprovalActions); new $$_snoitcAlavorppA3C({ target: __sveltets_2_any(), props: {                 "approveLabel":`Allow for this session`,"approveBusyLabel":`Allowing…`,"askReason":false,"busy":uiAct,"disabled":uiBusy && uiAct === null,"onapprove":() => runUiAct('approve', () => uiControl.allow(sessionId)),"ondeny":() => runUiAct('deny', () => uiControl.deny(sessionId)),"testid":`ui-control-actions`,}});}
       }
     }
  }
  if(session?.meta?.handover){ { const $$_lenaPyrevileDrevodnaH1C = __sveltets_2_ensureComponent(HandoverDeliveryPanel); new $$_lenaPyrevileDrevodnaH1C({ target: __sveltets_2_any(), props: {   session,"readonly":readOnly,}});}}
   { svelteHTML.createElement("div", {   "class":`pane-body`,...__sveltets_2_empty({"data-view":view}),});
    if(view === 'chat'){
       { svelteHTML.createElement("div", { "class":`pane-chat`,});
         { const $$_weiVnoitasrevnoC3C = __sveltets_2_ensureComponent(ConversationView); new $$_weiVnoitasrevnoC3C({ target: __sveltets_2_any(), props: {      sessionId,"workspaceId":session?.workspace_id ?? ws.currentId ?? '',"readonly":readOnly,resumeOnOpen,}});}
       }
    }else{
       { svelteHTML.createElement("div", { "class":`pane-term`,});
         { const $$_lanimreT3C = __sveltets_2_ensureComponent(Terminal); const $$_lanimreT3 = new $$_lanimreT3C({ target: __sveltets_2_any(), props: {                            sessionId,readOnly,resumable,"restartable":isAgent,"onrestart":restart,"restartNonce":ws.restartNonces[sessionId] ?? 0,"onstatus":onTermStatus,"onfontfit":(px) => (drawnFont = px),"showToolbar":false,"autoFocus":kbFocused,findRank,"preferDom":isAgent,"claimOnAttach":!readOnly,"keepAlive":true,scrollback,resumeOnOpen,}});termRef = $$_lanimreT3;}
       }
    }
   }
 }

if(attachIssueOpen){
   { const $$_eussIhcattA0C = __sveltets_2_ensureComponent(AttachIssue); new $$_eussIhcattA0C({ target: __sveltets_2_any(), props: {   sessionId,"onclose":() => (attachIssueOpen = false),}});}
}

if(attachProductOpen){
   { const $$_yrotStcudorPhcattA0C = __sveltets_2_ensureComponent(AttachProductStory); new $$_yrotStcudorPhcattA0C({ target: __sveltets_2_any(), props: {   sessionId,"onclose":() => (attachProductOpen = false),}});}
}

if(handoverOpen){
   { const $$_revodnaH0C = __sveltets_2_ensureComponent(Handover); new $$_revodnaH0C({ target: __sveltets_2_any(), props: {   sessionId,"onclose":() => (handoverOpen = false),}});}
}

if(roomOpen){ { const $$_ladoMmooRtratS0C = __sveltets_2_ensureComponent(StartRoomModal); new $$_ladoMmooRtratS0C({ target: __sveltets_2_any(), props: {   sessionId,"onclose":() => (roomOpen = false),}});}}
if(shareOpen && ShareModal){
   { const $$_ladoMerahS0C = __sveltets_2_ensureComponent(ShareModal); new $$_ladoMerahS0C({ target: __sveltets_2_any(), props: {   sessionId,"onclose":() => (shareOpen = false),}});}
}

if(mcpOpen && session){ { const $$_seliforPpcMnoisseS0C = __sveltets_2_ensureComponent(SessionMcpProfiles); new $$_seliforPpcMnoisseS0C({ target: __sveltets_2_any(), props: {     sessionId,"meta":session.meta,"onclose":() => (mcpOpen = false),}});}}
if(dirsOpen){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {   children:() => { return __sveltets_2_any(0); },"title":`Additional directories`,"onclose":() => (dirsOpen = false),footer:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("button", {   "class":`btn`,"onclick":() => (dirsOpen = false),});  }
       { svelteHTML.createElement("button", {     "class":`btn`,"disabled":dirsBusy,"onclick":() => saveDirs(false),});
        dirsBusy ? 'Saving…' : 'Save';
       }
       { svelteHTML.createElement("button", {     "class":`btn primary`,"disabled":dirsBusy,"onclick":() => saveDirs(true),});
        dirsBusy ? 'Saving…' : 'Save & restart';
       }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`sv-extra-dir`,});      { svelteHTML.createElement("span", { "class":`dim`,});    } }
      if(extraDirs.length > 0){
         { svelteHTML.createElement("ul", { "class":`dir-list`,});
             for(let dir of __sveltets_2_ensureArray(extraDirs)){dir;
             { svelteHTML.createElement("li", { "class":`dir-row`,});
               { svelteHTML.createElement("span", {   "class":`dir-path mono`,"title":dir,});dir; }
               { svelteHTML.createElement("button", {           "type":`button`,"class":`dir-remove`,"title":`Remove directory`,"aria-label":`Remove ${dir}`,"onclick":() => removeDir(dir),}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
             }
          }
         }
      }
       { svelteHTML.createElement("div", { "class":`dir-add`,});
         { const $$_dleiFhtaP3C = __sveltets_2_ensureComponent(PathField); const $$_dleiFhtaP3 = new $$_dleiFhtaP3C({ target: __sveltets_2_any(), props: {  children:() => { return __sveltets_2_any(0); },value:dirDraft,}});/*Ωignore_startΩ*/() => dirDraft = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_dleiFhtaP3.$$bindings = 'value'; { svelteHTML.createElement("input", {              "dir":`ltr`,"id":`sv-extra-dir`,"class":`input mono`,"bind:value":dirDraft,"spellcheck":`false`,"placeholder":`/absolute/path/to/repo`,"onkeydown":onDirKeydown,});/*Ωignore_startΩ*/() => dirDraft = __sveltets_2_any(null);/*Ωignore_endΩ*/} PathField}
         { svelteHTML.createElement("button", {       "type":`button`,"class":`btn`,"disabled":dirDraft.trim() === '',"onclick":addDir,});  }
       }
       { svelteHTML.createElement("span", { "class":`hint`,});   { svelteHTML.createElement("code", {});  }        }
     }

    
   Modal}
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SessionView__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SessionView__SvelteComponent_ = ReturnType<typeof SessionView__SvelteComponent_>;
/*Ωignore_endΩ*/export default SessionView__SvelteComponent_;