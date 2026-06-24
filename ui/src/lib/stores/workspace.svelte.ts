// Workspaces + sessions + tab/split state for the shell and Agent Mode.

import { api } from '../api/client';
import { router } from '../router.svelte';
import type {
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
import { ui, clientId } from './ui.svelte';

const LS_CURRENT = 'otto_workspace';
const LS_TABS = 'otto_tabs_'; // + workspace id

/** Sentinel tab/pane id for the docked DB Explorer (not a real session). Lets
 *  the DB Explorer live as a pane in the Agents split, beside an agent. */
export const DB_PANE_ID = '__db_explorer__';

export type SplitAxis = 'col' | 'row';

class WorkspaceStore {
  workspaces: WorkspaceWithRole[] = $state([]);
  currentId: Id | null = $state(null);
  sessions: Session[] = $state([]);
  /** Programmatic PTY input keyed by session id, with a bump counter so the
   *  Terminal applies each injection exactly once (e.g. DB rows → running agent). */
  injections: Record<Id, { text: string; n: number }> = $state({});
  sessionsLoading = $state(false);

  /** view mode for Agent Mode: tabbed (one at a time), tiled (grid), or the
   *  Mission Control work-queue surface. */
  viewMode: 'tabs' | 'tiled' | 'mission' = $state(
    (localStorage.getItem('otto_view_mode') as 'tabs' | 'tiled' | 'mission') ?? 'tabs',
  );

  /** In tiled view, a session id to show maximized (zoomed) on its own. */
  maximizedId: Id | null = $state(null);

  /** open session tabs (ids), in tab-bar order */
  openTabs: Id[] = $state([]);
  /** split panes: session ids rendered side by side (1–4) */
  panes: Id[] = $state([]);
  focusedPane = $state(0);
  splitAxis: SplitAxis = $state('col');
  colFrac = $state(0.5);
  rowFrac = $state(0.5);

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
    this.activeSessions.filter(
      (s) =>
        (s.meta.source !== 'channel' &&
          s.meta.source !== 'review' &&
          s.meta.source !== 'skilleval' &&
          s.meta.source !== 'product-analysis' &&
          s.meta.source !== 'swarm') ||
        this.openTabs.includes(s.id),
    ),
  );

  /** Active agent sessions (claude/codex/shell) — sidebar "Agents" group. */
  agentSessions: Session[] = $derived(
    this.sessions.filter((s) => !s.archived && s.kind === 'agent'),
  );

  /** Active connection sessions (ssh/db/custom) — sidebar "Connections" group. */
  connectionSessions: Session[] = $derived(
    this.sessions.filter((s) => !s.archived && s.kind === 'connection'),
  );

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

  /** Agent sessions started locally (not from a channel or PR review) —
   *  sidebar "Agents" group. Review agents are reached via the Review panel. */
  plainAgentSessions: Session[] = $derived(
    this.agentSessions.filter(
      (s) =>
        s.meta.source !== 'channel' &&
        s.meta.source !== 'review' &&
        s.meta.source !== 'skilleval' &&
        s.meta.source !== 'product-analysis' &&
        s.meta.source !== 'swarm',
    ),
  );

  /** Archived sessions — shown in a collapsible "Archived" section. */
  archivedSessions: Session[] = $derived(this.sessions.filter((s) => s.archived));

  // "Working" count for the Agents badge — only foreground agent sessions, not
  // background review/channel ones (those are hidden from the Agents list, so
  // counting them made the badge disagree with the list, e.g. badge 4 / list empty).
  workingCount: number = $derived(
    this.sessions.filter(
      (s) =>
        !s.archived &&
        this.statusMap[s.id] === 'working' &&
        s.meta.source !== 'review' &&
        s.meta.source !== 'channel' &&
        s.meta.source !== 'skilleval' &&
        s.meta.source !== 'product-analysis' &&
        s.meta.source !== 'swarm',
    ).length,
  );

  /** Foreground agent sessions currently flagged "needs you" — the sidebar
   *  "Needs you" badge/count (mirrors {@link workingCount}'s scoping). */
  needsYouCount: number = $derived(
    this.sessions.filter(
      (s) =>
        !s.archived &&
        this.needsYou[s.id] === true &&
        s.meta.source !== 'review' &&
        s.meta.source !== 'channel' &&
        s.meta.source !== 'skilleval' &&
        s.meta.source !== 'product-analysis' &&
        s.meta.source !== 'swarm',
    ).length,
  );

  /** Flag a session as needing the operator's attention (blocked on input). */
  markNeedsYou(id: Id): void {
    if (this.needsYou[id]) return;
    this.needsYou = { ...this.needsYou, [id]: true };
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
    const saved = localStorage.getItem(LS_CURRENT);
    const found = this.workspaces.find((w) => w.id === saved);
    const target = found ?? this.workspaces[0] ?? null;
    if (target) await this.select(target.id);
  }

  async select(id: Id): Promise<void> {
    if (this.currentId === id && this.sessions.length > 0) return;
    this.currentId = id;
    localStorage.setItem(LS_CURRENT, id);
    await this.refreshSessions();
    // restore tabs for this workspace
    const raw = localStorage.getItem(LS_TABS + id);
    const ids: Id[] = raw ? JSON.parse(raw) : [];
    // Keep real sessions + the DB-Explorer pane sentinel (it has no session row).
    const valid = ids.filter((t) => t === DB_PANE_ID || this.sessions.some((s) => s.id === t));
    this.openTabs = valid;
    this.panes = valid.length > 0 ? [valid[0]] : [];
    this.focusedPane = 0;
  }

  async refreshSessions(): Promise<void> {
    if (!this.currentId) return;
    this.sessionsLoading = true;
    try {
      const all = await api.get<Session[]>(`/workspaces/${this.currentId}/sessions`);
      // Insights runs spawn a throwaway global agent session (meta.source =
      // 'insights') hosted on an arbitrary workspace just for the FK — it's a
      // background scheduled job, not a user session, so keep it out of the
      // Agents list (its report is viewable in the Insights module).
      let kept = all.filter(
        (s) => (s.meta as { source?: string } | null)?.source !== 'insights',
      );
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
    }
  }

  private persistTabs(): void {
    if (this.currentId) {
      localStorage.setItem(LS_TABS + this.currentId, JSON.stringify(this.openTabs));
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

  closeTab(id: Id): void {
    this.openTabs = this.openTabs.filter((t) => t !== id);
    this.persistTabs();
    // panes showing this session fall back to another tab or collapse
    const fallback = this.openTabs[this.openTabs.length - 1] ?? null;
    const mapped: (Id | null)[] = this.panes.map((p) => (p === id ? fallback : p));
    const panes = mapped.filter((p, i, arr): p is Id => p !== null && arr.indexOf(p) === i);
    this.panes = panes.length > 0 ? panes : fallback ? [fallback] : [];
    this.focusedPane = Math.min(this.focusedPane, Math.max(0, this.panes.length - 1));
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
    if (this.activeSessionId) this.closeTab(this.activeSessionId);
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
    return true;
  }

  closePane(idx: number): void {
    if (this.panes.length <= 1) return;
    this.panes = this.panes.filter((_, i) => i !== idx);
    this.focusedPane = Math.min(this.focusedPane, this.panes.length - 1);
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
    localStorage.setItem('otto_view_mode', mode);
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
    delete this.statusMap[id];
    this.clearNeedsYou(id);
  }

  /** Archive: kill the PTY but keep the row + history in the Archived section. */
  async archiveSession(id: Id): Promise<void> {
    const s = await api.post<Session>(`/sessions/${id}/archive`);
    this.closeTab(id);
    this.sessions = this.sessions.map((x) => (x.id === id ? s : x));
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
  }

  /** Event-bus feed (WS /ws/events). */
  applyEvent(ev: OttoEvent): void {
    switch (ev.type) {
      case 'session_status': {
        this.statusMap[ev.session_id] = ev.status;
        this.sessions = this.sessions.map((s) =>
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
        break;
      }
      case 'session_created': {
        const s = ev.session;
        this.statusMap[s.id] = s.status;
        if (s.workspace_id === this.currentId && !this.sessions.some((x) => x.id === s.id)) {
          this.sessions = [...this.sessions, s];
        }
        break;
      }
      case 'session_meta_updated': {
        // Replace the cached session's meta in place (e.g. live handover flags).
        this.sessions = this.sessions.map((s) =>
          s.id === ev.session_id ? { ...s, meta: ev.meta } : s,
        );
        break;
      }
      case 'session_removed': {
        delete this.statusMap[ev.session_id];
        this.clearNeedsYou(ev.session_id);
        if (ev.workspace_id === this.currentId) {
          this.sessions = this.sessions.filter((s) => s.id !== ev.session_id);
          if (this.openTabs.includes(ev.session_id)) this.closeTab(ev.session_id);
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
