// Workspaces + sessions + tab/split state for the shell and Agent Mode.

import { api, getToken } from '../api/client';
import { plural } from '../plural';
import { listActiveWorkflowRuns } from '../api/workflows';
import { fetchWorkspace } from '../api/workspaces';
import { router } from '../router.svelte';
import type {
  ActiveWorkflowRun,
  AttachedIssue,
  CreateSessionReq,
  Id,
  OpenAgentSessionReq,
  OpenAgentSessionResp,
  OttoEvent,
  Session,
  SessionStatus,
  Workspace,
  WorkspaceWithRole,
} from '../api/types';
import { toasts } from '../toast.svelte';
import { confirmer } from '../confirm.svelte';
import { ui, clientId } from './ui.svelte';
import { winKey } from '../win';
import { lsGet, lsSet } from '../storage';
import { layout, type Axis } from './splitLayout.svelte';
import { MAX_PANES, LS_PANES } from './splitLayout';
import { isEmbedded } from '../desktop';
import { SCRATCH_WORKSPACE_ID, sessionVerbsApply } from './sessionScope';
import { applyStatusPatches, canDropExited, patchSessionIn, staleStatusIds, type StatusPatch } from './sessionPatch';
import { bucketSessions, idChunks, isForeground, isShownKind, shownListQuery } from './sessionBuckets';
import { whenIdle } from '../lazy-component.svelte';
import { toastError } from '../toastError';

/** Archived rows per "Load more" page (per scope: workspace / scratch). */
const ARCHIVED_PAGE = 100;
/** How long an exited background row stays in `sessions` before it is
 *  dropped (perf R2) — a draft dialog's embedded agent outlives its turn. */
const EXITED_BACKGROUND_GRACE_MS = 30_000;

// Layout state is per-WINDOW (multi-window): winKey() namespaces these by the
// window's label so two windows never clobber each other's workspace/tabs/view.
// The main window keeps the legacy unprefixed keys.
const LS_CURRENT = 'otto_workspace';

/** Boot-time shown-list requests for the saved workspace (perf G3). */
interface BootSessions {
  key: string;
  own: Promise<Session[]>;
  scratch: Promise<Session[]>;
}
const LS_TABS = 'otto_tabs_'; // + workspace id
// App-wide (deliberately NOT winKey-namespaced): whether the sidebar lists
// sessions from every workspace, grouped by workspace, instead of only the
// current one. Default ON — a session shouldn't vanish on a workspace switch.
const LS_ALL_WS = 'otto_nav_all_ws';

// The scratch ("No workspace") id lives in the rune-free `sessionScope.ts` so
// the pure scope rules there are unit-testable; re-exported for callers.
export { SCRATCH_WORKSPACE_ID };

// The background-source blacklist, `isForeground` and the one-pass sidebar
// bucketing live in the rune-free `sessionBuckets.ts` (unit-tested there).
export { isForeground };

/** Per-device session isolation (Settings → Appearance, opt-in): with it on, a
 *  FOREGROUND session is kept only on the device that started it
 *  (meta.client_id, stamped on create). Background engine sessions are always
 *  kept — no user-facing list shows them, and their owning panels (reviews,
 *  insights, assists…) must still find them. The ONE predicate every path that
 *  admits a session into the store goes through (initial load, the
 *  all-workspaces load, and `session_created` events), so a session started on
 *  another device can't slip in live and vanish again on the next refresh. */
export function visibleOnThisDevice(s: Session): boolean {
  if (!ui.sessionIsolation || !isForeground(s)) return true;
  return (s.meta as { client_id?: string } | null)?.client_id === clientId();
}

/** Sentinel tab/pane id for the docked DB Explorer (not a real session). Lets
 *  the DB Explorer live as a pane in the Agents split, beside an agent. */
export const DB_PANE_ID = '__db_explorer__';

/** What a close gesture resolved to. 'delete-deferred' = the remembered
 *  "Always delete": archive now, delete after the Undo toast expires. */
type CloseAction = 'close' | 'archive' | 'delete' | 'delete-deferred';
/** Undo window for a remembered "Always delete" close. */
const DELETE_UNDO_MS = 8000;

export type SplitAxis = Axis;

class WorkspaceStore {
  workspaces: WorkspaceWithRole[] = $state([]);
  currentId: Id | null = $state(null);
  /** The hidden scratch workspace (`GET /workspaces/scratch`) — null on a
   *  daemon without it. Its `root_path` is the daemon `$HOME`, the default cwd
   *  of a workspace-less session. */
  scratch: Workspace | null = $state(null);
  /** Sessions of the current workspace PLUS the scratch workspace's (always
   *  loaded, so workspace-less sessions work with zero workspaces). */
  sessions: Session[] = $state.raw([]);
  /** `sessions` by id — every per-row / per-pane lookup goes through
   *  {@link getSession} instead of an O(n) `.find`. Rows are immutable
   *  (`$state.raw`; every update replaces the array), so no deep proxies. */
  sessionById: Map<Id, Session> = $derived(new Map(this.sessions.map((s) => [s.id, s] as const)));
  /** The loaded row for `id`, or null. */
  getSession(id: Id | null | undefined): Session | null {
    return id ? (this.sessionById.get(id) ?? null) : null;
  }
  /** Every sidebar list in ONE pass over `sessions` (see `bucketSessions`). */
  private buckets = $derived(bucketSessions(this.sessions));
  /** Background sources a mounted panel asked to have loaded with the main
   *  list (e.g. the swarm page's `swarm` agents) — {@link includeSources}. */
  private extraSources = new Map<string, number>();
  /** Programmatic PTY input keyed by session id, with a bump counter so the
   *  Terminal applies each injection exactly once (e.g. DB rows → running agent). */
  injections: Record<Id, { text: string; n: number }> = $state({});
  sessionsLoading = $state(false);
  sessionsError = $state<string | null>(null);
  /** The first workspace-list load has picked the selection (or failed).
   *  Workspace-scoped boot loads (Home's Today) wait for it instead of
   *  loading once unscoped and again when the saved workspace resolves. */
  listSettled = $state(false);
  private selectionGeneration = 0;
  private loadGeneration = 0;
  private loadedToken: string | null | undefined;
  private sessionsGeneration = 0;
  private sessionsInFlight: Promise<void> | null = null;
  /** `${selectionGeneration}:${currentId}` of the in-flight load, and the one
   *  trailing refresh queued behind it (single-flight, F7). */
  private sessionsInFlightKey = '';
  private sessionsTrailing: Promise<void> | null = null;
  /** In-flight fetch-by-id per session id ({@link ensureSession}). */
  private ensuring = new Map<Id, Promise<Session | null>>();
  /** The saved workspace's shown-list requests, started WITH `/workspaces`
   *  on boot (perf G3) and taken once by the first {@link loadSessions} whose
   *  workspace + query + token match; dropped when the saved id isn't the one
   *  selected. Keyed `${token}|${wsId}|${query}`. */
  private bootSessions: BootSessions | null = null;
  /** `session_status` row stamps queued for the next frame (perf R6): a burst
   *  of N events is ONE `sessions` write, not N id-map + bucket rebuilds.
   *  `statusMap` is still written per event (dots / badges stay live). */
  private pendingStatus = new Map<Id, StatusPatch>();
  private statusFlushRaf: number | null = null;
  private statusFlushTimer: ReturnType<typeof setTimeout> | null = null;

  /** In-flight workflow runs (pending|running) in the current workspace, for the
   *  "Running" sidebar list + the Workflows nav count chip. Refreshed on each
   *  `workflow_run_updated` WS event and on workspace switch. */
  activeWorkflowRuns: ActiveWorkflowRun[] = $state([]);

  /** view mode for Agent Mode: tabbed (one at a time), tiled (grid), or the
   *  Mission Control work-queue surface. */
  viewMode: 'tabs' | 'tiled' | 'mission' = $state(
    (lsGet(winKey('otto_view_mode')) as 'tabs' | 'tiled' | 'mission') ?? 'tabs',
  );

  /** In tiled view, a session id to show maximized (zoomed) on its own. */
  maximizedId: Id | null = $state(null);

  /** open session tabs (ids), in tab-bar order */
  openTabs: Id[] = $state([]);
  /** Split panes — projections of the layout tree (leaf order; duplicates legal).
   *  Mutate via `layout` or the methods below. */
  get panes(): Id[] {
    return layout.panes;
  }
  get focusedPane(): number {
    return layout.focusedIndex;
  }
  set focusedPane(i: number) {
    layout.focusIndex(i);
  }
  get splitAxis(): SplitAxis {
    return layout.axis;
  }

  /** global session-status map (fed by loads + events WS) */
  statusMap: Record<Id, SessionStatus> = $state({});

  /** Sticky "needs you" flags: a session raised a Notification/blocked hook and
   *  is waiting on the operator (a permission or input it couldn't auto-accept).
   *  Distinct from plain `idle` (which conflates "thinking" and "blocked").
   *  Set when the `:waiting` notice arrives (see {@link markNeedsYou}); cleared
   *  when the user attends — opens the session or sends it input. */
  needsYou: Record<Id, boolean> = $state({});

  /** Sidebar filter toggle: show only sessions that need attention. */
  needsYouFilter = $state(false);

  /** Unread-activity flags: a background (non-active) tab's session finished a
   *  stretch of work (working → idle) or raised "needs you" while the user was
   *  looking elsewhere. Cleared when the tab is activated. Purely client-side —
   *  derived from status transitions the events WS already delivers. */
  unread: Record<Id, boolean> = $state({});

  /** Recently closed tab ids, newest last — the ⌘⇧T "reopen closed tab" stack.
   *  Close is non-destructive (the session lives on), so reopening is just
   *  re-adding the tab. Capped; ids whose session vanished are skipped. */
  recentlyClosed: Id[] = $state([]);

  current: WorkspaceWithRole | null = $derived(
    this.workspaces.find((w) => w.id === this.currentId) ?? null,
  );

  /** The caller's role in the current workspace. With NO workspace at all the
   *  only sessions are the scratch ones, where every user is an implicit
   *  Editor — so `editor`, not the `viewer` default, or a fresh account could
   *  not drive the workspace-less session it just started. Per-session gates
   *  use {@link canEditSession}. */
  myRole: 'viewer' | 'editor' | 'admin' = $derived(
    this.current?.my_role ?? (this.currentId === null ? 'editor' : 'viewer'),
  );

  /** Whether the caller may act on session `s` (rename / archive / delete):
   *  Editor+ in the current workspace — always true for a workspace-less
   *  session, since the scratch workspace grants every user Editor. */
  canEditSession(s: Session): boolean {
    return s.workspace_id === SCRATCH_WORKSPACE_ID || this.myRole !== 'viewer';
  }

  activeSessionId: Id | null = $derived(this.panes[this.focusedPane] ?? null);

  activeSession: Session | null = $derived(this.getSession(this.activeSessionId));

  /** Active (non-archived) sessions. */
  activeSessions: Session[] = $derived(this.buckets.active);

  /** Sessions the tiled grid shows: all active EXCEPT background-spawned ones
   *  (Slack/Telegram channels + PR-review agents) the user hasn't explicitly
   *  opened — those stay out of the way so they never interrupt current work.
   *  Review agents are opened on demand from the Review panel's "Open" button. */
  mainSessions: Session[] = $derived(
    // Background-spawned sessions (workflow steps, review agents, vault docs
    // writers, PR drafts, …) live in their own panels and stay out of the tiled
    // grid unless the user explicitly opened them as a tab. Workspace-less
    // (scratch) sessions are loaded in EVERY workspace for the sidebar's "No
    // workspace" group — the same rule keeps them out of this workspace's grid
    // (they are not its sessions, exactly as {@link agentSessions} has it)
    // unless the user opened one. With no workspace selected they are all there
    // is, so the grid is theirs.
    this.mainOf(this.buckets.active, new Set(this.openTabs)),
  );

  private mainOf(active: Session[], open: Set<Id>): Session[] {
    return active.filter(
      (s) =>
        (isForeground(s) && (this.currentId === null || s.workspace_id !== SCRATCH_WORKSPACE_ID)) ||
        open.has(s.id),
    );
  }

