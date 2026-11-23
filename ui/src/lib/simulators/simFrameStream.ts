// Shared WebSocket frame stream client for iOS Simulators & Android Emulators
// (`WS /ws/simulators/{id}/stream`). Handles generation/close guarding against
// stale sockets, JSON control message dispatch, binary frame decoding, and
// frame-ack flow control for both Browser's SimulatorView and IDE's Devices panel.

import { decodeSimFrame, simulatorsApi } from '../api/simulators';
import type {
  SimDevice,
  SimFrameHeader,
  SimStreamClientMsg,
  SimStreamServerMsg,
} from '../api/types';

export interface SimFrameStreamOptions {
  fps?: number;
  format?: 'jpeg' | 'png';
  /** Automatically send `{ type: 'ack', seq }` right after `onFrame` returns.
   *  Set `false` when decoding asynchronously (e.g. `createImageBitmap`) and
   *  call `handle.ack(seq)` once the frame finishes decoding. Default: `true`. */
  autoAck?: boolean;
}

export interface SimFrameStreamCallbacks {
  onOpen?: () => void;
  onHello?: (device: SimDevice, canDrive: boolean) => void;
  onState?: (device: SimDevice) => void;
  onFrame: (blob: Blob, header: SimFrameHeader) => void;
  onError?: (message: string) => void;
  onClosed?: (reason: string) => void;
  onClose?: () => void;
}

export interface SimFrameStreamHandle {
  readonly open: boolean;
  send: (msg: SimStreamClientMsg) => boolean;
  ack: (seq: number) => void;
  close: () => void;
}

let nextStreamGen = 0;

export function openSimFrameStream(
  deviceId: string,
  opts: SimFrameStreamOptions,
  callbacks: SimFrameStreamCallbacks,
): SimFrameStreamHandle {
  const gen = ++nextStreamGen;
  let closed = false;
  const autoAck = opts.autoAck ?? true;
  const sock = simulatorsApi.openStreamSocket(deviceId, {
    fps: opts.fps,
    format: opts.format,
  });

  const isActive = () => !closed && gen > 0;

  function send(msg: SimStreamClientMsg): boolean {
    if (!isActive() || sock.readyState !== WebSocket.OPEN) return false;
    try {
      sock.send(JSON.stringify(msg));
      return true;
    } catch {
      return false;
    }
  }

  function ack(seq: number): void {
    send({ type: 'ack', seq });
  }

  function close(): void {
    if (closed) return;
    closed = true;
    sock.onopen = null;
    sock.onmessage = null;
    sock.onerror = null;
    sock.onclose = null;
    try {
      sock.close(1000);
    } catch {
      /* socket may already be closing or closed */
    }
  }

  sock.onopen = () => {
    if (!isActive()) return;
    callbacks.onOpen?.();
  };

  sock.onmessage = (ev) => {
    if (!isActive()) return;
    if (typeof ev.data === 'string') {
      try {
        const msg = JSON.parse(ev.data) as SimStreamServerMsg;
        if (msg.type === 'hello') {
          callbacks.onHello?.(msg.device, msg.can_drive);
        } else if (msg.type === 'state') {
          callbacks.onState?.(msg.device);
        } else if (msg.type === 'error') {
          callbacks.onError?.(msg.message);
        } else if (msg.type === 'closed') {
          callbacks.onClosed?.(msg.reason);
        }
      } catch {
        /* ignore malformed control frame */
      }
      return;
    }
    if (ev.data instanceof ArrayBuffer) {
      const decoded = decodeSimFrame(ev.data);
      if (!decoded) return;
      const copy = new Uint8Array(decoded.bytes.byteLength);
      copy.set(decoded.bytes);
      const blob = new Blob([copy], { type: decoded.header.mime || 'image/jpeg' });
      callbacks.onFrame(blob, decoded.header);
      if (autoAck) ack(decoded.header.seq);
    }
  };

  sock.onerror = () => {
    if (!isActive()) return;
    callbacks.onError?.('Stream connection failed.');
  };

  sock.onclose = () => {
    if (!isActive()) return;
    closed = true;
    callbacks.onClose?.();
  };

  return {
    get open(): boolean {
      return !closed && sock.readyState === WebSocket.OPEN;
    },
    send,
    ack,
    close,
  };
}
