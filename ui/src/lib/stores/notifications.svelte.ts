// Notification center: persisted notices + unread count + settings.
//
// Fed by REST (`/notifications`) on load and by the events WS (`notification`
// event, routed from events.svelte.ts). Warn/error notices optionally raise a
// native OS notification via the Tauri notification plugin.

import { untrack } from 'svelte';
import { api } from '../api/client';
import type {
  BulkNoticeResult,
  Notice,
  NoticeAction,
  NoticeSeverity,
  NotificationSettings,
} from '../api/types';
import { toasts } from '../toast.svelte';
import { openExternal } from '../external';
import { ws } from './workspace.svelte';
import { router } from '../router.svelte';
import { parseNoticeRoute } from '../noticeRoute';
import { isEmbedded } from '../desktop';
import { toastError } from '../toastError';
import { confirmer } from '../confirm.svelte';

/** Most ids one bulk read / dismiss call may carry (daemon `BULK_MAX`). */
const BULK_MAX = 500;

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

/** The lifecycle suffixes of a per-session notice's `session:{id}:{suffix}`
 *  source key (monitor.rs idle/exited/waiting, activity.rs waiting/tasks_done). */
export type SessionNoticeState = 'waiting' | 'idle' | 'exited' | 'tasks_done' | 'other';

/**
 * One row of the notification center. Per-session notices are GROUPED: a
 * session that went idle, got nudged, then ended used to leave three
 * near-identical rows ("Session awaiting input" + a body repeating the session
 * name); it is now one row titled with the session, its latest state winning,
 * and a ×N count of the notices folded into it. Everything else is one row per
 * notice.
 */
export interface NoticeRow {
  /** `session:{id}` for a grouped session row, else the notice id. */
  key: string;
  /** Newest first; `notices[0]` is the one that speaks for the row. */
  notices: Notice[];
  latest: Notice;
  unread: boolean;
  /** Highest severity across the group. */
  severity: NoticeSeverity;
  title: string;
  /** Short secondary line (session rows) or the notice body. */
  text: string;
  sessionId: string | null;
  provider: string | null;
  state: SessionNoticeState | null;
  /** 0 = a session needs you · 1 = warn/error alert · 2 = everything else. */
  bucket: 0 | 1 | 2;
}

const SEV_RANK: Record<NoticeSeverity, number> = { info: 0, warn: 1, error: 2 };

function newestFirst(a: Notice, b: Notice): number {
  return new Date(b.created_at).getTime() - new Date(a.created_at).getTime();
}

/** `session:{id}:{suffix}` → `{ id, state }`; null for any other key. */
export function parseSessionKey(key: string | null | undefined): { id: string; state: SessionNoticeState } | null {
  if (!key?.startsWith('session:')) return null;
  const parts = key.split(':');
  if (parts.length < 3) return null;
  const suffix = parts[parts.length - 1];
  const id = parts.slice(1, -1).join(':');
  if (!id) return null;
  const known: SessionNoticeState[] = ['waiting', 'idle', 'exited', 'tasks_done'];
  return { id, state: (known as string[]).includes(suffix) ? (suffix as SessionNoticeState) : 'other' };
}

/** Recover the session title/provider from a notice body when the session is
 *  no longer in the workspace list: monitor.rs writes "«title» (provider) is
 *  idle… / has ended…", activity.rs writes "«title» · detail". */
function parseSessionBody(body: string): { title: string | null; provider: string | null; detail: string | null } {
  const m = body.match(/^(.+?) \(([^()]+)\) (?:is idle|has ended)/);
  const dot = body.indexOf(' · ');
  const detail = dot >= 0 ? body.slice(dot + 3).trim() || null : null;
  if (m) return { title: m[1], provider: m[2], detail };
  if (dot > 0) return { title: body.slice(0, dot), provider: null, detail };
  return { title: null, provider: null, detail: null };
}

