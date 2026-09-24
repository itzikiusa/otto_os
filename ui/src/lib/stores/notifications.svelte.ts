// Notification center: persisted notices + unread count + settings.
//
// Fed by REST (`/notifications`) on load and by the events WS (`notification`
// event, routed from events.svelte.ts). Warn/error notices optionally raise a
// native OS notification via the Tauri notification plugin.

import { untrack } from 'svelte';
import { api } from '../api/client';
import type { Notice, NoticeAction, NoticeSeverity, NotificationSettings } from '../api/types';
import { toasts } from '../toast.svelte';
import { openExternal } from '../external';
import { ws } from './workspace.svelte';

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
};

class NotificationStore {
  notices: Notice[] = $state([]);
  settings: NotificationSettings = $state({ ...DEFAULT_SETTINGS });
  loading = $state(false);
  loaded = $state(false);
  /** Last load failure (null once a load succeeds) — the bell shows it with a
   *  Retry instead of an empty "all caught up" that would be a lie. */
  error: string | null = $state(null);

  /** Number of unread notices. */
  unread: number = $derived(this.notices.filter((n) => !n.read).length);

  /** Center rows: session notices grouped per session (see `buildRows`). */
  rows: NoticeRow[] = $derived(
    buildRows(this.notices, (id) => ws.sessions.find((s) => s.id === id)),
  );

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

  /** Load notices + settings from the daemon. Safe to call more than once. */
  async load(): Promise<void> {
    // Read the re-entrancy guard UNtracked: callers like NotificationBell wrap
    // this in a bare `$effect(() => void notifications.load())` intending a
    // load-once-on-mount. Reading `this.loading` inside that effect's tracking
    // scope would subscribe the effect to it, and the `finally { this.loading =
    // false }` below would then re-trigger the effect after every fetch — an
    // infinite `GET /notifications` loop that pegs the webview + daemon. untrack
    // keeps the overlap guard working without leaking it as a dependency.
    if (untrack(() => this.loading)) return;
    this.loading = true;
    try {
      const [notices, settings] = await Promise.all([
        api.get<Notice[]>('/notifications'),
        api.get<NotificationSettings>('/notifications/settings').catch(() => this.settings),
      ]);
      this.notices = notices.filter((n) => !this.isChannelSessionNotice(n));
      this.settings = settings;
      this.loaded = true;
      this.error = null;
    } catch (e) {
      // Backend may not be ready yet (the events WS reloads on connect) — keep
      // whatever we had and surface the failure so the bell can offer Retry.
      this.error = (e instanceof Error ? e.message : String(e)) || 'Request failed';
    } finally {
      this.loading = false;
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
    return ws.sessions.find((x) => x.id === sid)?.meta?.source === 'channel';
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
    this.notices = [notice, ...this.notices.filter((n) => n.id !== notice.id)];
    if (
      this.settings.native_enabled &&
      (notice.severity === 'warn' || notice.severity === 'error')
    ) {
      void this.fireNative(notice);
    }
  }

  // ── Native OS notification (Tauri only) ───────────────────────────────────

  private async fireNative(notice: Notice): Promise<void> {
    if (!isTauri) return;
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

  /** Mark a group of notices read (a grouped session row). */
  async markManyRead(ids: string[]): Promise<void> {
    await Promise.all(ids.map((id) => this.markRead(id)));
  }

  /** Dismiss a group of notices (a grouped session row). */
  async dismissMany(ids: string[]): Promise<void> {
    const drop = new Set(ids);
    const prev = this.notices;
    this.notices = this.notices.filter((n) => !drop.has(n.id));
    const results = await Promise.allSettled(ids.map((id) => api.del(`/notifications/${id}`)));
    const failed = new Set(ids.filter((_, i) => results[i].status === 'rejected'));
    if (failed.size > 0) {
      // Put back only what the daemon still has.
      const back = prev.filter((n) => failed.has(n.id));
      this.notices = [...back, ...this.notices].sort(newestFirst);
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
    } catch {
      this.settings = prev;
    }
  }

  // ── Actions ───────────────────────────────────────────────────────────────

  /**
   * Run a notice's action button. Marks the notice read as a side effect.
   * - open_url   → open in the system browser
   * - open_session → focus the session + jump to the Agents module
   * - reauth     → toast guidance (the actual re-auth happens in a terminal)
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
      case 'open_session': {
        const found = ws.sessions.some((s) => s.id === action.session_id);
        if (!found) {
          toasts.warn('Session unavailable', 'It may have been closed or belongs to another workspace.');
          return;
        }
        ws.navigateToSession(action.session_id);
        break;
      }
      case 'reauth':
        this.guideReauth(action.target);
        break;
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