  /** Active agent sessions (claude/codex/shell) of the current workspace —
   *  sidebar "Agents" group. Scratch sessions have their own group
   *  ({@link scratchSessions}). */
  agentSessions: Session[] = $derived(this.buckets.agent);

  /** Foreground agent sessions of the hidden scratch workspace — the sidebar
   *  "No workspace" group. Present in every workspace and with none. */
  scratchSessions: Session[] = $derived(this.buckets.scratch);

  /** Active connection sessions (ssh/db/custom) — sidebar "Connections" group. */
  connectionSessions: Session[] = $derived(this.buckets.connection);

  // ── All-workspaces sidebar view ────────────────────────────────────────────

  /** Sidebar toggle: also list sessions from every OTHER workspace, grouped by
   *  workspace name. Persisted app-wide; default ON. */
  allWorkspaces = $state(lsGet(LS_ALL_WS) !== '0');

  setAllWorkspaces(on: boolean): void {
    this.allWorkspaces = on;
    lsSet(LS_ALL_WS, on ? '1' : '0');
    if (on) void this.refreshOtherSessions();
  }

  /** Non-archived sessions from workspaces other than the current one, loaded
   *  by fanning out over the membership list ({@link refreshOtherSessions}).
   *  RBAC holds — each per-workspace list is the same one the user would see
   *  after switching there. */
  otherWsSessions: Session[] = $state.raw([]);

  /** The all-workspaces view, grouped: every OTHER workspace that has at least
   *  one foreground agent session, with its sessions newest-first. The current
   *  workspace keeps its normal flat list above these groups. */
  otherWsGroups: { ws: WorkspaceWithRole; sessions: Session[] }[] = $derived(
    this.workspaces
      .filter((w) => w.id !== this.currentId)
      .map((w) => ({
        ws: w,
        sessions: this.otherWsSessions
          .filter((s) => s.workspace_id === w.id && s.kind === 'agent' && isForeground(s))
          .sort((a, b) => b.last_active_at.localeCompare(a.last_active_at)),
      }))
      .filter((g) => g.sessions.length > 0),
  );

  /** Load sessions of every non-current workspace (for the grouped sidebar
   *  view) in ONE cross-workspace call (`GET /sessions`, live rows only —
   *  the daemon owner-scopes each workspace). It used to fan
   *  `/workspaces/{id}/sessions` out per workspace, each decoding that
   *  workspace's whole archived history. A failure is silent and keeps the
   *  previous groups. */
  async refreshOtherSessions(): Promise<void> {
    if (!this.allWorkspaces) return;
    const token = getToken();
    const selection = this.selectionGeneration;
    const others = new Set(this.workspaces.filter((w) => w.id !== this.currentId).map((w) => w.id));
    let rows: Session[];
    try {
      // Only rows the grouped view can show (foreground agents): background
      // engine sessions of every workspace used to ride along.
      rows = await api.get<Session[]>('/sessions?archived=false&foreground=true');
    } catch {
      return;
    }
    if (token !== getToken() || selection !== this.selectionGeneration || !this.allWorkspaces) return;
    const flat = rows.filter((s) => others.has(s.workspace_id) && !s.archived && visibleOnThisDevice(s));
    this.otherWsSessions = flat;
    // Seed statuses without clobbering fresher event-fed values.
    for (const s of flat) if (!(s.id in this.statusMap)) this.statusMap[s.id] = s.status;
    // Prune statusMap entries for sessions no longer present anywhere (left
    // workspaces, reaped sessions) so the map doesn't grow without bound.
    this.pruneStatusMap();
  }

  /** Drop `statusMap` entries for sessions no loaded list holds (left
   *  workspaces, reaped / exited background sessions) — after every list
   *  load, so the map doesn't grow with every id an event ever named (R2).
   *  Live (`working`/`running`) entries stay: a panel watching a background
   *  session it fetched itself reads its dot from here. */
  private pruneStatusMap(): void {
    const known = new Set<Id>();
    for (const s of this.sessions) known.add(s.id);
    for (const s of this.otherWsSessions) known.add(s.id);
    for (const s of this.archivedSessions) known.add(s.id);
    for (const id of this.ensuring.keys()) known.add(id);
    for (const id of staleStatusIds(this.statusMap, known)) delete this.statusMap[id];
  }

  /** Open a session that lives in another workspace: switch there, then focus
   *  it (the sidebar's grouped rows route through this). */
  async openInWorkspace(wsId: Id, sessionId: Id): Promise<void> {
    if (wsId !== this.currentId && !(await this.select(wsId))) return;
    if (this.currentId !== wsId) return;
    this.navigateToSession(sessionId);
  }

  /** Agent sessions opened from a Telegram chat — sidebar "Telegram" group.
   *  Newest first (RFC3339 last_active_at sorts chronologically) so the
   *  sidebar's "most recent N" cap keeps the freshest tickets visible. */
  telegramSessions: Session[] = $derived(this.buckets.telegram);

  /** Agent sessions opened from a Slack chat — sidebar "Slack" group.
   *  Newest first, like {@link telegramSessions}. */
  slackSessions: Session[] = $derived(this.buckets.slack);

  /** Agent sessions started locally (not by an engine) — sidebar "Agents"
   *  group. Background sessions (workflow steps, review agents, vault docs
   *  writers, PR drafts, …) run embedded in their own panels and are still
   *  openable from there via `openSession`, which reads `this.sessions`.
   *  `BACKGROUND_SOURCES` mirrors `BACKGROUND_SESSION_SOURCES` in
   *  crates/otto-core/src/domain.rs — the daemon exempts exactly the
   *  foreground complement from auto-pruning, so what the Agents tab shows is
   *  what retention protects. */
  plainAgentSessions: Session[] = $derived(this.buckets.plainAgent);

  /** Archived sessions — the collapsible "Archived" section. Loaded LAZILY,
   *  {@link ARCHIVED_PAGE} at a time, the first time the section opens
   *  ({@link loadArchived}); newest first. The main list never carries them. */
  archivedSessions: Session[] = $state.raw([]);
  /** At least one page was loaded for the current selection. */
  archivedLoaded = $state(false);
  archivedLoading = $state(false);
  /** Older archived rows exist beyond what is loaded ("Load more"). */
  archivedHasMore = $state(false);
  /** The current workspace (or scratch) has archived rows at all — a 1-row
   *  probe on every list load, so the folded header shows without paging. */
  hasArchived = $state(false);
  /** The probe (or an archive from here) already found archived rows for this
   *  selection (perf G7): they don't go away on their own, so later refreshes
   *  skip the 1–2 probe requests. Only a positive answer is kept — "none yet"
   *  is re-asked, so an archive from another client still shows the header.
   *  Cleared on a workspace switch and on an unarchive. */
  private archivedKnown = false;
  /** Per-scope paging cursor: the oldest loaded `created_at`, or null once
   *  that scope is exhausted. */
  private archivedCursor: Record<Id, string | null> = {};
  private archivedGeneration = 0;

  // "Working" count for the Agents badge — only foreground agent sessions, not
  // background review/channel ones (those are hidden from the Agents list, so
  // counting them made the badge disagree with the list, e.g. badge 4 / list empty).
  workingCount: number = $derived(
    this.buckets.foregroundActive.filter((s) => this.statusMap[s.id] === 'working').length,
  );

  /** Foreground agent sessions currently flagged "needs you" — the sidebar
   *  "Needs you" badge/count (mirrors {@link workingCount}'s scoping). */
  needsYouCount: number = $derived(
    this.buckets.foregroundActive.filter((s) => this.needsYou[s.id] === true).length,
  );

  /** Foreground live sessions (the scope of the badge counts) — exported for
   *  the Home "waiting" box so it doesn't re-filter the whole list. */
  get foregroundActive(): Session[] {
    return this.buckets.foregroundActive;
  }

  /** Flag a session as needing the operator's attention (blocked on input). */
  markNeedsYou(id: Id): void {
    if (this.needsYou[id]) return;
    this.needsYou = { ...this.needsYou, [id]: true };
    // Needing the operator while not on screen is unread activity too.
    if (id !== this.activeSessionId) this.unread = { ...this.unread, [id]: true };
  }

  /** Reload the in-flight workflow runs for the current workspace. Cheap query;
   *  called on workspace switch and on every `workflow_run_updated` WS event so
   *  the "Running" sidebar list + nav count stay live without per-page polling. */
  async refreshActiveWorkflowRuns(): Promise<void> {
    const wsId = this.currentId;
    if (!wsId) {
      this.activeWorkflowRuns = [];
      return;
    }
    try {
      const runs = await listActiveWorkflowRuns(wsId);
      // Guard against an out-of-order response after a workspace switch.
      if (this.currentId === wsId) this.activeWorkflowRuns = runs;
    } catch {
      /* transient; the next event re-fetches */
    }
  }

  /** Apply a `workflow_run_updated` WS event to the "Running" sidebar list IN
   *  PLACE: update a known run's status/progress/approval flag, drop it on a
   *  terminal status, and fall back to a full refetch only for runs the list
   *  doesn't know yet (or events from an older daemon without progress fields,
   *  signalled by `nodes_total` 0/absent). */
  applyWorkflowRunEvent(ev: {
    workspace_id: Id;
    run_id: Id;
    status: string;
    nodes_done?: number;
    nodes_total?: number;
    waiting_approval?: boolean;
  }): void {
    if (ev.workspace_id !== this.currentId) return;
    const terminal = ev.status === 'success' || ev.status === 'error' || ev.status === 'canceled';
    const idx = this.activeWorkflowRuns.findIndex((r) => r.run_id === ev.run_id);
    if (terminal) {
      if (idx >= 0) this.activeWorkflowRuns.splice(idx, 1);
      return;
    }
    if (idx < 0) {
      // A run the list doesn't know yet (started elsewhere) — one full refetch
      // brings it in with its workflow name.
      void this.refreshActiveWorkflowRuns();
      return;
    }
    const r = this.activeWorkflowRuns[idx];
    r.status = ev.status as ActiveWorkflowRun['status'];
    if (ev.nodes_total && ev.nodes_total > 0) {
      r.nodes_done = ev.nodes_done ?? r.nodes_done;
      r.nodes_total = ev.nodes_total;
    }
    if (ev.waiting_approval !== undefined) r.waiting_approval = ev.waiting_approval;
  }

  /** Clear a session's "needs you" flag — the user has attended to it. */
  clearNeedsYou(id: Id): void {
    if (!this.needsYou[id]) return;
    const next = { ...this.needsYou };
    delete next[id];
    this.needsYou = next;
  }

  async load(): Promise<void> {
    const generation = ++this.loadGeneration;
    const token = getToken();
    if (this.loadedToken !== token) {
      this.loadedToken = token;
      ++this.selectionGeneration;
      this.workspaces = [];
      this.currentId = null;
      this.scratch = null;
      this.sessions = [];
      this.otherWsSessions = [];
      this.activeWorkflowRuns = [];
      this.resetArchived();
    }
    const selection = this.selectionGeneration;
    // `fresh`: this is still the newest load for this identity — its LIST is
    // valid. `current`: nobody selected a workspace meanwhile either (the side
    // pane's guest follows the host's workspace before its own list lands) —
    // only then may it pick the default selection. A selection made meanwhile
    // must not leave the list empty and `listSettled` false forever.
    const fresh = () => generation === this.loadGeneration && token === getToken();
    const current = () => fresh() && selection === this.selectionGeneration;
    // The hidden scratch workspace — best-effort: a daemon without it leaves
    // `scratch` null and the sheet falls back to `~`. Fetched WITH the list
    // (perf F3: one round-trip instead of two before the session list).
    const scratchReq = api.get<Workspace>(`/workspaces/${SCRATCH_WORKSPACE_ID}`).catch(() => null);
    // The saved workspace's session list goes out WITH the list too (perf
    // G3): it used to wait for `/workspaces` to answer, then for `select()`,
    // so the sidebar filled after three round trips instead of two. Only used
    // if the saved id survives validation below; otherwise dropped unread.
    const saved = lsGet(winKey(LS_CURRENT));
    const boot = saved && saved !== SCRATCH_WORKSPACE_ID ? this.startBootSessions(token, saved) : null;
    this.bootSessions = boot;
    // Drop ours only — a newer load() may have replaced it meanwhile.
    const dropBoot = () => {
      if (this.bootSessions === boot) this.bootSessions = null;
    };
    let workspaces: WorkspaceWithRole[];
    try {
      workspaces = await api.get<WorkspaceWithRole[]>('/workspaces');
    } catch (e) {
      dropBoot();
      this.listSettled = true;
      throw e;
    }
    if (!fresh()) return;
    this.workspaces = workspaces;
    const scratch = await scratchReq;
    if (!fresh()) return;
    this.scratch = scratch;
    if (!current()) {
      // Someone already chose the workspace — keep their choice.
      dropBoot();
      this.listSettled = true;
      return;
    }
    const target = workspaces.find((w) => w.id === saved) ?? workspaces[0] ?? null;
    if (target?.id !== saved) dropBoot();
    // select()/selectNone() set currentId before their first await, so the
    // flag flips with the selection already in place.
    const selecting = target ? this.select(target.id) : this.selectNone();
    this.listSettled = true;
    try {
      await selecting;
    } finally {
      // One-shot: a later refresh always asks the daemon again.
      dropBoot();
    }
  }

