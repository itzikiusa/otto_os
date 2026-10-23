// Room and game invitation capabilities captured from the URL (`#/room/<id>?invite=…`,
// `#/game/<id>/<invite>`). They live in window memory only — never persisted,
// never sent to owner APIs — and are read back by the rooms module when the
// guest joins. Kept in `lib/` so the router captures them without importing
// `modules/rooms` (architecture-1-13).

const gameInvites = new Map<string, string>();
const roomInvites = new Map<string, string>();

export function captureGameInvite(id: string, invite: string): void { gameInvites.set(id, invite); }
/** The captured game invite for `id`, removed as it is read (single use). */
export function consumeGameInvite(id: string): string | undefined {
  const value = gameInvites.get(id);
  gameInvites.delete(id);
  return value;
}

export function captureRoomInvite(roomId: string, invite: string): void { roomInvites.set(roomId, invite); }
export function roomInvite(roomId: string): string | undefined { return roomInvites.get(roomId); }
export function dropRoomInvite(roomId: string): void { roomInvites.delete(roomId); }
