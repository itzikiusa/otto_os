// Events WS client (/ws/events) with auto-reconnect + exponential backoff.
// Feeds the workspace store (session statuses) and the toast store (notices).

import { wsUrl } from './api/client';
import type { OttoEvent } from './api/types';
import { ws } from './stores/workspace.svelte';
import { notifications } from './stores/notifications.svelte';
import { activity } from './stores/activity.svelte';

export type EventsState = 'connecting' | 'connected' | 'offline';

class EventsClient {
  state: EventsState = $state('offline');

  private sock: WebSocket | null = null;
  private backoff = 1000;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private stopped = false;

  start(): void {
    this.stopped = false;
    this.connect();
  }

  stop(): void {
    this.stopped = true;
    if (this.timer) clearTimeout(this.timer);
    this.sock?.close();
    this.sock = null;
    this.state = 'offline';
  }

  private connect(): void {
    if (this.stopped) return;
    this.state = 'connecting';
    try {
      this.sock = new WebSocket(wsUrl('/ws/events'));
    } catch {
      this.scheduleReconnect();
      return;
    }
    this.sock.onopen = () => {
      this.state = 'connected';
      this.backoff = 1000;
    };
    this.sock.onmessage = (ev: MessageEvent) => {
      if (typeof ev.data !== 'string') return;
      try {
        const parsed = JSON.parse(ev.data) as OttoEvent;
        if (parsed.type === 'notification') {
          notifications.ingest(parsed.notice);
        } else if (parsed.type === 'trail_appended' || parsed.type === 'tasks_updated') {
          activity.applyEvent(parsed);
        } else {
          if (parsed.type === 'session_removed') activity.forget(parsed.session_id);
          ws.applyEvent(parsed);
        }
      } catch {
        /* malformed frame — ignore */
      }
    };
    this.sock.onclose = () => {
      this.state = 'offline';
      this.scheduleReconnect();
    };
    this.sock.onerror = () => {
      this.sock?.close();
    };
  }

  private scheduleReconnect(): void {
    if (this.stopped) return;
    if (this.timer) clearTimeout(this.timer);
    this.timer = setTimeout(() => this.connect(), this.backoff);
    this.backoff = Math.min(this.backoff * 2, 30_000);
  }
}

export const events = new EventsClient();
