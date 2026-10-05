// Streaming API-client transport store. Manages one live connection (SSE or
// WebSocket) bridged through the daemon's `/ws/api-client/stream` relay, and
// accumulates the events/messages for the response console.

import { baseUrl, getToken, WS_BEARER_SUBPROTOCOL } from '../api/client';
import type { ExecuteApiReq } from '../api/types';
import { confirmNewHost } from './apiClient.svelte';

export type StreamStatus = 'idle' | 'connecting' | 'open' | 'closed' | 'error';
export type StreamItemKind = 'open' | 'event' | 'message' | 'error' | 'closed';

export interface StreamItem {
  t: number;
  kind: StreamItemKind;
  /** SSE event name (for kind==='event'). */
  event?: string;
  /** WebSocket direction (for kind==='message'). */
  dir?: 'in' | 'out';
  data: string;
  id?: string;
  binary?: boolean;
}

interface DaemonMsg {
  type: string;
  detail?: string;
  event?: string;
  data?: string;
  id?: string;
  dir?: 'in' | 'out';
  binary?: boolean;
  message?: string;
}

class ApiStreamStore {
  status: StreamStatus = $state('idle');
  /** The console's messages, oldest first — a snapshot of the ring, replaced
   *  (never mutated) at most once per animation frame. */
  items: StreamItem[] = $state.raw([]);
  error = $state('');
  dropped = $state(0);
  workspaceId = $state('');
  mode: 'sse' | 'websocket' = $state('sse');

  private ws: WebSocket | null = null;
  // Plain ring (not reactive): live entries are ring[head..]. A 1,000/s stream
  // used to copy + re-sum the whole 1,000-entry array and flush reactivity per
  // message; now a message is O(1) and the view updates once per frame.
  private ring: StreamItem[] = [];
  private head = 0;
  private bytes = 0;
  private pendingDrops = 0;
  private flushQueued = false;
  /** The last `connect` call — replayed once with `confirm_new_host` when the
   *  daemon refuses to send a stored secret to an unbound host and the person
   *  confirms. */
  private last: { workspaceId: string; kind: 'sse' | 'websocket'; request: ExecuteApiReq } | null = null;

  get active(): boolean {
    return this.status === 'connecting' || this.status === 'open';
  }

  /** Open a streaming connection of the given kind. */
  connect(workspaceId: string, kind: 'sse' | 'websocket', request: ExecuteApiReq): void {
    this.disconnect();
    this.reset();
    this.workspaceId = workspaceId;
    this.error = '';
    this.mode = kind;
    this.status = 'connecting';
    const last = { workspaceId, kind, request };
    this.last = last;

    let wsUrl: string;
    try {
      const base = new URL(baseUrl());
      const proto = base.protocol === 'https:' ? 'wss:' : 'ws:';
      wsUrl = `${proto}//${base.host}/ws/api-client/stream?workspace_id=${encodeURIComponent(workspaceId)}`;
    } catch {
      this.fail('Invalid daemon base URL');
      return;
    }

    let sock: WebSocket;
    try {
      // The bearer rides in the otto-bearer subprotocol, never the URL (S11-11).
      const token = getToken();
      sock = token ? new WebSocket(wsUrl, [WS_BEARER_SUBPROTOCOL, token]) : new WebSocket(wsUrl);
    } catch {
      this.fail('Could not open relay socket');
      return;
    }
    this.ws = sock;

    sock.onopen = () => {
      if (this.ws !== sock) return;
      sock.send(
        JSON.stringify({
          action: 'open',
          kind,
          request,
        }),
      );
    };
    sock.onmessage = (e) => {
      if (this.ws !== sock) return;
      let msg: DaemonMsg;
      try {
        msg = JSON.parse(typeof e.data === 'string' ? e.data : '');
      } catch {
        return;
      }
      this.handle(msg);
    };
    sock.onerror = () => { if (this.ws === sock) this.fail('Relay connection error'); };
    sock.onclose = () => {
      if (this.ws !== sock) return;
      if (this.status !== 'error' && this.active) this.status = 'closed';
      this.ws = null;
    };
  }