function sessionStateLabel(state: SessionNoticeState, latest: Notice): string {
  switch (state) {
    case 'waiting':
      return latest.severity === 'info' ? 'Waiting for your input' : 'Needs your attention';
    case 'idle':
      return 'Idle';
    case 'exited':
      return 'Ended';
    case 'tasks_done':
      return 'Finished its tasks';
    default:
      return latest.title;
  }
}

/** Fold the flat notice list into center rows (see `NoticeRow`). Pure except
 *  for the session lookup, so the store can `$derived` it. */
export function buildRows(
  notices: Notice[],
  findSession: (id: string) => { title: string; provider: string } | undefined,
): NoticeRow[] {
  const groups = new Map<string, Notice[]>();
  const rows: NoticeRow[] = [];
  for (const n of notices) {
    const sk = parseSessionKey(n.source_key);
    if (n.kind === 'session' && sk) {
      const key = `session:${sk.id}`;
      const g = groups.get(key);
      if (g) g.push(n);
      else groups.set(key, [n]);
      continue;
    }
    rows.push({
      key: n.id,
      notices: [n],
      latest: n,
      unread: !n.read,
      severity: n.severity,
      title: n.title,
      text: n.body,
      sessionId: null,
      provider: null,
      state: null,
      bucket: n.severity === 'info' ? 2 : 1,
    });
  }
  for (const [key, group] of groups) {
    group.sort(newestFirst);
    const latest = group[0];
    const sk = parseSessionKey(latest.source_key)!;
    const live = findSession(sk.id);
    const parsed = parseSessionBody(latest.body);
    const label = sessionStateLabel(sk.state, latest);
    const severity = group.reduce<NoticeSeverity>(
      (hi, n) => (SEV_RANK[n.severity] > SEV_RANK[hi] ? n.severity : hi),
      'info',
    );
    rows.push({
      key,
      notices: group,
      latest,
      unread: group.some((n) => !n.read),
      severity,
      title: live?.title || parsed.title || 'Session',
      text: parsed.detail ? `${label} · ${parsed.detail}` : label,
      sessionId: sk.id,
      provider: live?.provider ?? parsed.provider,
      state: sk.state,
      bucket: sk.state === 'waiting' ? 0 : latest.severity === 'info' ? 2 : 1,
    });
  }
  return rows.sort((a, b) => a.bucket - b.bucket || newestFirst(a.latest, b.latest));
}

const DEFAULT_SETTINGS: NotificationSettings = {
  expiry_threshold_days: 7,
  native_enabled: true,
  session_events: true,
  native_on_waiting: true,
};

/** Client-side ceiling on held notices: the server list is capped at 200, but
 *  live `ingest` only prepends, so a long day would otherwise grow forever. */
export const NOTICE_CAP = 300;

/** Trim `list` (newest first) to `cap`, dropping the oldest READ notices
 *  first; unread ones go only when they alone exceed the cap. */
export function capNotices(list: Notice[], cap = NOTICE_CAP): Notice[] {
  let excess = list.length - cap;
  if (excess <= 0) return list;
  const drop = new Set<Notice>();
  for (let i = list.length - 1; i >= 0 && excess > 0; i--) {
    if (list[i].read) {
      drop.add(list[i]);
      excess--;
    }
  }
  const kept = drop.size ? list.filter((n) => !drop.has(n)) : list;
  return kept.length > cap ? kept.slice(0, cap) : kept;
}

/** Merge notices the event stream delivered while a `/notifications` GET was
 *  in flight (`ingested`, oldest first) into its snapshot (`fetched`, newest
 *  first). The snapshot may predate them: an ingested notice the snapshot
 *  lacks — or carries an OLDER copy of (a re-fired `:waiting` keeps its id
 *  with a newer `created_at`) — goes back on top; a snapshot copy at least as
 *  new is the server's truth (read state included) and wins. */
