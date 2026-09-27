// A lean, topic-filtered `/ws/events` socket for the hidden always-alive
// windows (the menu-bar tray). The main window's EventsClient
// (events.svelte.ts) drags every event-fed store into its bundle and
// registers the document for agent UI control; the tray needs neither — it
// needs a handful of event types to keep its glyph live. It sends
// `{"type":"subscribe","topics":[…]}` (ws.md) so the daemon drops every other
// event before authorizing/serializing it, and feeds a `LiveRegistry` that
// `liveQuery` consumes. Reconnects with backoff; a reconnect (or a changed
// daemon `boot_id`, or a lag `resync` frame) resyncs the registry.

import { wsConnect } from './api/client';
import { LiveRegistry, type LiveEvent } from './live';

export class TopicSocket {
  readonly registry = new LiveRegistry();
  private sock: WebSocket | null = null;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private backoff = 1000;
  private stopped = true;
  private everConnected = false;
  private bootId: string | null = null;

  constructor(private readonly topics: readonly string[]) {}

  start(): void {
    if (!this.stopped) return;
    this.stopped = false;
    this.connect();
  }

  stop(): void {
    this.stopped = true;
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
    const s = this.sock;
    this.sock = null;
    if (s) {
      s.onclose = null;
      s.close();
    }
    this.registry.setConnected(false);
  }

  private connect(): void {
    if (this.stopped) return;
    let s: WebSocket;
    try {
      s = wsConnect('/ws/events');
    } catch {
      this.scheduleReconnect();
      return;
    }
    this.sock = s;
    s.onopen = () => {
      this.backoff = 1000;
      try {
        s.send(JSON.stringify({ type: 'subscribe', topics: [...this.topics] }));
      } catch {
        /* closing */
      }
      const reconnected = this.everConnected;
      this.everConnected = true;
      this.registry.setConnected(true);
      if (reconnected) this.registry.resync();
    };
    s.onmessage = (ev: MessageEvent) => {
      if (typeof ev.data !== 'string') return;
      let data: LiveEvent;
      try {
        data = JSON.parse(ev.data) as LiveEvent;
      } catch {
        return;
      }
      if (data?.type === 'resync') {
        this.registry.resync();
      } else if (data?.type === 'subscribe_ack') {
        const boot = typeof data.boot_id === 'string' ? data.boot_id : null;
        // Same socket generation, different daemon: everything cached is stale.
        if (boot && this.bootId && boot !== this.bootId) this.registry.resync();
        if (boot) this.bootId = boot;
      } else if (typeof data?.type === 'string') {
        this.registry.dispatch(data);
      }
    };
    s.onclose = () => {
      if (this.sock === s) this.sock = null;
      this.registry.setConnected(false);
      this.scheduleReconnect();
    };
    s.onerror = () => s.close();
  }

  private scheduleReconnect(): void {
    if (this.stopped) return;
    if (this.timer) clearTimeout(this.timer);
    this.timer = setTimeout(() => {
      this.timer = null;
      this.connect();
    }, this.backoff);
    this.backoff = Math.min(this.backoff * 2, 30_000);
  }
}