  /** Start the shown-list requests {@link loadSessions} would make for `wsId`
   *  (its own rows + the scratch workspace's). A rejection is held for the
   *  consumer, never reported as unhandled when the requests go unused. */
  private startBootSessions(token: string | null, wsId: Id): BootSessions {
    const q = shownListQuery(this.extraSources.keys());
    const own = api.get<Session[]>(`/workspaces/${wsId}/sessions${q}`);
    own.catch(() => {});
    const scratch = api.get<Session[]>(`/workspaces/${SCRATCH_WORKSPACE_ID}/sessions${q}`).catch(() => [] as Session[]);
    return { key: `${token}|${wsId}|${q}`, own, scratch };
  }

  private workspaceSelectionRequest = 0;

  /** Ask before changing the API/persistence context. A newer selection (even
   * selecting the current workspace again) or auth reset cancels an older
   * pending decision. The no-guard path stays synchronous for boot/retry. */
  private maySelectWorkspace(id: Id | null): boolean | Promise<boolean> {
    const request = ++this.workspaceSelectionRequest;
    const generation = this.selectionGeneration;
    if (this.currentId === id || this.currentId === null) return true;
    const decision = router.mayChangeWorkspace();
    if (typeof decision === 'boolean') return decision;
    return decision.then((ok) => ok && request === this.workspaceSelectionRequest && generation === this.selectionGeneration);
  }

  async select(id: Id): Promise<boolean> {
    const decision = this.maySelectWorkspace(id);
    if (decision !== true && !(await decision)) return false;
    return this.selectApproved(id);
  }

  /** Selection after leave approval; also used by an approved archive so its
   * fallback does not ask to leave an already archived workspace again. */
  private async selectApproved(id: Id): Promise<boolean> {
    if (this.currentId === id && this.sessions.length > 0 && !this.sessionsError && this.layoutReady) return true;
    if (this.currentId !== id) {
      this.sessions = [];
      this.otherWsSessions = [];
      this.openTabs = [];
    }
    const generation = ++this.selectionGeneration;
    this.currentId = id;
    this.resetArchived();
    lsSet(winKey(LS_CURRENT), id);
    // Pin both persistence keys NOW, before the await below: the route→store
    // effect may `openSession` while sessions are still loading, and that
    // persist must land under this workspace so `restoreLayout` sees it.
    this.tabsKey = id;
    this.bindTabsKey(id);
    // No phantom-tab reconcile on a switch: it would prune the OLD workspace's
    // tabs against the NEW session list and persist that under the new key,
    // clobbering this workspace's saved layout before `restoreLayout` reads it.
    await this.refreshSessions({ reconcile: false });
    await this.waitForSessions(generation);
    if (generation !== this.selectionGeneration || this.currentId !== id) return false;
    if (this.sessionsError) return true; // Context changed; list failure remains retryable.
    void this.refreshActiveWorkflowRuns();
    void this.refreshOtherSessions();
    this.restoreLayout(id);
    return true;
  }

  /** Retry discovery and complete the layout restoration interrupted by failure. */
  async retrySessions(): Promise<void> {
    if (!this.layoutReady) {
      if (this.currentId) await this.select(this.currentId);
      else await this.selectNone();
    } else await this.refreshSessions();
  }

  /** Zero-workspace mode (a fresh account, or the last workspace archived):
   *  no current workspace, only scratch sessions, layout keyed on
   *  `SCRATCH_WORKSPACE_ID` so tabs/panes still survive reloads. */
  private async selectNone(): Promise<boolean> {
    const decision = this.maySelectWorkspace(null);
    if (decision !== true && !(await decision)) return false;
    return this.selectNoneApproved();
  }

  private async selectNoneApproved(): Promise<boolean> {
    const generation = ++this.selectionGeneration;
    this.currentId = null;
    this.resetArchived();
    this.activeWorkflowRuns = [];
    this.otherWsSessions = [];
    this.tabsKey = SCRATCH_WORKSPACE_ID;
    this.bindTabsKey(SCRATCH_WORKSPACE_ID);
    await this.refreshSessions({ reconcile: false });
    await this.waitForSessions(generation);
    if (generation !== this.selectionGeneration || this.currentId !== null) return false;
    if (this.sessionsError) return true;
    this.restoreLayout(SCRATCH_WORKSPACE_ID);
    return true;
  }

  /** Point both persistence keys at `key` and stop writing under it until
   *  {@link restoreLayout} has READ it — `select()` awaits the session refresh
   *  in between, and an `openSession` landing in that window (the route→store
   *  effect replaying `#/agents/<id>` on a reload) would otherwise persist a
   *  one-tab / one-pane state over the very payload we are about to restore.
   *  What it opened is not lost: both halves replay it after the read. */
  private bindTabsKey(key: string): void {
    this.tabsKey = key;
    this.tabsHydrated = false;
    this.layoutReady = false;
    this.pendingTabs = [];
    layout.bindKey(key);
  }

  /** Restore the open tabs + split layout persisted under `key` (a workspace
   *  id, or `SCRATCH_WORKSPACE_ID` with no workspace selected). Runs after
   *  {@link refreshSessions} so only ids that still exist survive. */
  private restoreLayout(key: string): void {
    // Pin the tabs key here, exactly like `layout.restore(key)` pins `wsKey`:
    // `select()` sets `currentId` and then AWAITS `refreshSessions`, so a
    // `persistTabs()` in between (a `session_removed` event → `closeTab`) would
    // otherwise write the OLD workspace's tabs under the NEW id.
    this.tabsKey = key;
    // restore tabs for this workspace
    const raw = lsGet(winKey(LS_TABS + key));
    const ids: Id[] = raw ? JSON.parse(raw) : [];
    // Keep real sessions + the DB-Explorer pane sentinel (it has no session row).
    const valid = ids.filter((t) => t === DB_PANE_ID || this.sessionById.has(t));
    // Tabs opened while this key was still un-hydrated (see {@link bindTabsKey})
    // are appended — the route→store effect's session must survive the restore.
    const pendingAll = this.pendingTabs.filter((t) => !valid.includes(t));
    const pending = pendingAll.filter((t) => t === DB_PANE_ID || this.sessionById.has(t));
    // A route-opened session the shown list doesn't carry (a background
    // agent's `#/agents/<id>`): fetch it by id, then open it.
    for (const t of pendingAll) if (!pending.includes(t)) this.openWhenLoaded(t);
    this.pendingTabs = [];
    this.tabsHydrated = true;
    this.openTabs = [...valid, ...pending];
    if (pending.length > 0) this.persistTabs();
    // Restore the split layout persisted alongside the tabs (v2 tree, or a v1
    // {panes, axis} payload migrated through the old window fractions).
    const open = this.openTabs;
    layout.restore(key, (sid) => open.includes(sid), open[0] ?? null);
    this.layoutReady = true;
  }

  /** Whether a session with this workspace id belongs in `sessions`: the
   *  current workspace's, plus the scratch workspace's (always loaded). */
  private belongsHere(wsId: Id): boolean {
    return wsId === this.currentId || wsId === SCRATCH_WORKSPACE_ID;
  }

  /** Reload `sessions` for the current workspace (+ scratch). `reconcile`
   *  (default on) prunes phantom tabs afterwards; a workspace switch turns it
   *  off because {@link restoreLayout} replaces the layout wholesale. */
  refreshSessions(opts: { reconcile?: boolean } = {}): Promise<void> {
    // Single-flight (F7): a refresh while one is already loading for the SAME
    // selection queues at most ONE trailing load (it starts after the current
    // one settles, so it sees every change) instead of another request each.
    const key = `${this.selectionGeneration}:${this.currentId}`;
    if (this.sessionsInFlight && this.sessionsInFlightKey === key) {
      this.sessionsTrailing ??= this.sessionsInFlight
        .catch(() => {})
        .then(() => {
          this.sessionsTrailing = null;
          return this.refreshSessions(opts);
        });
      return this.sessionsTrailing;
    }
    this.sessionsInFlightKey = key;
    const run: Promise<void> = this.loadSessions(opts).finally(() => {
      if (this.sessionsInFlight === run) this.sessionsInFlight = null;
    });
    this.sessionsInFlight = run;
    return run;
  }

  /** Ask for a background source to be loaded with the main list while a
   *  panel that lists those sessions is mounted (the swarm org tree reads
   *  `meta.source = 'swarm'` rows). Returns the release function. */
  includeSources(...sources: string[]): () => void {
    // Ref-counted: two mounted panels asking for the same source share it.
    const fresh = sources.filter((src) => !this.extraSources.has(src));
    for (const src of sources) this.extraSources.set(src, (this.extraSources.get(src) ?? 0) + 1);
    if (fresh.length > 0 && this.layoutReady) void this.refreshSessions().catch(() => {});
    let released = false;
    return () => {
      if (released) return;
      released = true;
      for (const src of sources) {
        const n = (this.extraSources.get(src) ?? 1) - 1;
        if (n <= 0) this.extraSources.delete(src);
        else this.extraSources.set(src, n);
      }
    };
  }

  /** Fetch ONE session by id into `sessions` (when it belongs to the current
   *  workspace or scratch) — for an id the shown list doesn't carry: a
   *  background agent opened from its panel, a notification, a handover
   *  source. Deduped per id; null when missing / not visible. */
  ensureSession(id: Id): Promise<Session | null> {
    const have = this.sessionById.get(id);
    if (have) return Promise.resolve(have);
    const inflight = this.ensuring.get(id);
    if (inflight) return inflight;
    const selection = this.selectionGeneration;
    const p = api
      .get<Session[]>(`/sessions?ids=${encodeURIComponent(id)}`)
      .then((rows) => {
        const s = rows.find((r) => r.id === id) ?? null;
        if (!s || selection !== this.selectionGeneration) return s;
        if (this.belongsHere(s.workspace_id) && visibleOnThisDevice(s) && !this.sessionById.has(id)) {
          this.sessions = [...this.sessions, s];
          if (!(s.id in this.statusMap)) this.statusMap[s.id] = s.status;
        }
        return s;
      })
      .catch(() => null)
      .finally(() => this.ensuring.delete(id));
    this.ensuring.set(id, p);
    return p;
  }

  /** {@link ensureSession}, then open it — once (no retry loop when the row
   *  turns out not to be listable here). */
  private openWhenLoaded(id: Id): void {
    void this.ensureSession(id).then(() => {
      if (this.sessionById.has(id)) this.openSession(id);
    });
  }

  private resetArchived(): void {
    ++this.archivedGeneration;
    this.archivedSessions = [];
    this.archivedLoaded = false;
    this.archivedLoading = false;
    this.archivedHasMore = false;
    this.hasArchived = false;
    this.archivedKnown = false;
    this.archivedCursor = {};
  }

