/**
 * The capability WebSocket both in-memory relays use: session rooms
 * (`modules/rooms/room-client.ts`) and room games
 * (`modules/rooms/games/client.ts`). The daemon side shares one primitive set
 * too (`otto-rooms/src/capability.rs`).
 *
 * One transport: the token rides only in the `Sec-WebSocket-Protocol` pair
 * (`<protocol>, <token>`), a silence deadline closes a socket that stops
 * talking (a stalled handshake included), a heartbeat keeps a live one warm,
 * and a bounded backoff reconnects. When the attempts run out the status is
 * `gave-up` (not a forever "Reconnecting…"); `reconnect()` resets the budget,
 * and so does the network coming back (`online`) or the page becoming visible.
 *
 * No owner API imports, storage or native privileges: a remote guest runs this.
 */
export type RelayStatus = 'connecting' | 'connected' | 'disconnected' | 'gave-up' | 'ended';

export interface RelaySocketOptions<E> {
  url: string;
  /** Subprotocol name sent before the token (`otto-room`, `otto-game`). */
  protocol: string;
  token: string;
  /** Frames longer than this are dropped unread. */
  maxBytes: number;
  /** Close (and reconnect) when no valid frame arrived for this long. */
  silenceMs: number;
  /** Heartbeat cadence once open, and the frame it sends. */
  heartbeatMs: number;
  heartbeat: string;
  /** Automatic reconnects before `gave-up`. */
  maxAttempts: number;
  /** Delay before reconnect number `attempt` (0-based). */
  backoff: (attempt: number) => number;
  /** Parse and validate a text frame; null drops it (it does not count as liveness). */
  parse: (data: string) => E | null;
  /** The frame that proves membership: status `connected`, attempts reset. */
  connectedBy: (event: E) => boolean;
  /** The frame that ends the relay for good: status `ended`, no reconnects. */
  endedBy: (event: E) => boolean;
  onEvent: (event: E) => void;
  onStatus: (status: RelayStatus) => void;
  /** Refuse sends while this many bytes are still queued (0 = no cap). */
  maxBuffered?: number;
}

export class RelaySocket<E> {
  private socket: WebSocket | null = null;
  private heartbeatTimer: ReturnType<typeof setInterval> | undefined;
  private retry: ReturnType<typeof setTimeout> | undefined;
  private silence: ReturnType<typeof setTimeout> | undefined;
  private attempts = 0;
  private disposed = false;
  private status: RelayStatus = 'connecting';
  private readonly opts: RelaySocketOptions<E>;
  private readonly wake = () => {
    if (typeof document !== 'undefined' && document.visibilityState === 'hidden') return;
    if (this.status === 'disconnected' || this.status === 'gave-up') this.reconnect();
  };

  constructor(opts: RelaySocketOptions<E>) {
    this.opts = opts;
    if (typeof window !== 'undefined') window.addEventListener('online', this.wake);
    if (typeof document !== 'undefined') document.addEventListener('visibilitychange', this.wake);
  }

  get ended(): boolean { return this.disposed; }

  connect(): void {
    if (this.disposed) return;
    clearTimeout(this.retry);
    this.detach();
    this.emit('connecting');
    const socket = new WebSocket(this.opts.url, [this.opts.protocol, this.opts.token]);
    this.socket = socket;
    // A failed upgrade may never reach onopen; bound that wait as well.
    this.armSilence(socket);
    socket.onopen = () => {
      this.armSilence(socket);
      this.heartbeatTimer = setInterval(() => {
        if (this.socket === socket && socket.readyState === WebSocket.OPEN) socket.send(this.opts.heartbeat);
      }, this.opts.heartbeatMs);
    };
    socket.onmessage = (message) => {
      if (typeof message.data !== 'string' || message.data.length > this.opts.maxBytes) return;
      const event = this.opts.parse(message.data);
      if (event === null) return;
      this.armSilence(socket);
      if (this.opts.connectedBy(event)) { this.attempts = 0; this.emit('connected'); }
      if (this.opts.endedBy(event)) { this.disposed = true; this.unlisten(); this.detach(); this.emit('ended'); }
      this.opts.onEvent(event);
    };
    socket.onerror = () => this.emit('disconnected');
    socket.onclose = () => {
      clearInterval(this.heartbeatTimer);
      clearTimeout(this.silence);
      if (this.disposed) return;
      if (this.attempts < this.opts.maxAttempts) {
        this.emit('disconnected');
        this.retry = setTimeout(() => this.connect(), this.opts.backoff(this.attempts++));
      } else {
        this.emit('gave-up');
      }
    };
  }

  /** An explicit retry (the "Reconnect" button, the network coming back): a fresh attempt budget. */
  reconnect(): void {
    if (this.disposed) return;
    this.attempts = 0;
    this.connect();
  }

  /** False when closed, disposed or (with `maxBuffered`) backed up; the frame is dropped. */
  send(frame: string): boolean {
    const socket = this.socket;
    if (this.disposed || socket?.readyState !== WebSocket.OPEN) return false;
    if (this.opts.maxBuffered && socket.bufferedAmount > this.opts.maxBuffered) return false;
    socket.send(frame);
    return true;
  }

  dispose(): void {
    this.disposed = true;
    clearTimeout(this.retry);
    this.unlisten();
    this.detach();
  }

  private emit(status: RelayStatus): void {
    this.status = status;
    this.opts.onStatus(status);
  }

  private armSilence(socket: WebSocket): void {
    clearTimeout(this.silence);
    this.silence = setTimeout(() => {
      if (socket !== this.socket || this.disposed) return;
      this.emit('disconnected');
      socket.close();
    }, this.opts.silenceMs);
  }

  private detach(): void {
    clearInterval(this.heartbeatTimer);
    clearTimeout(this.silence);
    const socket = this.socket;
    if (!socket) return;
    socket.onclose = null; socket.onmessage = null; socket.onerror = null; socket.onopen = null;
    socket.close();
    this.socket = null;
  }

  private unlisten(): void {
    if (typeof window !== 'undefined') window.removeEventListener('online', this.wake);
    if (typeof document !== 'undefined') document.removeEventListener('visibilitychange', this.wake);
  }
}
