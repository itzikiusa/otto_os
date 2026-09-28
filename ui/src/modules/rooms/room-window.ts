import { isTauri } from '../../lib/desktop';
import { router } from '../../lib/router.svelte';
import { recallRoom } from './room-access';

/** Local host credentials stay in memory and cross only the native IPC boundary.
 * The source document never navigates or connects a second room socket. */
export async function openHostedRoom(roomId: string): Promise<void> {
  const context = recallRoom(roomId);
  if (!context) throw new Error('Open this room from the window where you started it.');
  if (!isTauri) { router.go(`rooms/${roomId}`); return; }
  const { invoke } = await import('@tauri-apps/api/core');
  await invoke<string>('open_host_room_window', context);
}
