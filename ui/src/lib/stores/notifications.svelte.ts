// Notification center: persisted notices + unread count + settings.
//
// Fed by REST (`/notifications`) on load and by the events WS (`notification`
// event, routed from events.svelte.ts). Warn/error notices optionally raise a
// native OS notification via the Tauri notification plugin.

import { api } from '../api/client';
import type { Notice, NoticeAction, NotificationSettings } from '../api/types';
import { toasts } from '../toast.svelte';
import { openExternal } from '../external';
import { router } from '../router.svelte';
import { ws } from './workspace.svelte';

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

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

  /** Number of unread notices — drives the bell badge. */
  unread: number = $derived(this.notices.filter((n) => !n.read).length);

  /** Whether we've already asked the OS for notification permission this run. */
  private permissionRequested = false;

  /** Load notices + settings from the daemon. Safe to call more than once. */
  async load(): Promise<void> {
    if (this.loading) return;
    this.loading = true;
    try {
      const [notices, settings] = await Promise.all([
        api.get<Notice[]>('/notifications'),
        api.get<NotificationSettings>('/notifications/settings').catch(() => this.settings),
      ]);
      this.notices = notices.filter((n) => !this.isChannelSessionNotice(n));
      this.settings = settings;
      this.loaded = true;
    } catch {
      // Backend may not be ready yet (built in parallel) — leave empty.
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
    if (this.notices.some((n) => n.id === notice.id)) return;
    if (this.isChannelSessionNotice(notice)) return;
    this.notices = [notice, ...this.notices];
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
        ws.openSession(action.session_id);
        router.go('agents');
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