  /** Load the first (or, with `more`, the next) page of archived sessions for
   *  the current workspace + scratch — `?archived=true&limit=100&before=…`. */
  async loadArchived(more = false): Promise<void> {
    if (this.archivedLoading || (more && !this.archivedHasMore) || (!more && this.archivedLoaded)) return;
    const generation = this.archivedGeneration;
    const scopes = [this.currentId, SCRATCH_WORKSPACE_ID].filter((w, i, a): w is Id => !!w && a.indexOf(w) === i);
    const todo = scopes.filter((w) => !more || this.archivedCursor[w] != null);
    this.archivedLoading = true;
    try {
      const pages = await Promise.all(
        todo.map((w) => {
          const before = more ? this.archivedCursor[w] : null;
          const q = `?archived=true&limit=${ARCHIVED_PAGE}${before ? `&before=${encodeURIComponent(before)}` : ''}`;
          return api
            .get<Session[]>(`/workspaces/${w}/sessions${q}`)
            .catch((e) => (w === SCRATCH_WORKSPACE_ID ? ([] as Session[]) : Promise.reject(e)))
            .then((rows) => ({ w, rows }));
        }),
      );
      if (generation !== this.archivedGeneration) return;
      const seen = new Set(more ? this.archivedSessions.map((s) => s.id) : []);
      const next = more ? [...this.archivedSessions] : [];
      for (const { w, rows } of pages) {
        // Rows come oldest-first; the oldest is the next page's cursor.
        this.archivedCursor[w] = rows.length >= ARCHIVED_PAGE ? rows[0].created_at : null;
        for (const s of rows) {
          if (seen.has(s.id) || !visibleOnThisDevice(s)) continue;
          seen.add(s.id);
          next.push(s);
        }
      }
      next.sort((a, b) => b.created_at.localeCompare(a.created_at));
      this.archivedSessions = next;
      this.archivedLoaded = true;
      this.archivedHasMore = scopes.some((w) => this.archivedCursor[w] != null);
      if (next.length > 0) this.hasArchived = true;
    } finally {
      if (generation === this.archivedGeneration) this.archivedLoading = false;
    }
  }

  /** Ids to fetch by id alongside the shown list: open tabs / panes (this
   *  workspace's persisted ones too, before `restoreLayout` reads them) and
   *  background rows still live in the store (an embedded PR draft, a review
   *  agent being watched) — so a refresh never drops what is on screen. */
  private pinnedIds(): Id[] {
    const ids = new Set<Id>();
    try {
      const raw = lsGet(winKey(LS_TABS + this.tabsKey));
      for (const t of raw ? (JSON.parse(raw) as Id[]) : []) ids.add(t);
    } catch {
      /* corrupt payload: restoreLayout handles it */
    }
    for (const t of this.openTabs) ids.add(t);
    for (const t of this.pendingTabs) ids.add(t);
    for (const t of layout.panes) ids.add(t);
    const live = this.sessions
      .filter(
        (s) =>
          !isShownKind(s) &&
          !s.archived &&
          this.belongsHere(s.workspace_id) &&
          (this.statusMap[s.id] ?? s.status) !== 'exited',
      )
      .sort((a, b) => b.last_active_at.localeCompare(a.last_active_at))
      .slice(0, 64);
    for (const s of live) ids.add(s.id);
    ids.delete(DB_PANE_ID);
    return [...ids];
  }

  /** Events may refresh again while selection is waiting. Restore persisted
   * tabs only after the latest response for this selection is settled. */
  private async waitForSessions(selection: number): Promise<void> {
    while (selection === this.selectionGeneration && (this.sessionsTrailing || this.sessionsInFlight)) {
      await (this.sessionsTrailing ?? this.sessionsInFlight)?.catch(() => {});
    }
  }

  private async loadSessions(opts: { reconcile?: boolean }): Promise<void> {
    const generation = ++this.sessionsGeneration;
    const selection = this.selectionGeneration;
    const wsId = this.currentId;
    const current = () => generation === this.sessionsGeneration && selection === this.selectionGeneration && wsId === this.currentId;
    this.sessionsLoading = true;
    this.sessionsError = null;
    try {
      // The current workspace's sessions (when one is selected) plus the
      // scratch workspace's — always, best-effort (a daemon without the
      // scratch row answers 403, which leaves workspace-less sessions empty
      // and everything else unchanged). Deduped by id defensively.
      //
      // ONLY the rows the sidebar shows (F1): live connections, foreground
      // agents and channel tickets (+ sources a mounted panel asked for). A
      // main workspace with 1.9 k hidden review agents used to ship ~1.2 MB
      // per refresh. Open tabs / panes and live background rows ride along by
      // id (in parallel), and anything else is fetched on demand
      // ({@link ensureSession}). Archived rows load lazily ({@link loadArchived}).
      const q = shownListQuery(this.extraSources.keys());
      // Not once rows are known (G7, which also covers R4's "once per selection"
      // for any workspace that has archived rows); "none yet" is re-asked so an
      // archive from another client still shows the header.
      const probeArchived = !this.archivedLoaded && !this.archivedKnown;
      const probe = (w: Id) =>
        api
          .get<Session[]>(`/workspaces/${w}/sessions?archived=true&limit=1`)
          .then((r) => r.length > 0)
          .catch(() => false);
      // The boot's speculative requests for exactly this list, if any (G3).
      const boot =
        this.bootSessions && this.bootSessions.key === `${getToken()}|${wsId}|${q}` ? this.bootSessions : null;
      if (boot) this.bootSessions = null;
      const [own, scratch, pinned] = await Promise.all([
        boot ? boot.own : wsId ? api.get<Session[]>(`/workspaces/${wsId}/sessions${q}`) : Promise.resolve([]),
        boot
          ? boot.scratch
          : api.get<Session[]>(`/workspaces/${SCRATCH_WORKSPACE_ID}/sessions${q}`).catch(() => [] as Session[]),
        Promise.all(
          idChunks(this.pinnedIds()).map((chunk) =>
            api
              .get<Session[]>(`/sessions?ids=${chunk.map(encodeURIComponent).join(',')}`)
              .catch(() => [] as Session[]),
          ),
        ).then((pages) => pages.flat()),
      ]);
      if (!current()) return;
      const seen = new Set<Id>();
      const all: Session[] = [];
      // Fetch-by-id rows span every workspace (`GET /sessions`): keep this
      // one's + scratch's.
      for (const s of [...own, ...scratch, ...pinned.filter((p) => this.belongsHere(p.workspace_id))]) {
        if (seen.has(s.id)) continue;
        seen.add(s.id);
        all.push(s);
      }
      this.hasArchived = this.archivedKnown || this.archivedSessions.length > 0;
      // Once a page is loaded the section knows on its own; until then a
      // 1-row probe decides whether the folded header shows at all; archiving
      // here flips `hasArchived` locally. Off the boot path (idle): it only
      // gates a folded header, and on a cold boot its two requests went out
      // before first paint (desktop-boot-perf's request budget).
      if (probeArchived) {
        whenIdle(() => {
          if (!current()) return;
          void Promise.all([...(wsId ? [probe(wsId)] : []), probe(SCRATCH_WORKSPACE_ID)]).then((r) => {
            if (!current() || !r.some(Boolean)) return;
            this.archivedKnown = true;
            this.hasArchived = true;
          });
        });
      }
      // Background engine sessions that ARE here (open tabs, live ones this
      // document saw created, `includeSources` panels) stay in `this.sessions`
      // so their owning panels can look them up / open them; every
      // user-facing list still filters them via `isForeground` — one shared
      // blacklist (`BACKGROUND_SOURCES`) rather than per-list drift.
      // Per-device session isolation (opt-in, default off): show only sessions
      // this device started. When off, leave the list unchanged so every device
      // sees every session. Drives tabs/Navigator/agents list consistently since
      // they all derive from `this.sessions`. The setter re-runs this so flips
      // apply live.
      const next = all.filter(visibleOnThisDevice);
      // The fetched rows are the truth: a stamp queued before they arrived
      // must not re-apply an older status over them on the next frame.
      for (const s of next) this.pendingStatus.delete(s.id);
      this.sessions = next;
      for (const s of next) this.statusMap[s.id] = s.status;
      this.pruneStatusMap();
      if (opts.reconcile !== false) this.reconcileTabs();
    } catch (e) {
      if (current()) this.sessionsError = e instanceof Error ? e.message : String(e);
    } finally {
      if (current()) this.sessionsLoading = false;
    }
  }

  /** Drop tabs/panes that reference a session no longer present (a "phantom"
   *  tab left behind when a session ends or is reaped server-side without a
   *  `session_removed` event reaching this client). Keeps the DB-Explorer
   *  sentinel pane, which has no session row. */
  private reconcileTabs(): void {
    const exists = (t: Id): boolean =>
      t === DB_PANE_ID || this.sessionById.has(t);
    const tabs = this.openTabs.filter(exists);
    if (tabs.length !== this.openTabs.length) {
      this.openTabs = tabs;
      this.persistTabs();
    }
    layout.retain(exists);
    if (layout.panes.length === 0 && tabs.length > 0) layout.setFocusedSession(tabs[0]);
  }

  /** Storage-key suffix for the open tabs: whatever {@link restoreLayout} last
   *  restored — the current workspace, or the scratch id when none is selected
   *  (workspace-less sessions still keep their tabs). NOT derived from
   *  `currentId`: it must not move until the new workspace's tabs are loaded. */
  private tabsKey: string = SCRATCH_WORKSPACE_ID;
  /** Mirrors the layout store's own gate: false between {@link bindTabsKey} and
   *  the {@link restoreLayout} that reads the key. */
  private tabsHydrated = true;
  /** True once the current workspace's tabs + split layout have been restored
   *  (false from {@link bindTabsKey} until {@link restoreLayout}). Reactive, so
   *  a page can tell "no panes yet" apart from "nothing open". */
  layoutReady = $state(false);
  /** Tabs opened during that window, replayed by {@link restoreLayout}. */
  private pendingTabs: Id[] = [];

  private persistTabs(): void {
    if (!this.tabsHydrated) return;
    lsSet(winKey(LS_TABS + this.tabsKey), JSON.stringify(this.openTabs));
  }

  /** Is `key` (a `storage` event key) this workspace's persisted tabs or
   *  split layout? */
  ownsLayoutKey(key: string | null): boolean {
    return key === winKey(LS_TABS + this.tabsKey) || key === winKey(LS_PANES + this.tabsKey);
  }

  /** Re-read the open tabs + split layout that ANOTHER document of this
   *  window persisted — the side-by-side pane (an iframe) shares the window's
   *  keys, so whichever pane shows Agents is the owner and the other adopts
   *  its tabs when they change (lib/stores/sidePane.svelte.ts). A no-op until
   *  the current workspace's layout has been restored once. */
  adoptPersistedLayout(): void {
    if (!this.tabsHydrated || !this.layoutReady) return;
    this.restoreLayout(this.tabsKey);
  }

  /** Persist the split layout per workspace, so an arrangement of up to
   *  MAX_PANES (15) panes survives reloads (restored in {@link select}). */
  private persistPanes(): void {
    layout.persist();
  }

  /** Update tab + pane bookkeeping to make `id` the focused session.
   *
   * This is the **pure store mutation** — it does NOT navigate the router.
   * Call it when you already know the route reflects the session (e.g. from
   * a route→store sync `$effect` in App.svelte, or internal store housekeeping).
   * To navigate AND open a session from a user action, call
   * {@link navigateToSession} instead.
   */
  openSession(id: Id): void {
    // Don't open a tab for a session that's known not to exist — e.g. a stale id
    // left in the `#/agents/<id>` route hash after the session was reaped (the
    // cause of an undismissable "phantom" tab). Allowed while sessions are still
    // loading; reconcileTabs() prunes any that turn out invalid once loaded.
    if (id !== DB_PANE_ID && !this.sessionsLoading && !this.sessionById.has(id)) {
      // Not in the shown list — maybe a background agent opened from its
      // panel (the list no longer carries those): fetch it by id and open it
      // if it exists. A reaped id stays closed (no phantom tab).
      this.openWhenLoaded(id);
      return;
    }
    // Opening a session counts as attending to it — drop any "needs you" flag.
    this.clearNeedsYou(id);
    if (!this.openTabs.includes(id)) {
      this.openTabs = [...this.openTabs, id];
      if (!this.tabsHydrated) this.pendingTabs = [...this.pendingTabs, id];
      this.persistTabs();
    }
    layout.setFocusedSession(id);
    this.persistPanes();
    // Activating a tab clears its unread-activity dot.
    if (this.unread[id]) {
      const next = { ...this.unread };
      delete next[id];
      this.unread = next;
    }
  }