export function mergeInFlight(fetched: Notice[], ingested: Notice[]): Notice[] {
  let out = fetched;
  for (const n of ingested) {
    const have = out.find((x) => x.id === n.id);
    if (have && have.created_at >= n.created_at) continue;
    out = [n, ...out.filter((x) => x.id !== n.id)];
  }
  return capNotices(out);
}

/** Internal marker: a load answered for a previous identity (dropped). */
const STALE = Symbol('stale-identity');

class NotificationStore {
  /** Raw: every write replaces the array (and edited notices) wholesale. */
  notices: Notice[] = $state.raw([]);
  settings: NotificationSettings = $state({ ...DEFAULT_SETTINGS });
  loading = $state(false);
  loaded = $state(false);
  /** Last load failure (null once a load succeeds) — the bell shows it with a
   *  Retry instead of an empty "all caught up" that would be a lie. */
  error: string | null = $state(null);

  /** Number of unread notices. */
  unread: number = $derived(this.notices.filter((n) => !n.read).length);

  /** What rows read from the session list — a status flip (the common
   *  `session_status` event) leaves it equal, so the rows don't rebuild. */
  private sessionLabels: string = $derived(ws.sessions.map((s) => `${s.id}\u0001${s.title}\u0001${s.provider}`).join('\u0002'));

  /** Center rows: session notices grouped per session (see `buildRows`). */
  rows: NoticeRow[] = $derived.by(() => {
    void this.sessionLabels;
    const byId = new Map(untrack(() => ws.sessions).map((s) => [s.id, s] as const));
    return buildRows(this.notices, (id) => byId.get(id));
  });

  /** Unread ROWS — drives the bell badge, so it matches what the panel lists. */
  unreadRows: number = $derived(this.rows.filter((r) => r.unread).length);

  /** Highest severity among unread notices (null when all read) — colours the
   *  badge, so an info-only backlog doesn't shout in red. */
  unreadSeverity: NoticeSeverity | null = $derived(
    this.notices.reduce<NoticeSeverity | null>(
      (hi, n) => (n.read ? hi : !hi || SEV_RANK[n.severity] > SEV_RANK[hi] ? n.severity : hi),
      null,
    ),
  );

  /** Whether we've already asked the OS for notification permission this run. */
  private permissionRequested = false;

  /** The first load in flight, for {@link ensureLoaded} to join. */
  private inflight: Promise<void> | null = null;

  /** Make sure notices are loaded at least once: joins a load already in
   *  flight instead of queueing a second fetch (Home and Settings mount while
   *  the bell's first load is still out — a queued reload doubled boot's
   *  `/notifications` + `/notifications/settings` requests). Use {@link load}
   *  when newer server state is wanted (reconnect resync, Retry). */
  ensureLoaded(): Promise<void> {
    if (untrack(() => this.loaded)) return Promise.resolve();
    const running = untrack(() => this.inflight);
    if (running) return running;
    return this.load();
  }

  /** Load notices + settings from the daemon. Safe to call more than once. */
  load(): Promise<void> {
    // Already fetching: loadOnce queues one trailing reload; hand back the
    // running load rather than recording the queued no-op as "in flight".
    const running = untrack(() => this.inflight);
    if (running && untrack(() => this.loading)) {
      void this.loadOnce();
      return running;
    }
    const p = this.loadOnce();
    this.inflight = p;
    void p.finally(() => {
      if (this.inflight === p) this.inflight = null;
    });
    return p;
  }

