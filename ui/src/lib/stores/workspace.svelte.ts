// Workspaces + sessions + tab/split state for the shell and Agent Mode.

import { api } from '../api/client';
import { listActiveWorkflowRuns } from '../api/workflows';
import { router } from '../router.svelte';
import type {
  ActiveWorkflowRun,
  AttachedIssue,
  CreateSessionReq,
  Id,
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

// Layout state is per-WINDOW (multi-window): winKey() namespaces these by the
// window's label so two windows never clobber each other's workspace/tabs/view.
// The main window keeps the legacy unprefixed keys.
const LS_CURRENT = 'otto_workspace';
const LS_TABS = 'otto_tabs_'; // + workspace id
const LS_PANES = 'otto_panes_'; // + workspace id — split panes + axis
// App-wide (deliberately NOT winKey-namespaced): whether the sidebar lists
// sessions from every workspace, grouped by workspace, instead of only the
// current one. Default ON — a session shouldn't vanish on a workspace switch.
const LS_ALL_WS = 'otto_nav_all_ws';

/** Background-spawned session sources that never surface in the sidebar's flat
 *  session lists (they live in their own panels/views). MUST stay byte-identical
 *  to the Rust source of truth: `BACKGROUND_SESSION_SOURCES` in
 *  `crates/otto-core/src/domain.rs` (which also drives server-side durability).
 *  Every derived list below filters through `isForeground` — never re-inline a
 *  source blacklist. */
const BACKGROUND_SOURCES = new Set([
  'channel',
  'review',
  'review_summarizer',
  'skilleval',
  'skillreview',
  'product-analysis',
  'product_refine',
  'swarm',
  'canvas_assist',
  'canvas_assist_preview',
  'mockup_assist',
  'db_assist',
  'workflow',
  'vault-docs',
  'vault-docs-review',
  'pr-draft',
  'commit-draft',
  'insights',
  'run_with_otto',
  'goal_loop',
  'discovery_chat',
  'scheduled_task',
  'finding',
]);

/** A user-facing foreground session (sidebar-listable). */
function isForeground(s: Session): boolean {
  const src = (s.meta as { source?: string } | null)?.source;
  return src == null || !BACKGROUND_SOURCES.has(src);
}

/** Sentinel tab/pane id for the docked DB Explorer (not a real session). Lets
 *  the DB Explorer live as a pane in the Agents split, beside an agent. */
export const DB_PANE_ID = '__db_explorer__';

export type SplitAxis = 'col' | 'row';

/** Restore a persisted split-gutter fraction (0.2–0.8), else the 50/50 default. */
function readFrac(key: string): number {
  try {
    const v = Number(localStorage.getItem(winKey(key)));
    return Number.isFinite(v) && v >= 0.2 && v <= 0.8 ? v : 0.5;
  } catch {
    return 0.5;
  }
}

class WorkspaceStore {
  workspaces: WorkspaceWithRole[] = $state([]);
  currentId: Id | null = $state(null);
  sessions: Session[] = $state([]);
  /** Programmatic PTY input keyed by session id, with a bump counter so the
   *  Terminal applies each injection exactly once (e.g. DB rows → running agent). */
  injections: Record<Id, { text: string; n: number }> = $state({});
  sessionsLoading = $state(false);

  /** In-flight workflow runs (pending|running) in the current workspace, for the
   *  "Running" sidebar list + the Workflows nav count chip. Refreshed on each
   *  `workflow_run_updated` WS event and on workspace switch. */
  activeWorkflowRuns: ActiveWorkflowRun[] = $state([]);

  /** view mode for Agent Mode: tabbed (one at a time), tiled (grid), or the
   *  Mission Control work-queue surface. */
  viewMode: 'tabs' | 'tiled' | 'mission' = $state(
    (localStorage.getItem(winKey('otto_view_mode')) as 'tabs' | 'tiled' | 'mission') ?? 'tabs',
  );

  /** In tiled view, a session id to show maximized (zoomed) on its own. */
  maximizedId: Id | null = $state(null);

  /** open session tabs (ids), in tab-bar order */
  openTabs: Id[] = $state([]);
  /** split panes: session ids rendered side by side (1–4) */
  panes: Id[] = $state([]);
  focusedPane = $state(0);
  splitAxis: SplitAxis = $state('col');
  // Split gutter fractions survive reloads (per window, like the other layout
  // state); writes go through setSplitFrac so the drag persists what it sets.
  colFrac = $state(readFrac('otto_split_col_frac'));
  rowFrac = $state(readFrac('otto_split_row_frac'));

  setSplitFrac(axis: SplitAxis, frac: number): void {
    const f = Math.min(0.8, Math.max(0.2, frac));
    if (axis === 'col') this.colFrac = f;
    else this.rowFrac = f;
    try {
      localStorage.setItem(
        winKey(axis === 'col' ? 'otto_split_col_frac' : 'otto_split_row_frac'),
        String(f),
      );
    } catch {
      /* private mode */
    }
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

  myRole: 'viewer' | 'editor' | 'admin' = $derived(this.current?.my_role ?? 'viewer');

  activeSessionId: Id | null = $derived(this.panes[this.focusedPane] ?? null);

  activeSession: Session | null = $derived(
    this.sessions.find((s) => s.id === this.activeSessionId) ?? null,
  );

  /** Active (non-archived) sessions. */
  activeSessions: Session[] = $derived(this.sessions.filter((s) => !s.archived));

  /** Sessions the tiled grid shows: all active EXCEPT background-spawned ones
   *  (Slack/Telegram channels + PR-review agents) the user hasn't explicitly
   *  opened — those stay out of the way so they never interrupt current work.
   *  Review agents are opened on demand from the Review panel's "Open" button. */
  mainSessions: Session[] = $derived(
    // Background-spawned sessions (workflow steps, review agents, vault docs
    // writers, PR drafts, …) live in their own panels and stay out of the tiled
    // grid unless the user explicitly opened them as a tab.
    this.activeSessions.filter((s) => isForeground(s) || this.openTabs.includes(s.id)),
  );

  /** Active agent sessions (claude/codex/shell) — sidebar "Agents" group. */
  agentSessions: Session[] = $derived(
    this.sessions.filter((s) => !s.archived && s.kind === 'agent'),
  );

  /** Active connection sessions (ssh/db/custom) — sidebar "Connections" group. */
  connectionSessions: Session[] = $derived(
    this.sessions.filter((s) => !s.archived && s.kind === 'connection'),
  );

  // ── All-workspaces sidebar view ────────────────────────────────────────────

  /** Sidebar toggle: also list sessions from every OTHER workspace, grouped by
   *  workspace name. Persisted app-wide; default ON. */
  allWorkspaces = $state(localStorage.getItem(LS_ALL_WS) !== '0');

  setAllWorkspaces(on: boolean): void {
    this.allWorkspaces = on;
    localStorage.setItem(LS_ALL_WS, on ? '1' : '0');
    if (on) void this.refreshOtherSessions();
  }

  /** Non-archived sessions from workspaces other than the current one, loaded
   *  by fanning out over the membership list ({@link refreshOtherSessions}).
   *  RBAC holds — each per-workspace list is the same one the user would see
   *  after switching there. */
  otherWsSessions: Session[] = $state([]);

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
   *  view). Failures are per-workspace and silent — one revoked membership
   *  must not blank the rest. */
  async refreshOtherSessions(): Promise<void> {
    if (!this.allWorkspaces) return;
    const others = this.workspaces.filter((w) => w.id !== this.currentId);
    const lists = await Promise.all(
      others.map(async (w) => {
        try {
          return await api.get<Session[]>(`/workspaces/${w.id}/sessions`);
        } catch {
          return [] as Session[];
        }
      }),
    );
    const flat = lists.flat().filter((s) => !s.archived);
    this.otherWsSessions = flat;
    // Seed statuses without clobbering fresher event-fed values.
    for (const s of flat) if (!(s.id in this.statusMap)) this.statusMap[s.id] = s.status;
    // Prune statusMap entries for sessions no longer present anywhere (left
    // workspaces, reaped sessions) so the map doesn't grow without bound.
    const known = new Set<Id>([...this.sessions.map((s) => s.id), ...flat.map((s) => s.id)]);
    for (const id of Object.keys(this.statusMap)) {
      if (!known.has(id)) delete this.statusMap[id];
    }
  }

  /** Open a session that lives in another workspace: switch there, then focus
   *  it (the sidebar's grouped rows route through this). */
  async openInWorkspace(wsId: Id, sessionId: Id): Promise<void> {
    if (wsId !== this.currentId) await this.select(wsId);
    this.navigateToSession(sessionId);
  }

  /** Agent sessions opened from a Telegram chat — sidebar "Telegram" group.
   *  Newest first (RFC3339 last_active_at sorts chronologically) so the
   *  sidebar's "most recent N" cap keeps the freshest tickets visible. */
  telegramSessions: Session[] = $derived(
    this.agentSessions
      .filter((s) => s.meta.channel === 'telegram')
      .sort((a, b) => b.last_active_at.localeCompare(a.last_active_at)),
  );

  /** Agent sessions opened from a Slack chat — sidebar "Slack" group.
   *  Newest first, like {@link telegramSessions}. */
  slackSessions: Session[] = $derived(
    this.agentSessions
      .filter((s) => s.meta.channel === 'slack')
      .sort((a, b) => b.last_active_at.localeCompare(a.last_active_at)),
  );

  /** Agent sessions started locally (not by an engine) — sidebar "Agents"
   *  group. Background sessions (workflow steps, review agents, vault docs
   *  writers, PR drafts, …) run embedded in their own panels and are still
   *  openable from there via `openSession`, which reads `this.sessions`.
   *  `BACKGROUND_SOURCES` mirrors `BACKGROUND_SESSION_SOURCES` in
   *  crates/otto-core/src/domain.rs — the daemon exempts exactly the
   *  foreground complement from auto-pruning, so what the Agents tab shows is
   *  what retention protects. */
  plainAgentSessions: Session[] = $derived(this.agentSessions.filter(isForeground));

  /** Archived sessions — shown in a collapsible "Archived" section. */
  archivedSessions: Session[] = $derived(this.sessions.filter((s) => s.archived));

  // "Working" count for the Agents badge — only foreground agent sessions, not
  // background review/channel ones (those are hidden from the Agents list, so
  // counting them made the badge disagree with the list, e.g. badge 4 / list empty).
  workingCount: number = $derived(
    this.sessions.filter(
      (s) => !s.archived && this.statusMap[s.id] === 'working' && isForeground(s),
    ).length,
  );

  /** Foreground agent sessions currently flagged "needs you" — the sidebar
   *  "Needs you" badge/count (mirrors {@link workingCount}'s scoping). */
  needsYouCount: number = $derived(
    this.sessions.filter(
      (s) => !s.archived && this.needsYou[s.id] === true && isForeground(s),
    ).length,
  );

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
    this.workspaces = await api.get<WorkspaceWithRole[]>('/workspaces');
    const saved = localStorage.getItem(winKey(LS_CURRENT));
    const found = this.workspaces.find((w) => w.id === saved);
    const target = found ?? this.workspaces[0] ?? null;
    if (target) await this.select(target.id);
  }

  async select(id: Id): Promise<void> {
    if (this.currentId === id && this.sessions.length > 0) return;
    this.currentId = id;
    localStorage.setItem(winKey(LS_CURRENT), id);
    await this.refreshSessions();
    void this.refreshActiveWorkflowRuns();
    void this.refreshOtherSessions();
    // restore tabs for this workspace
    const raw = localStorage.getItem(winKey(LS_TABS + id));
    const ids: Id[] = raw ? JSON.parse(raw) : [];
    // Keep real sessions + the DB-Explorer pane sentinel (it has no session row).
    const valid = ids.filter((t) => t === DB_PANE_ID || this.sessions.some((s) => s.id === t));
    this.openTabs = valid;
    // Restore the split layout (pane membership + axis) persisted alongside the
    // tabs, so a 2–4 pane arrangement survives reloads like colFrac/rowFrac do.
    let panes: Id[] = [];
    try {
      const savedLayout = localStorage.getItem(winKey(LS_PANES + id));
      if (savedLayout) {
        const layout = JSON.parse(savedLayout) as { panes?: Id[]; axis?: SplitAxis };
        panes = (layout.panes ?? []).filter((p) => valid.includes(p)).slice(0, 4);
        if (layout.axis === 'col' || layout.axis === 'row') this.splitAxis = layout.axis;
      }
    } catch {
      /* corrupt/private mode — fall through to the single-pane default */
    }
    this.panes = panes.length > 0 ? panes : valid.length > 0 ? [valid[0]] : [];
    this.focusedPane = 0;
  }

  async refreshSessions(): Promise<void> {
    if (!this.currentId) return;
    this.sessionsLoading = true;
    try {
      const all = await api.get<Session[]>(`/workspaces/${this.currentId}/sessions`);
      // Background engine sessions (insights, canvas/db assist, workflow steps,
      // review agents, PR drafts, …) are NOT stripped here: they stay in
      // `this.sessions` so their owning panels can look them up / open them,
      // and every user-facing list filters them via `isForeground` instead —
      // one shared blacklist (`BACKGROUND_SOURCES`) rather than per-list drift.
      let kept = all;
      // Per-device session isolation (opt-in, default off): show only sessions
      // this device started (stamped meta.client_id on create). When off, leave
      // the list unchanged so every device sees every session (current behavior).
      // Drives tabs/Navigator/agents list consistently since they all derive
      // from `this.sessions`. The setter re-runs this so flips apply live.
      if (ui.sessionIsolation) {
        const me = clientId();
        kept = kept.filter(
          (s) => (s.meta as { client_id?: string } | null)?.client_id === me,
        );
      }
      this.sessions = kept;
      for (const s of this.sessions) this.statusMap[s.id] = s.status;
      this.reconcileTabs();
    } finally {
      this.sessionsLoading = false;
    }
  }

  /** Drop tabs/panes that reference a session no longer present (a "phantom"
   *  tab left behind when a session ends or is reaped server-side without a
   *  `session_removed` event reaching this client). Keeps the DB-Explorer
   *  sentinel pane, which has no session row. */
  private reconcileTabs(): void {
    const exists = (t: Id): boolean =>
      t === DB_PANE_ID || this.sessions.some((s) => s.id === t);
    const tabs = this.openTabs.filter(exists);
    if (tabs.length !== this.openTabs.length) {
      this.openTabs = tabs;
      this.persistTabs();
    }
    const panes = this.panes.filter(exists);
    if (panes.length !== this.panes.length) {
      this.panes = panes.length > 0 ? panes : tabs.length > 0 ? [tabs[0]] : [];
      if (this.focusedPane >= this.panes.length) {
        this.focusedPane = Math.max(0, this.panes.length - 1);
      }
      this.persistPanes();
    }
  }

  private persistTabs(): void {
    if (this.currentId) {
      localStorage.setItem(winKey(LS_TABS + this.currentId), JSON.stringify(this.openTabs));
    }
  }

  /** Persist the split layout (pane membership + axis) per workspace, so a
   *  2–4 pane arrangement survives reloads (restored in {@link select}). */
  private persistPanes(): void {
    if (!this.currentId) return;
    try {
      localStorage.setItem(
        winKey(LS_PANES + this.currentId),
        JSON.stringify({ panes: this.panes, axis: this.splitAxis }),
      );
    } catch {
      /* private mode */
    }
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
    if (id !== DB_PANE_ID && !this.sessionsLoading && !this.sessions.some((s) => s.id === id)) {
      return;
    }
    // Opening a session counts as attending to it — drop any "needs you" flag.
    this.clearNeedsYou(id);
    if (!this.openTabs.includes(id)) {
      this.openTabs = [...this.openTabs, id];
      this.persistTabs();
    }
    if (this.panes.length === 0) {
      this.panes = [id];
      this.focusedPane = 0;
    } else {
      this.panes[this.focusedPane] = id;
      this.panes = [...this.panes];
    }
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
    if (s.workspace_id === this.currentId && !this.sessions.some((x) => x.id === s.id)) {
      this.sessions = [...this.sessions, s];
    }
    this.statusMap[s.id] = s.status;
    if (s.workspace_id === this.currentId) this.navigateToSession(s.id);
  }

  /**
   * Register a freshly created session and place it **beside** the current
   * pane(s) (a new split pane) rather than replacing the active tab — used to
   * attach an opened connection terminal next to an agent. Mirrors `addSession`'s
   * bookkeeping but routes the open through `openInSplit`. Returns `false` when
   * the 1–4 pane cap was hit (caller can toast).
   */
  addSessionInSplit(s: Session): boolean {
    if (s.workspace_id === this.currentId && !this.sessions.some((x) => x.id === s.id)) {
      this.sessions = [...this.sessions, s];
    }
    this.statusMap[s.id] = s.status;
    if (s.workspace_id !== this.currentId) return false;
    return this.openInSplit(s.id);
  }

  /**
   * Like {@link createSession} but does NOT route to the new session — for
   * hosts that embed the session where they are (the Browser page's agent
   * dock) and must stay put. Same device stamp + list/status bookkeeping.
   */
  async createSessionQuiet(req: CreateSessionReq): Promise<Session> {
    if (!this.currentId) throw new Error('no workspace selected');
    const stamped: CreateSessionReq = {
      ...req,
      meta: { ...(req.meta ?? {}), client_id: clientId() },
    };
    const s = await api.post<Session>(`/workspaces/${this.currentId}/sessions`, stamped);
    if (s.workspace_id === this.currentId && !this.sessions.some((x) => x.id === s.id)) {
      this.sessions = [...this.sessions, s];
    }
    this.statusMap[s.id] = s.status;
    return s;
  }

  async createSession(req: CreateSessionReq): Promise<Session> {
    if (!this.currentId) throw new Error('no workspace selected');
    // Stamp the device that started this session (preserving any caller meta,
    // e.g. {origin:'manual'}) so the opt-in per-device isolation filter can
    // recognize its own sessions.
    const stamped: CreateSessionReq = {
      ...req,
      meta: { ...(req.meta ?? {}), client_id: clientId() },
    };
    const s = await api.post<Session>(`/workspaces/${this.currentId}/sessions`, stamped);
    this.addSession(s);
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
   *  files are untouched. Switches away first when it's the current one. */
  async archiveWorkspace(id: Id): Promise<void> {
    await api.del(`/workspaces/${id}`);
    this.workspaces = this.workspaces.filter((x) => x.id !== id);
    this.otherWsSessions = this.otherWsSessions.filter((s) => s.workspace_id !== id);
    if (this.currentId === id) {
      const next = this.workspaces[0];
      if (next) await this.select(next.id);
      else {
        this.currentId = null;
        this.sessions = [];
        this.openTabs = [];
        this.panes = [];
      }
    }
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
    const mapped: (Id | null)[] = this.panes.map((p) => (p === id ? fallback : p));
    const panes = mapped.filter((p, i, arr): p is Id => p !== null && arr.indexOf(p) === i);
    this.panes = panes.length > 0 ? panes : fallback ? [fallback] : [];
    this.focusedPane = Math.min(this.focusedPane, Math.max(0, this.panes.length - 1));
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
   * Appearance). The DB pane and already-archived rows just close.
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
  private async endSession(id: Id, action: 'close' | 'archive' | 'delete'): Promise<void> {
    if (action === 'close' || !this.isEndable(id)) {
      this.closeTab(id);
      return;
    }
    try {
      if (action === 'delete') await this.killSession(id);
      else await this.archiveSession(id);
    } catch (e) {
      toasts.error(action === 'delete' ? 'Delete failed' : 'Archive failed', e instanceof Error ? e.message : String(e));
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

  /** Shared confirm step for {@link requestCloseTab}/{@link requestCloseTabs}:
   *  returns 'archive' | 'delete' (or 'close' when nothing needs ending), or
   *  null for cancel. Applies (and records) the remembered preference. */
  private async resolveCloseAction(ids: Id[]): Promise<'close' | 'archive' | 'delete' | null> {
    const ending = ids.filter((id) => this.isEndable(id));
    if (ending.length === 0) return 'close';
    if (ui.closeTabPref === 'archive' || ui.closeTabPref === 'delete') return ui.closeTabPref;
    const many = ending.length > 1;
    const n = ending.length;
    const message = many
      ? `Closing these tabs ends ${n} sessions. Archive stops them and keeps their history (resumable from the Archived list); Delete stops them and removes their history for good.`
      : `Closing this tab ends the session. Archive stops it and keeps its history (resumable from the Archived list); Delete stops it and removes its history for good.`;
    const picked = await confirmer.choose(message, {
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

  /** Reopen the most recently closed tab (⌘⇧T). Skips ids whose session no
   *  longer exists. */
  reopenClosedTab(): void {
    while (this.recentlyClosed.length > 0) {
      const id = this.recentlyClosed[this.recentlyClosed.length - 1];
      this.recentlyClosed = this.recentlyClosed.slice(0, -1);
      if (id === DB_PANE_ID || this.sessions.some((s) => s.id === id)) {
        this.navigateToSession(id);
        return;
      }
    }
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

  closeActiveTab(): void {
    if (this.activeSessionId) void this.requestCloseTab(this.activeSessionId);
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
    if (this.panes.length >= 4 || this.panes.length === 0) return;
    if (this.panes.length === 1) this.splitAxis = axis;
    const cur = this.panes[this.focusedPane];
    this.panes = [...this.panes, cur];
    this.focusedPane = this.panes.length - 1;
    this.persistPanes();
  }

  /**
   * Open a session **beside** the current one(s): append its id to `panes` as a
   * new split pane (respecting the 1–4 cap) and focus it, so it sits side by side
   * with the existing panes rather than replacing the active tab. Used to attach
   * an opened connection terminal next to an agent.
   *
   * Returns `true` if it landed in a pane, or `false` when the 1–4 cap is hit
   * (the caller can surface a toast). Unlike `openSession`, this never replaces
   * the focused pane — except when at the cap, where the focused pane is reused.
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

    // Already on screen → just focus it.
    const existing = this.panes.indexOf(id);
    if (existing >= 0) {
      this.focusedPane = existing;
      return true;
    }
    // Empty layout → this becomes the sole pane.
    if (this.panes.length === 0) {
      this.panes = [id];
      this.focusedPane = 0;
      return true;
    }
    // At the 1–4 cap → reuse the focused pane and report the cap was hit.
    if (this.panes.length >= 4) {
      this.panes[this.focusedPane] = id;
      this.panes = [...this.panes];
      return false;
    }
    // Append as a new pane beside the current one(s) and focus it.
    this.panes = [...this.panes, id];
    this.focusedPane = this.panes.length - 1;
    this.persistPanes();
    return true;
  }

  closePane(idx: number): void {
    if (this.panes.length <= 1) return;
    this.panes = this.panes.filter((_, i) => i !== idx);
    this.focusedPane = Math.min(this.focusedPane, this.panes.length - 1);
    this.persistPanes();
  }

  focusPane(idx: number): void {
    if (idx < 0 || idx >= this.panes.length) return;
    this.focusedPane = idx;
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
    localStorage.setItem(winKey('otto_view_mode'), mode);
  }

  /**
   * Make a set of sessions visible side-by-side: switch to the tiled grid and
   * register them as open tabs (≤4 ⇒ also lay them out as split panes so they
   * tile even in tabs view). Used by the Plan tab to surface its live planning
   * agents the moment they spawn. Unknown ids are tolerated — `reconcileTabs`
   * prunes any that never materialize; `session_created` events fill the rest in.
   */
  tileSessions(ids: Id[]): void {
    const fresh = ids.filter((id) => !this.openTabs.includes(id));
    if (fresh.length > 0) {
      this.openTabs = [...this.openTabs, ...fresh];
      this.persistTabs();
    }
    // Lay out up to 4 as side-by-side panes (the grid shows them all in tiled
    // view; panes give a clean split if the user flips back to tabs view).
    const paneset = [...this.panes];
    for (const id of ids) {
      if (paneset.length >= 4) break;
      if (!paneset.includes(id)) paneset.push(id);
    }
    this.panes = paneset.length > 0 ? paneset : this.panes;
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
  async killSession(id: Id): Promise<void> {
    await api.del(`/sessions/${id}`);
    this.closeTab(id);
    this.sessions = this.sessions.filter((s) => s.id !== id);
    this.otherWsSessions = this.otherWsSessions.filter((s) => s.id !== id);
    delete this.statusMap[id];
    this.clearNeedsYou(id);
  }

  /** Bulk archive (sidebar multi-select): one toast for the batch instead of
   *  N, and one error report — never stops at the first failure. */
  async archiveSessions(ids: Id[]): Promise<number> {
    let failed = 0;
    for (const id of ids) {
      try {
        const s = await api.post<Session>(`/sessions/${id}/archive`);
        this.closeTab(id);
        this.sessions = this.sessions.map((x) => (x.id === id ? s : x));
        this.otherWsSessions = this.otherWsSessions.filter((x) => x.id !== id);
        this.statusMap[id] = s.status;
        this.clearNeedsYou(id);
      } catch {
        failed++;
      }
    }
    const ok = ids.length - failed;
    if (ok > 0) toasts.info(`${ok} session${ok === 1 ? '' : 's'} archived`);
    if (failed > 0) toasts.error('Archive failed', `${failed} session${failed === 1 ? '' : 's'} could not be archived.`);
    return failed;
  }

  /** Bulk delete (sidebar multi-select): caller confirms first. */
  async killSessions(ids: Id[]): Promise<number> {
    let failed = 0;
    for (const id of ids) {
      try { await this.killSession(id); } catch { failed++; }
    }
    if (failed > 0) toasts.error('Delete failed', `${failed} session${failed === 1 ? '' : 's'} could not be deleted.`);
    return failed;
  }

  /** Archive: kill the PTY but keep the row + history in the Archived section. */
  async archiveSession(id: Id): Promise<void> {
    const s = await api.post<Session>(`/sessions/${id}/archive`);
    this.closeTab(id);
    this.sessions = this.sessions.map((x) => (x.id === id ? s : x));
    // Archived rows leave the all-workspaces view (it lists non-archived only).
    this.otherWsSessions = this.otherWsSessions.filter((x) => x.id !== id);
    this.statusMap[id] = s.status;
    toasts.info('Session archived', s.title);
  }

  async unarchiveSession(id: Id): Promise<void> {
    const s = await api.post<Session>(`/sessions/${id}/unarchive`);
    this.sessions = this.sessions.map((x) => (x.id === id ? s : x));
    this.statusMap[id] = s.status;
  }

  async restartSession(id: Id): Promise<void> {
    const s = await api.post<Session>(`/sessions/${id}/restart`);
    this.sessions = this.sessions.map((x) => (x.id === id ? s : x));
    this.statusMap[id] = s.status;
    toasts.info('Session restarted', s.title);
  }

  async renameSession(id: Id, title: string): Promise<void> {
    const s = await api.patch<Session>(`/sessions/${id}`, { title });
    this.sessions = this.sessions.map((x) => (x.id === id ? s : x));
    this.otherWsSessions = this.otherWsSessions.map((x) => (x.id === id ? s : x));
  }

  /** Event-bus feed (WS /ws/events). */
  applyEvent(ev: OttoEvent): void {
    switch (ev.type) {
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
        this.sessions = this.sessions.map((s) =>
          s.id === ev.session_id ? { ...s, status: ev.status } : s,
        );
        this.otherWsSessions = this.otherWsSessions.map((s) =>
          s.id === ev.session_id ? { ...s, status: ev.status } : s,
        );
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
          const s = this.sessions.find((x) => x.id === ev.session_id);
          if (
            s?.meta?.source === 'workflow' &&
            ev.session_id !== this.activeSessionId &&
            this.openTabs.includes(ev.session_id)
          ) {
            this.closeTab(ev.session_id);
          }
        }
        break;
      }
      case 'session_created': {
        const s = ev.session;
        this.statusMap[s.id] = s.status;
        if (s.workspace_id === this.currentId && !this.sessions.some((x) => x.id === s.id)) {
          this.sessions = [...this.sessions, s];
        } else if (
          s.workspace_id !== this.currentId &&
          this.allWorkspaces &&
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
        this.sessions = this.sessions.map((s) =>
          s.id === ev.session_id ? { ...s, meta: ev.meta } : s,
        );
        this.otherWsSessions = this.otherWsSessions.map((s) =>
          s.id === ev.session_id ? { ...s, meta: ev.meta } : s,
        );
        break;
      }
      case 'session_renamed': {
        // A session's title changed — user PATCH or the background auto-namer
        // adopting the CLI's own session title. `session_meta_updated` only
        // carries meta, so this is the only event that refreshes `title` live;
        // pane header and sidebar re-render off `session.title` reactively.
        this.sessions = this.sessions.map((s) =>
          s.id === ev.session_id ? { ...s, title: ev.title } : s,
        );
        this.otherWsSessions = this.otherWsSessions.map((s) =>
          s.id === ev.session_id ? { ...s, title: ev.title } : s,
        );
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
        if (ev.workspace_id === this.currentId) {
          this.sessions = this.sessions.filter((s) => s.id !== ev.session_id);
          if (this.openTabs.includes(ev.session_id)) this.closeTab(ev.session_id);
        } else {
          this.otherWsSessions = this.otherWsSessions.filter((s) => s.id !== ev.session_id);
        }
        break;
      }
      case 'notice': {
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
  }

  async saveNotes(notes: string): Promise<void> {
    if (!this.currentId || !this.current) return;
    const settings = { ...this.current.settings, notes };
    const updated = await api.patch<Workspace>(`/workspaces/${this.currentId}`, { settings });
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