  /** Navigate to a session via the router (route = `#/agents/<id>`).
   *
   * This is the **user-facing navigation action**: it pushes a history entry so
   * browser/in-app Back/Forward walk session history. The route change triggers
   * App.svelte's route→store `$effect`, which calls {@link openSession} to
   * update tabs/panes — no double-push, no loop.
   *
   * All external callers (Navigator, TabBar, palette, notifications, …) should
   * use this instead of the old `ws.openSession(id) + router.go('agents')` pair.
   */
  navigateToSession(id: Id): void {
    router.go(`agents/${id}`);
  }

  /**
   * Write text into a session's PTY **server-side** (`POST /sessions/{id}/input`),
   * which works even when no Terminal is mounted for the session yet — unlike
   * {@link injectInput}, which relies on an open Terminal applying the store
   * update. `submit` appends a newline so the agent runs it immediately (default).
   * Used by the first-run coach to seed a freshly launched session with a prompt.
   */
  async sendInput(sessionId: Id, text: string, submit = true): Promise<void> {
    await api.post(`/sessions/${sessionId}/input`, { text, submit });
  }

  /** Inject text into a session's PTY (the Terminal for `sessionId` applies it). */
  injectInput(sessionId: Id, text: string): void {
    // Sending input is attending to it — drop any "needs you" flag.
    this.clearNeedsYou(sessionId);
    const prev = this.injections[sessionId]?.n ?? 0;
    this.injections = { ...this.injections, [sessionId]: { text, n: prev + 1 } };
  }

  /** Best agent session to receive injected input: the focused pane if it's an
   *  agent, else the most-recently-active agent in this workspace (or null). */
  get targetAgentId(): Id | null {
    const active = this.activeSessionId;
    const cur = active ? this.sessions.find((s) => s.id === active) : null;
    if (cur && cur.kind === 'agent' && !cur.archived) return cur.id;
    const agents = this.sessions.filter((s) => !s.archived && s.kind === 'agent');
    return agents.length ? agents[agents.length - 1].id : null;
  }

  /** Add a freshly created session object and navigate to it. */
  addSession(s: Session): void {
    if (this.belongsHere(s.workspace_id) && !this.sessionById.has(s.id)) {
      this.sessions = [...this.sessions, s];
    }
    this.statusMap[s.id] = s.status;
    if (this.belongsHere(s.workspace_id)) this.navigateToSession(s.id);
  }

  /**
   * Register a freshly created session and place it **beside** the current
   * pane(s) (a new split pane) rather than replacing the active tab — used to
   * attach an opened connection terminal next to an agent. Mirrors `addSession`'s
   * bookkeeping but routes the open through `openInSplit`. Returns `false` when
   * the 1–4 pane cap was hit (caller can toast).
   */
  addSessionInSplit(s: Session): boolean {
    if (this.belongsHere(s.workspace_id) && !this.sessionById.has(s.id)) {
      this.sessions = [...this.sessions, s];
    }
    this.statusMap[s.id] = s.status;
    if (!this.belongsHere(s.workspace_id)) return false;
    return this.openInSplit(s.id);
  }

  /** Where a new session goes: the hidden scratch workspace when asked for a
   *  workspace-less session, else the current workspace. Throws only when
   *  neither exists. */
  private createTarget(opts?: { scratch?: boolean }): Id {
    const target = opts?.scratch ? SCRATCH_WORKSPACE_ID : this.currentId;
    if (!target) throw new Error('no workspace selected');
    return target;
  }

  /**
   * Like {@link createSession} but does NOT route to the new session — for
   * hosts that embed the session where they are (the Browser page's agent
   * dock) and must stay put. Same device stamp + list/status bookkeeping.
   * `opts.scratch` starts a workspace-less session (scratch workspace).
   */
  async createSessionQuiet(req: CreateSessionReq, opts?: { scratch?: boolean }): Promise<Session> {
    const target = this.createTarget(opts);
    const stamped: CreateSessionReq = {
      ...req,
      meta: { ...(req.meta ?? {}), client_id: clientId() },
    };
    const s = await api.post<Session>(`/workspaces/${target}/sessions`, stamped);
    if (this.belongsHere(s.workspace_id) && !this.sessionById.has(s.id)) {
      this.sessions = [...this.sessions, s];
    }
    this.statusMap[s.id] = s.status;
    return s;
  }

  async createSession(req: CreateSessionReq, opts?: { scratch?: boolean }): Promise<Session> {
    const target = this.createTarget(opts);
    // Stamp the device that started this session (preserving any caller meta,
    // e.g. {origin:'manual'}) so the opt-in per-device isolation filter can
    // recognize its own sessions.
    const stamped: CreateSessionReq = {
      ...req,
      meta: { ...(req.meta ?? {}), client_id: clientId() },
    };
    const s = await api.post<Session>(`/workspaces/${target}/sessions`, stamped);
    this.addSession(s);
    return s;
  }

  /**
   * Start an agent session WITH an opening prompt (`POST …/sessions/open`): the
   * daemon waits for the CLI to be ready (trust dialog / cold start), pastes
   * the prompt, verifies the echo and presses Enter — no client-side timer
   * racing the spawn (A6). Always stamps `meta.work = {origin: 'manual'}`: the
   * route otherwise marks the session a delegation, which makes it
   * engine-owned (5-minute idle grace, hidden from foreground counts). Same
   * device stamp + list/status bookkeeping as {@link createSessionQuiet};
   * `opts.route` also navigates to it like {@link createSession}.
   */
  async openSessionWithPrompt(
    req: OpenAgentSessionReq,
    opts?: { scratch?: boolean; route?: boolean },
  ): Promise<Session> {
    const target = this.createTarget(opts);
    const body: OpenAgentSessionReq = {
      ...req,
      meta: { ...(req.meta ?? {}), client_id: clientId(), work: { origin: 'manual' } },
    };
    const { session: s } = await api.post<OpenAgentSessionResp>(`/workspaces/${target}/sessions/open`, body);
    if (opts?.route) {
      this.addSession(s);
    } else {
      if (this.belongsHere(s.workspace_id) && !this.sessionById.has(s.id)) {
        this.sessions = [...this.sessions, s];
      }
      this.statusMap[s.id] = s.status;
    }
    return s;
  }

  /**
   * Create a workspace (the backend expands `~` and creates the directory) and
   * switch to it. The creator becomes its admin, so we add it locally as such.
   */
  async createWorkspace(name: string, rootPath: string): Promise<WorkspaceWithRole> {
    const w = await api.post<Workspace>('/workspaces', {
      name: name.trim(),
      root_path: rootPath.trim(),
    });
    const withRole: WorkspaceWithRole = { ...w, my_role: 'admin' };
    this.workspaces = [...this.workspaces, withRole];
    await this.select(w.id);
    return withRole;
  }

  /** Rename a workspace and/or change its working directory (root path). */
  async updateWorkspace(
    id: Id,
    patch: { name?: string; root_path?: string },
  ): Promise<void> {
    const w = await api.patch<Workspace>(`/workspaces/${id}`, patch);
    this.workspaces = this.workspaces.map((x) => (x.id === id ? { ...x, ...w } : x));
  }

  /** Archive (soft-delete) a workspace: it leaves the sidebar; its sessions and
   *  files are untouched. Current-workspace editors approve leaving before
   *  the archive request; a successful archive selects the fallback. */
  async archiveWorkspace(id: Id): Promise<boolean> {
    if (this.currentId === id) {
      const decision = this.maySelectWorkspace(null);
      if (decision !== true && !(await decision)) return false;
    }
    const generation = this.selectionGeneration;
    await api.del(`/workspaces/${id}`);
    this.workspaces = this.workspaces.filter((x) => x.id !== id);
    this.otherWsSessions = this.otherWsSessions.filter((s) => s.workspace_id !== id);
    // A switch that completed during DELETE owns the current context.
    if (this.currentId === id && generation === this.selectionGeneration) {
      const next = this.workspaces[0];
      if (next) await this.selectApproved(next.id);
      else await this.selectNoneApproved();
    }
    return true;
  }

  /** Remove the tab (local bookkeeping only — the session keeps running).
   *  For user-facing close gestures use {@link requestCloseTab}, which adds the
   *  live-session confirm / archive-instead flow in front of this. */
  closeTab(id: Id): void {
    const closedIdx = this.openTabs.indexOf(id);
    this.openTabs = this.openTabs.filter((t) => t !== id);
    this.persistTabs();
    // Remember for ⌘⇧T "reopen closed tab" (close is non-destructive).
    this.recentlyClosed = [...this.recentlyClosed.filter((t) => t !== id), id].slice(-10);
    // Fall back to the closed tab's NEIGHBOR (the one that slid into its slot,
    // else the new last tab) — matching every mainstream tabbed UI, instead of
    // jumping to the far end of the strip.
    const fallback =
      closedIdx >= 0
        ? this.openTabs[Math.min(closedIdx, this.openTabs.length - 1)] ?? null
        : this.openTabs[this.openTabs.length - 1] ?? null;
    layout.replaceSession(id, fallback);
    this.persistPanes();
    // Keep the route in step: if the hash still points at the closed session,
    // the route→store effect would resurrect the tab on the next reload / Back /
    // module return (openSession only refuses ids that DON'T exist). Navigate
    // to the fallback (or bare agents) so the closed id leaves the URL.
    if (router.module === 'agents' && router.parts[1] === id) {
      router.go(fallback ? `agents/${fallback}` : 'agents');
    }
  }

  /**
   * User-facing tab close: every close gesture (× button, middle-click, ⌘W,
   * context menu, mobile bar, sidebar ×) funnels here. Closing a session's tab
   * ENDS the session — the same outcome as Archive / Delete from its menu — so
   * a session can never linger running behind a closed tab. The user picks
   * Archive (stop, history kept, resumable) or Delete (stop, history gone) in
   * a confirm dialog with a "remember my choice" checkbox (reset in Settings →
   * Appearance); once remembered, a single close applies it without asking.
   * The DB pane and already-archived rows just close.
   */
  async requestCloseTab(id: Id): Promise<void> {
    const action = await this.resolveCloseAction([id]);
    if (action === null) return;
    await this.endSession(id, action);
  }

  /** Bulk variant (Close Others / Close to the Right / Close All): ONE dialog
   *  covering all sessions in the set — never N prompts. */
  async requestCloseTabs(ids: Id[]): Promise<void> {
    if (ids.length === 0) return;
    const action = await this.resolveCloseAction(ids);
    if (action === null) return;
    for (const id of ids) await this.endSession(id, action);
  }

  /** Apply a resolved close action to one id: end the session (archive or
   *  delete — both close the tab themselves) or, for a non-session id, just
   *  close the tab. A failed end falls back to closing the tab so a bulk
   *  close never leaves a dead tab behind, and reports the error. */
  private async endSession(id: Id, action: CloseAction): Promise<void> {
    if (action === 'close' || !this.isEndable(id)) {
      this.closeTab(id);
      return;
    }
    try {
      if (action === 'delete-deferred') await this.deleteWithUndo(id);
      else if (action === 'delete') await this.killSession(id);
      else await this.archiveSession(id);
    } catch (e) {
      toastError(action === 'archive' ? 'Couldn’t archive the session' : 'Couldn’t delete the session', e);
      this.closeTab(id);
    }
  }

  /** Whether closing this tab must end a session: any non-archived session
   *  row (live, idle, suspended or exited — an exited row still sits in the
   *  Running list until archived). The DB pane and unknown ids are excluded. */
  private isEndable(id: Id): boolean {
    if (id === DB_PANE_ID) return false;
    const s = this.sessions.find((x) => x.id === id);
    return !!s && !s.archived;
  }