  private async loadOnce(): Promise<void> {
    // Read the re-entrancy guard UNtracked: callers like NotificationBell wrap
    // this in a bare `$effect(() => void notifications.load())` intending a
    // load-once-on-mount. Reading `this.loading` inside that effect's tracking
    // scope would subscribe the effect to it, and the `finally { this.loading =
    // false }` below would then re-trigger the effect after every fetch — an
    // infinite `GET /notifications` loop that pegs the webview + daemon. untrack
    // keeps the overlap guard working without leaking it as a dependency.
    // A load requested while one is in flight (a reconnect resync racing the
    // bell's mount) may carry newer server state: queue ONE trailing reload
    // instead of dropping it.
    if (untrack(() => this.loading)) {
      this.reloadQueued = true;
      return;
    }
    this.loading = true;
    // Notices ingested from the event stream while the GET is in flight may
    // be newer than its snapshot — merged back below, never overwritten.
    const since = this.ingestSeq;
    const epoch = this.identityEpoch;
    try {
      const [notices, settings] = await Promise.all([
        api.get<Notice[]>('/notifications'),
        api.get<NotificationSettings>('/notifications/settings').catch(() => this.settings),
      ]);
      // Answered for the previous identity: drop it (the queued reload
      // below fetches the new identity's notices).
      if (epoch !== this.identityEpoch) throw STALE;
      const fetched = notices.filter((n) => !this.isChannelSessionNotice(n));
      this.notices = mergeInFlight(fetched, this.ingestedSince(since));
      this.settings = settings;
      this.loaded = true;
      try {
        this.restoreNeedsYou();
      } catch {
        /* a best-effort re-derivation — never fail the load over it */
      }
      this.error = null;
    } catch (e) {
      // A previous identity's answer (or failure) is dropped; the queued
      // reload below runs for the new one.
      if (e !== STALE && epoch === this.identityEpoch) {
        // Backend may not be ready yet (the events WS reloads on connect) — keep
        // whatever we had and surface the failure so the bell can offer Retry.
        this.error = (e instanceof Error ? e.message : String(e)) || 'Request failed';
      }
    } finally {
      this.loading = false;
    }
    if (this.reloadQueued) {
      this.reloadQueued = false;
      await this.load();
    }
  }

  private reloadQueued = false;
  /** Bumped on an identity change; a load answered for the old one is dropped. */
  private identityEpoch = 0;

  /** The signed-in identity changed (impersonate / stop / re-login, S13-02):
   *  notices are per user on the daemon, so the previous identity's list,
   *  in-flight load and ingest log must not mix with the new one's. Clears
   *  (a load in flight is dropped); the caller then calls `load()`, which
   *  queues behind a dropped in-flight one. */
  resetForIdentity(): void {
    this.identityEpoch += 1;
    this.notices = [];
    this.ingestLog = [];
    this.loaded = false;
    this.error = null;
  }
  /** Event-stream ingests, in order, for {@link load}'s in-flight merge. */
  private ingestSeq = 0;
  private ingestLog: { seq: number; notice: Notice }[] = [];

  private ingestedSince(seq: number): Notice[] {
    return this.ingestLog.filter((e) => e.seq > seq).map((e) => e.notice);
  }

  /** The live "needs you" flag is raised from the `:waiting` WS notice (see
   *  events.svelte.ts), so a reload — or a waiting notice that arrived while
   *  the app was closed — left the bell saying "Waiting for your input" while
   *  the sidebar and Home's "Needs you" said nothing. Re-derive it from the
   *  loaded list: a session whose LATEST notice is an unread `:waiting` one,
   *  and that isn't working now, still needs the operator. */
  private restoreNeedsYou(): void {
    const latest = new Map<string, Notice>();
    for (const n of this.notices) {
      const parsed = parseSessionKey(n.source_key);
      if (!parsed) continue;
      const prev = latest.get(parsed.id);
      if (!prev || Date.parse(n.created_at) > Date.parse(prev.created_at)) latest.set(parsed.id, n);
    }
    for (const [sid, n] of latest) {
      if (n.read || !n.source_key?.endsWith(':waiting') || n.action?.type !== 'open_session') continue;
      const st = ws.statusMap[sid];
      if (st === 'working' || st === 'exited') continue;
      ws.markNeedsYou(sid);
    }
  }

