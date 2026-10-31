import { isTauri } from '../../lib/desktop';
import { router } from '../../lib/router.svelte';
import { recallRoom } from './room-access';
import type { RoomCredential } from '../../lib/api/room-types';

/** Local host credentials stay in memory and cross only the native IPC boundary
 *  (desktop) or a same-origin `postMessage` to a tab THIS tab opened (browser).
 *  Never a URL, never storage. The source document never connects a second
 *  room socket. */
export async function openHostedRoom(roomId: string): Promise<void> {
  const context = recallRoom(roomId);
  if (!context) throw new Error('Open this room from the window where you started it.');
  if (!isTauri) { openRoomTab(roomId); return; }
  const { invoke } = await import('@tauri-apps/api/core');
  await invoke<string>('open_host_room_window', context);
}

/** Browser: the room gets its own tab, so this one stays free for other
 *  sessions (navigating away no longer has to leave the room). */
const REQUEST = 'otto-room-context-request';
const REPLY = 'otto-room-context';
const opened = new Map<string, Window>();
let listening = false;

function openRoomTab(roomId: string): void {
  const existing = opened.get(roomId);
  if (existing && !existing.closed) { existing.focus(); return; }
  listen();
  const tab = window.open(`${location.pathname}#/room-host/${encodeURIComponent(roomId)}`, `otto-room-${roomId}`);
  // Pop-ups blocked: keep the old in-tab room view rather than fail.
  if (!tab) { router.go(`rooms/${roomId}`); return; }
  opened.set(roomId, tab);
}

function listen(): void {
  if (listening) return;
  listening = true;
  window.addEventListener('message', (e: MessageEvent) => {
    const data = e.data as { type?: unknown; roomId?: unknown } | null;
    if (e.origin !== location.origin || data?.type !== REQUEST || typeof data.roomId !== 'string') return;
    // Answer only the tab we opened for exactly this room.
    const tab = opened.get(data.roomId);
    if (!tab || e.source !== tab) return;
    const context = recallRoom(data.roomId);
    if (context) tab.postMessage({ type: REPLY, roomId: data.roomId, context }, location.origin);
  });
}

/** In a room tab: ask the tab that opened it for this room's credential. */
export function requestRoomContext(roomId: string, timeoutMs = 5000): Promise<{ origin: string; credential: RoomCredential }> {
  return new Promise((resolve, reject) => {
    const parent = window.opener as Window | null;
    if (!parent) { reject(new Error('Open this room from Otto’s Rooms page.')); return; }
    const done = (fn: () => void) => { clearTimeout(timer); window.removeEventListener('message', onMessage); fn(); };
    const onMessage = (e: MessageEvent) => {
      const data = e.data as { type?: unknown; roomId?: unknown; context?: { origin: string; credential: RoomCredential } } | null;
      if (e.origin !== location.origin || e.source !== parent || data?.type !== REPLY || data.roomId !== roomId || !data.context) return;
      done(() => resolve(data.context!));
    };
    const timer = setTimeout(() => done(() => reject(new Error('The Otto tab that started this room didn’t answer. Reopen the room from its Rooms page.'))), timeoutMs);
    window.addEventListener('message', onMessage);
    parent.postMessage({ type: REQUEST, roomId }, location.origin);
  });
}