  /** Whether ending `id` now would stop an AGENT mid-turn — the one case a
   *  remembered "Always archive/delete" still asks about. `working` only means
   *  "printed in the last few seconds", so a plain shell (whose prompt redraw
   *  or `ls` output reads as working) is not mid-turn: its close/delete
   *  honours the remembered choice like an idle session's. */
  isAgentMidTurn(id: Id): boolean {
    if (this.statusMap[id] !== 'working') return false;
    const s = this.sessions.find((x) => x.id === id);
    return !s || (s.kind === 'agent' && s.provider !== 'shell');
  }

  /** Shared confirm step for {@link requestCloseTab}/{@link requestCloseTabs}:
   *  returns 'archive' | 'delete' (or 'close' when nothing needs ending), or
   *  null for cancel. Applies (and records) the remembered preference: a
   *  single-tab close under "Always archive" or "Always delete" asks nothing
   *  — the user opted out of the question in Settings → Appearance, and
   *  asking anyway made the setting a lie. But "Always delete" is never
   *  silently irreversible: it resolves to 'delete-deferred' (archive now,
   *  delete after an Undo toast — see {@link deleteWithUndo}), so a stray
   *  ⌘W / ⌫ / middle-click can be taken back. Two more guards: a close that ends
   *  **more than one** session (Close others / to the right / all), or one
   *  that stops a **working** agent mid-turn, always confirms once, whatever
   *  the preference. */
  private async resolveCloseAction(ids: Id[]): Promise<CloseAction | null> {
    const ending = ids.filter((id) => this.isEndable(id));
    if (ending.length === 0) return 'close';
    const n = ending.length;
    const many = n > 1;
    const busy = ending.filter((id) => this.isAgentMidTurn(id)).length;
    const pref = ui.closeTabPref;
    if (pref === 'archive' && !many && busy === 0) return 'archive';
    if (pref === 'delete' && !many && busy === 0) return 'delete-deferred';
    const name = this.sessions.find((s) => s.id === ending[0])?.title?.trim() || 'this session';
    const busyNote =
      busy === 0
        ? ''
        : many
          ? `\n\n${plural(busy, 'session')} ${busy === 1 ? 'is' : 'are'} working right now and will stop mid-turn.`
          : `\n\n“${name}” is working right now and will stop mid-turn.`;
    if (pref === 'archive' || pref === 'delete') {
      const del = pref === 'delete';
      const what = !many
        ? del
          ? `Closing this tab deletes “${name}”: it stops and its history is removed for good. This can’t be undone.`
          : `Closing this tab archives “${name}”: it stops and keeps its history (resumable from the Archived list).`
        : del
          ? `Closing these tabs deletes ${n} sessions: they stop and their history is removed for good. This can’t be undone.`
          : `Closing these tabs archives ${n} sessions: they stop and keep their history (resumable from the Archived list).`;
      const ok = await confirmer.ask(
        `${what}${busyNote}\n\nYour remembered choice is “Always ${pref}” — change it in Settings → Appearance.`,
        {
          title: many ? `${del ? 'Delete' : 'Archive'} ${n} sessions?` : `${del ? 'Delete' : 'Archive'} working session?`,
          confirmLabel: many ? `${del ? 'Delete' : 'Archive'} ${n} sessions` : `${del ? 'Delete' : 'Archive'} session`,
          danger: del,
        },
      );
      return ok ? pref : null;
    }
    const message = many
      ? `Closing these tabs ends ${n} sessions. Archive stops them and keeps their history (resumable from the Archived list); Delete stops them and removes their history for good.`
      : `Closing this tab ends “${name}”. Archive stops it and keeps its history (resumable from the Archived list); Delete stops it and removes its history for good.`;
    const picked = await confirmer.choose(message + busyNote, {
      title: many ? `Close ${n} sessions?` : 'Close session?',
      options: [
        { label: many ? `Archive ${n} sessions` : 'Archive session', value: 'archive', kind: 'primary' },
        { label: many ? `Delete ${n} sessions` : 'Delete session', value: 'delete', kind: 'danger' },
      ],
      checkboxLabel: 'Remember my choice (change in Settings → Appearance)',
    });
    if (picked.value !== 'archive' && picked.value !== 'delete') return null;
    if (picked.remember) ui.setCloseTabPref(picked.value);
    return picked.value;
  }

  /** Tooltip for a session tab's × (and the split pane ×): says what closing
   *  it will actually do under the current remembered preference. */
  closeTabTitle(id: Id, noun: 'tab' | 'session' = 'tab'): string {
    const base = `Close ${noun} (⌘W)`;
    if (!this.isEndable(id)) return base;
    if (ui.closeTabPref === 'archive') return `${base} — archives the session (resumable)`;
    if (ui.closeTabPref === 'delete') return `${base} — deletes the session and its history`;
    return `${base} — asks to archive or delete the session`;
  }

  /** Reopen the most recently closed tab (⌘⇧T). Closing a session's tab
   *  archives it, so an id missing from the live list is unarchived and
   *  reopened; one that is gone for good (deleted — {@link killSession} also
   *  drops it from the list) is skipped. */
  async reopenClosedTab(): Promise<void> {
    while (this.recentlyClosed.length > 0) {
      const id = this.recentlyClosed[this.recentlyClosed.length - 1];
      this.recentlyClosed = this.recentlyClosed.slice(0, -1);
      if (id === DB_PANE_ID || this.sessionById.has(id)) {
        this.navigateToSession(id);
        return;
      }
      const row = await this.ensureSession(id);
      if (!row) continue; // deleted (or no longer visible) — try the next one
      try {
        if (row.archived) await this.unarchiveSession(id);
        if (row.workspace_id !== this.currentId && !this.belongsHere(row.workspace_id)) {
          await this.openInWorkspace(row.workspace_id, id);
        } else {
          this.navigateToSession(id);
        }
        return;
      } catch (e) {
        toastError('Couldn’t reopen the session', e);
        return;
      }
    }
  }

  /** Undo an archive: bring the session back to the live list and open it. */
  async unarchiveAndOpen(id: Id): Promise<void> {
    await this.unarchiveSession(id);
    this.navigateToSession(id);
  }

  /** Move tab `id` to `targetIndex` in `openTabs` and persist the order. */
  reorderTab(id: Id, targetIndex: number): void {
    const from = this.openTabs.indexOf(id);
    if (from < 0 || from === targetIndex) return;
    const tabs = [...this.openTabs];
    tabs.splice(from, 1);
    tabs.splice(Math.max(0, Math.min(targetIndex, tabs.length)), 0, id);
    this.openTabs = tabs;
    this.persistTabs();
  }

  /** ⌘W / File ▸ Close Tab / the palette's "Close tab" / the mobile bar.
   *  Only while the Agents page is on screen: `activeSessionId` is the
   *  focused pane's session whatever module is showing, so ⌘W on Git / Vault
   *  / Settings used to end a session the user wasn't looking at. Elsewhere
   *  it is a no-op (open dialogs already consume ⌘W via `modalKeyVerdict`;
   *  the side pane closes itself via `embeddedKeyTarget`). Returns whether a
   *  close was requested. */
  closeActiveTab(): boolean {
    if (!sessionVerbsApply(router.module)) return false;
    if (!this.activeSessionId) return false;
    void this.requestCloseTab(this.activeSessionId);
    return true;
  }

  cycleTab(dir: 1 | -1): void {
    if (this.openTabs.length === 0) return;
    const cur = this.activeSessionId;
    const idx = cur ? this.openTabs.indexOf(cur) : -1;
    const next = this.openTabs[(idx + dir + this.openTabs.length) % this.openTabs.length];
    this.navigateToSession(next);
  }

  /** Focus the Nth open session tab (1-based, matching the tab-bar order). */
  focusSessionByIndex(n: number): void {
    const target = this.openTabs[n - 1];
    if (target) this.navigateToSession(target);
  }

  split(axis: SplitAxis): void {
    if (layout.splitFocused(axis)) this.persistPanes();
  }

  /**
   * Open a session **beside** the current one(s): insert it as a new leaf next
   * to the focused pane (respecting the MAX_PANES (15) cap) and focus it, so it
   * sits side by side with the existing panes rather than replacing the active
   * tab. Used to attach an opened connection terminal next to an agent.
   *
   * Returns `true` if it landed in a pane, or `false` when the cap is hit (the
   * caller can surface a toast). Unlike `openSession`, this never replaces the
   * focused pane — except when at the cap, where the focused pane is reused.
   */
  openInSplit(id: Id): boolean {
    // Keep tab bookkeeping consistent (same as openSession).
    if (!this.openTabs.includes(id)) {
      this.openTabs = [...this.openTabs, id];
      this.persistTabs();
    }
    // Panes only render side by side in the split (tabs) view; tiled view shows
    // every session and ignores `panes`. Switch so the new pane is actually seen.
    if (this.viewMode !== 'tabs') this.setViewMode('tabs');
    this.maximizedId = null;

    const ok = layout.addBeside(id); // existing leaf → focus; at MAX_PANES → focused leaf reused + false
    this.persistPanes();
    return ok;
  }

  closePane(idx: number): void {
    layout.removeAt(idx);
    this.persistPanes();
  }

  focusPane(idx: number): void {
    if (idx < 0 || idx >= this.panes.length) return;
    layout.focusIndex(idx);
    // Keep the route in sync with the focused pane so the URL + Back/Forward and
    // the navigator highlight track the click. The route→store effect reads
    // activeSessionId untracked, so this never clobbers; router.go dedupes a
    // same-hash navigation, so re-focusing the current pane is a no-op.
    const id = this.panes[idx];
    if (id) this.navigateToSession(id);
  }

  setViewMode(mode: 'tabs' | 'tiled' | 'mission'): void {
    this.viewMode = mode;
    if (mode === 'tabs') this.maximizedId = null;
    lsSet(winKey('otto_view_mode'), mode);
  }

  /**
   * Make a set of sessions visible side-by-side: switch to the tiled grid and
   * register them as open tabs (also laid out as split panes, up to MAX_PANES
   * (15), so they tile even in tabs view). Used by the Plan tab to surface its live planning
   * agents the moment they spawn. Unknown ids are tolerated — `reconcileTabs`
   * prunes any that never materialize; `session_created` events fill the rest in.
   */
  tileSessions(ids: Id[]): void {
    const fresh = ids.filter((id) => !this.openTabs.includes(id));
    if (fresh.length > 0) {
      this.openTabs = [...this.openTabs, ...fresh];
      this.persistTabs();
    }
    // Lay them out as side-by-side panes (the grid shows them all in tiled
    // view; panes give a clean split if the user flips back to tabs view).
    // Stop AT the cap: `addBeside` at MAX_PANES reuses the focused leaf, which
    // would silently replace the focused session once per surplus id — and
    // persist it. The tiled grid still shows every id.
    for (const id of ids) {
      if (layout.panes.length >= MAX_PANES) break;
      if (!layout.panes.includes(id)) layout.addBeside(id, { focus: false });
    }
    this.persistPanes();
    this.maximizedId = null;
    this.setViewMode('tiled');
  }

  /** Whether the UI is currently focused on a single session (tabbed view, or
   *  a maximized tile) — the right panel only shows in this case. */
  get singleSessionView(): boolean {
    return this.viewMode === 'tabs' || this.maximizedId !== null;
  }

  toggleMaximize(id: Id): void {
    this.maximizedId = this.maximizedId === id ? null : id;
    if (this.maximizedId) this.openSession(id);
  }

