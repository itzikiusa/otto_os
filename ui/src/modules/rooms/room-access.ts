import type { RoomCredential } from '../../lib/api/room-types';
/** Window-memory only. Never persist capabilities or send them to owner APIs. */
const invitations = new Map<string, string>();
const credentials = new Map<string, { origin: string; credential: RoomCredential }>();
export function captureRoomInvite(roomId: string, invite: string): void { invitations.set(roomId, invite); }
export function roomInvite(roomId: string): string | undefined { return invitations.get(roomId); }
export function rememberRoom(origin: string, credential: RoomCredential): void {
  credentials.set(credential.room_id, { origin, credential });
  invitations.delete(credential.room_id);
}
export function recallRoom(roomId: string) { return credentials.get(roomId); }
export function forgetRoom(roomId: string): void { credentials.delete(roomId); invitations.delete(roomId); }
