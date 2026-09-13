// Streaming API-client transport store. Manages one live connection (SSE or
// WebSocket) bridged through the daemon's `/ws/api-client/stream` relay, and
// accumulates the events/messages for the response console.

import { baseUrl, getToken } from '../api/client';
import type { ExecuteApiReq } from '../api/types';

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
  items: StreamItem[] = $state([]);
  error = $state('');
  dropped = $state(0);
  workspaceId = $state('');
  mode: 'sse' | 'websocket' = $state('sse');

  private ws: WebSocket | null = null;

  get active(): boolean {
    return this.status === 'connecting' || this.status === 'open';
  }

  /** Open a streaming connection of the given kind. */
  connect(workspaceId: string, kind: 'sse' | 'websocket', request: ExecuteApiReq): void {
    this.disconnect();
    this.items = [];
    this.dropped = 0;
    this.workspaceId = workspaceId;
    this.error = '';
    this.mode = kind;
    this.status = 'connecting';

    let wsUrl: string;
    try {
      const base = new URL(baseUrl());
      const proto = base.protocol === 'https:' ? 'wss:' : 'ws:';
      const token = getToken() ?? '';
      wsUrl = `${proto}//${base.host}/ws/api-client/stream?token=${encodeURIComponent(token)}&workspace_id=${encodeURIComponent(workspaceId)}`;
    } catch {
      this.fail('Invalid daemon base URL');
      return;
    }

    let sock: WebSocket;
    try {
      sock = new WebSocket(wsUrl);
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
    this.items = [];
    this.dropped = 0;
  }

  private fail(msg: string): void {
    this.status = 'error';
    this.error = msg;
    this.push({ kind: 'error', data: msg });
  }

  private push(item: Omit<StreamItem, 't'>): void {
    const next = [...this.items, { t: Date.now(), ...item, data: item.data.slice(0, 64 * 1024) }];
    let bytes = next.reduce((n, entry) => n + entry.data.length, 0);
    while (next.length > 1000 || bytes > 4 * 1024 * 1024) {
      bytes -= next.shift()!.data.length; this.dropped++;
    }
    this.items = next;
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
        break;
      case 'closed':
        if (this.status !== 'error') this.status = 'closed';
        this.push({ kind: 'closed', data: msg.detail ?? 'closed' });
        break;
    }
  }
}

export const apiStream = new ApiStreamStore();