  /** Delete: remove the session entirely (PTY killed, row + history gone). */
  /** User-facing Delete of ONE session (tab menu, pane ⋯, sidebar row).
   *  Asks first — unless the user chose "Always delete" in Settings →
   *  Appearance: they opted out of the question, so an explicit Delete is
   *  honoured just like closing its tab (asking anyway is what made the
   *  setting feel broken). Bulk deletes keep their own one-time confirm.
   *  Failures surface as a toast. */
  async requestDeleteSession(id: Id): Promise<void> {
    // "Always delete" skips this — but never for an agent working mid-turn.
    const working = this.isAgentMidTurn(id);
    if (ui.closeTabPref !== 'delete' || working) {
      const name = this.sessions.find((s) => s.id === id)?.title?.trim();
      const ok = await confirmer.ask(
        `Delete ${name ? `“${name}”` : 'this session'} and its entire history? This cannot be undone.${working ? ' It is working right now and will stop mid-turn.' : ''}`,
        { title: 'Delete session', confirmLabel: 'Delete' },
      );
      if (!ok) return;
    }
    try {
      await this.killSession(id);
    } catch (e) {
      toastError('Couldn’t delete', e);
    }
  }

  async killSession(id: Id): Promise<void> {
    await api.del(`/sessions/${id}`);
    this.closeTab(id);
    // Gone for good: ⌘⇧T / "Reopen closed tab" must not offer it.
    this.recentlyClosed = this.recentlyClosed.filter((t) => t !== id);
    this.sessions = this.sessions.filter((s) => s.id !== id);
    this.otherWsSessions = this.otherWsSessions.filter((s) => s.id !== id);
    this.dropArchived(id);
    delete this.statusMap[id];
    this.clearNeedsYou(id);
  }

  private dropArchived(id: Id): void {
    if (this.archivedSessions.some((s) => s.id === id)) {
      this.archivedSessions = this.archivedSessions.filter((s) => s.id !== id);
    }
  }

  /** A session just got archived: it leaves the live lists and joins the
   *  (loaded) Archived section, newest first. */
  private moveToArchived(s: Session): void {
    this.sessions = this.sessions.filter((x) => x.id !== s.id);
    this.otherWsSessions = this.otherWsSessions.filter((x) => x.id !== s.id);
    this.hasArchived = true;
    this.archivedKnown = true;
    if (this.archivedLoaded) this.archivedSessions = [s, ...this.archivedSessions.filter((x) => x.id !== s.id)];
  }

  /** HTTP responses and events share the same idempotent membership update. */
  private applyArchiveState(s: Session): void {
    this.pendingStatus.delete(s.id);
    this.statusMap[s.id] = s.status;
    this.clearNeedsYou(s.id);
    if (s.archived) {
      this.closeTab(s.id);
      if (this.belongsHere(s.workspace_id) && visibleOnThisDevice(s)) this.moveToArchived(s);
      else this.otherWsSessions = this.otherWsSessions.filter(x => x.id !== s.id);
      return;
    }
    this.dropArchived(s.id);
    this.archivedKnown = false;
    this.hasArchived = this.archivedSessions.length > 0 || this.archivedHasMore || !this.archivedLoaded;
    if (!visibleOnThisDevice(s)) return;
    if (this.belongsHere(s.workspace_id)) this.sessions = [...this.sessions.filter(x => x.id !== s.id), s];
    else if (this.allWorkspaces && s.kind === 'agent' && isForeground(s))
      this.otherWsSessions = [...this.otherWsSessions.filter(x => x.id !== s.id), s];
  }

  /** Bulk archive (sidebar multi-select): one toast for the batch instead of
   *  N, and one error report — never stops at the first failure. */
  async archiveSessions(ids: Id[]): Promise<number> {
    let failed = 0;
    for (const id of ids) {
      try {
        const s = await api.post<Session>(`/sessions/${id}/archive`);
        this.applyArchiveState(s);
      } catch {
        failed++;
      }
    }
    const ok = ids.length - failed;
    if (ok > 0) toasts.info(`${plural(ok, 'session')} archived`);
    if (failed > 0) toasts.error('Couldn’t archive', `${plural(failed, 'session')} couldn’t be archived.`);
    return failed;
  }

  /** Bulk delete (sidebar multi-select): caller confirms first. */
  async killSessions(ids: Id[]): Promise<number> {
    let failed = 0;
    for (const id of ids) {
      try { await this.killSession(id); } catch { failed++; }
    }
    if (failed > 0) toasts.error('Couldn’t delete', `${plural(failed, 'session')} couldn’t be deleted.`);
    return failed;
  }

  /** Archive: kill the PTY but keep the row + history in the Archived section. */
  async archiveSession(id: Id): Promise<void> {
    const s = await api.post<Session>(`/sessions/${id}/archive`);
    this.applyArchiveState(s);
    toasts.push('info', 'Session archived', s.title, 8000, {
      action: { label: 'Undo', run: () => this.unarchiveAndOpen(id) },
    });
  }

  /** Pending "Always delete" closes: id → timer. See {@link deleteWithUndo}. */
  private pendingDeletes = new Map<Id, ReturnType<typeof setTimeout>>();

  /**
   * A remembered "Always delete" close (S13-01): no dialog, but not silently
   * irreversible either. The session is ARCHIVED now (it stops, its tab
   * closes, its history is kept) and permanently deleted only after the Undo
   * toast runs out. Undo restores it exactly like the archive undo. If the
   * window goes away inside the window, the session simply stays archived —
   * the failure mode is "kept", never "lost".
   */
  async deleteWithUndo(id: Id, graceMs = DELETE_UNDO_MS): Promise<void> {
    const s = await api.post<Session>(`/sessions/${id}/archive`);
    this.applyArchiveState(s);
    const timer = setTimeout(() => {
      this.pendingDeletes.delete(id);
      this.killSession(id).catch((e) => toastError('Couldn’t delete the session', e));
    }, graceMs);
    this.pendingDeletes.set(id, timer);
    toasts.push('info', 'Session deleted', s.title, graceMs, {
      action: {
        label: 'Undo',
        run: async () => {
          const t = this.pendingDeletes.get(id);
          if (t === undefined) throw new Error('The session was already deleted.');
          clearTimeout(t);
          this.pendingDeletes.delete(id);
          await this.unarchiveAndOpen(id);
        },
      },
    });
  }

  /** User-facing archive (session menu, History, Classrooms' detention).
   *  Archiving kills the PTY, so — like closing the tab or restarting — a
   *  WORKING agent asks first (its in-flight turn is lost; Undo unarchives
   *  the session but cannot bring the turn back). Idle/exited archive at once,
   *  and so does a plain shell ({@link isAgentMidTurn}: its output is not a turn).
   *  Resolves false when the user cancelled; a failed archive rejects. */
  async requestArchive(id: Id): Promise<boolean> {
    if (this.isAgentMidTurn(id)) {
      const name = this.sessions.find((x) => x.id === id)?.title?.trim() || this.otherWsSessions.find((x) => x.id === id)?.title?.trim() || 'this session';
      const ok = await confirmer.ask(
        `“${name}” is working right now and will stop mid-turn. Archiving stops the agent; you can restore the session from the Archived list, but its current turn is lost.`,
        { title: 'Archive working session?', confirmLabel: 'Archive session', danger: true },
      );
      if (!ok) return false;
    }
    await this.archiveSession(id);
    return true;
  }

  async unarchiveSession(id: Id): Promise<void> {
    const s = await api.post<Session>(`/sessions/${id}/unarchive`);
    this.applyArchiveState(s);
  }

  /** Bumped per session on every successful restart. The embedded Terminal
   *  watches it to drop its exited overlay and reconnect to the respawned PTY
   *  (the session id is unchanged, so this is the only signal). Kept here so
   *  EVERY restart path — pane header, sidebar, ⌘K, native menu, directories
   *  save — reconnects the terminal, not just the pane header's button. */
  restartNonces: Record<Id, number> = $state({});

  /** Open/resume preserves an already-live process, even from a stale row. */
  async resumeSession(id: Id): Promise<Session> {
    const s = await api.post<Session>(`/sessions/${id}/resume`);
    this.sessions = this.sessions.map((x) => (x.id === id ? s : x));
    this.statusMap[id] = s.status;
    return s;
  }

  async restartSession(id: Id, opts?: { quiet?: boolean }): Promise<void> {
    const s = await api.post<Session>(`/sessions/${id}/restart`);
    this.sessions = this.sessions.map((x) => (x.id === id ? s : x));
    this.statusMap[id] = s.status;
    this.restartNonces[id] = (this.restartNonces[id] ?? 0) + 1;
    if (!opts?.quiet) toasts.info('Session restarted', s.title);
  }

  /** User-facing restart (pane header, sidebar, ⌘K, native menu). Restart
   *  kills the live process and respawns it — a working agent loses its
   *  in-flight turn, so that one case asks first (idle/exited don't). Only
   *  the pane header used to ask; the other paths restarted silently.
   *  Failures surface as a toast. */
  async requestRestart(id: Id): Promise<void> {
    if (!(await this.confirmRestart(id))) return;
    try {
      await this.restartSession(id);
    } catch (e) {
      toastError('Couldn’t restart', e);
    }
  }

  /** The restart working-guard on its own, for callers that restart as part
   *  of a larger action (SessionView's "Save & restart"). True = go ahead. */
  async confirmRestart(id: Id): Promise<boolean> {
    if (this.statusMap[id] !== 'working') return true;
    const name = this.sessions.find((x) => x.id === id)?.title?.trim() || 'this session';
    return confirmer.ask(
      `“${name}” is working right now. Restarting stops its current turn and starts the agent again, resuming its saved conversation where it can.`,
      { title: 'Restart working session?', confirmLabel: 'Restart session', danger: true },
    );
  }

  async renameSession(id: Id, title: string): Promise<void> {
    const s = await api.patch<Session>(`/sessions/${id}`, { title });
    this.sessions = this.sessions.map((x) => (x.id === id ? s : x));
    this.otherWsSessions = this.otherWsSessions.map((x) => (x.id === id ? s : x));
  }

  /** Patch one session in whichever list holds it; a list without it (or an
   *  unchanged patch) keeps its identity, so nothing downstream re-runs. */
  private patchSession(id: Id, patch: (s: Session) => Session): void {
    const mine = patchSessionIn(this.sessions, id, patch);
    if (mine !== this.sessions) this.sessions = mine;
    const other = patchSessionIn(this.otherWsSessions, id, patch);
    if (other !== this.otherWsSessions) this.otherWsSessions = other;
  }

  /** Drop an exited background row unless it came back to life or a tab,
   *  pane or in-flight fetch holds it (R2). */
  dropExitedBackground(id: Id): void {
    const s = this.sessionById.get(id);
    if (!s || isShownKind(s) || (this.statusMap[id] ?? s.status) !== 'exited') return;
    const held = { tabs: [...this.openTabs, ...this.pendingTabs], panes: layout.panes, ensuring: new Set(this.ensuring.keys()) };
    if (!canDropExited(id, held)) return;
    this.sessions = this.sessions.filter((x) => x.id !== id);
    this.pendingStatus.delete(id);
  }

  /** Queue a row stamp for {@link flushStatus} (next animation frame, or a
   *  100 ms timer when frames are paused — a hidden window). */
  private queueStatus(id: Id, patch: StatusPatch): void {
    this.pendingStatus.set(id, patch);
    if (this.statusFlushTimer != null) return;
    const flush = () => this.flushStatus();
    this.statusFlushTimer = setTimeout(flush, 100);
    if (typeof requestAnimationFrame === 'function') this.statusFlushRaf = requestAnimationFrame(flush);
  }

  /** Apply every queued `session_status` stamp to both lists in one write. */
  flushStatus(): void {
    if (this.statusFlushTimer != null) clearTimeout(this.statusFlushTimer);
    if (this.statusFlushRaf != null && typeof cancelAnimationFrame === 'function') cancelAnimationFrame(this.statusFlushRaf);
    this.statusFlushTimer = null;
    this.statusFlushRaf = null;
    if (this.pendingStatus.size === 0) return;
    const pending = this.pendingStatus;
    this.pendingStatus = new Map();
    const mine = applyStatusPatches(this.sessions, pending);
    if (mine !== this.sessions) this.sessions = mine;
    const other = applyStatusPatches(this.otherWsSessions, pending);
    if (other !== this.otherWsSessions) this.otherWsSessions = other;
  }