  /**
   * Handle an incoming `notification` WS event: prepend the notice, bump the
   * unread count, and (when enabled) raise a native OS notification for
   * warn/error severities.
   */
  /** Background channel (Slack/Telegram) sessions end after every reply, so
   *  their "session ended / awaiting input" notices would flood the center.
   *  Suppress notices that target a channel-spawned session. */
  private isChannelSessionNotice(notice: Notice): boolean {
    if (notice.kind !== 'session') return false;
    const key = notice.source_key ?? '';
    const sid = key.startsWith('session:') ? key.split(':')[1] : null;
    if (!sid) return false;
    return ws.getSession(sid)?.meta?.source === 'channel';
  }

  ingest(notice: Notice): void {
    // The daemon de-dupes on `source_key`: a re-fired notice (a session that is
    // waiting AGAIN) comes back with the SAME id, refreshed in place — unread,
    // new body/severity/created_at. Replace ours (and move it to the top) or
    // the bell keeps showing it read while the server says unread. Only an
    // identical re-delivery is ignored.
    const known = this.notices.find((n) => n.id === notice.id);
    if (known && known.created_at === notice.created_at) return;
    if (this.isChannelSessionNotice(notice)) return;
    this.ingestLog = [...this.ingestLog.slice(-49), { seq: ++this.ingestSeq, notice }];
    this.notices = capNotices([notice, ...this.notices.filter((n) => n.id !== notice.id)]);
    if (this.wantsNative(notice)) void this.fireNative(notice);
  }

  /** Whether `notice` earns a native banner: any warn/error, plus — behind
   *  `native_on_waiting` (A2) — an info "Session awaiting input" (`:waiting`)
   *  for a session the user is NOT watching (window hidden / unfocused, or
   *  another session active). Codex, agy and custom providers only ever
   *  produce that info notice, so without this they never banner. */
  wantsNative(notice: Notice): boolean {
    if (!this.settings.native_enabled) return false;
    if (notice.severity === 'warn' || notice.severity === 'error') return true;
    if (this.settings.native_on_waiting === false) return false;
    if (!(notice.source_key ?? '').endsWith(':waiting')) return false;
    const action = notice.action;
    if (action?.type !== 'open_session') return false;
    const doc = typeof document !== 'undefined' ? document : null;
    const away = doc ? doc.hidden || !doc.hasFocus() : false;
    return away || ws.activeSessionId !== action.session_id;
  }

  // ── Native OS notification (Tauri only) ───────────────────────────────────