  /** Send a message to the upstream (WebSocket only). */
  send(data: string): void {
    if (this.ws && this.status === 'open') {
      this.ws.send(JSON.stringify({ action: 'send', data }));
    }
  }

  disconnect(): void {
    this.last = null;
    if (this.ws) {
      const socket = this.ws;
      this.ws = null;
      try {
        socket.send(JSON.stringify({ action: 'close' }));
      } catch {
        /* ignore */
      }
      try {
        socket.close();
      } catch {
        /* ignore */
      }
      this.ws = null;
    }
    if (this.active) this.status = 'closed';
  }

  clear(): void {
    this.reset();
  }

  private reset(): void {
    this.ring = [];
    this.head = 0;
    this.bytes = 0;
    this.pendingDrops = 0;
    this.items = [];
    this.dropped = 0;
  }

  private fail(msg: string): void {
    this.status = 'error';
    this.error = msg;
    this.push({ kind: 'error', data: msg });
  }

  private async offerNewHostConfirm(
    last: { workspaceId: string; kind: 'sse' | 'websocket'; request: ExecuteApiReq },
    message: string,
  ): Promise<void> {
    const host = /host '([^']*)'/.exec(message)?.[1] ?? '';
    if (!(await confirmNewHost(host))) return;
    // The person started (or stopped) another stream meanwhile — don't revive this one.
    if (this.last !== last) return;
    this.connect(last.workspaceId, last.kind, { ...last.request, confirm_new_host: true });
  }

  private push(item: Omit<StreamItem, 't'>): void {
    const entry: StreamItem = { t: Date.now(), ...item, data: item.data.slice(0, 64 * 1024) };
    this.ring.push(entry);
    this.bytes += entry.data.length;
    while (this.ring.length - this.head > 1 && (this.ring.length - this.head > 1000 || this.bytes > 4 * 1024 * 1024)) {
      this.bytes -= this.ring[this.head].data.length;
      this.head++;
      this.pendingDrops++;
    }
    // Compact once the dead prefix dominates (amortised O(1) per message).
    if (this.head > 1024 && this.head * 2 > this.ring.length) {
      this.ring = this.ring.slice(this.head);
      this.head = 0;
    }
    this.scheduleFlush();
  }

  private scheduleFlush(): void {
    if (typeof requestAnimationFrame !== 'function') {
      this.flush();
      return;
    }
    if (this.flushQueued) return;
    this.flushQueued = true;
    requestAnimationFrame(() => {
      this.flushQueued = false;
      this.flush();
    });
  }

  private flush(): void {
    this.items = this.ring.slice(this.head);
    if (this.pendingDrops) {
      this.dropped += this.pendingDrops;
      this.pendingDrops = 0;
    }
  }

  private handle(msg: DaemonMsg): void {
    switch (msg.type) {
      case 'open':
        this.status = 'open';
        this.push({ kind: 'open', data: msg.detail ?? 'connected' });
        break;
      case 'event':
        this.push({ kind: 'event', event: msg.event, data: msg.data ?? '', id: msg.id });
        break;
      case 'message':
        this.push({ kind: 'message', dir: msg.dir, data: msg.data ?? '', binary: msg.binary });
        break;
      case 'error':
        this.error = msg.message ?? 'error';
        this.status = 'error';
        this.push({ kind: 'error', data: msg.message ?? 'error' });
        if (this.error.includes('needs_confirm=new_host') && this.last && !this.last.request.confirm_new_host) {
          void this.offerNewHostConfirm(this.last, this.error);
        }
        break;
      case 'closed':
        if (this.status !== 'error') this.status = 'closed';
        this.push({ kind: 'closed', data: msg.detail ?? 'closed' });
        break;
    }
  }
}

export const apiStream = new ApiStreamStore();