  /** Event-bus feed (WS /ws/events). */
  applyEvent(ev: OttoEvent): void {
    switch (ev.type) {
      case 'session_archive_changed': {
        this.applyArchiveState(ev.session);
        break;
      }
      case 'session_status': {
        // Unread dot: a background tab's session just finished a stretch of
        // work (working → idle/exited) while the user was looking elsewhere.
        const prevStatus = this.statusMap[ev.session_id];
        if (
          prevStatus === 'working' &&
          (ev.status === 'idle' || ev.status === 'exited') &&
          ev.session_id !== this.activeSessionId &&
          this.openTabs.includes(ev.session_id)
        ) {
          this.unread = { ...this.unread, [ev.session_id]: true };
        }
        this.statusMap[ev.session_id] = ev.status;
        // The daemon stamps `last_active_at` on every status write; mirror it so
        // the idle "suspends in N" countdown starts from THIS transition instead
        // of whatever the row said when the list loaded (a session that just
        // went working → idle showed "37m idle · suspending…").
        // Coalesced (R6): one `sessions` write per frame for a burst.
        this.queueStatus(ev.session_id, { status: ev.status, at: new Date().toISOString() });
        // The agent resuming work means the operator already responded to
        // whatever it was blocked on — clear the sticky "needs you" flag. Also
        // clear it once the session exits or becomes reconnectable: a dead agent
        // can't need you, so it shouldn't keep a stale badge.
        if (
          ev.status === 'working' ||
          ev.status === 'running' ||
          ev.status === 'exited' ||
          ev.status === 'reconnectable'
        ) {
          this.clearNeedsYou(ev.session_id);
        }
        // R11: a finished workflow STEP session (PTY suspended → reconnectable, or
        // exited) no longer needs a tab. Auto-close it to declutter — but KEEP the
        // session (closeTab never deletes; the run detail's "Open session" reopens
        // it) — and never yank the tab the user is actively viewing.
        if (ev.status === 'reconnectable' || ev.status === 'exited') {
          const s = this.sessionById.get(ev.session_id);
          if (
            s?.meta?.source === 'workflow' &&
            ev.session_id !== this.activeSessionId &&
            this.openTabs.includes(ev.session_id)
          ) {
            this.closeTab(ev.session_id);
          }
        }
        // R2: a background row this document picked up live (a review agent,
        // a PR draft) leaves the list once it has exited — they used to pile
        // up until the next refresh. After a grace period: a PR / commit
        // draft dialog keeps showing its agent until the POST hands it the id.
        if (ev.status === 'exited') {
          const s = this.sessionById.get(ev.session_id);
          if (s && !isShownKind(s)) {
            const id = s.id;
            const selection = this.selectionGeneration;
            setTimeout(() => {
              if (selection === this.selectionGeneration) this.dropExitedBackground(id);
            }, EXITED_BACKGROUND_GRACE_MS);
          }
        }
        break;
      }
      case 'session_created': {
        const s = ev.session;
        this.statusMap[s.id] = s.status;
        // Another device's session under isolation: not ours to list.
        if (!visibleOnThisDevice(s)) break;
        if (this.belongsHere(s.workspace_id)) {
          // Background rows too: WipPanel / CreatePr watch their draft agent
          // appear here (exited ones leave again — `session_status`).
          if (!this.sessionById.has(s.id)) this.sessions = [...this.sessions, s];
        } else if (
          this.allWorkspaces &&
          // The grouped view renders foreground agents only (R2): background
          // review/workflow agents of other workspaces never belonged here.
          s.kind === 'agent' &&
          isForeground(s) &&
          !this.otherWsSessions.some((x) => x.id === s.id)
        ) {
          this.otherWsSessions = [...this.otherWsSessions, s];
        }
        break;
      }
      case 'session_meta_updated': {
        // Replace the cached session's meta in place (e.g. live handover flags).
        // Mirrors session_renamed: BOTH lists, so cross-workspace sidebar rows
        // (issue chips, handover flags) don't go stale.
        this.patchSession(ev.session_id, (s) => ({ ...s, meta: ev.meta }));
        break;
      }
      case 'session_renamed': {
        // A session's title changed — user PATCH or the background auto-namer
        // adopting the CLI's own session title. `session_meta_updated` only
        // carries meta, so this is the only event that refreshes `title` live;
        // pane header and sidebar re-render off `session.title` reactively.
        this.patchSession(ev.session_id, (s) => (s.title === ev.title ? s : { ...s, title: ev.title }));
        break;
      }
      case 'session_removed': {
        delete this.statusMap[ev.session_id];
        this.clearNeedsYou(ev.session_id);
        if (this.unread[ev.session_id]) {
          const next = { ...this.unread };
          delete next[ev.session_id];
          this.unread = next;
        }
        this.dropArchived(ev.session_id);
        if (this.belongsHere(ev.workspace_id)) {
          this.sessions = this.sessions.filter((s) => s.id !== ev.session_id);
          if (this.openTabs.includes(ev.session_id)) this.closeTab(ev.session_id);
        } else {
          this.otherWsSessions = this.otherWsSessions.filter((s) => s.id !== ev.session_id);
        }
        break;
      }
      case 'notice': {
        // The side-by-side pane gets the same event stream: the main window
        // toasts it once.
        if (isEmbedded) break;
        const level = ev.level === 'error' ? 'error' : ev.level === 'warn' ? 'warn' : 'info';
        toasts.push(level, ev.title, ev.body);
        break;
      }
    }
  }

  async attachIssue(sessionId: Id, issue: AttachedIssue): Promise<void> {
    const s = await api.patch<Session>(`/sessions/${sessionId}`, { meta: { issue } });
    this.sessions = this.sessions.map((x) => (x.id === sessionId ? s : x));
  }

  async detachIssue(sessionId: Id): Promise<void> {
    const s = await api.patch<Session>(`/sessions/${sessionId}`, { meta: { issue: null } });
    this.sessions = this.sessions.map((x) => (x.id === sessionId ? s : x));
  }

  async attachProductStory(sessionId: Id, storyId: Id): Promise<void> {
    const s = await api.post<Session>(`/sessions/${sessionId}/attach-product`, {
      story_id: storyId,
    });
    this.sessions = this.sessions.map((x) => (x.id === sessionId ? s : x));
  }

  /**
   * Shallow-merge a patch into a session's `meta` (server-side merge), then sync
   * the returned session locally. Use for e.g. `{ extra_dirs }`. Does not restart
   * — launch-time meta (like `--add-dir`) only takes effect on the next restart.
   */
  async updateSessionMeta(sessionId: Id, patch: Record<string, unknown>): Promise<void> {
    const s = await api.patch<Session>(`/sessions/${sessionId}`, { meta: patch });
    this.sessions = this.sessions.map((x) => (x.id === sessionId ? s : x));
  }

  /** Update extra_dirs for an agent session, then restart it so the new dirs take effect. */
  async setSessionDirs(sessionId: Id, dirs: string[]): Promise<void> {
    const patched = await api.patch<Session>(`/sessions/${sessionId}`, {
      meta: { extra_dirs: dirs },
    });
    this.sessions = this.sessions.map((x) => (x.id === sessionId ? patched : x));
    const restarted = await api.post<Session>(`/sessions/${sessionId}/restart`);
    this.sessions = this.sessions.map((x) => (x.id === sessionId ? restarted : x));
    this.statusMap[sessionId] = restarted.status;
    // The PTY was respawned: bump the nonce so the mounted terminal reconnects
    // (restartSession does the same) instead of staring at the dead socket.
    this.restartNonces[sessionId] = (this.restartNonces[sessionId] ?? 0) + 1;
  }

  /** Save `notes` into workspace `wsId` (default: current). The PATCH replaces
   *  the whole settings object, so merge into a FRESH read — a cached copy
   *  could revert keys changed since (e.g. api_client.allow_local). */
  async saveNotes(notes: string, wsId: Id | null = this.currentId): Promise<void> {
    if (!wsId) return;
    const token = getToken();
    const fresh = await fetchWorkspace(wsId);
    if (token !== getToken()) return;
    const settings = { ...fresh.settings, notes };
    const updated = await api.patch<Workspace>(`/workspaces/${wsId}`, { settings });
    if (token !== getToken()) return;
    this.workspaces = this.workspaces.map((w) =>
      w.id === updated.id ? { ...w, ...updated } : w,
    );
  }

  /** API-client opt-in: allow requests to localhost/private networks from this
   *  workspace. Reads `settings.api_client.allow_local` (off by default). */
  get apiAllowLocal(): boolean {
    const api = this.current?.settings?.api_client as { allow_local?: boolean } | undefined;
    return api?.allow_local === true;
  }

  /** Toggle the API client's local/private-target opt-in (admin-gated by the
   *  workspaces PATCH route). Shallow-merges into the settings JSON. */
  async setApiAllowLocal(allow: boolean): Promise<void> {
    if (!this.currentId || !this.current) return;
    const prev = (this.current.settings?.api_client as Record<string, unknown>) ?? {};
    const settings = { ...this.current.settings, api_client: { ...prev, allow_local: allow } };
    const updated = await api.patch<Workspace>(`/workspaces/${this.currentId}`, { settings });
    this.workspaces = this.workspaces.map((w) =>
      w.id === updated.id ? { ...w, ...updated } : w,
    );
  }

  /** API-client history retention for this workspace (the daemon trims after
   *  every run): `settings.api_client.history_max_rows` / `history_max_days`,
   *  default 0 / 0 = keep everything (opt-in); 0 disables that limit. */
  get apiHistoryRetention(): { rows: number; days: number } {
    const api = this.current?.settings?.api_client as
      | { history_max_rows?: number; history_max_days?: number }
      | undefined;
    const pick = (v: unknown, fallback: number): number =>
      typeof v === 'number' && Number.isInteger(v) && v >= 0 ? v : fallback;
    // 0 = no limit: retention is opt-in (pruning deletes recorded runs).
    return { rows: pick(api?.history_max_rows, 0), days: pick(api?.history_max_days, 0) };
  }

  /** Set the API-client history retention (admin-gated by the workspaces
   *  PATCH route). Shallow-merges into the settings JSON. */
  async setApiHistoryRetention(rows: number, days: number): Promise<void> {
    if (!this.currentId || !this.current) return;
    const prev = (this.current.settings?.api_client as Record<string, unknown>) ?? {};
    const settings = {
      ...this.current.settings,
      api_client: { ...prev, history_max_rows: rows, history_max_days: days },
    };
    const updated = await api.patch<Workspace>(`/workspaces/${this.currentId}`, { settings });
    this.workspaces = this.workspaces.map((w) =>
      w.id === updated.id ? { ...w, ...updated } : w,
    );
  }

  /** Newest finished automation runs kept per automation
   *  (`settings.api_client.automation_runs_keep`; 0 = keep all, the opt-in
   *  default). */
  get apiRunsKeep(): number {
    const v = (this.current?.settings?.api_client as { automation_runs_keep?: unknown } | undefined)?.automation_runs_keep;
    return typeof v === 'number' && Number.isInteger(v) && v >= 0 ? v : 0;
  }

  /** Set the run-history retention (same admin-gated PATCH, shallow merge). */
  async setApiRunsKeep(keep: number): Promise<void> {
    if (!this.currentId || !this.current) return;
    const prev = (this.current.settings?.api_client as Record<string, unknown>) ?? {};
    const settings = { ...this.current.settings, api_client: { ...prev, automation_runs_keep: keep } };
    const updated = await api.patch<Workspace>(`/workspaces/${this.currentId}`, { settings });
    this.workspaces = this.workspaces.map((w) => (w.id === updated.id ? { ...w, ...updated } : w));
  }

  /** Set this workspace's default agent CLI. '' clears it (use the global
   *  default). Shallow-merges into the workspace settings JSON. */
  async saveDefaultAgent(provider: string): Promise<void> {
    if (!this.currentId || !this.current) return;
    const settings = { ...this.current.settings, default_provider: provider };
    const updated = await api.patch<Workspace>(`/workspaces/${this.currentId}`, { settings });
    this.workspaces = this.workspaces.map((w) =>
      w.id === updated.id ? { ...w, ...updated } : w,
    );
  }
}

export const ws = new WorkspaceStore();