  private async fireNative(notice: Notice): Promise<void> {
    // The side-by-side pane (an iframe) sees the same notices as its window:
    // one native banner per notice, from the main document only.
    if (!isTauri || isEmbedded) return;
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      // Returns true | false | null (null = "prompt", not yet decided).
      let granted = await invoke<boolean | null>('plugin:notification|is_permission_granted');
      if (granted !== true && !this.permissionRequested) {
        this.permissionRequested = true;
        const perm = await invoke<string>('plugin:notification|request_permission');
        granted = perm === 'granted';
      }
      if (granted !== true) return;
      await invoke('plugin:notification|notify', {
        options: { title: notice.title, body: notice.body },
      });
    } catch {
      // Plugin unavailable / denied — silently skip; the in-app center still has it.
    }
  }

  // ── Mutations (API + optimistic local state) ──────────────────────────────

  async markRead(id: string): Promise<void> {
    const target = this.notices.find((n) => n.id === id);
    if (!target || target.read) return;
    this.notices = this.notices.map((n) => (n.id === id ? { ...n, read: true } : n));
    try {
      await api.post(`/notifications/${id}/read`);
    } catch {
      this.notices = this.notices.map((n) => (n.id === id ? { ...n, read: false } : n));
    }
  }

  async markAllRead(): Promise<void> {
    if (this.notices.every((n) => n.read)) return;
    const prev = this.notices;
    this.notices = this.notices.map((n) => (n.read ? n : { ...n, read: true }));
    try {
      await api.post('/notifications/read-all');
    } catch {
      this.notices = prev;
    }
  }

  /** Mark every notice this client currently holds as read — what the open
   *  panel showed. One bulk call (the client holds ≤ 200); a backlog past the
   *  daemon's bulk cap falls back to one read-all call. */
  async markSeenRead(): Promise<void> {
    const ids = this.notices.filter((n) => !n.read).map((n) => n.id);
    if (ids.length === 0) return;
    if (ids.length > BULK_MAX) return this.markAllRead();
    await this.markManyRead(ids);
  }

  /** Mark a group of notices read (a grouped session row / the seen set) in
   *  ONE request + one `notifications_changed` broadcast (perf §15 N4). */
  async markManyRead(ids: string[]): Promise<void> {
    const want = new Set(ids.filter((id) => this.notices.some((n) => n.id === id && !n.read)));
    if (want.size === 0) return;
    this.notices = this.notices.map((n) => (want.has(n.id) ? { ...n, read: true } : n));
    try {
      await api.post<BulkNoticeResult>('/notifications/read', { ids: [...want] });
    } catch {
      this.notices = this.notices.map((n) => (want.has(n.id) ? { ...n, read: false } : n));
    }
  }

  /** Dismiss a group of notices (a grouped session row) in ONE request. */
  async dismissMany(ids: string[]): Promise<void> {
    const drop = new Set(ids);
    const prev = this.notices;
    const back = prev.filter((n) => drop.has(n.id));
    if (back.length === 0) return;
    this.notices = this.notices.filter((n) => !drop.has(n.id));
    try {
      await api.post<BulkNoticeResult>('/notifications/dismiss', { ids: back.map((n) => n.id) });
    } catch {
      // Nothing was removed (one statement): put the group back.
      const kept = new Set(this.notices.map((n) => n.id));
      this.notices = [...back.filter((n) => !kept.has(n.id)), ...this.notices].sort(newestFirst);
    }
  }

  async dismiss(id: string): Promise<void> {
    const prev = this.notices;
    this.notices = this.notices.filter((n) => n.id !== id);
    try {
      await api.del(`/notifications/${id}`);
    } catch {
      this.notices = prev;
    }
  }

  async clear(): Promise<void> {
    if (this.notices.length === 0) return;
    const prev = this.notices;
    this.notices = [];
    try {
      await api.del('/notifications');
    } catch {
      this.notices = prev;
    }
  }

  async saveSettings(next: NotificationSettings): Promise<void> {
    const prev = this.settings;
    this.settings = next;
    try {
      this.settings = await api.put<NotificationSettings>('/notifications/settings', next);
    } catch (e) {
      // Revert AND say so — a silent revert looks like the toggle "didn't take".
      this.settings = prev;
      toastError('Couldn’t save notification settings', e);
    }
  }

  // ── Actions ───────────────────────────────────────────────────────────────

  /**
   * Run a notice's action button. Marks the notice read as a side effect.
   * - open_url   → open in the system browser
   * - open_session → focus the session + jump to the Agents module
   * - reauth     → toast guidance (the actual re-auth happens in a terminal)
   * - open_route → open the run / task / loop an automation notice is about
   */
  async runAction(notice: Notice): Promise<void> {
    void this.markRead(notice.id);
    const action = notice.action;
    if (!action) return;
    await this.dispatch(action);
  }

  private async dispatch(action: NoticeAction): Promise<void> {
    switch (action.type) {
      case 'open_url':
        await openExternal(action.url);
        break;
      case 'open_session':
        await this.openSession(action.session_id);
        break;
      case 'reauth':
        this.guideReauth(action.target);
        break;
      case 'open_route':
        await this.openRoute(action.route, action.workspace_id ?? null);
        break;
    }
  }

  /** A session notice ("needs you", finished, …): open the session wherever it
   *  is. The main list carries only this workspace's sidebar sessions, so
   *  fetch others by id — then switch to ITS workspace, or (archived since
   *  the notice fired) offer to bring it back. */
  private async openSession(id: string): Promise<void> {
    const row = ws.getSession(id) ?? (await ws.ensureSession(id));
    if (!row) {
      toasts.warn('Session unavailable', 'It may have been deleted, or you no longer have access.');
      return;
    }
    const elsewhere = !ws.getSession(id) && row.workspace_id !== ws.currentId;
    if (elsewhere && !ws.workspaces.some((w) => w.id === row.workspace_id)) {
      toasts.warn('Workspace unavailable', 'It may have been removed, or you no longer have access.');
      return;
    }
    if (row.archived) {
      const name = row.title?.trim() || 'This session';
      const ok = await confirmer.ask(
        `“${name}” was archived. Unarchive it and open it? It resumes its saved conversation where it can.`,
        { title: 'Session archived', confirmLabel: 'Unarchive and open' },
      );
      if (!ok) return;
      try {
        await ws.unarchiveSession(id);
      } catch (e) {
        toastError('Couldn’t unarchive the session', e);
        return;
      }
    }
    if (elsewhere) await ws.openInWorkspace(row.workspace_id, id);
    else ws.navigateToSession(id);
  }

  /** Automation notices (failed task / workflow run / goal loop, workflow
   *  awaiting approval): switch to the item's workspace, then open the page
   *  and the item itself through its page port. */
  private async openRoute(route: string, workspaceId: string | null): Promise<void> {
    try {
      if (workspaceId && ws.currentId !== workspaceId) {
        if (!ws.workspaces.some((w) => w.id === workspaceId)) {
          toasts.warn('Workspace unavailable', 'It may have been removed, or you no longer have access.');
          return;
        }
        if (!(await ws.select(workspaceId))) return;
      }
      const target = parseNoticeRoute(route);
      const signal = new AbortController().signal;
      // The item's own URL (`<module>/<id>`) selects it — the same link a
      // reload, Back or a shared address restores. The page port only opens
      // what the URL doesn't carry (a workflow's run, a task's expanded row).
      switch (target.kind) {
        case 'workflow_run': {
          const { workflowsPagePort } = await import('../uiCommands/workflows');
          router.go(`workflows/${encodeURIComponent(target.workflowId)}`);
          const page = await workflowsPagePort.get(signal);
          if (await page.open(target.workflowId)) await page.openRun(target.workflowId, target.runId);
          break;
        }
        case 'scheduled_task': {
          const { scheduledTasksPort } = await import('../uiCommands/scheduled');
          router.go(`scheduled-tasks/${encodeURIComponent(target.taskId)}`);
          (await scheduledTasksPort.get(signal)).expand(target.taskId);
          break;
        }
        case 'goal_loop':
          router.go(`loops/${encodeURIComponent(target.loopId)}`);
          break;
        case 'route':
          // proof/<id>, swarm/<id>, product/<id>, workflows/<id>, … as sent.
          router.go(target.route);
          break;
      }
    } catch (e) {
      toastError('Couldn’t open it', e);
    }
  }

  private guideReauth(target: string): void {
    const map: Record<string, string> = {
      claude: 'Run `claude login` in a terminal to re-authenticate.',
      codex: 'Run `codex login` in a terminal to re-authenticate.',
    };
    let guidance = map[target];
    if (!guidance) {
      if (target.startsWith('git:')) guidance = 'Update the git account token in Settings → Git.';
      else if (target.startsWith('issue:')) guidance = 'Update the issue account token in Settings → Issues.';
      else guidance = `Re-authenticate ${target} to continue.`;
    }
    toasts.info('Re-authentication needed', guidance);
  }
}

export const notifications = new NotificationStore();
